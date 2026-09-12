//! `FileReading`: what `/run/nopass/<uid>.state` says — and nothing more.
//!
//! There is deliberately NO `Default`, NO `is_active()`, NO
//! `unwrap_or(Inactive)`, and NO `From<FileReading> for TrayState`.
//! Absence, an I/O error, malformed bytes, and `schema != 1` are all arms
//! that carry no activity claim at all: the type has no way to express
//! "inactive because the file was missing". The ONLY route from
//! `Absent`/`Faulted` to a non-`Unknown` `TrayState` is `reconcile::merge`
//! (Phase 3), whose signature demands a probe result — see design.md §3.1,
//! Architecture Decision D3.

use std::io::ErrorKind;
use std::path::Path;

use nopass_core::state::{HelperStatus, SCHEMA_VERSION};

/// The only schema version this reader accepts. Aliases
/// `nopass_core::state::SCHEMA_VERSION` rather than a literal `1`, so a
/// future bump to the shared wire type cannot silently desync the tray.
pub const SUPPORTED_SCHEMA: u32 = SCHEMA_VERSION;

/// Every way the state file failed to parse as a supported [`HelperStatus`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadFault {
    /// The file exists but could not be read (any I/O error other than
    /// "not found").
    Io,
    /// The bytes are not valid UTF-8, not valid JSON, or do not match the
    /// `HelperStatus` shape once the schema check has passed.
    Malformed,
    /// The file parses far enough to read `schema`, and it is not
    /// [`SUPPORTED_SCHEMA`]. Reported distinctly from `Malformed` so a
    /// future helper's file is never diagnosed as corruption.
    UnsupportedSchema { found: u32 },
}

/// What the state FILE says — and nothing more.
///
/// There is deliberately NO `Default`, NO `is_active()`, NO
/// `unwrap_or(Inactive)`, and NO `From<FileReading> for TrayState`.
/// Absence, an I/O error, malformed bytes, and `schema != 1` are all arms
/// that carry no activity claim at all: the type has no way to express
/// "inactive because the file was missing". The ONLY route from
/// `Absent`/`Faulted` to a non-`Unknown` `TrayState` is `merge`, whose
/// signature demands a probe result (design.md §3.1, D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileReading {
    Parsed(HelperStatus),
    Absent,
    Faulted(ReadFault),
}

impl FileReading {
    /// Returns the parsed status only when the file was fully readable and
    /// matched the supported schema. Every other variant yields `None` —
    /// there is no shortcut that turns absence or a fault into a status.
    pub fn parsed(&self) -> Option<&HelperStatus> {
        match self {
            FileReading::Parsed(status) => Some(status),
            FileReading::Absent | FileReading::Faulted(_) => None,
        }
    }
}

/// Parses raw state-file bytes into a [`HelperStatus`].
///
/// Two-stage, deliberately: the `schema` field is extracted and validated
/// against [`SUPPORTED_SCHEMA`] BEFORE the bytes are deserialized as a full
/// `HelperStatus`. A future schema-2 file therefore always reports
/// [`ReadFault::UnsupportedSchema`], never [`ReadFault::Malformed`], even if
/// the rest of its shape would not match today's `HelperStatus` — a version
/// mismatch must never be misdiagnosed as corruption (design.md §3.1).
pub fn parse(bytes: &[u8]) -> Result<HelperStatus, ReadFault> {
    let text = std::str::from_utf8(bytes).map_err(|_| ReadFault::Malformed)?;
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| ReadFault::Malformed)?;

    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .and_then(|s| u32::try_from(s).ok())
        .ok_or(ReadFault::Malformed)?;

    if schema != SUPPORTED_SCHEMA {
        return Err(ReadFault::UnsupportedSchema { found: schema });
    }

    serde_json::from_value(value).map_err(|_| ReadFault::Malformed)
}

/// Reads and parses the state file at `path`.
///
/// `ENOENT` maps to [`FileReading::Absent`] — never to a `Faulted` variant,
/// and never to anything that carries an activity claim. Every other I/O
/// error maps to `Faulted(ReadFault::Io)`.
pub fn read(path: &Path) -> FileReading {
    match std::fs::read(path) {
        Ok(bytes) => match parse(&bytes) {
            Ok(status) => FileReading::Parsed(status),
            Err(fault) => FileReading::Faulted(fault),
        },
        Err(err) if err.kind() == ErrorKind::NotFound => FileReading::Absent,
        Err(_) => FileReading::Faulted(ReadFault::Io),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nopass_core::expiry::Expiry;

    fn status(user: &str, active: bool) -> HelperStatus {
        HelperStatus {
            schema: 1,
            uid: 1000,
            user: user.to_string(),
            active,
            expires: None,
            rule_path: "/etc/sudoers.d/90-nopass-1000".to_string(),
            updated_at: 1,
        }
    }

    #[test]
    fn parsed_returns_the_status_only_for_the_parsed_variant() {
        let s = status("jorge", true);
        let reading = FileReading::Parsed(s.clone());
        assert_eq!(reading.parsed(), Some(&s));
    }

    #[test]
    fn parsed_returns_none_for_absent_and_every_faulted_variant() {
        assert_eq!(FileReading::Absent.parsed(), None);
        assert_eq!(FileReading::Faulted(ReadFault::Io).parsed(), None);
        assert_eq!(FileReading::Faulted(ReadFault::Malformed).parsed(), None);
        assert_eq!(
            FileReading::Faulted(ReadFault::UnsupportedSchema { found: 2 }).parsed(),
            None
        );
    }

    #[test]
    fn parse_accepts_a_valid_schema_1_file_and_returns_the_exact_status() {
        let bytes = br#"{"schema":1,"uid":1000,"user":"jorge","active":true,"expires":{"kind":"at","epoch":1789000000},"rule_path":"/etc/sudoers.d/90-nopass-1000","updated_at":1789000000}"#;
        let parsed = parse(bytes).expect("valid schema-1 file must parse");
        assert_eq!(
            parsed,
            HelperStatus {
                schema: 1,
                uid: 1000,
                user: "jorge".to_string(),
                active: true,
                expires: Some(Expiry::At { epoch: 1_789_000_000 }),
                rule_path: "/etc/sudoers.d/90-nopass-1000".to_string(),
                updated_at: 1_789_000_000,
            }
        );
    }

    #[test]
    fn parse_rejects_schema_zero_as_unsupported_not_malformed() {
        let bytes = br#"{"schema":0,"uid":1000,"user":"jorge","active":false,"expires":null,"rule_path":"/etc/sudoers.d/90-nopass-1000","updated_at":1}"#;
        assert_eq!(parse(bytes), Err(ReadFault::UnsupportedSchema { found: 0 }));
    }

    #[test]
    fn parse_rejects_schema_two_as_unsupported_even_when_the_rest_of_the_shape_would_not_match() {
        // Deliberately a shape today's HelperStatus would never accept —
        // the point is that a version mismatch is caught BEFORE the full
        // struct is deserialized, so it is never misdiagnosed as
        // `Malformed`.
        let bytes = br#"{"schema":2,"totally":"different","shape":true}"#;
        assert_eq!(parse(bytes), Err(ReadFault::UnsupportedSchema { found: 2 }));
    }

    #[test]
    fn parse_rejects_truncated_json_as_malformed() {
        let bytes = br#"{"schema":1,"uid":1000,"user":"jorge","#;
        assert_eq!(parse(bytes), Err(ReadFault::Malformed));
    }

    #[test]
    fn parse_rejects_an_empty_file_as_malformed() {
        assert_eq!(parse(b""), Err(ReadFault::Malformed));
    }

    #[test]
    fn parse_rejects_non_utf8_bytes_as_malformed() {
        let bytes: &[u8] = &[0x7B, 0xFF, 0xFE, 0x00, 0x7D];
        assert_eq!(parse(bytes), Err(ReadFault::Malformed));
    }

    #[test]
    fn parse_accepts_active_true_with_a_null_expiry_without_adding_semantic_validation() {
        // `parse` performs no semantic validation beyond the schema check —
        // deciding whether this combination is trustworthy is
        // `reconcile::merge`'s job (Phase 3, design.md §3.2 row 5), never
        // this reader's.
        let bytes = br#"{"schema":1,"uid":1000,"user":"jorge","active":true,"expires":null,"rule_path":"/etc/sudoers.d/90-nopass-1000","updated_at":42}"#;
        let parsed = parse(bytes).expect("a well-formed schema-1 file always parses");
        assert!(parsed.active);
        assert_eq!(parsed.expires, None);
    }

    /// Design threat matrix row 1, "State misrepresentation" — restricted to
    /// what `FileReading` alone can observe. The probe-dependent adversarial
    /// cases from that same row (spawn failure, a probe older than the
    /// file, a rule deleted out of band) belong to Phase 3's `reconcile`
    /// module; every case here is provable from `parse`/`read` alone.
    ///
    /// No case below lets a caller derive "this uid currently has an
    /// active grant" without a probe: absence and every fault carry no
    /// `HelperStatus` at all, and the two cases that DO parse successfully
    /// still require `reconcile::merge` (Phase 3, design.md §3.2 rows 4-5)
    /// before they can become `TrayState::Active` — `parse`/`read` return a
    /// raw status, never a verdict.
    #[test]
    fn state_misrepresentation_cases_never_yield_an_activity_verdict_without_a_probe() {
        // Absent never carries a status to misread as active.
        assert_eq!(FileReading::Absent.parsed(), None);

        // schema != 1 never carries a status either — the reader refuses
        // to guess at an unsupported wire shape.
        let unsupported = parse(
            br#"{"schema":2,"uid":1000,"user":"jorge","active":true,"expires":null,"rule_path":"x","updated_at":1}"#,
        );
        assert_eq!(unsupported, Err(ReadFault::UnsupportedSchema { found: 2 }));

        // Truncated, empty, and non-UTF-8 bytes are all Faulted — no
        // status survives any of them.
        for bytes in [&b""[..], &br#"{"schema":1,"#[..], &[0xFF, 0xFE][..]] {
            assert_eq!(parse(bytes), Err(ReadFault::Malformed));
        }

        // `active:true` with an already-passed expiry, and `active:true`
        // with `expires:null`, both parse successfully — `FileReading`
        // reflects exactly what the file claims, with zero interpretation.
        // Neither becomes a `TrayState` here: that verdict is
        // `reconcile::merge`'s alone, and it demands a probe.
        let expired = parse(
            br#"{"schema":1,"uid":1000,"user":"jorge","active":true,"expires":{"kind":"at","epoch":1},"rule_path":"x","updated_at":2}"#,
        )
        .expect("a well-formed schema-1 file always parses, regardless of its own expiry");
        assert!(expired.active);
        assert_eq!(expired.expires, Some(Expiry::At { epoch: 1 }));

        let no_expiry = parse(
            br#"{"schema":1,"uid":1000,"user":"jorge","active":true,"expires":null,"rule_path":"x","updated_at":2}"#,
        )
        .expect("a well-formed schema-1 file always parses, regardless of its own expiry");
        assert!(no_expiry.active);
        assert_eq!(no_expiry.expires, None);
    }
}

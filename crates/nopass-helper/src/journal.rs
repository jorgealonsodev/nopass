//! journald audit trail (design.md §9; helper-observability §Journald
//! Audit Records).
//!
//! Two responsibilities: [`init`] configures the global `tracing`
//! subscriber once, at process start, and [`audit`] emits one structured
//! record per `enable`/`disable`/`expire` outcome.
//!
//! **Why the audit field names are NOT already prefixed `NOPASS_` at the
//! call site.** `tracing_journald::Layer::with_field_prefix` (configured
//! in [`init`] below, per design.md §9's literal code block) prepends
//! its configured prefix — `"NOPASS"` here — to every user-defined field
//! name automatically, once, at the journald-transport layer. If [`audit`]
//! *also* named its fields `NOPASS_EVENT`/`NOPASS_UID`/etc., a record
//! routed through the journald layer would come out double-prefixed
//! (`NOPASS_NOPASS_EVENT`). So [`audit`] emits the bare, unprefixed names
//! design.md §9 lists the *values* for (`EVENT`, `UID`, `USER`,
//! `OUTCOME`, `EXPIRES`, `EXIT`, `REASON`), and the journald layer's own
//! `with_field_prefix` is solely responsible for turning `EVENT` into
//! `NOPASS_EVENT` on that specific transport. The stderr fallback layer
//! has no equivalent prefixing mechanism and therefore prints the bare
//! names — acceptable, since design.md only requires the `NOPASS_`
//! prefix for the journald audit trail itself, not for the
//! no-journald-available fallback.

use nopass_core::expiry::Expiry;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// `tracing_journald`'s syslog identifier for every record this helper
/// emits (design.md §9, `SYSLOG_IDENTIFIER=nopass-helper`).
pub const SYSLOG_IDENTIFIER: &str = "nopass-helper";
/// `tracing_journald`'s field prefix — see the module doc comment for
/// why [`audit`]'s own field names must NOT also carry this prefix.
pub const FIELD_PREFIX: &str = "NOPASS";

/// Configures the global `tracing` subscriber: a `tracing-journald`
/// layer when the journald socket is reachable, otherwise a stderr `fmt`
/// layer (design.md §9 — "If the journald socket is unavailable the
/// helper falls back to the stderr layer and continues — logging never
/// fails an operation"). Uses `try_init` rather than the plain `init`
/// design.md's code block shows so a second call (a defensive re-init,
/// or multiple tests in the same process) is a harmless no-op instead of
/// a panic; the configuration is otherwise identical.
pub fn init() {
    match tracing_journald::layer() {
        Ok(layer) => {
            let _ = tracing_subscriber::registry()
                .with(
                    layer
                        .with_syslog_identifier(SYSLOG_IDENTIFIER.to_string())
                        .with_field_prefix(Some(FIELD_PREFIX.to_string())),
                )
                .try_init();
        }
        Err(_) => {
            let _ = tracing_subscriber::registry()
                .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
                .try_init();
        }
    }
}

/// Which transaction produced an audit record (`NOPASS_EVENT`). `Status`
/// is part of the documented value domain (design.md §9) but is not
/// currently emitted anywhere: `ops::status` (design.md §4.5) takes no
/// lock and performs no write, and Phase 8's wiring (tasks.md 8.3) scopes
/// `journal::audit` calls to `enable`/`disable`/`expire` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEvent {
    Enable,
    Disable,
    Expire,
    Status,
}

impl AuditEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditEvent::Enable => "enable",
            AuditEvent::Disable => "disable",
            AuditEvent::Expire => "expire",
            AuditEvent::Status => "status",
        }
    }
}

/// The outcome of one audited transaction (`NOPASS_OUTCOME`), per
/// design.md §9's exact documented value domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    Ok,
    Rejected,
    SkippedNotExpired,
    RolledBack,
    Error,
}

impl AuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditOutcome::Ok => "ok",
            AuditOutcome::Rejected => "rejected",
            AuditOutcome::SkippedNotExpired => "skipped_not_expired",
            AuditOutcome::RolledBack => "rolled_back",
            AuditOutcome::Error => "error",
        }
    }
}

/// Renders `NOPASS_EXPIRES`'s three-value domain (`never|reboot|<epoch>`,
/// design.md §9). There is no "not applicable" value in that domain, so
/// callers auditing an outcome with no meaningful expiry (e.g. an
/// `enable` that never reached the point of establishing one) pass
/// `Expiry::Never` as the closest honest sentinel.
fn expires_field(expires: Expiry) -> String {
    match expires {
        Expiry::Never => "never".to_string(),
        Expiry::Reboot => "reboot".to_string(),
        Expiry::At { epoch } => epoch.to_string(),
    }
}

/// One audit record's full field set. `reason` is a short stable token
/// (design.md §9: "never raw subprocess text") — `sudo`/`visudo` stderr
/// is captured into the log only via `tracing::error!`/`debug!` call
/// sites elsewhere, never through this struct.
pub struct AuditRecord<'a> {
    pub event: AuditEvent,
    pub uid: u32,
    pub user: &'a str,
    pub outcome: AuditOutcome,
    pub expires: Expiry,
    pub exit: i32,
    pub reason: &'a str,
}

/// Emits one journald/stderr record for `record` with exactly the seven
/// fields design.md §9 documents (before the journald layer's own
/// `NOPASS_` prefixing — see the module doc comment): `EVENT`, `UID`,
/// `USER`, `OUTCOME`, `EXPIRES`, `EXIT`, `REASON`. `MESSAGE` and
/// `SYSLOG_IDENTIFIER` are supplied by the `tracing`/`tracing-journald`
/// machinery itself, not by this function.
pub fn audit(record: &AuditRecord) {
    let expires = expires_field(record.expires);
    tracing::info!(
        EVENT = record.event.as_str(),
        UID = record.uid,
        USER = record.user,
        OUTCOME = record.outcome.as_str(),
        EXPIRES = expires.as_str(),
        EXIT = record.exit,
        REASON = record.reason,
        "nopass audit event"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedBuf {
        type Writer = SharedBuf;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn syslog_identifier_and_field_prefix_are_pinned_per_design_md_section_9() {
        assert_eq!(SYSLOG_IDENTIFIER, "nopass-helper");
        assert_eq!(FIELD_PREFIX, "NOPASS");
    }

    #[test]
    fn init_does_not_panic_in_a_sandboxed_environment_with_no_journald_socket() {
        // This test process has no journald socket, so `init` exercises
        // the `Err(_)` branch — the stderr fallback layer (design.md §9:
        // "logging never fails an operation"). `try_init` (not `init`)
        // makes this safe to call alongside any other test in the same
        // binary that also configures a global subscriber.
        init();
    }

    #[test]
    fn audit_emits_every_documented_field_name_and_value() {
        let buf = SharedBuf::default();
        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_writer(buf.clone()).with_ansi(false));

        tracing::subscriber::with_default(subscriber, || {
            audit(&AuditRecord {
                event: AuditEvent::Enable,
                uid: 1000,
                user: "jorge",
                outcome: AuditOutcome::Ok,
                expires: Expiry::At { epoch: 1_789_000_000 },
                exit: 0,
                reason: "",
            });
        });

        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        // `tracing_subscriber::fmt`'s default formatter quotes string
        // field values (`EVENT="enable"`) but not numeric ones
        // (`UID=1000`) — this is a formatting detail of the STDERR
        // fallback layer's text renderer; the actual journald transport
        // (`tracing_journald::Layer`) encodes fields with a fixed binary
        // framing, not this text format, and applies its own
        // `with_field_prefix`, per the module doc comment.
        for expected in [
            "EVENT=\"enable\"",
            "UID=1000",
            "USER=\"jorge\"",
            "OUTCOME=\"ok\"",
            "EXPIRES=\"1789000000\"",
            "EXIT=0",
        ] {
            assert!(text.contains(expected), "expected {expected:?} in captured audit output: {text}");
        }
    }

    #[test]
    fn audit_reports_the_never_and_reboot_expiry_sentinels_and_a_nonzero_exit() {
        let buf = SharedBuf::default();
        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_writer(buf.clone()).with_ansi(false));

        tracing::subscriber::with_default(subscriber, || {
            audit(&AuditRecord {
                event: AuditEvent::Disable,
                uid: 2000,
                user: "ana",
                outcome: AuditOutcome::Error,
                expires: Expiry::Never,
                exit: 16,
                reason: "fs_error",
            });
            audit(&AuditRecord {
                event: AuditEvent::Expire,
                uid: 3000,
                user: "sam",
                outcome: AuditOutcome::SkippedNotExpired,
                expires: Expiry::Reboot,
                exit: 0,
                reason: "",
            });
        });

        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("EVENT=\"disable\""));
        assert!(text.contains("OUTCOME=\"error\""));
        assert!(text.contains("EXPIRES=\"never\""));
        assert!(text.contains("EXIT=16"));
        assert!(text.contains("REASON=\"fs_error\""));
        assert!(text.contains("EVENT=\"expire\""));
        assert!(text.contains("OUTCOME=\"skipped_not_expired\""));
        assert!(text.contains("EXPIRES=\"reboot\""));
    }
}

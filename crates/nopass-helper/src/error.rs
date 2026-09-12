//! `HelperError`: the one typed error enum whose discriminant IS the exit
//! code (design.md §9, helper-cli §Typed Exit Code Mapping).
//!
//! `exit_code` is exhaustive over every non-zero code the helper can
//! return: 1 (internal/binary-missing), 10 (invocation context), 11 (uid
//! rejected), 12 (not a sudoer), 13 (bad duration), 14 (visudo rejected),
//! 15 (lock busy), 16 (filesystem failure), 17 (timer failed). Exit code 2
//! is owned entirely by clap and never reaches this mapping; exit 0 is
//! `Ok(())`, not a `HelperError` variant.
//!
//! `lock`, `fileops`, `timer`, and `ops` (Phases 6-7) are the modules that
//! actually construct most of these variants; `ops.rs` (Phase 7) is the
//! only caller wired into `main`'s `dispatch` so far. No
//! `#[allow(dead_code)]` is needed despite that: `HelperError` and
//! `exit_code` are `pub` inside this crate's `pub mod error` (declared in
//! `lib.rs`), which makes them public library API exempt from the
//! `dead_code` lint.

use crate::checks::UidRejection;

#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    #[error("internal error: {0}")]
    Internal(String),
    #[error("binary not found: {name}")]
    BinaryMissing { name: &'static str },
    #[error("invalid invocation context: {0}")]
    Context(&'static str),
    /// Target uid failed admission (`checks::admit_uid`). The rejection
    /// reason is carried through for the Phase 8 journald audit record
    /// and does not split the exit code — every cause maps to 11.
    #[error("target uid rejected")]
    UidRejected(UidRejection),
    #[error("target user is not a sudoer")]
    NotSudoer,
    #[error("invalid duration: {0}")]
    Duration(#[from] nopass_core::expiry::DurationError),
    #[error("visudo rejected the rule: {stderr}")]
    VisudoRejected { stderr: String },
    #[error("lock busy")]
    LockBusy,
    #[error("filesystem failure: {0}")]
    Fs(String),
    #[error("timer scheduling failed (rolled back: {rolled_back})")]
    TimerFailed { rolled_back: bool },
}

impl HelperError {
    /// Maps this error to its documented process exit code.
    pub fn exit_code(&self) -> i32 {
        match self {
            HelperError::Internal(_) | HelperError::BinaryMissing { .. } => 1,
            HelperError::Context(_) => 10,
            HelperError::UidRejected(_) => 11,
            HelperError::NotSudoer => 12,
            HelperError::Duration(_) => 13,
            HelperError::VisudoRejected { .. } => 14,
            HelperError::LockBusy => 15,
            HelperError::Fs(_) => 16,
            HelperError::TimerFailed { .. } => 17,
        }
    }

    /// A short, stable audit-record token for this error (helper-observability
    /// §Journald Audit Records: "a journald record distinguishes the
    /// rejection outcome from a success record"; design.md §9: the
    /// `REASON` field is "never raw subprocess text"). Deliberately NOT
    /// derived from `Display`/`thiserror`'s `{0}`/`{stderr}` interpolation
    /// — those carry raw filesystem paths and subprocess stderr, exactly
    /// what design.md §9 forbids from an audit record.
    pub fn audit_reason(&self) -> &'static str {
        match self {
            HelperError::Internal(_) => "internal",
            HelperError::BinaryMissing { .. } => "binary_missing",
            HelperError::Context(_) => "invocation_context",
            HelperError::UidRejected(rejection) => match rejection {
                UidRejection::Root => "uid_rejected_root",
                UidRejection::BelowMin { .. } => "uid_rejected_below_min",
                UidRejection::AboveMax { .. } => "uid_rejected_above_max",
                UidRejection::Unknown => "uid_rejected_unknown",
            },
            HelperError::NotSudoer => "not_sudoer",
            HelperError::Duration(_) => "invalid_duration",
            HelperError::VisudoRejected { .. } => "visudo_rejected",
            HelperError::LockBusy => "lock_busy",
            HelperError::Fs(_) => "fs_error",
            HelperError::TimerFailed { .. } => "timer_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_and_binary_missing_map_to_exit_1() {
        assert_eq!(HelperError::Internal("boom".to_string()).exit_code(), 1);
        assert_eq!(HelperError::BinaryMissing { name: "visudo" }.exit_code(), 1);
    }

    #[test]
    fn context_maps_to_exit_10() {
        assert_eq!(HelperError::Context("missing PKEXEC_UID").exit_code(), 10);
    }

    #[test]
    fn uid_rejected_maps_to_exit_11_regardless_of_rejection_cause() {
        assert_eq!(HelperError::UidRejected(UidRejection::Root).exit_code(), 11);
        assert_eq!(HelperError::UidRejected(UidRejection::Unknown).exit_code(), 11);
        assert_eq!(HelperError::UidRejected(UidRejection::BelowMin { min: 1000 }).exit_code(), 11);
        assert_eq!(HelperError::UidRejected(UidRejection::AboveMax { max: 60000 }).exit_code(), 11);
    }

    #[test]
    fn not_sudoer_maps_to_exit_12() {
        assert_eq!(HelperError::NotSudoer.exit_code(), 12);
    }

    #[test]
    fn duration_maps_to_exit_13() {
        let err: HelperError = nopass_core::expiry::DurationError::NotInFuture.into();
        assert_eq!(err.exit_code(), 13);
    }

    #[test]
    fn visudo_rejected_maps_to_exit_14() {
        assert_eq!(HelperError::VisudoRejected { stderr: "bad syntax".to_string() }.exit_code(), 14);
    }

    #[test]
    fn lock_busy_maps_to_exit_15() {
        assert_eq!(HelperError::LockBusy.exit_code(), 15);
    }

    #[test]
    fn fs_maps_to_exit_16() {
        assert_eq!(HelperError::Fs("rename failed".to_string()).exit_code(), 16);
    }

    #[test]
    fn timer_failed_maps_to_exit_17_regardless_of_rollback_flag() {
        assert_eq!(HelperError::TimerFailed { rolled_back: true }.exit_code(), 17);
        assert_eq!(HelperError::TimerFailed { rolled_back: false }.exit_code(), 17);
    }

    #[test]
    fn audit_reason_gives_a_short_stable_token_per_error_variant() {
        assert_eq!(HelperError::Internal("boom".to_string()).audit_reason(), "internal");
        assert_eq!(HelperError::BinaryMissing { name: "visudo" }.audit_reason(), "binary_missing");
        assert_eq!(HelperError::Context("missing PKEXEC_UID").audit_reason(), "invocation_context");
        assert_eq!(HelperError::UidRejected(UidRejection::Root).audit_reason(), "uid_rejected_root");
        assert_eq!(HelperError::NotSudoer.audit_reason(), "not_sudoer");
        assert_eq!(
            HelperError::Duration(nopass_core::expiry::DurationError::NotInFuture).audit_reason(),
            "invalid_duration"
        );
        assert_eq!(HelperError::VisudoRejected { stderr: "bad syntax".to_string() }.audit_reason(), "visudo_rejected");
        assert_eq!(HelperError::LockBusy.audit_reason(), "lock_busy");
        assert_eq!(HelperError::Fs("rename failed".to_string()).audit_reason(), "fs_error");
        assert_eq!(HelperError::TimerFailed { rolled_back: true }.audit_reason(), "timer_failed");
    }

    #[test]
    fn uid_rejected_audit_reason_carries_a_distinct_token_per_rejection_variant() {
        // Phase 8 correction: task 5.2 widened `UidRejected` to carry the
        // `UidRejection` payload specifically "so the rejection reason
        // survives to the Phase 8 journald audit record" — a single
        // shared "uid_rejected" token for all four causes discards that
        // payload right before the audit boundary, making the widening
        // pointless. Each cause must produce its own stable token.
        let root = HelperError::UidRejected(UidRejection::Root).audit_reason();
        let below_min = HelperError::UidRejected(UidRejection::BelowMin { min: 1000 }).audit_reason();
        let above_max = HelperError::UidRejected(UidRejection::AboveMax { max: 60_000 }).audit_reason();
        let unknown = HelperError::UidRejected(UidRejection::Unknown).audit_reason();

        let tokens = [root, below_min, above_max, unknown];
        let mut sorted = tokens.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), tokens.len(), "each UidRejection variant must produce a distinct audit token: {tokens:?}");

        // Every sub-token still shares the "uid_rejected" prefix, so
        // `journalctl -t nopass-helper | grep uid_rejected` keeps finding
        // every uid-admission failure regardless of sub-cause.
        for token in tokens {
            assert!(token.starts_with("uid_rejected"), "token must share the uid_rejected prefix, got {token:?}");
        }

        // A token is a stable identifier, never a message: the numeric
        // bound values belong to `AuditRecord`'s own fields, not baked
        // into the reason token.
        assert!(!below_min.contains("1000"), "token must not embed the numeric bound: {below_min:?}");
        assert!(!above_max.contains("60000"), "token must not embed the numeric bound: {above_max:?}");
    }

    #[test]
    fn audit_reason_never_leaks_raw_subprocess_stderr_text() {
        let err = HelperError::VisudoRejected { stderr: "syntax error near line 4".to_string() };
        assert_eq!(err.audit_reason(), "visudo_rejected", "must be the stable token, never the raw stderr text");
    }

    #[test]
    fn every_failure_cause_maps_to_a_distinct_code() {
        let codes = [
            HelperError::Context("x").exit_code(),
            HelperError::UidRejected(UidRejection::Root).exit_code(),
            HelperError::NotSudoer.exit_code(),
            HelperError::Duration(nopass_core::expiry::DurationError::NotInFuture).exit_code(),
            HelperError::VisudoRejected { stderr: String::new() }.exit_code(),
            HelperError::LockBusy.exit_code(),
            HelperError::Fs(String::new()).exit_code(),
            HelperError::TimerFailed { rolled_back: false }.exit_code(),
        ];
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "no two distinct failure causes may share a code");
    }
}

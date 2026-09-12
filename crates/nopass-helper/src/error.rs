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
//! `checks`, `lock`, `fileops`, `timer`, and `ops` (Phases 5-7) are the
//! modules that actually construct these variants; until they land, a
//! normal (non-test) build never constructs them, hence the blanket
//! allow below. Remove it once Phase 5 wires `checks::admit_uid` etc.
#![allow(dead_code)]

#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    #[error("internal error: {0}")]
    Internal(String),
    #[error("binary not found: {name}")]
    BinaryMissing { name: &'static str },
    #[error("invalid invocation context: {0}")]
    Context(&'static str),
    /// Target uid failed admission (`checks::admit_uid`, landing in Phase
    /// 5). The rejection reason is carried for the audit record only and
    /// does not split the exit code — every cause maps to 11.
    #[error("target uid rejected")]
    UidRejected,
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
            HelperError::UidRejected => 11,
            HelperError::NotSudoer => 12,
            HelperError::Duration(_) => 13,
            HelperError::VisudoRejected { .. } => 14,
            HelperError::LockBusy => 15,
            HelperError::Fs(_) => 16,
            HelperError::TimerFailed { .. } => 17,
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
    fn uid_rejected_maps_to_exit_11() {
        assert_eq!(HelperError::UidRejected.exit_code(), 11);
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
    fn every_failure_cause_maps_to_a_distinct_code() {
        let codes = [
            HelperError::Context("x").exit_code(),
            HelperError::UidRejected.exit_code(),
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

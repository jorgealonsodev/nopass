//! The ground-truth probe: `/usr/bin/sudo -k -n true`, interpreted, and its
//! freshness policy relative to the state file (design.md §3.3, §4.3).
//!
//! `-k` ignores the caller's cached `sudo` credentials for this invocation
//! only — without it, a user who ran `sudo` two minutes ago would get a
//! false positive from the timestamp cache rather than from an actual
//! NOPASSWD rule. `-n` never prompts, so the probe cannot block the tray
//! waiting for a password nobody is there to type.

use std::path::Path;

use crate::runner::{CommandSpec, RunnerError, SpawnOutcome};

/// 1.5 × the 60 s reconciliation tick: one missed tick is tolerated, two
/// are not (design.md §3.3).
pub const MAX_PROBE_AGE_SECS: u64 = 90;

/// The two-valued result of asking `sudo` whether it would prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Passwordless,
    PasswordRequired,
}

/// Every way the probe itself failed to produce a `Probe` value. Distinct
/// from [`crate::runner::RunnerError`]: this is the probe's own
/// interpretation of a runner failure, not the runner's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeError {
    /// The probe could not even be attempted (for example, a
    /// non-absolute program path reached the runner).
    #[error("the sudo probe could not be spawned")]
    Spawn,
    /// The probe process was killed by a signal.
    #[error("the sudo probe was killed by a signal")]
    Signaled,
    /// The runner's own OS-level spawn failure — overwhelmingly the sudo
    /// binary being absent or not executable at the resolved path.
    #[error("the sudo binary appears to be missing")]
    BinaryMissing,
}

/// Builds the exact, documented probe invocation: `sudo -k -n true` with
/// `LANG=C, LC_ALL=C` — the probe's own stderr is logged, so unlike
/// `pkexec` (design.md D9) its locale is deliberately forced (design.md
/// §4.3).
pub fn spec(sudo: &Path) -> CommandSpec {
    CommandSpec {
        program: sudo.to_path_buf(),
        args: vec!["-k".to_string(), "-n".to_string(), "true".to_string()],
        env: vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())],
    }
}

/// Interprets a runner result into a `Probe` value. Exit 0 means `sudo`
/// would not have prompted (`Passwordless`); any other exit means it
/// would have (`PasswordRequired`) — the probe result is binary by
/// construction, never a third "maybe".
pub fn interpret(outcome: Result<SpawnOutcome, RunnerError>) -> Result<Probe, ProbeError> {
    match outcome {
        Ok(SpawnOutcome { status: Some(0), .. }) => Ok(Probe::Passwordless),
        Ok(SpawnOutcome { status: Some(_), .. }) => Ok(Probe::PasswordRequired),
        // `status: None` never occurs from `SystemRunner` (it maps that
        // case to `RunnerError::Signaled` itself), but a `ScriptedRunner`
        // script can still express it — interpret it the same way.
        Ok(SpawnOutcome { status: None, .. }) => Err(ProbeError::Signaled),
        // The probe never spawned at all — a structural refusal, not
        // evidence the binary is missing.
        Err(RunnerError::NonAbsoluteProgram { .. }) => Err(ProbeError::Spawn),
        // An OS-level spawn failure against an already-resolved absolute
        // path is overwhelmingly ENOENT/EACCES — the binary vanished or
        // lost its permissions between resolution and this call.
        Err(RunnerError::Spawn { .. }) => Err(ProbeError::BinaryMissing),
        Err(RunnerError::Signaled { .. }) => Err(ProbeError::Signaled),
    }
}

/// A probe result taken at `taken_at`, cached until it either goes stale
/// or a newer state-file observation supersedes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeCache {
    pub value: Probe,
    pub taken_at: u64,
}

impl ProbeCache {
    /// `Some(v)` iff `taken_at >= file_observed_at` (the probe is not
    /// older than the last file observation it must corroborate) and
    /// `now - taken_at <= MAX_PROBE_AGE_SECS` (design.md §3.3). A probe
    /// somehow dated after `now` is never usable either — it is not
    /// evidence about anything.
    pub fn usable(&self, file_observed_at: u64, now: u64) -> Option<Probe> {
        if self.taken_at < file_observed_at {
            return None;
        }
        match now.checked_sub(self.taken_at) {
            Some(age) if age <= MAX_PROBE_AGE_SECS => Some(self.value),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{CommandRunner as _, ScriptedRunner};
    use std::path::PathBuf;

    #[test]
    fn max_probe_age_is_ninety_seconds() {
        assert_eq!(MAX_PROBE_AGE_SECS, 90);
    }

    #[test]
    fn spec_builds_the_exact_documented_sudo_probe_invocation() {
        let s = spec(&PathBuf::from("/usr/bin/sudo"));
        assert_eq!(s.program, PathBuf::from("/usr/bin/sudo"));
        assert_eq!(s.args, vec!["-k".to_string(), "-n".to_string(), "true".to_string()]);
        assert_eq!(s.env, vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())]);
    }

    #[test]
    fn spec_matches_field_by_field_through_a_scripted_runner() {
        let expected = spec(&PathBuf::from("/usr/bin/sudo"));
        let outcome = SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] };
        let runner = ScriptedRunner::new(vec![(expected.clone(), Ok(outcome.clone()))]);
        assert_eq!(runner.run(&expected).unwrap(), outcome);
    }

    #[test]
    fn interpret_maps_exit_zero_to_passwordless() {
        let outcome = SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] };
        assert_eq!(interpret(Ok(outcome)), Ok(Probe::Passwordless));
    }

    #[test]
    fn interpret_maps_any_nonzero_exit_to_password_required() {
        for code in [1, 2, 42] {
            let outcome = SpawnOutcome { status: Some(code), stdout: vec![], stderr: vec![] };
            assert_eq!(interpret(Ok(outcome)), Ok(Probe::PasswordRequired));
        }
    }

    #[test]
    fn interpret_maps_a_missing_status_to_signaled() {
        let outcome = SpawnOutcome { status: None, stdout: vec![], stderr: vec![] };
        assert_eq!(interpret(Ok(outcome)), Err(ProbeError::Signaled));
    }

    #[test]
    fn interpret_maps_non_absolute_program_to_a_spawn_error() {
        let err = RunnerError::NonAbsoluteProgram { program: "sudo".to_string() };
        assert_eq!(interpret(Err(err)), Err(ProbeError::Spawn));
    }

    #[test]
    fn interpret_maps_an_os_level_spawn_failure_to_binary_missing() {
        let err = RunnerError::Spawn {
            program: "/usr/bin/sudo".to_string(),
            reason: "No such file or directory".to_string(),
        };
        assert_eq!(interpret(Err(err)), Err(ProbeError::BinaryMissing));
    }

    #[test]
    fn interpret_maps_a_runner_level_signal_death_to_signaled() {
        let err = RunnerError::Signaled { program: "/usr/bin/sudo".to_string() };
        assert_eq!(interpret(Err(err)), Err(ProbeError::Signaled));
    }

    #[test]
    fn probe_cache_rejects_a_probe_taken_before_the_file_was_observed() {
        let cache = ProbeCache { value: Probe::Passwordless, taken_at: 100 };
        assert_eq!(cache.usable(101, 105), None, "a probe older than the file is not evidence about it");
    }

    #[test]
    fn probe_cache_boundary_at_89_seconds_is_usable() {
        let cache = ProbeCache { value: Probe::Passwordless, taken_at: 0 };
        assert_eq!(cache.usable(0, 89), Some(Probe::Passwordless));
    }

    #[test]
    fn probe_cache_boundary_at_90_seconds_is_usable() {
        let cache = ProbeCache { value: Probe::Passwordless, taken_at: 0 };
        assert_eq!(cache.usable(0, 90), Some(Probe::Passwordless));
    }

    #[test]
    fn probe_cache_boundary_at_91_seconds_is_unusable() {
        let cache = ProbeCache { value: Probe::Passwordless, taken_at: 0 };
        assert_eq!(cache.usable(0, 91), None);
    }

    #[test]
    fn probe_cache_rejects_a_probe_dated_after_now() {
        let cache = ProbeCache { value: Probe::PasswordRequired, taken_at: 100 };
        assert_eq!(cache.usable(0, 50), None, "a probe timestamped after `now` is not usable evidence");
    }
}

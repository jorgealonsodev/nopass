//! uid/sudoer admission checks (privilege-admission §UID Range Admission,
//! §Existing-Sudoer Probe; design.md §2, §4.1 steps 5-6; threat matrix
//! "External command composition").
//!
//! `lookup_user` and `admit_uid` are the uid-range gate; `is_sudoer` is the
//! authoritative "may already run anything as root" probe via
//! `CommandRunner`. `in_admin_group` is advisory-only pre-check plumbing —
//! it MUST NEVER be consulted by `is_sudoer` or otherwise override the
//! probe's verdict (privilege-admission §Existing-Sudoer Probe).
//!
//! Phase 7's `ops.rs` is the production caller of every function here;
//! until it lands, a normal (non-test) build never calls them, hence the
//! blanket allow below.
#![allow(dead_code)]

use std::ffi::CString;

use nopass_core::logindefs::UidRange;

use crate::bins::Binaries;
use crate::error::HelperError;
use crate::runner::{CommandRunner, CommandSpec, Expect};

/// Group names treated as an advisory "likely already a sudoer" signal.
/// Never authoritative — see the module doc comment.
const ADMIN_GROUPS: &[&str] = &["sudo", "wheel", "admin"];

/// Why `admit_uid` rejected a uid. Carried through `HelperError::UidRejected`
/// for the audit record only; every cause still maps to exit 11.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UidRejection {
    /// No `getpwuid` entry exists for the uid.
    Unknown,
    /// The uid is 0. Rejected unconditionally, independent of
    /// `/etc/login.defs`.
    Root,
    /// The uid is below the admissible range's `min`.
    BelowMin { min: u32 },
    /// The uid is above the admissible range's `max`.
    AboveMax { max: u32 },
}

/// Looks up the username for `uid` via `getpwuid` (`nix::unistd::User`).
/// Used for the audit record and by `ops::disable`'s username fallback
/// chain (design.md §4.2: `getpwuid` failure falls back to the rule
/// header's `nopass-user`, then to `""`).
pub fn lookup_user(uid: u32) -> Result<String, HelperError> {
    match nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)) {
        Ok(Some(user)) => Ok(user.name),
        Ok(None) => Err(HelperError::Internal(format!("no passwd entry for uid {uid}"))),
        Err(errno) => Err(HelperError::Internal(format!("getpwuid_r failed for uid {uid}: {errno}"))),
    }
}

/// Admits `uid` only when it is non-zero, falls within `range`
/// (`[min, max]` inclusive), and has a `getpwuid` entry
/// (privilege-admission §UID Range Admission). uid 0 is rejected before the
/// range is even consulted, so a `range.min == 0` (e.g. a malformed
/// `/etc/login.defs` before `logindefs::parse`'s own floor clamp) can never
/// admit it.
pub fn admit_uid(uid: u32, range: &UidRange) -> Result<(), UidRejection> {
    if uid == 0 {
        return Err(UidRejection::Root);
    }
    if uid < range.min {
        return Err(UidRejection::BelowMin { min: range.min });
    }
    if uid > range.max {
        return Err(UidRejection::AboveMax { max: range.max });
    }
    match nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)) {
        Ok(Some(_)) => Ok(()),
        _ => Err(UidRejection::Unknown),
    }
}

/// Advisory-only fast pre-check: does `user` (uid `uid`) belong to
/// `sudo`/`wheel`/`admin`? Never grants or denies admission by itself —
/// callers MUST still run [`is_sudoer`], whose exit-0 probe is the sole
/// authority (privilege-admission §Existing-Sudoer Probe). Returns `false`
/// on any lookup failure rather than propagating an error, since a failed
/// advisory check simply means the fast path is skipped, not that
/// anything is rejected.
pub fn in_admin_group(user: &str, uid: u32) -> bool {
    let Ok(cuser) = CString::new(user) else {
        return false;
    };
    let primary_gid = match nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)) {
        Ok(Some(u)) => u.gid,
        _ => return false,
    };
    let Ok(gids) = nix::unistd::getgrouplist(&cuser, primary_gid) else {
        return false;
    };
    gids.into_iter().any(|gid| {
        nix::unistd::Group::from_gid(gid)
            .ok()
            .flatten()
            .is_some_and(|g| ADMIN_GROUPS.contains(&g.name.as_str()))
    })
}

/// The authoritative "already a sudoer" probe: `user` is treated as an
/// existing sudoer if and only if `LANG=C /usr/bin/sudo -n -l -U <user>
/// /bin/sh` exits 0 (privilege-admission §Existing-Sudoer Probe). Never
/// parses `sudo`'s stderr/denial text. `sudo` and `sh` are resolved
/// through `Binaries` (never a hand-built path), and the spec carries no
/// `PATH` and no `PKEXEC_UID` — only `LANG`/`LC_ALL`, matching every other
/// `CommandSpec` in this crate (design.md §3).
///
/// `user` is written into `CommandSpec.args` verbatim — this function does
/// **not** sanitize it. The caller MUST sanitize (via
/// `nopass_core::template::sanitize_username`) before calling; that
/// caller is Phase 7's `ops.rs`, which does not exist yet, and the
/// obligation is tracked in `tasks.md` against that phase, not enforced
/// here. Today an unsanitized value still fails closed independently of
/// that missing sanitization: `CommandSpec.args` is a `Vec<String>`
/// consumed with no shell (see `runner.rs`), so a hostile value arrives
/// at the child process as one literal argv token bound to `-U`, never as
/// additional flags or shell-interpreted metacharacters. That is a
/// defense-in-depth property of `CommandSpec`/`SystemRunner`, not a
/// substitute for the caller's sanitization obligation.
pub fn is_sudoer(runner: &dyn CommandRunner, binaries: &Binaries, user: &str) -> Result<(), HelperError> {
    let sudo = binaries.resolve("sudo")?.to_path_buf();
    let sh = binaries.resolve("sh")?.to_path_buf();
    let spec = CommandSpec {
        program: sudo,
        args: vec!["-n".to_string(), "-l".to_string(), "-U".to_string(), user.to_string(), sh.display().to_string()],
        env: vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())],
        expect: Expect::Any,
    };
    let outcome = runner.run(&spec).map_err(|e| HelperError::Internal(e.to_string()))?;
    match outcome.status {
        Some(0) => Ok(()),
        _ => Err(HelperError::NotSudoer),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::runner::{CommandOutcome, RunnerError, ScriptedRunner};

    fn real_binaries() -> Binaries {
        // `/usr/bin/sudo` and `/bin/sh` are asserted to exist by the same
        // convention `runner.rs`'s own tests already rely on for `/bin/sh`
        // and `/bin/echo` — this crate's test suite targets a real Linux
        // dev/CI machine, not a hermetic filesystem.
        Binaries::from_candidates(&[("sudo", &[Path::new("/usr/bin/sudo")]), ("sh", &[Path::new("/bin/sh")])])
    }

    fn expected_probe_spec(user: &str) -> CommandSpec {
        CommandSpec {
            program: PathBuf::from("/usr/bin/sudo"),
            args: vec!["-n".to_string(), "-l".to_string(), "-U".to_string(), user.to_string(), "/bin/sh".to_string()],
            env: vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())],
            expect: Expect::Any,
        }
    }

    // --- admit_uid -----------------------------------------------------

    #[test]
    fn default_range_uid_is_admitted_when_it_exists_in_the_passwd_database() {
        let uid = nix::unistd::getuid().as_raw();
        if uid == 0 {
            // Running as root in this sandbox: root is unconditionally
            // rejected by admit_uid (see the next test), so the positive
            // "admitted" scenario cannot be exercised against the literal
            // current uid here. Skipping is honest; asserting Ok(()) for
            // uid 0 would be wrong.
            return;
        }
        let range = UidRange { min: uid.min(1), max: uid.max(60_000) };
        assert_eq!(admit_uid(uid, &range), Ok(()));
    }

    #[test]
    fn uid_0_is_always_rejected_even_with_uid_min_0() {
        let range = UidRange { min: 0, max: 60_000 };
        assert_eq!(admit_uid(0, &range), Err(UidRejection::Root));
    }

    #[test]
    fn uid_65534_is_rejected_for_exceeding_the_default_uid_max() {
        let range = nopass_core::logindefs::parse(""); // defaults: 1000..=60000
        assert_eq!(admit_uid(65_534, &range), Err(UidRejection::AboveMax { max: range.max }));
    }

    #[test]
    fn uid_below_uid_min_is_rejected() {
        let range = UidRange { min: 2000, max: 60_000 };
        assert_eq!(admit_uid(1500, &range), Err(UidRejection::BelowMin { min: 2000 }));
    }

    #[test]
    fn uid_absent_from_getpwuid_is_rejected() {
        // An implausibly large uid, inside a synthetic range that admits
        // it on the uid==0/range checks alone, so only the passwd-lookup
        // check can reject it.
        let range = UidRange { min: 1000, max: 4_294_967_294 };
        assert_eq!(admit_uid(4_294_967_294, &range), Err(UidRejection::Unknown));
    }

    // --- in_admin_group (advisory) --------------------------------------

    #[test]
    fn in_admin_group_returns_false_for_a_uid_with_no_passwd_entry() {
        assert!(!in_admin_group("nopass-test-nonexistent-user", 4_294_967_294));
    }

    // --- is_sudoer -------------------------------------------------------

    #[test]
    fn is_sudoer_builds_the_exact_pinned_argv_with_no_shell_and_only_lang_c_env() {
        // `ScriptedRunner` asserts full `CommandSpec` field-by-field
        // equality (program, args, env, expect) — a mismatch on ANY
        // field, including an extra/renamed env var such as `PKEXEC_UID`
        // leaking in, panics here rather than silently passing.
        let outcome = CommandOutcome { status: Some(0), stdout: vec![], stderr: vec![] };
        let runner = ScriptedRunner::new(vec![(expected_probe_spec("ana"), Ok(outcome))]);
        assert!(is_sudoer(&runner, &real_binaries(), "ana").is_ok());
    }

    #[test]
    fn probe_exit_zero_admits() {
        let outcome = CommandOutcome { status: Some(0), stdout: vec![], stderr: vec![] };
        let runner = ScriptedRunner::new(vec![(expected_probe_spec("jorge"), Ok(outcome))]);
        assert!(is_sudoer(&runner, &real_binaries(), "jorge").is_ok());
    }

    #[test]
    fn probe_nonzero_rejects_even_when_the_user_is_group_flagged() {
        // `in_admin_group` is never consulted by `is_sudoer` at all — a
        // non-zero probe rejects unconditionally, group membership
        // notwithstanding (privilege-admission §Existing-Sudoer Probe,
        // "Probe exits non-zero rejects even a group-flagged user").
        let outcome = CommandOutcome { status: Some(1), stdout: vec![], stderr: b"user jorge is not allowed".to_vec() };
        let runner = ScriptedRunner::new(vec![(expected_probe_spec("jorge"), Ok(outcome))]);
        let err = is_sudoer(&runner, &real_binaries(), "jorge").unwrap_err();
        assert!(matches!(err, HelperError::NotSudoer));
        assert_eq!(err.exit_code(), 12);
    }

    // `SystemRunner` (see `runner.rs`) never returns `Ok(CommandOutcome {
    // status: None, .. })` — a signaled child surfaces as
    // `Err(RunnerError::Signaled)`, per `runner.rs`'s own doc comment and
    // its `system_runner_treats_signal_death_as_failure_even_under_expect_any`
    // test. The three tests below instead script every `RunnerError`
    // variant `is_sudoer` can actually receive from `runner.run(&spec)`
    // and pin what `is_sudoer`'s `Err(RunnerError)` branch really does:
    // `.map_err(|e| HelperError::Internal(e.to_string()))` — exit 1, not
    // `HelperError::NotSudoer` (exit 12). Exit 12 is reserved for a probe
    // that actually ran and exited non-zero (see
    // `probe_nonzero_rejects_even_when_the_user_is_group_flagged` above);
    // a probe that never produced an exit status at all is an internal
    // failure of the probe mechanism itself, not a sudoer verdict.

    #[test]
    fn probe_signaled_death_maps_to_internal_error_exit_1_not_not_sudoer() {
        let runner = ScriptedRunner::new(vec![(
            expected_probe_spec("jorge"),
            Err(RunnerError::Signaled { program: "/usr/bin/sudo".to_string() }),
        )]);
        let err = is_sudoer(&runner, &real_binaries(), "jorge").unwrap_err();
        assert!(matches!(err, HelperError::Internal(_)), "expected Internal, got {err:?}");
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn probe_spawn_failure_maps_to_internal_error_exit_1() {
        let runner = ScriptedRunner::new(vec![(
            expected_probe_spec("jorge"),
            Err(RunnerError::Spawn { program: "/usr/bin/sudo".to_string(), reason: "ENOENT".to_string() }),
        )]);
        let err = is_sudoer(&runner, &real_binaries(), "jorge").unwrap_err();
        assert!(matches!(err, HelperError::Internal(_)), "expected Internal, got {err:?}");
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn probe_non_absolute_program_error_maps_to_internal_error_exit_1() {
        let runner = ScriptedRunner::new(vec![(
            expected_probe_spec("jorge"),
            Err(RunnerError::NonAbsoluteProgram { program: "sudo".to_string() }),
        )]);
        let err = is_sudoer(&runner, &real_binaries(), "jorge").unwrap_err();
        assert!(matches!(err, HelperError::Internal(_)), "expected Internal, got {err:?}");
        assert_eq!(err.exit_code(), 1);
    }
}

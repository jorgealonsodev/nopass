//! The four privileged transactions — `enable`, `disable`, `status`,
//! `expire` — with the exact step order and rollback rules of design.md
//! §4. This is the only module `main::dispatch` calls into for real work.
//!
//! Every `pub fn` below (`enable`, `disable`, `status`, `expire`) is a
//! thin **production wrapper**: it is the call site obligated to read the
//! live invocation context — `std::env::var("PKEXEC_UID")` and
//! `nix::unistd::getuid()` (the REAL uid, never `geteuid()`), exactly as
//! `uid::resolve`'s doc comment prescribes — then hands off to a fully
//! parameterized, deterministically testable `*_inner` function. Every
//! `*_inner` function and every pure helper takes its inputs explicitly,
//! matching the injection convention already established by
//! `uid::resolve`, `checks::admit_uid`, and `lock::LockGuard::acquire`.

use std::path::Path;

use nopass_core::expiry::Expiry;
use nopass_core::header::{self, RuleHeader};
use nopass_core::logindefs::{self, UidRange};
use nopass_core::paths::Layout;
use nopass_core::state::HelperStatus;
use nopass_core::template::{render_rule, sanitize_username};

use crate::bins::Binaries;
use crate::checks;
use crate::cli::Cmd;
use crate::error::HelperError;
use crate::fileops;
use crate::lock::LockGuard;
use crate::runner::CommandRunner;
use crate::timer;
use crate::uid::{self, InvocationContext};

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

// --- enable -----------------------------------------------------------

/// Production `enable` entry point (helper-cli, sudoers-rule-lifecycle,
/// expiry-policy §Temporary Duration Validation; design.md §4.1). Reads
/// the live invocation context and the target's real username, then
/// hands off to [`enable_inner`].
pub fn enable(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    until: Option<u64>,
    until_reboot: bool,
) -> Result<(), HelperError> {
    let cmd = Cmd::Enable { until, until_reboot };
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    let ctx = uid::resolve(&cmd, pkexec_uid.as_deref(), real_uid)?;
    let InvocationContext::Pkexec(uid) = ctx else {
        unreachable!("Cmd::Enable always resolves to InvocationContext::Pkexec")
    };

    let now = unix_now();
    let expiry = resolve_expiry(until, until_reboot, now)?;
    let raw_user = checks::lookup_user(uid)?;
    let login_defs = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let uid_range = logindefs::parse(&login_defs);

    enable_inner(layout, runner, binaries, uid, &raw_user, &uid_range, expiry)
}

/// Resolves `--until`/`--until-reboot` into an [`Expiry`], validating any
/// `--until` duration against `nopass_core::expiry::validate_until`
/// (expiry-policy §Temporary Duration Validation; design.md §4.1 step 4).
/// Pure and directly testable — this is where exit 13 originates, always
/// BEFORE `enable_inner` acquires the lock or touches the filesystem
/// (design.md §4.1 rollback table rows 1-7: "nothing was created").
fn resolve_expiry(until: Option<u64>, until_reboot: bool, now: u64) -> Result<Expiry, HelperError> {
    if until_reboot {
        return Ok(Expiry::Reboot);
    }
    match until {
        None => Ok(Expiry::Never),
        Some(epoch) => {
            nopass_core::expiry::validate_until(now, epoch)?;
            Ok(Expiry::At { epoch })
        }
    }
}

/// The full `enable` transaction (design.md §4.1 steps 5-15), fully
/// parameterized for deterministic testing: `uid`/`raw_user` are already
/// resolved by [`enable`]'s live wrapper, and `expiry` is already
/// validated by [`resolve_expiry`]. `raw_user` is deliberately NOT
/// pre-sanitized by the caller — sanitizing it here, once, before it
/// reaches the sudoer probe is exactly the obligation `tasks.md`'s Phase
/// 7 blockquote records against this module: `checks::is_sudoer` does not
/// sanitize its `user` argument.
fn enable_inner(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    uid: u32,
    raw_user: &str,
    uid_range: &UidRange,
    expiry: Expiry,
) -> Result<(), HelperError> {
    // Step 5: getpwuid + UID range admission (exit 11).
    checks::admit_uid(uid, uid_range).map_err(HelperError::UidRejected)?;

    // Step 6: sudoer probe. The one and only sanitization point before
    // the username reaches the probe argv.
    let user = sanitize_username(raw_user);
    checks::is_sudoer(runner, binaries, &user)?;

    // Step 7: acquire the mutation lock. Steps 8+ all happen inside it.
    let _guard = LockGuard::acquire(layout)?;

    // Steps 8-13: atomic write + visudo validation + rename. Tmp cleanup
    // on any failure is handled internally by `write_rule_atomic`.
    let content = render_rule(uid, &user, expiry);
    fileops::write_rule_atomic(layout, runner, binaries, uid, &content)?;

    // Step 14: always stop a stale timer, regardless of the new expiry
    // kind — clears a leftover timer from a previous `At` activation even
    // when this `enable` is `Never`/`Reboot`.
    timer::stop(runner, binaries, uid);

    // Step 15: schedule a new timer only for `At`. Failure rolls the rule
    // back and reports exit 17 (design.md §4.1 rollback table).
    if let Expiry::At { epoch } = expiry {
        if timer::schedule(runner, binaries, uid, epoch).is_err() {
            rollback_rule(layout, uid);
            return Err(HelperError::TimerFailed { rolled_back: true });
        }
    }

    // Steps 16-17 (state file, journald audit) land in Phase 8.
    Ok(())
    // Step 18: `_guard` drops here, releasing the lock.
}

/// Rollback for a step-15 `systemd-run` failure: unlink the rule just
/// written and best-effort `fsync` the containing directory (design.md
/// §4.1 rollback table, row 15). Both are best-effort — this function
/// runs only from inside an error path and has nothing further to roll
/// back to.
fn rollback_rule(layout: &Layout, uid: u32) {
    let path = layout.rule_path(uid);
    let _ = std::fs::remove_file(&path);
    fsync_parent(&path);
}

fn fsync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

// --- disable ------------------------------------------------------------

/// Production `disable` entry point (sudoers-rule-lifecycle §Rule
/// Removal; design.md §4.2).
pub fn disable(layout: &Layout, runner: &dyn CommandRunner, binaries: &Binaries) -> Result<(), HelperError> {
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    let ctx = uid::resolve(&Cmd::Disable, pkexec_uid.as_deref(), real_uid)?;
    let InvocationContext::Pkexec(uid) = ctx else {
        unreachable!("Cmd::Disable always resolves to InvocationContext::Pkexec")
    };
    disable_inner(layout, runner, binaries, uid)
}

/// design.md §4.2: no UID-range or sudoer admission is required —
/// removing a privilege must never be blocked, and a deleted account must
/// still be revocable. Unlink precedes timer stop, deliberately inverted
/// from the naive order: if the unlink failed after the timer was already
/// stopped, the grant would become permanent with no scheduled
/// revocation; in this order the worst case is an orphan timer that later
/// fires `expire --uid` and finds nothing — a no-op.
fn disable_inner(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    uid: u32,
) -> Result<(), HelperError> {
    let _guard = LockGuard::acquire(layout)?;
    fileops::remove_rule(layout, uid)?;
    timer::stop(runner, binaries, uid);
    Ok(())
}

/// Username fallback chain for the disable audit record (design.md §4.2:
/// `getpwuid` failure falls back to the header's `nopass-user`, then to
/// `""`). Pure and independently tested here; Phase 8 wires its result
/// into `journal::audit`, so `disable_inner` does not call it yet. Not
/// yet reachable from `main::dispatch` — like every other not-yet-wired
/// `pub` item in this crate (see `lib.rs`'s doc comment), `pub` inside a
/// `pub mod` of a lib-target crate is exempt from the `dead_code` lint.
pub fn resolve_username(passwd_lookup: Result<String, HelperError>, header: Option<&RuleHeader>) -> String {
    match passwd_lookup {
        Ok(name) => name,
        Err(_) => header.map(|h| h.user.clone()).unwrap_or_default(),
    }
}

// --- status ---------------------------------------------------------------

/// Production `status` entry point (helper-observability §HelperStatus
/// JSON Contract; design.md §4.5).
pub fn status(layout: &Layout) -> Result<(), HelperError> {
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    let ctx = uid::resolve(&Cmd::Status, pkexec_uid.as_deref(), real_uid)?;
    let InvocationContext::Pkexec(uid) = ctx else {
        unreachable!("Cmd::Status always resolves to InvocationContext::Pkexec")
    };
    let status = status_inner(layout, uid, unix_now());
    print!("{}", status.to_json_line());
    Ok(())
}

/// design.md §4.5: no lock, no write, reads the authoritative rule file
/// directly (not the state-file cache). Never returns an error — even a
/// genuine I/O failure (not just a missing file) degrades to
/// `active: false` rather than an exit 15/16, matching "`status` can
/// therefore never return 15 or 16."
fn status_inner(layout: &Layout, uid: u32, now: u64) -> HelperStatus {
    let rule_path = layout.rule_path(uid).display().to_string();
    let content = std::fs::read_to_string(layout.rule_path(uid)).ok();
    let parsed = content.as_deref().and_then(|c| header::parse(c).ok());
    match parsed {
        Some(header) => HelperStatus::active_from(uid, &header, rule_path, now),
        None => {
            let user = checks::lookup_user(uid).unwrap_or_default();
            HelperStatus::inactive(uid, user, rule_path, now)
        }
    }
}

// --- expire -----------------------------------------------------------

/// Production `expire` entry point (expiry-policy §Expiry Re-validation
/// Before Deletion, §Boot-Time Cleanup Sweep; design.md §4.3-4.4). Routes
/// to the boot-sweep path when `--boot` is set (restricted to `uid` when
/// both flags are given, per design.md §4.4), otherwise the single-uid
/// path — `clap`'s `ArgGroup` on `Cmd::Expire` guarantees `uid` is `Some`
/// whenever `boot` is `false`.
pub fn expire(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    uid: Option<u32>,
    boot: bool,
) -> Result<(), HelperError> {
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    uid::resolve(&Cmd::Expire { uid, boot }, pkexec_uid.as_deref(), real_uid)?;

    let now = unix_now();
    if boot {
        expire_boot_inner(layout, uid, now)
    } else {
        let target = uid.expect("clap's ArgGroup guarantees --uid or --boot is present");
        expire_uid_inner(layout, runner, binaries, target, now)
    }
}

/// design.md §4.3: re-reads the header under the lock and deletes only
/// when genuinely expired — the "Stale revocation" threat-matrix row. A
/// missing rule is treated as already-expired (exit 0, no-op); a file
/// that is not NoPass-owned is never deleted; `Never`, `Reboot`, and a
/// still-future `At` are all left intact.
fn expire_uid_inner(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    uid: u32,
    now: u64,
) -> Result<(), HelperError> {
    let _guard = LockGuard::acquire(layout)?;

    let Some(content) = fileops::read_rule(layout, uid)? else {
        return Ok(()); // absent rule — already-expired, exit 0 no-op
    };
    let Ok(header) = header::parse(&content) else {
        return Ok(()); // foreign/corrupted file — never delete it
    };
    if !header.expires.is_expired(now) {
        return Ok(()); // Never, Reboot, or still-future At — skipped_not_expired
    }

    let path = layout.rule_path(uid);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // raced externally — still success
        Err(e) => return Err(HelperError::Fs(format!("{}: {e}", path.display()))),
    }
    fsync_parent(&path);
    timer::stop(runner, binaries, uid);
    Ok(())
}

/// design.md §4.4: sweeps every canonical rule file directly under
/// `layout.sudoers_dir()` (restricted to `only_uid` when given), removing
/// each whose header indicates `Reboot` or a past epoch, and leaving
/// `Never`/future-`At` rules untouched. No `systemctl`/`systemd-run` call
/// is ever made in boot mode — this function does not even take a
/// `CommandRunner`/`Binaries` parameter, so that invariant holds
/// structurally, not merely by test assertion. A per-file failure is
/// counted but never aborts the sweep.
fn expire_boot_inner(layout: &Layout, only_uid: Option<u32>, now: u64) -> Result<(), HelperError> {
    let _guard = LockGuard::acquire(layout)?;

    let mut any_failed = false;
    for candidate_uid in fileops::list_rule_uids(layout)? {
        if only_uid.is_some_and(|restrict| restrict != candidate_uid) {
            continue;
        }
        if sweep_one(layout, candidate_uid, now).is_err() {
            any_failed = true;
        }
    }

    if any_failed {
        return Err(HelperError::Fs("one or more rule files failed to sweep".to_string()));
    }
    Ok(())
}

/// Boot-sweep handling for a single uid: `Expiry::is_expired_at_boot`
/// (unlike `expire --uid`'s `is_expired`) treats `Reboot` as expired too.
fn sweep_one(layout: &Layout, uid: u32, now: u64) -> Result<(), HelperError> {
    let path = layout.rule_path(uid);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()), // raced away externally
        Err(e) => return Err(HelperError::Fs(format!("{}: {e}", path.display()))),
    };
    let Ok(header) = header::parse(&content) else {
        return Ok(()); // not NoPass-owned — leave untouched
    };
    if !header.expires.is_expired_at_boot(now) {
        return Ok(()); // Never, or a still-future At — leave untouched
    }
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(HelperError::Fs(format!("{}: {e}", path.display()))),
    }
    fsync_parent(&path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::runner::{CommandOutcome, CommandSpec, RunnerError, ScriptedRunner};

    fn temp_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!(
            "nopass_test_ops_{tag}_{}_{:?}_{nanos}_{n}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    fn fresh_layout(tag: &str) -> (PathBuf, Layout) {
        let root = temp_root(tag);
        let layout = Layout::under(&root);
        std::fs::create_dir_all(root.join("sudoers.d")).unwrap();
        (root, layout)
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"").unwrap();
    }

    fn fake_binaries(root: &Path) -> Binaries {
        let sudo = root.join("sudo");
        let sh = root.join("sh");
        let visudo = root.join("visudo");
        let systemctl = root.join("systemctl");
        let systemd_run = root.join("systemd-run");
        for p in [&sudo, &sh, &visudo, &systemctl, &systemd_run] {
            touch(p);
        }
        Binaries::from_candidates(&[
            ("sudo", &[sudo.as_path()]),
            ("sh", &[sh.as_path()]),
            ("visudo", &[visudo.as_path()]),
            ("systemctl", &[systemctl.as_path()]),
            ("systemd-run", &[systemd_run.as_path()]),
        ])
    }

    fn lang_c() -> Vec<(String, String)> {
        vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())]
    }

    fn sudo_probe_spec(binaries: &Binaries, user: &str) -> CommandSpec {
        CommandSpec {
            program: binaries.resolve("sudo").unwrap().to_path_buf(),
            args: vec![
                "-n".to_string(),
                "-l".to_string(),
                "-U".to_string(),
                user.to_string(),
                binaries.resolve("sh").unwrap().display().to_string(),
            ],
            env: lang_c(),
            expect: crate::runner::Expect::Any,
        }
    }

    fn visudo_spec(binaries: &Binaries, layout: &Layout, uid: u32) -> CommandSpec {
        CommandSpec {
            program: binaries.resolve("visudo").unwrap().to_path_buf(),
            args: vec!["-cf".to_string(), layout.rule_tmp_path(uid).display().to_string()],
            env: lang_c(),
            expect: crate::runner::Expect::Any,
        }
    }

    fn systemctl_stop_spec(binaries: &Binaries, uid: u32) -> CommandSpec {
        CommandSpec {
            program: binaries.resolve("systemctl").unwrap().to_path_buf(),
            args: vec!["stop".to_string(), format!("{}.timer", timer::unit_name(uid))],
            env: lang_c(),
            expect: crate::runner::Expect::Any,
        }
    }

    fn systemd_run_spec(binaries: &Binaries, uid: u32, epoch: u64) -> CommandSpec {
        CommandSpec {
            program: binaries.resolve("systemd-run").unwrap().to_path_buf(),
            args: vec![
                format!("--unit={}", timer::unit_name(uid)),
                format!("--description=NoPass expiry for uid {uid}"),
                format!("--on-calendar={}", nopass_core::timefmt::format_utc_rfc3339(epoch)),
                "--timer-property=AccuracySec=1s".to_string(),
                "--timer-property=Persistent=false".to_string(),
                "--timer-property=WakeSystem=false".to_string(),
                "--timer-property=RemainAfterElapse=false".to_string(),
                "--property=Type=oneshot".to_string(),
                nopass_core::paths::HELPER_PATH.to_string(),
                "expire".to_string(),
                "--uid".to_string(),
                uid.to_string(),
            ],
            env: lang_c(),
            expect: crate::runner::Expect::Zero,
        }
    }

    fn ok(status: i32) -> CommandOutcome {
        CommandOutcome { status: Some(status), stdout: vec![], stderr: vec![] }
    }

    // uid 1 (`daemon` on essentially every Linux distro) is used for every
    // admission-success scenario below — this crate's test suite already
    // targets "a real Linux dev/CI machine, not a hermetic filesystem"
    // (see `checks.rs`), and `checks::admit_uid`/`checks::lookup_user`
    // call the real `getpwuid` syscall with no injection point.
    const REAL_UID: u32 = 1;
    fn wide_range() -> UidRange {
        UidRange { min: 1, max: 60_000 }
    }

    // --- enable: context wiring (obligation 2) -----------------------

    #[test]
    fn enable_live_wrapper_rejects_context_when_pkexec_uid_is_absent() {
        // No PKEXEC_UID is set in this test process's real environment —
        // proves `enable`'s call site genuinely reads the live variable
        // rather than an injected stub.
        let (root, layout) = fresh_layout("enable_live_ctx");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]); // nothing must run
        let err = enable(&layout, &runner, &binaries, None, false).unwrap_err();
        assert_eq!(err.exit_code(), 10);
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- resolve_expiry (pure, duration validation, exit 13) ---------

    #[test]
    fn resolve_expiry_defaults_to_never_with_no_flags() {
        assert_eq!(resolve_expiry(None, false, 1_000_000).unwrap(), Expiry::Never);
    }

    #[test]
    fn resolve_expiry_returns_reboot_when_until_reboot_is_set() {
        assert_eq!(resolve_expiry(None, true, 1_000_000).unwrap(), Expiry::Reboot);
    }

    #[test]
    fn resolve_expiry_accepts_an_in_range_until_as_at() {
        let now = 1_000_000;
        assert_eq!(resolve_expiry(Some(now + 3600), false, now).unwrap(), Expiry::At { epoch: now + 3600 });
    }

    #[test]
    fn resolve_expiry_rejects_until_below_the_minimum_with_exit_13() {
        let now = 1_000_000;
        let err = resolve_expiry(Some(now + 30), false, now).unwrap_err();
        assert_eq!(err.exit_code(), 13);
    }

    #[test]
    fn resolve_expiry_rejects_until_above_the_maximum_with_exit_13() {
        let now = 1_000_000;
        let err = resolve_expiry(Some(now + 30_000), false, now).unwrap_err();
        assert_eq!(err.exit_code(), 13);
    }

    #[test]
    fn resolve_expiry_rejects_until_in_the_past_with_exit_13() {
        let now = 1_000_000;
        let err = resolve_expiry(Some(now - 1), false, now).unwrap_err();
        assert_eq!(err.exit_code(), 13);
    }

    // --- enable_inner: admission chain (exit 11/12) -------------------

    #[test]
    fn enable_inner_uid_0_is_rejected_with_exit_11_before_any_command_runs() {
        let (root, layout) = fresh_layout("enable_uid0");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]); // empty script — proves zero mutation
        let err = enable_inner(&layout, &runner, &binaries, 0, "jorge", &wide_range(), Expiry::Never).unwrap_err();
        assert_eq!(err.exit_code(), 11);
        assert!(!layout.rule_path(0).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enable_inner_sudoer_probe_rejection_maps_to_exit_12_with_nothing_created() {
        let (root, layout) = fresh_layout("enable_notsudoer");
        let binaries = fake_binaries(&root);
        let mut outcome = ok(1);
        outcome.stderr = b"jorge is not allowed to run sudo".to_vec();
        let runner = ScriptedRunner::new(vec![(sudo_probe_spec(&binaries, "jorge"), Ok(outcome))]);
        let err =
            enable_inner(&layout, &runner, &binaries, REAL_UID, "jorge", &wide_range(), Expiry::Never).unwrap_err();
        assert_eq!(err.exit_code(), 12);
        assert!(!layout.rule_path(REAL_UID).exists());
        assert!(!layout.rule_tmp_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: obligation 1, sanitized username in probe argv --

    #[test]
    fn enable_inner_sanitizes_the_raw_username_before_the_sudo_probe() {
        let (root, layout) = fresh_layout("enable_sanitize");
        let binaries = fake_binaries(&root);
        // Matches `template.rs`'s own fixture: "a;rm -rf /" sanitizes to
        // "arm-rf" ([A-Za-z0-9._-] survives; ';', ' ', '/' are dropped).
        let raw_user = "a;rm -rf /";
        let sanitized = "arm-rf";
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, sanitized), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
        ]);
        enable_inner(&layout, &runner, &binaries, REAL_UID, raw_user, &wide_range(), Expiry::Never).unwrap();
        let content = std::fs::read_to_string(layout.rule_path(REAL_UID)).unwrap();
        assert!(content.contains("# nopass-user: arm-rf\n"));
        assert!(!content.contains(';'));
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: visudo rejection -> exit 14, tmp unlinked -------

    #[test]
    fn enable_inner_visudo_rejection_unlinks_tmp_and_leaves_final_path_untouched() {
        let (root, layout) = fresh_layout("enable_visudo_reject");
        let binaries = fake_binaries(&root);
        let mut reject = ok(1);
        reject.stderr = b"syntax error near line 4".to_vec();
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(reject)),
        ]);
        let err =
            enable_inner(&layout, &runner, &binaries, REAL_UID, "jorge", &wide_range(), Expiry::Never).unwrap_err();
        assert_eq!(err.exit_code(), 14);
        assert!(!layout.rule_tmp_path(REAL_UID).exists());
        assert!(!layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: rename failure -> exit 16 ------------------------

    #[test]
    fn enable_inner_rename_failure_over_a_symlink_yields_exit_16() {
        let (root, layout) = fresh_layout("enable_rename_fail");
        let binaries = fake_binaries(&root);
        let elsewhere = root.join("elsewhere");
        touch(&elsewhere);
        std::os::unix::fs::symlink(&elsewhere, layout.rule_path(REAL_UID)).unwrap();
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
        ]);
        let err =
            enable_inner(&layout, &runner, &binaries, REAL_UID, "jorge", &wide_range(), Expiry::Never).unwrap_err();
        assert_eq!(err.exit_code(), 16);
        assert!(!layout.rule_tmp_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: systemd-run failure rolls back, exit 17 ----------

    #[test]
    fn enable_inner_systemd_run_failure_rolls_back_the_rule_and_exits_17() {
        let (root, layout) = fresh_layout("enable_timer_fail");
        let binaries = fake_binaries(&root);
        let epoch = unix_now() + 3600;
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(5))),
            (
                systemd_run_spec(&binaries, REAL_UID, epoch),
                Err(RunnerError::NonZero { program: "systemd-run".to_string(), status: 1 }),
            ),
        ]);
        let err = enable_inner(
            &layout,
            &runner,
            &binaries,
            REAL_UID,
            "jorge",
            &wide_range(),
            Expiry::At { epoch },
        )
        .unwrap_err();
        assert!(matches!(err, HelperError::TimerFailed { rolled_back: true }));
        assert_eq!(err.exit_code(), 17);
        assert!(!layout.rule_path(REAL_UID).exists(), "the rule must be rolled back on systemd-run failure");
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: Never/Reboot never call systemd-run --------------

    #[test]
    fn enable_inner_never_and_reboot_never_call_systemd_run() {
        for expiry in [Expiry::Never, Expiry::Reboot] {
            let (root, layout) = fresh_layout("enable_never_reboot");
            let binaries = fake_binaries(&root);
            let runner = ScriptedRunner::new(vec![
                (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
                (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
                (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
            ]);
            enable_inner(&layout, &runner, &binaries, REAL_UID, "jorge", &wide_range(), expiry).unwrap();
            assert!(layout.rule_path(REAL_UID).exists());
            let _ = std::fs::remove_dir_all(&root);
            // `ScriptedRunner`'s `Drop` would panic here if a fourth
            // (systemd-run) call had been attempted against a
            // three-entry script — proof by construction, not merely by
            // assertion.
        }
    }

    // --- enable_inner: full At success, exact systemd-run argv ---------

    #[test]
    fn enable_inner_at_expiry_schedules_the_timer_with_the_exact_pinned_argv() {
        let (root, layout) = fresh_layout("enable_at_happy");
        let binaries = fake_binaries(&root);
        let epoch = unix_now() + 3600;
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
            (systemd_run_spec(&binaries, REAL_UID, epoch), Ok(ok(0))),
        ]);
        enable_inner(&layout, &runner, &binaries, REAL_UID, "jorge", &wide_range(), Expiry::At { epoch }).unwrap();
        let content = std::fs::read_to_string(layout.rule_path(REAL_UID)).unwrap();
        assert!(content.contains(&format!("# nopass-expires: {epoch}\n")));
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- disable ----------------------------------------------------------

    #[test]
    fn disable_live_wrapper_rejects_context_when_pkexec_uid_is_absent() {
        let (root, layout) = fresh_layout("disable_live_ctx");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]);
        let err = disable(&layout, &runner, &binaries).unwrap_err();
        assert_eq!(err.exit_code(), 10);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disable_inner_unlink_precedes_timer_stop_and_removes_an_existing_rule() {
        let (root, layout) = fresh_layout("disable_removes");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::Never);
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        disable_inner(&layout, &runner, &binaries, REAL_UID).unwrap();
        assert!(!layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disable_inner_is_idempotent_when_the_rule_was_already_externally_deleted() {
        let (root, layout) = fresh_layout("disable_idempotent");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        disable_inner(&layout, &runner, &binaries, REAL_UID).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disable_inner_leaves_a_foreign_non_nopass_file_untouched() {
        let (root, layout) = fresh_layout("disable_foreign");
        let binaries = fake_binaries(&root);
        std::fs::write(layout.rule_path(REAL_UID), "foo ALL=(ALL) ALL\n").unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        disable_inner(&layout, &runner, &binaries, REAL_UID).unwrap();
        assert!(layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disable_inner_lock_busy_yields_exit_15() {
        let (root, layout) = fresh_layout("disable_lock_busy");
        let binaries = fake_binaries(&root);
        let _held = LockGuard::acquire(&layout).unwrap();
        let runner = ScriptedRunner::new(vec![]); // no command may run before the lock is even acquired
        let err = disable_inner(&layout, &runner, &binaries, REAL_UID).unwrap_err();
        assert_eq!(err.exit_code(), 15);
        let _ = std::fs::remove_dir_all(&root);
    }

    // Independent-verifier finding: design.md §4.2 orders the unlink
    // before the timer stop specifically so that a genuine unlink
    // failure never leaves a permanent grant with nothing scheduled to
    // revoke it. Every other `disable_inner` test above only exercises
    // the already-absent or successful-removal paths — nothing proved
    // that a real, non-`ENOENT` `remove_rule` failure actually prevents
    // `timer::stop` from running until this test.
    #[test]
    fn disable_inner_removal_failure_propagates_and_never_stops_the_timer() {
        if nix::unistd::geteuid().is_root() {
            // root bypasses the DAC permission check this test relies on
            // to simulate an unlink failure — same convention as
            // `expire_boot_inner_a_per_file_failure_does_not_abort_the_sweep`.
            return;
        }
        let (root, layout) = fresh_layout("disable_removal_fails");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::Never);
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        // POSIX requires write permission on the CONTAINING directory to
        // unlink an entry, regardless of the entry's own permissions —
        // stripping it here induces a genuine, non-ENOENT `remove_file`
        // failure while leaving the rule file itself fully readable.
        std::fs::set_permissions(layout.sudoers_dir(), std::fs::Permissions::from_mode(0o555)).unwrap();

        // Empty script: `ScriptedRunner::run` panics on any unscripted
        // call and its `Drop` panics if the script is left unexhausted —
        // together they prove `timer::stop` is never invoked when the
        // unlink fails, not merely that this test forgot to assert it.
        let runner = ScriptedRunner::new(vec![]);
        let result = disable_inner(&layout, &runner, &binaries, REAL_UID);

        // Restore write permission before any assertion can fail this
        // test early and skip cleanup, leaving a stuck temp directory.
        std::fs::set_permissions(layout.sudoers_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();

        let err = result.unwrap_err();
        assert!(matches!(err, HelperError::Fs(_)), "expected a genuine Fs error, got {err:?}");
        assert!(layout.rule_path(REAL_UID).exists(), "the rule file must survive an unlink failure");
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- resolve_username (pure fallback chain) --------------------------

    #[test]
    fn resolve_username_prefers_a_successful_passwd_lookup() {
        let header = RuleHeader { user: "header-user".to_string(), expires: Expiry::Never };
        assert_eq!(resolve_username(Ok("passwd-user".to_string()), Some(&header)), "passwd-user");
    }

    #[test]
    fn resolve_username_falls_back_to_the_header_user_when_getpwuid_fails() {
        let header = RuleHeader { user: "header-user".to_string(), expires: Expiry::Never };
        assert_eq!(resolve_username(Err(HelperError::Internal("no such uid".to_string())), Some(&header)), "header-user");
    }

    #[test]
    fn resolve_username_falls_back_to_empty_string_when_both_are_unavailable() {
        assert_eq!(resolve_username(Err(HelperError::Internal("no such uid".to_string())), None), "");
    }

    // --- status -------------------------------------------------------

    #[test]
    fn status_live_wrapper_rejects_context_when_pkexec_uid_is_absent() {
        let (root, layout) = fresh_layout("status_live_ctx");
        let err = status(&layout).unwrap_err();
        assert_eq!(err.exit_code(), 10);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_inner_reports_inactive_for_a_missing_rule() {
        let (root, layout) = fresh_layout("status_missing");
        // An implausibly large uid: `getpwuid` deterministically fails,
        // exercising the `unwrap_or_default()` ("") branch too.
        let status = status_inner(&layout, 4_294_967_294, 1_000_000);
        assert!(!status.active);
        assert_eq!(status.expires, None);
        assert_eq!(status.user, "");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_inner_reports_active_with_exact_fields_for_an_existing_valid_rule() {
        let (root, layout) = fresh_layout("status_active");
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 1_789_000_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let status = status_inner(&layout, REAL_UID, 1_789_000_500);
        assert!(status.active);
        assert_eq!(status.user, "jorge");
        assert_eq!(status.expires, Some(Expiry::At { epoch: 1_789_000_000 }));
        assert_eq!(status.rule_path, layout.rule_path(REAL_UID).display().to_string());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_inner_treats_a_foreign_unparseable_file_as_inactive() {
        let (root, layout) = fresh_layout("status_foreign");
        std::fs::write(layout.rule_path(REAL_UID), "not a nopass rule at all\n").unwrap();
        let status = status_inner(&layout, REAL_UID, 1_000_000);
        assert!(!status.active);
        assert_eq!(status.expires, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_inner_never_takes_the_lock_or_creates_any_directory() {
        let (root, layout) = fresh_layout("status_no_lock");
        let _ = status_inner(&layout, REAL_UID, 1_000_000);
        assert!(!layout.lock_path().exists(), "status must never take the mutation lock");
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- expire: context wiring -----------------------------------------

    #[test]
    fn expire_live_wrapper_rejects_context_when_the_real_uid_is_not_root() {
        // The real test-process uid is never 0 in this suite's target
        // environment (a real Linux dev/CI machine, non-root) — proves
        // `expire`'s call site genuinely reads `nix::unistd::getuid()`.
        let (root, layout) = fresh_layout("expire_live_ctx");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]);
        let err = expire(&layout, &runner, &binaries, Some(1000), false).unwrap_err();
        assert_eq!(err.exit_code(), 10);
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- expire_uid_inner (task 7.5) -------------------------------------

    #[test]
    fn expire_uid_inner_absent_rule_is_a_no_op_exit_0() {
        let (root, layout) = fresh_layout("expire_uid_absent");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]);
        expire_uid_inner(&layout, &runner, &binaries, REAL_UID, 1_000_000).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_uid_inner_foreign_file_is_never_deleted() {
        let (root, layout) = fresh_layout("expire_uid_foreign");
        let binaries = fake_binaries(&root);
        std::fs::write(layout.rule_path(REAL_UID), "foo ALL=(ALL) ALL\n").unwrap();
        let runner = ScriptedRunner::new(vec![]);
        expire_uid_inner(&layout, &runner, &binaries, REAL_UID, 1_000_000).unwrap();
        assert!(layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_uid_inner_future_at_is_skipped_not_expired_and_the_file_stays_intact() {
        // The "Stale revocation" threat-matrix row: a stale timer fires
        // after a newer `enable` moved the epoch further into the future.
        let (root, layout) = fresh_layout("expire_uid_future");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 2_000_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![]);
        expire_uid_inner(&layout, &runner, &binaries, REAL_UID, 1_000_000).unwrap();
        assert!(layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_uid_inner_never_and_reboot_headers_are_never_deleted() {
        for expiry in [Expiry::Never, Expiry::Reboot] {
            let (root, layout) = fresh_layout("expire_uid_never_reboot");
            let binaries = fake_binaries(&root);
            let content = render_rule(REAL_UID, "jorge", expiry);
            std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
            let runner = ScriptedRunner::new(vec![]);
            expire_uid_inner(&layout, &runner, &binaries, REAL_UID, 1_000_000).unwrap();
            assert!(layout.rule_path(REAL_UID).exists());
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn expire_uid_inner_deletes_a_genuinely_past_epoch_rule_and_stops_the_timer() {
        let (root, layout) = fresh_layout("expire_uid_past");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 500_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        expire_uid_inner(&layout, &runner, &binaries, REAL_UID, 1_000_000).unwrap();
        assert!(!layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- expire_boot_inner (task 7.6) -------------------------------------

    #[test]
    fn expire_boot_inner_removes_reboot_and_past_at_rules_but_leaves_never_and_future_at() {
        let (root, layout) = fresh_layout("expire_boot_sweep");
        let reboot_uid = 2001;
        let never_uid = 2002;
        let past_uid = 2003;
        let future_uid = 2004;
        std::fs::write(layout.rule_path(reboot_uid), render_rule(reboot_uid, "a", Expiry::Reboot)).unwrap();
        std::fs::write(layout.rule_path(never_uid), render_rule(never_uid, "b", Expiry::Never)).unwrap();
        std::fs::write(
            layout.rule_path(past_uid),
            render_rule(past_uid, "c", Expiry::At { epoch: 500_000 }),
        )
        .unwrap();
        std::fs::write(
            layout.rule_path(future_uid),
            render_rule(future_uid, "d", Expiry::At { epoch: 2_000_000 }),
        )
        .unwrap();

        expire_boot_inner(&layout, None, 1_000_000).unwrap();

        assert!(!layout.rule_path(reboot_uid).exists());
        assert!(layout.rule_path(never_uid).exists());
        assert!(!layout.rule_path(past_uid).exists());
        assert!(layout.rule_path(future_uid).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_boot_inner_restricts_the_sweep_to_only_uid_when_given() {
        let (root, layout) = fresh_layout("expire_boot_restrict");
        let reboot_uid = 4001;
        let past_uid = 4002;
        std::fs::write(layout.rule_path(reboot_uid), render_rule(reboot_uid, "a", Expiry::Reboot)).unwrap();
        std::fs::write(
            layout.rule_path(past_uid),
            render_rule(past_uid, "b", Expiry::At { epoch: 500_000 }),
        )
        .unwrap();

        expire_boot_inner(&layout, Some(reboot_uid), 1_000_000).unwrap();

        assert!(!layout.rule_path(reboot_uid).exists(), "the restricted uid must still be swept");
        assert!(layout.rule_path(past_uid).exists(), "an unrestricted uid must be left alone");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_boot_inner_a_per_file_failure_does_not_abort_the_sweep() {
        if nix::unistd::geteuid().is_root() {
            // root bypasses the DAC permission check this test relies on
            // to simulate an unreadable rule file — see the module-level
            // convention already established by `checks.rs`.
            return;
        }
        let (root, layout) = fresh_layout("expire_boot_partial_failure");
        let ok_uid = 3001;
        let broken_uid = 3002;
        std::fs::write(layout.rule_path(ok_uid), render_rule(ok_uid, "a", Expiry::At { epoch: 500_000 })).unwrap();
        let broken_path = layout.rule_path(broken_uid);
        std::fs::write(&broken_path, render_rule(broken_uid, "b", Expiry::At { epoch: 500_000 })).unwrap();
        std::fs::set_permissions(&broken_path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let err = expire_boot_inner(&layout, None, 1_000_000).unwrap_err();

        assert_eq!(err.exit_code(), 16);
        assert!(!layout.rule_path(ok_uid).exists(), "a failure on one file must not abort the rest of the sweep");
        assert!(broken_path.exists(), "the unreadable file itself is left untouched, not force-removed");

        let _ = std::fs::set_permissions(&broken_path, std::fs::Permissions::from_mode(0o644));
        let _ = std::fs::remove_dir_all(&root);
    }
}

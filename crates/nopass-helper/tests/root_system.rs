//! Root-only integration suite (design.md §8 Integration row; task 10.5).
//!
//! Gated: every test below is admitted only when `NOPASS_ROOT_TESTS=1` is
//! set **and** `geteuid().is_root()` is true — both conditions, never
//! either alone. On any other machine (including the box this suite was
//! authored on) every test here compiles and then SKIPS, printing to
//! stderr exactly why. Run with `-- --nocapture` to see those lines; a
//! reader who does so can tell "ran and passed" (silent) apart from
//! "compiled and skipped" (printed) — see `tests/containers/README.md`
//! for how to actually run this lane.
//!
//! design.md §8's "Lane reconciliation" paragraph moved three
//! delta-spec-labeled-root-only scenarios into the unprivileged
//! `cargo test --workspace` lane instead, because none of them genuinely
//! need uid 0: the flock-busy rejection (`lock.rs`, unprivileged since
//! Phase 6), the `expire --boot` directory-sweep LOGIC (`ops.rs`, Phase
//! 7.6), and binary-candidate resolution with no existing path (`bins.rs`,
//! Phase 4.4). This suite does not re-litigate any of those; it covers
//! only what genuinely requires root, per design.md §8's Integration row
//! plus the concurrency, rollback, and non-root-invocation scenarios the
//! delta specs label root-only: a real `/etc/sudoers.d` write with real
//! `root:root` ownership and mode, a real `visudo -cf` accepting and
//! rejecting, real `getpwuid`/`getgrouplist`, a real `/run/nopass` state
//! file, concurrent `enable` calls contending on the real lock, a
//! rename-failure rollback, and `expire --boot` against real files.
//!
//! Every test that exercises a full `enable`/`disable`/`expire` transaction
//! spawns the REAL compiled `nopass-helper` binary
//! (`env!("CARGO_BIN_EXE_nopass-helper")`) as a child process with
//! `PKEXEC_UID` set in that child's own environment — never by mutating
//! this test process's environment, which `std::env::set_var` cannot do
//! soundly across parallel `#[test]` threads under edition 2024 (the same
//! reasoning `ops.rs`'s own doc comment gives for reading the live
//! environment only at the production call site, never injecting it via a
//! process-global mutation). A handful of tests call `fileops`/`checks`
//! library functions directly instead, where the CLI surface cannot force
//! the exact fixture needed (a malformed candidate for `visudo -cf`, a
//! symlink at the rename target).
//!
//! This suite deliberately mutates real system state: `/etc/sudoers.d/`,
//! `/etc/passwd` (via `useradd`/`userdel`), and `/run/nopass/`. It must
//! only ever run inside a disposable container — see
//! `tests/containers/README.md`. All mutating tests are serialized through
//! `SERIAL` below: they share real system-global paths (`/etc/sudoers.d`,
//! the `/run/nopass/lock` file, `/etc/passwd`), so running them
//! concurrently would produce spurious `LockBusy` failures and `useradd`
//! races unrelated to what each test is actually proving.

#![cfg(unix)]

use std::fs::Permissions;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use nopass_core::expiry::Expiry;
use nopass_core::logindefs;
use nopass_core::paths::Layout;
use nopass_core::state::HelperStatus;
use nopass_core::template::render_rule;
use nopass_helper::bins::Binaries;
use nopass_helper::checks;
use nopass_helper::fileops;
use nopass_helper::lock::LockGuard;
use nopass_helper::runner::SystemRunner;

// --- the gate itself ----------------------------------------------------

/// Pure AND-not-OR gate logic (design.md §8: "gated: skips unless
/// NOPASS_ROOT_TESTS=1 **and** geteuid().is_root()"). Kept standalone so
/// its truth table is provable by `gate_requires_both_conditions_true`
/// below without needing root — that test runs unconditionally on every
/// machine, including this one, and is therefore genuinely executed
/// evidence that the gate cannot be satisfied by either condition alone.
fn gate_satisfied(env_flag_is_one: bool, is_root: bool) -> bool {
    env_flag_is_one && is_root
}

/// The three outcomes `root_gate` can reach for a given
/// `(env_flag_is_one, is_root)` pair. Kept as a pure, unconditionally
/// testable classification — same reasoning as `gate_satisfied` — so the
/// truth table (specifically: which single combination must panic
/// instead of quietly skip) is real, provable evidence rather than a
/// claim (verify-report R / W8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateOutcome {
    Admitted,
    SkippedQuietly,
    /// `NOPASS_ROOT_TESTS=1` is set but the process is not root. This
    /// combination is never a legitimate configuration on its own — it
    /// is always a runner that intended to execute the root-only lane
    /// and did not actually get root (a misconfigured CI job, a
    /// container run without `--privileged`/`--user root`, etc.).
    /// Quietly skipping here is exactly the failure mode that lets a CI
    /// root lane rot green: it would report "13/13 passing" while 12
    /// tests executed zero assertions.
    Misconfigured,
}

fn classify_gate(env_flag_is_one: bool, is_root: bool) -> GateOutcome {
    if gate_satisfied(env_flag_is_one, is_root) {
        GateOutcome::Admitted
    } else if env_flag_is_one && !is_root {
        GateOutcome::Misconfigured
    } else {
        GateOutcome::SkippedQuietly
    }
}

/// Checks the real environment and, when not admitted, either panics
/// (see [`GateOutcome::Misconfigured`]) or prints to stderr exactly which
/// of the two conditions failed and returns quietly. With `--nocapture`
/// the printed line is how a reader tells a genuine, intentional skip
/// apart from a test that ran and found nothing wrong — a suite that
/// silently reports "ok" for a test that never executed a single
/// assertion would read as evidence when it is not. Every skip
/// combination OTHER than `Misconfigured` (neither condition set, or
/// real root without the env flag) is a legitimate developer-machine
/// default and keeps skipping quietly, exactly as before this fix.
fn root_gate(test_name: &str) -> bool {
    let env_flag_is_one = std::env::var("NOPASS_ROOT_TESTS").as_deref() == Ok("1");
    let is_root = nix::unistd::geteuid().is_root();
    match classify_gate(env_flag_is_one, is_root) {
        GateOutcome::Admitted => true,
        GateOutcome::Misconfigured => panic!(
            "{test_name}: NOPASS_ROOT_TESTS=1 is set but this process is not root \
             (geteuid().is_root() == false). This combination is never a legitimate configuration: it means a \
             runner intended to execute the root-only lane but did not actually get root. Run as real root inside \
             tests/containers/Containerfile.{{debian,fedora}} with NOPASS_ROOT_TESTS=1 set, or unset \
             NOPASS_ROOT_TESTS to skip this lane intentionally — see tests/containers/README.md."
        ),
        GateOutcome::SkippedQuietly => {
            eprintln!(
                "SKIPPED {test_name}: root-only gate not satisfied (NOPASS_ROOT_TESTS=1: {env_flag_is_one}, \
                 geteuid().is_root(): {is_root}). This test compiled but executed zero assertions. Run inside \
                 tests/containers/Containerfile.{{debian,fedora}} as root with NOPASS_ROOT_TESTS=1 set — see \
                 tests/containers/README.md."
            );
            false
        }
    }
}

/// Always runs, on every machine: proves the gate genuinely requires BOTH
/// conditions rather than either alone.
#[test]
fn gate_requires_both_conditions_true_before_admitting() {
    assert!(!gate_satisfied(false, false), "neither condition set must never admit");
    assert!(!gate_satisfied(true, false), "the env flag alone must not admit — real root is still required");
    assert!(!gate_satisfied(false, true), "real root alone must not admit — NOPASS_ROOT_TESTS=1 is still required");
    assert!(gate_satisfied(true, true), "both conditions together must admit");
}

/// verify-report R / W8: confirmed RED first — before this fix,
/// `root_gate` had no `Misconfigured` branch at all and every non-admitted
/// combination (including `NOPASS_ROOT_TESTS=1` without real root) quietly
/// returned `false`; the equivalent of this table would have asserted
/// `SkippedQuietly` for that combination too. This is the same
/// "unconditionally provable truth table" pattern
/// `gate_requires_both_conditions_true_before_admitting` already
/// establishes for `gate_satisfied` — it runs on every machine, including
/// this one, with no root and no env var required.
#[test]
fn classify_gate_panics_only_when_the_env_flag_is_set_without_real_root() {
    assert_eq!(classify_gate(false, false), GateOutcome::SkippedQuietly, "neither condition: skip quietly");
    assert_eq!(classify_gate(false, true), GateOutcome::SkippedQuietly, "real root alone: skip quietly");
    assert_eq!(classify_gate(true, true), GateOutcome::Admitted, "both conditions: admitted");
    assert_eq!(
        classify_gate(true, false),
        GateOutcome::Misconfigured,
        "the env flag set without real root is a misconfigured runner, never a quiet skip"
    );
}

/// Wraps a test body so it always compiles and always registers as a real
/// `#[test]`, but returns immediately (printing why, via `root_gate`)
/// unless the root-only gate is satisfied.
macro_rules! root_only_test {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            if !root_gate(stringify!($name)) {
                return;
            }
            $body
        }
    };
}

// --- shared fixtures ------------------------------------------------------

/// Every test that mutates real system-global state (`/etc/sudoers.d`,
/// `/etc/passwd`, `/run/nopass/lock`) takes this for its entire body, so
/// the suite runs as if `--test-threads=1` regardless of the harness's
/// actual thread count.
static SERIAL: Mutex<()> = Mutex::new(());

fn serialize() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn helper_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nopass-helper"))
}

/// Runs the real compiled `nopass-helper` binary with a clean environment
/// (matching how `pkexec`/systemd actually invoke it — see design.md §3),
/// optionally setting `PKEXEC_UID` in the CHILD's own environment. Never
/// mutates this test process's own environment.
fn run_helper(args: &[&str], pkexec_uid: Option<u32>) -> std::process::Output {
    let mut cmd = Command::new(helper_binary());
    cmd.args(args);
    cmd.env_clear();
    if let Some(uid) = pkexec_uid {
        cmd.env("PKEXEC_UID", uid.to_string());
    }
    cmd.output().expect("spawn the real nopass-helper binary")
}

/// Grants `name` a REGULAR (password-required) `ALL=(ALL:ALL) ALL` sudo
/// rule via its own `/etc/sudoers.d/` snippet, so the privilege-admission
/// §Existing-Sudoer Probe (`sudo -n -l -U <user> /bin/sh` exiting 0) admits
/// it. This mirrors NoPass's actual use case exactly: upgrading an
/// ALREADY-legitimate sudoer's interactive password prompt to a
/// temporary/permanent passwordless grant — never creating sudo access
/// from nothing. Validated with a real `visudo -cf` before being installed
/// into the live `/etc/sudoers.d/`, mirroring `write_rule_atomic`'s own
/// validate-before-install discipline; never trust hand-authored sudoers
/// syntax unvalidated against a real system.
fn grant_baseline_sudo(name: &str) {
    let content = format!("{name} ALL=(ALL:ALL) ALL\n");
    let check_path = std::env::temp_dir().join(format!("nopass_root_test_sudoer_check_{name}"));
    std::fs::write(&check_path, &content).expect("write the sudoers.d candidate to a scratch path");
    let checked = Command::new("/usr/sbin/visudo").arg("-cf").arg(&check_path).status();
    let _ = std::fs::remove_file(&check_path);
    assert!(
        checked.map(|s| s.success()).unwrap_or(false),
        "generated sudoers.d snippet for {name} failed a real visudo -cf; refusing to install it"
    );
    let final_path = format!("/etc/sudoers.d/00-nopass-roottest-{name}");
    std::fs::write(&final_path, &content).expect("install the baseline sudo grant");
    std::fs::set_permissions(&final_path, Permissions::from_mode(0o440))
        .expect("mode the baseline sudo grant 0440 — sudo refuses to include a world/group-writable file");
}

fn revoke_baseline_sudo(name: &str) {
    let _ = std::fs::remove_file(format!("/etc/sudoers.d/00-nopass-roottest-{name}"));
}

/// Creates a fresh, disposable system user with no home directory and a
/// baseline (password-required) sudo grant, returning its real uid via a
/// real `getpwnam` lookup. Idempotent: a `userdel` first clears any
/// leftover account from a prior interrupted run of this suite.
fn create_test_user(name: &str) -> u32 {
    let _ = Command::new("userdel").arg("-f").arg(name).status();
    let status = Command::new("useradd").arg("-M").arg(name).status().expect("spawn useradd");
    assert!(status.success(), "useradd {name} failed with status {status:?}");
    let uid = nix::unistd::User::from_name(name)
        .expect("getpwnam syscall")
        .unwrap_or_else(|| panic!("useradd reported success but getpwnam found no entry for {name}"))
        .uid
        .as_raw();
    grant_baseline_sudo(name);
    uid
}

fn delete_test_user(name: &str) {
    revoke_baseline_sudo(name);
    let _ = Command::new("userdel").arg("-f").arg(name).status();
}

/// Best-effort removal of every path a rule/state pair for `uid` could
/// occupy, before and after a test — tolerates a leftover from a prior
/// interrupted run without failing the current one.
fn cleanup_rule_and_state(layout: &Layout, uid: u32) {
    let _ = std::fs::remove_file(layout.rule_path(uid));
    let _ = std::fs::remove_file(layout.rule_tmp_path(uid));
    let _ = std::fs::remove_file(layout.state_path(uid));
    let _ = std::fs::remove_file(layout.state_tmp_path(uid));
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

// --- 1. successful enable: real write, real ownership/mode, real state file

root_only_test!(real_enable_writes_a_root_owned_mode_0440_rule_and_a_mode_0644_state_file, {
    let _guard = serialize();
    let layout = Layout::system();
    let user = "nopasstest01";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);

    // sudoers-rule-lifecycle §Atomic Rule Creation, "Successful atomic
    // write"; helper-cli §Typed Exit Code Mapping, "Successful enable
    // exits 0".
    let output = run_helper(&["enable"], Some(uid));
    assert_eq!(
        output.status.code(),
        Some(0),
        "enable failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let rule_path = layout.rule_path(uid);
    let meta = std::fs::metadata(&rule_path).expect("rule file must exist after a successful enable");
    assert_eq!(meta.permissions().mode() & 0o777, 0o440, "rule file mode must be 0440");
    assert_eq!(meta.uid(), 0, "rule file must be owned by root");
    assert_eq!(meta.gid(), 0, "rule file must be group root");

    // helper-observability §State File Placement and Permissions, "Enable
    // writes the state file with correct mode".
    let state_path = layout.state_path(uid);
    let state_meta = std::fs::metadata(&state_path).expect("state file must exist after a successful enable");
    assert_eq!(state_meta.permissions().mode() & 0o777, 0o644, "state file mode must be 0644");
    let raw = std::fs::read_to_string(&state_path).unwrap();
    let status: HelperStatus = serde_json::from_str(raw.trim_end()).expect("state file must be valid HelperStatus JSON");
    assert_eq!(status.uid, uid);
    assert!(status.active);
    assert_eq!(status.expires, Some(Expiry::Never));

    let _ = run_helper(&["disable"], Some(uid));
    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
});

// --- 1b. status stdout for an active grant ---------------------------------

// verify-report W4: `helper-observability` "HelperStatus JSON Contract,
// stdout and state-file content share the same shape" had no test
// executing the real `status` subcommand's stdout for either case. The
// "missing rule" half is covered unprivileged in
// `tests/process_boundary.rs`; the "active grant" half needs a real,
// root-owned rule file, hence its place here.
root_only_test!(real_status_stdout_for_an_active_grant_matches_the_state_file_exactly, {
    let _guard = serialize();
    let layout = Layout::system();
    let user = "nopasstest01b";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);

    let enable_output = run_helper(&["enable"], Some(uid));
    assert_eq!(enable_output.status.code(), Some(0), "setup: enable must succeed before status is tested");

    let status_output = run_helper(&["status"], Some(uid));
    assert_eq!(
        status_output.status.code(),
        Some(0),
        "status failed: stderr={}",
        String::from_utf8_lossy(&status_output.stderr)
    );
    let stdout = String::from_utf8(status_output.stdout).expect("stdout must be valid UTF-8");
    assert_eq!(stdout.matches('\n').count(), 1, "stdout must be exactly one JSON line: {stdout:?}");

    let stdout_status: HelperStatus =
        serde_json::from_str(stdout.trim_end()).expect("stdout must be valid HelperStatus JSON");
    assert_eq!(stdout_status.uid, uid);
    assert_eq!(stdout_status.user, user);
    assert!(stdout_status.active);
    assert_eq!(stdout_status.expires, Some(Expiry::Never));

    // The state file `enable` just wrote is the other half of "share the
    // same shape" — same serializer, same JSON line, differing only in
    // `updated_at` (each is stamped with its own call's clock read).
    let raw_state = std::fs::read_to_string(layout.state_path(uid)).unwrap();
    let state_status: HelperStatus =
        serde_json::from_str(raw_state.trim_end()).expect("state file must be valid HelperStatus JSON");
    assert_eq!(stdout_status.schema, state_status.schema);
    assert_eq!(stdout_status.uid, state_status.uid);
    assert_eq!(stdout_status.user, state_status.user);
    assert_eq!(stdout_status.active, state_status.active);
    assert_eq!(stdout_status.expires, state_status.expires);
    assert_eq!(stdout_status.rule_path, state_status.rule_path);

    let _ = run_helper(&["disable"], Some(uid));
    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
});

// --- 2. disable removes an existing rule ----------------------------------

root_only_test!(real_disable_removes_an_existing_rule_and_marks_the_state_file_inactive, {
    let _guard = serialize();
    let layout = Layout::system();
    let user = "nopasstest02";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);

    let enable_output = run_helper(&["enable"], Some(uid));
    assert_eq!(enable_output.status.code(), Some(0), "setup: enable must succeed before disable is tested");
    assert!(layout.rule_path(uid).exists(), "setup: rule file must exist before disable");

    // sudoers-rule-lifecycle §Rule Removal, "Disable removes an existing
    // rule".
    let disable_output = run_helper(&["disable"], Some(uid));
    assert_eq!(disable_output.status.code(), Some(0));
    assert!(!layout.rule_path(uid).exists(), "rule file must be gone after disable");

    let raw = std::fs::read_to_string(layout.state_path(uid)).expect("state file must still exist, now inactive");
    let status: HelperStatus = serde_json::from_str(raw.trim_end()).unwrap();
    assert!(!status.active);
    assert_eq!(status.expires, None);

    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
});

// --- 3. real visudo -cf: accept and reject --------------------------------

root_only_test!(real_visudo_cf_accepts_valid_content_and_rejects_malformed_content, {
    let _guard = serialize();
    let layout = Layout::system();
    let binaries = Binaries::system();
    // `fileops::write_rule_atomic` never calls `getpwuid` — these uids are
    // not tied to any real system account.
    let accept_uid = 999_901;
    let reject_uid = 999_902;
    cleanup_rule_and_state(&layout, accept_uid);
    cleanup_rule_and_state(&layout, reject_uid);

    // sudoers-rule-lifecycle §Atomic Rule Creation, "Template renders the
    // fixed format" makes this content syntactically valid; the REAL
    // `/usr/sbin/visudo -cf` (not a fake script) validates it here.
    let valid_content = render_rule(accept_uid, "roottestaccept", Expiry::Never);
    fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, accept_uid, &valid_content)
        .expect("real visudo -cf must accept syntactically valid content");
    assert!(layout.rule_path(accept_uid).exists());
    assert!(!layout.rule_tmp_path(accept_uid).exists());

    // sudoers-rule-lifecycle §Atomic Rule Creation, "visudo -cf rejects a
    // corrupted candidate" (root-only half); helper-cli exit-14 root-only
    // row.
    let malformed = "this line is not valid sudoers syntax at all\n";
    let err = fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, reject_uid, malformed)
        .expect_err("real visudo -cf must reject malformed content");
    assert_eq!(err.exit_code(), 14);
    assert!(!layout.rule_tmp_path(reject_uid).exists(), "tmp must be unlinked after a real rejection");
    assert!(!layout.rule_path(reject_uid).exists(), "final path must never be written on rejection");

    cleanup_rule_and_state(&layout, accept_uid);
    cleanup_rule_and_state(&layout, reject_uid);
});

// --- 4. rename failure over a symlink rolls back --------------------------

root_only_test!(real_rename_failure_over_a_symlink_rolls_back_and_leaves_no_partial_rule, {
    let _guard = serialize();
    let layout = Layout::system();
    let binaries = Binaries::system();
    let uid = 999_903;
    let _ = std::fs::remove_file(layout.rule_path(uid));
    cleanup_rule_and_state(&layout, uid);

    let elsewhere = layout.sudoers_dir().join(".nopass_root_test_rename_elsewhere");
    std::fs::write(&elsewhere, b"decoy, must never be written through").unwrap();
    std::os::unix::fs::symlink(&elsewhere, layout.rule_path(uid)).expect("plant a symlink at the final path");

    // sudoers-rule-lifecycle §Atomic Rule Creation, "Rename fails after
    // successful validation"; helper-cli exit-16 root-only row.
    let content = render_rule(uid, "roottestrename", Expiry::Never);
    let err = fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, uid, &content)
        .expect_err("a symlink at the final path must make write_rule_atomic refuse to rename over it");
    assert_eq!(err.exit_code(), 16);
    assert!(!layout.rule_tmp_path(uid).exists(), "tmp file must be unlinked after the refused rename");
    assert!(
        std::fs::symlink_metadata(layout.rule_path(uid)).unwrap().file_type().is_symlink(),
        "the symlink itself must be left untouched, never renamed over"
    );
    assert_eq!(
        std::fs::read(&elsewhere).unwrap(),
        b"decoy, must never be written through",
        "the symlink's target must never be written through"
    );

    let _ = std::fs::remove_file(layout.rule_path(uid));
    let _ = std::fs::remove_file(&elsewhere);
});

// --- 5. timer scheduling failure rolls back, real exit 17 -----------------

root_only_test!(real_timer_scheduling_failure_rolls_back_the_rule_when_systemd_is_unavailable, {
    let _guard = serialize();
    // Containerfile.debian/.fedora deliberately do not run systemd as PID
    // 1 (design.md §8: "No systemd as PID 1 is required for this lane"),
    // so `systemd-run` genuinely fails here rather than being scripted to
    // fail — this is the real, unforced exit-17 rollback path (helper-cli
    // spec: exit codes 14/16/17 "additionally covered by root-only
    // container tests"; `timer::schedule`'s own doc comment: resolving the
    // binary, spawning it, or a non-zero exit are all mapped identically).
    // The Containerfile.systemd manual lane is where timer scheduling
    // actually succeeds.
    let layout = Layout::system();
    let user = "nopasstest05";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);

    let until = unix_now() + 3600;
    let output = run_helper(&["enable", "--until", &until.to_string()], Some(uid));
    assert_eq!(
        output.status.code(),
        Some(17),
        "expected exit 17 (timer scheduling failure) with no working systemd; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!layout.rule_path(uid).exists(), "the rule must be rolled back, not left behind, on timer failure");

    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
});

// --- 6-7. real getpwuid / getgrouplist ------------------------------------

root_only_test!(real_getpwuid_resolves_a_real_user_and_admits_the_uid_into_range, {
    let _guard = serialize();
    let user = "nopasstest06";
    let uid = create_test_user(user);

    assert_eq!(
        checks::lookup_user(uid).expect("real getpwuid must resolve the just-created user"),
        user,
        "real getpwuid must resolve the created user's name"
    );

    let login_defs = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let range = logindefs::parse(&login_defs);
    assert!(
        checks::admit_uid(uid, &range).is_ok(),
        "a freshly useradd'ed uid must fall within the real /etc/login.defs admissible range"
    );

    delete_test_user(user);
});

root_only_test!(real_getgrouplist_advisory_precheck_runs_against_a_real_user, {
    let _guard = serialize();
    let user = "nopasstest07";
    let uid = create_test_user(user);

    // privilege-admission §Existing-Sudoer Probe: `in_admin_group` is
    // advisory-only and MUST NEVER itself grant admission. A freshly
    // useradd'ed account belongs to no admin group, so this must be
    // false — the point of this test is exercising the REAL
    // `getgrouplist(3)` syscall path (never stubbed in the unprivileged
    // lane), not the boolean outcome by itself.
    assert!(!checks::in_admin_group(user, uid), "a freshly created user must not be in sudo/wheel/admin yet");

    delete_test_user(user);
});

// --- 8. concurrent enable calls contend on the real lock ------------------

root_only_test!(concurrent_enable_calls_serialize_second_caller_gets_lock_busy_and_state_stays_consistent, {
    let _guard = serialize();
    // sudoers-rule-lifecycle §Mutation Serialization, "Concurrent enable
    // requests serialize instead of corrupting the file": the spec's
    // GIVEN/WHEN/THEN wording says the second call "blocks... until the
    // first completes". `lock.rs`'s own doc comment is explicit that this
    // is NOT what is implemented: the flock is
    // `FlockArg::LockExclusiveNonblock`, deliberately fail-fast rather
    // than stalling a polkit dialog (design.md §3 Architecture Decision:
    // "flock on /run/nopass/lock" is non-blocking by design). Design.md is
    // authoritative over the spec's literal wording here (the same
    // precedent task 7.1 already applied to the `--timer-property` count).
    // What this test proves is the property the scenario actually cares
    // about: the two invocations never interleave writes and never
    // corrupt file state — the second is cleanly rejected (exit 15)
    // rather than silently racing the first.
    let layout = Layout::system();
    let user = "nopasstest08";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);

    let held = LockGuard::acquire(&layout).expect("this test holds the real production lock path directly");

    let output = run_helper(&["enable"], Some(uid));
    assert_eq!(
        output.status.code(),
        Some(15),
        "a concurrent enable while the lock is held must fail fast with exit 15 (LockBusy), never corrupt state"
    );
    assert!(!layout.rule_path(uid).exists(), "nothing may be written while the lock is held by another caller");

    drop(held);

    let retry = run_helper(&["enable"], Some(uid));
    assert_eq!(retry.status.code(), Some(0), "once the lock is released, the same request must succeed cleanly");
    assert!(layout.rule_path(uid).exists());

    let _ = run_helper(&["disable"], Some(uid));
    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
});

// --- 9. expire --uid: real past-epoch deletion, real future-epoch retention

root_only_test!(real_expire_uid_deletes_a_past_epoch_rule_and_leaves_a_future_epoch_rule_intact, {
    let _guard = serialize();
    let layout = Layout::system();
    let binaries = Binaries::system();
    let past_uid = 999_909;
    let future_uid = 999_910;
    cleanup_rule_and_state(&layout, past_uid);
    cleanup_rule_and_state(&layout, future_uid);

    let now = unix_now();
    let past_content = render_rule(past_uid, "roottestexpire", Expiry::At { epoch: now.saturating_sub(120) });
    let future_content = render_rule(future_uid, "roottestexpire", Expiry::At { epoch: now + 3600 });
    fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, past_uid, &past_content).unwrap();
    fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, future_uid, &future_content).unwrap();

    // expiry-policy §Expiry Re-validation Before Deletion, "Expire deletes
    // a genuinely past-epoch rule". `expire` requires PKEXEC_UID unset and
    // a real root invocation (privilege-admission §UID Resolution by
    // Invocation Context) — this test process already satisfies both.
    let past_out = run_helper(&["expire", "--uid", &past_uid.to_string()], None);
    assert_eq!(past_out.status.code(), Some(0));
    assert!(!layout.rule_path(past_uid).exists(), "a genuinely past-epoch rule must be deleted");

    // "Stale timer fires after a newer enable" (future-epoch retention
    // half): a still-future epoch is a no-op, not an error.
    let future_out = run_helper(&["expire", "--uid", &future_uid.to_string()], None);
    assert_eq!(future_out.status.code(), Some(0));
    assert!(layout.rule_path(future_uid).exists(), "a still-future epoch rule must be left intact");

    cleanup_rule_and_state(&layout, past_uid);
    cleanup_rule_and_state(&layout, future_uid);
});

// --- 10. expire --boot against real files ---------------------------------

root_only_test!(real_boot_sweep_removes_only_stale_and_reboot_marked_rules_among_real_files, {
    let _guard = serialize();
    // The sweep LOGIC (reboot/past-epoch removed, never/future-epoch
    // untouched) is already proven unprivileged against a `TempDir`
    // (Phase 7.6, `ops.rs`'s own test suite) — design.md §8's lane
    // reconciliation places it there deliberately, since it does not need
    // root. This test is the end-to-end confirmation that the real
    // `expire --boot` dispatch (real uid-0 context, real
    // `/etc/sudoers.d`) reaches that same logic against real files
    // (expiry-policy §Boot-Time Cleanup Sweep, "Boot sweep removes only
    // stale and reboot-marked rules").
    let layout = Layout::system();
    let binaries = Binaries::system();
    let reboot_uid = 999_911;
    let past_uid = 999_912;
    let permanent_uid = 999_913;
    for uid in [reboot_uid, past_uid, permanent_uid] {
        cleanup_rule_and_state(&layout, uid);
    }

    let now = unix_now();
    let reboot_content = render_rule(reboot_uid, "roottestsweep", Expiry::Reboot);
    let past_content = render_rule(past_uid, "roottestsweep", Expiry::At { epoch: now.saturating_sub(120) });
    let permanent_content = render_rule(permanent_uid, "roottestsweep", Expiry::Never);
    fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, reboot_uid, &reboot_content).unwrap();
    fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, past_uid, &past_content).unwrap();
    fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, permanent_uid, &permanent_content).unwrap();

    let output = run_helper(&["expire", "--boot"], None);
    assert_eq!(output.status.code(), Some(0));
    assert!(!layout.rule_path(reboot_uid).exists(), "a reboot-marked rule must be removed at boot");
    assert!(!layout.rule_path(past_uid).exists(), "a past-epoch rule must be removed at boot");
    assert!(layout.rule_path(permanent_uid).exists(), "a permanent (Never) rule must survive the boot sweep");

    for uid in [reboot_uid, past_uid, permanent_uid] {
        cleanup_rule_and_state(&layout, uid);
    }
});

// --- 11. /run/nopass missing at helper start ------------------------------

root_only_test!(real_run_nopass_directory_is_created_at_0755_when_missing_before_enable, {
    let _guard = serialize();
    let layout = Layout::system();
    let user = "nopasstest11";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);
    // Simulates a boot where `nopass.tmpfiles.conf` has not yet run
    // (helper-observability §State File Placement and Permissions,
    // "/run/nopass/ missing at helper start"). Best-effort: any other real
    // state file left over from an interrupted earlier test run is lost
    // with it — acceptable inside a disposable container.
    let _ = std::fs::remove_dir_all("/run/nopass");
    assert!(!Path::new("/run/nopass").exists(), "setup: /run/nopass must be genuinely absent before this test");

    let output = run_helper(&["enable"], Some(uid));
    assert_eq!(output.status.code(), Some(0), "the sudoers rule outcome must be unaffected by a missing /run/nopass");
    let run_dir_meta = std::fs::metadata("/run/nopass").expect("the helper must create /run/nopass itself");
    assert!(run_dir_meta.is_dir());
    assert_eq!(run_dir_meta.permissions().mode() & 0o777, 0o755, "/run/nopass must be created at mode 0755");
    assert!(layout.state_path(uid).exists(), "the state file must still be written despite the missing directory");

    let _ = run_helper(&["disable"], Some(uid));
    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
});

// --- 12. expire as a genuinely non-root invocation ------------------------

root_only_test!(real_expire_as_non_root_without_pkexec_uid_is_rejected_with_exit_10, {
    let _guard = serialize();
    // privilege-admission §UID Resolution by Invocation Context, "Expire
    // invoked as non-root without PKEXEC_UID" — explicitly labeled
    // root-only because it requires a genuinely non-root real invocation,
    // which is only reachable once this whole suite is already running as
    // real root (able to demote a child via `Command::uid`, a real
    // `setuid(2)` before `exec`, not a simulation).
    let user = "nopasstest12";
    let uid = create_test_user(user);

    let mut cmd = Command::new(helper_binary());
    cmd.args(["expire", "--uid", "1000"]);
    cmd.env_clear();
    cmd.uid(uid);
    let output = cmd.output().expect("spawn the real nopass-helper binary demoted to a non-root uid");
    assert_eq!(
        output.status.code(),
        Some(10),
        "a non-root, PKEXEC_UID-unset expire invocation must exit 10 with no system change"
    );

    delete_test_user(user);
});

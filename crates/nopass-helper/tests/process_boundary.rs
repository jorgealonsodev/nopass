//! Unprivileged process-boundary tests: spawn the real compiled
//! `nopass-helper` binary and assert its process exit code / stdout.
//!
//! verify-report C1: every existing admission test drives `enable_inner`
//! directly, which is handed an already-resolved `raw_user` — so the
//! production `enable` wrapper's own step order (`ops.rs` L88-94:
//! duration validate, then the `getpwuid` lookup, then admission) was
//! never verified at the point that actually decides the process's exit
//! code. `enable_with_a_pkexec_uid_absent_from_getpwuid_exits_11_not_1`
//! below closes exactly that seam by spawning the real binary, the same
//! way `tests/root_system.rs` already does for its own (root-gated)
//! scenarios.
//!
//! verify-report W4: `helper-observability`'s "stdout and state-file
//! content share the same shape" scenario had no test executing the real
//! `status` subcommand's stdout at all — only the struct-returning
//! `status_inner` was tested. `status_stdout_for_a_missing_rule_is_the_
//! exact_inactive_json_line` below closes the unprivileged half of that
//! gap; the "active grant" half needs a real, root-owned rule file and is
//! covered by `tests/root_system.rs`'s root-gated lane instead.
//!
//! Every test here is deliberately root-free: admission is decided (or a
//! missing rule is reported) before any privileged filesystem write, so
//! none of these scenarios ever touch `/etc/sudoers.d` or `/run/nopass`
//! on the machine running this suite.

use std::process::Command;

use nopass_core::state::HelperStatus;

fn helper_binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_nopass-helper"))
}

/// Mirrors `tests/root_system.rs`'s own `run_helper`: a clean child
/// environment (matching how `pkexec`/systemd actually invoke this
/// binary — design.md §3), `PKEXEC_UID` set only in the CHILD's own
/// environment, never in this test process's.
///
/// Also sets `journal::DISABLE_JOURNALD_ENV`: every test below spawns
/// the REAL compiled binary on an unprivileged, otherwise-ungated host
/// (unlike `root_system.rs`/`root_journal.rs`, which only ever run
/// inside a disposable container) purely to check exit codes and
/// stdout, never the audit trail. Without this, a host that genuinely
/// has a live journald socket would have each spawn write a real,
/// fabricated audit record as an unrelated side effect of testing
/// process-boundary behaviour.
fn run_helper(args: &[&str], pkexec_uid: Option<u32>) -> std::process::Output {
    let mut cmd = Command::new(helper_binary());
    cmd.args(args);
    cmd.env_clear();
    cmd.env(nopass_helper::journal::DISABLE_JOURNALD_ENV, "1");
    if let Some(uid) = pkexec_uid {
        cmd.env("PKEXEC_UID", uid.to_string());
    }
    cmd.output().expect("spawn the real nopass-helper binary")
}

/// verify-report C1's exact live reproduction: a `PKEXEC_UID` with no
/// `getpwuid` entry must exit 11 (`UidRejection::Unknown`, the
/// documented "uid not present in getpwuid" cause), never 1
/// (`HelperError::Internal`) — the fail-closed outcome is identical
/// either way (nothing written), but the two exit codes have documented,
/// distinct meanings and the M2 tray is specified to react differently
/// to each.
#[test]
fn enable_with_a_pkexec_uid_absent_from_getpwuid_exits_11_not_1() {
    let missing_uid = 4_294_967_294u32;
    let output = run_helper(&["enable"], Some(missing_uid));
    assert_eq!(
        output.status.code(),
        Some(11),
        "expected exit 11 (UidRejection::Unknown), got {:?}; stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !std::path::Path::new(&format!("/etc/sudoers.d/90-nopass-{missing_uid}")).exists(),
        "a rejected admission must never write a rule file"
    );
}

/// Regression guard for the sibling seam this same production wrapper
/// owns: with no `PKEXEC_UID` in the environment at all, context
/// resolution (design.md §4.1 step 3) must reject before admission is
/// ever attempted.
#[test]
fn enable_with_pkexec_uid_missing_still_exits_10() {
    let output = run_helper(&["enable"], None);
    assert_eq!(output.status.code(), Some(10));
}

/// A `PKEXEC_UID` present but genuinely admitted (uid 65534 / `nobody`
/// exceeds the default `UID_MAX`) must still reach `UidRejection::AboveMax`
/// (exit 11) through the same wrapper path — pinned here so the C1 fix's
/// new `Ok(None)` branch in `ops::enable` cannot be mistaken for the only
/// path that produces exit 11.
#[test]
fn enable_with_pkexec_uid_65534_still_exits_11_via_the_range_check() {
    let output = run_helper(&["enable"], Some(65_534));
    assert_eq!(output.status.code(), Some(11));
}

/// verify-report W4: no test previously executed the real `status`
/// subcommand's stdout. This uid is implausibly large, so it almost
/// certainly has no NoPass rule anywhere on the machine running this
/// suite — the "missing rule" half of "HelperStatus JSON Contract, stdout
/// and state-file content share the same shape".
#[test]
fn status_stdout_for_a_missing_rule_is_the_exact_inactive_json_line() {
    let uid = 4_294_967_293u32;
    let output = run_helper(&["status"], Some(uid));
    assert_eq!(
        output.status.code(),
        Some(0),
        "status must never fail even for a missing rule; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be valid UTF-8");
    assert_eq!(stdout.matches('\n').count(), 1, "stdout must be exactly one JSON line: {stdout:?}");
    assert!(stdout.ends_with('\n'));

    let status: HelperStatus = serde_json::from_str(stdout.trim_end())
        .unwrap_or_else(|e| panic!("stdout was not a valid HelperStatus JSON line: {e}; stdout={stdout:?}"));
    assert_eq!(status.schema, 1);
    assert_eq!(status.uid, uid);
    assert!(!status.active);
    assert_eq!(status.expires, None);
    assert_eq!(status.user, "", "an unresolvable uid must fall back to the empty-string username");
    assert_eq!(status.rule_path, format!("/etc/sudoers.d/90-nopass-{uid}"));
}

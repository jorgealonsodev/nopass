//! Journald read-back suite (design.md §0 G1, §8 "Testing strategy, per
//! lane"; task 8.3-8.6). Proves the `NOPASS_CONTEXT`/`NOPASS_UID`
//! wiring Phase 7 already implements against a REAL `systemd-journald`
//! read back through `journalctl` — not against our own writer, which
//! `helper-observability`'s scenarios explicitly call out as
//! insufficient ("a real journald is needed to read the record back;
//! asserting on our own writer proves only what we wrote").
//!
//! Gated on `NOPASS_JOURNAL_TESTS=1` alone — unlike `root_system.rs`'s
//! AND-gate (`env flag` **and** `geteuid().is_root()`), this lane's own
//! entrypoint (`tests/containers/Containerfile.journald`) only ever runs
//! this binary as real root with a real standalone journald already
//! started, so a second root check would only ever agree with the first.
//! On any other machine every test below compiles and skips, printing
//! why with `--nocapture`, matching Lane B's `dbus_session.rs` pattern.
//!
//! Every mutating test here contends on the SAME real, global
//! `/run/nopass/lock` path `enable_inner`/`disable_inner` always acquire
//! (`nopass_core::paths::Layout::system().lock_path()`) — `SERIAL`
//! below serializes them exactly the way `root_system.rs`'s own
//! `SERIAL` does, for the identical reason: concurrent `#[test]` threads
//! would otherwise race for that one lock and produce spurious
//! `LockBusy` failures unrelated to what each test actually proves.
//! Each test still uses its own disposable user/uid, so the journalctl
//! queries below are correct independent of `cargo test`'s (undefined)
//! execution order.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use nopass_core::paths::Layout;

fn lane_active() -> bool {
    std::env::var("NOPASS_JOURNAL_TESTS").ok().as_deref() == Some("1")
}

macro_rules! skip_unless_journal_lane {
    ($name:expr) => {
        if !lane_active() {
            eprintln!(
                "skipping {}: set NOPASS_JOURNAL_TESTS=1 (inside Containerfile.journald) to run",
                $name
            );
            return;
        }
    };
}

static SERIAL: Mutex<()> = Mutex::new(());

fn serialize() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// --- shared fixtures (trimmed copies of root_system.rs's own helpers;
// see that file for the full rationale each one restates in brief here)

fn helper_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nopass-helper"))
}

/// Runs the real compiled `nopass-helper` binary with a clean
/// environment, optionally setting `PKEXEC_UID` in the CHILD's own
/// environment (never this test process's).
fn run_helper(args: &[&str], pkexec_uid: Option<u32>) -> std::process::Output {
    let mut cmd = Command::new(helper_binary());
    cmd.args(args);
    cmd.env_clear();
    if let Some(uid) = pkexec_uid {
        cmd.env("PKEXEC_UID", uid.to_string());
    }
    cmd.output().expect("spawn the real nopass-helper binary")
}

/// A REGULAR (password-required) baseline sudo grant, validated with a
/// real `visudo -cf` before install — the same precondition
/// `root_system.rs::grant_baseline_sudo` documents: `enable`/`grant`
/// require an already-legitimate sudoer, never granting from nothing.
fn grant_baseline_sudo(name: &str) {
    let content = format!("{name} ALL=(ALL:ALL) ALL\n");
    let check_path = std::env::temp_dir().join(format!("nopass_journal_test_sudoer_check_{name}"));
    std::fs::write(&check_path, &content).expect("write the sudoers.d candidate to a scratch path");
    let checked = Command::new("/usr/sbin/visudo")
        .arg("-cf")
        .arg(&check_path)
        .status();
    let _ = std::fs::remove_file(&check_path);
    assert!(
        checked.map(|s| s.success()).unwrap_or(false),
        "generated sudoers.d snippet for {name} failed a real visudo -cf; refusing to install it"
    );
    let final_path = format!("/etc/sudoers.d/00-nopass-journaltest-{name}");
    std::fs::write(&final_path, &content).expect("install the baseline sudo grant");
    std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o440))
        .expect("mode the baseline sudo grant 0440");
}

fn create_test_user(name: &str) -> u32 {
    let _ = Command::new("userdel").arg("-f").arg(name).status();
    let status = Command::new("useradd")
        .arg("-M")
        .arg(name)
        .status()
        .expect("spawn useradd");
    assert!(
        status.success(),
        "useradd {name} failed with status {status:?}"
    );
    let uid = nix::unistd::User::from_name(name)
        .expect("getpwnam syscall")
        .unwrap_or_else(|| {
            panic!("useradd reported success but getpwnam found no entry for {name}")
        })
        .uid
        .as_raw();
    grant_baseline_sudo(name);
    uid
}

fn delete_test_user(name: &str) {
    let _ = std::fs::remove_file(format!("/etc/sudoers.d/00-nopass-journaltest-{name}"));
    let _ = Command::new("userdel").arg("-f").arg(name).status();
}

fn cleanup_rule_and_state(layout: &Layout, uid: u32) {
    let _ = std::fs::remove_file(layout.rule_path(uid));
    let _ = std::fs::remove_file(layout.rule_tmp_path(uid));
    let _ = std::fs::remove_file(layout.state_path(uid));
    let _ = std::fs::remove_file(layout.state_tmp_path(uid));
}

// --- journalctl read-back -------------------------------------------------

/// Runs `journalctl --no-pager -o json <matches...>` and parses every
/// line of the (possibly empty) output as one JSON object — `journalctl`
/// matches on different field names AND together, matches on the same
/// field name OR together, per its own documented semantics.
fn journalctl_json(matches: &[String]) -> Vec<serde_json::Value> {
    let mut cmd = Command::new("journalctl");
    cmd.arg("--no-pager").arg("-o").arg("json");
    for m in matches {
        cmd.arg(m);
    }
    let output = cmd.output().expect("spawn journalctl");
    assert!(
        output.status.success(),
        "journalctl {matches:?} failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l)
                .unwrap_or_else(|e| panic!("journalctl produced invalid JSON line {l:?}: {e}"))
        })
        .collect()
}

/// Polls `journalctl_json` up to 2s (matching the entrypoint's own
/// journald-startup grace period) so a real but not-yet-indexed record
/// does not read as a false negative. Returns whatever was last read —
/// including empty, when the caller's own point is that nothing matches.
fn poll_journal(matches: &[String]) -> Vec<serde_json::Value> {
    for _ in 0..20 {
        let entries = journalctl_json(matches);
        if !entries.is_empty() {
            return entries;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Vec::new()
}

fn field(entry: &serde_json::Value, name: &str) -> String {
    entry
        .get(name)
        .unwrap_or_else(|| panic!("journal entry missing field {name}: {entry}"))
        .as_str()
        .unwrap_or_else(|| panic!("journal field {name} is not a string: {entry}"))
        .to_string()
}

// --- 8.3: a successful enable is journaled --------------------------------

#[test]
fn successful_enable_is_journaled() {
    skip_unless_journal_lane!("successful_enable_is_journaled");
    let _guard = serialize();
    let layout = Layout::system();
    let user = "nopassjournal01";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);

    let output = run_helper(&["enable"], Some(uid));
    assert_eq!(
        output.status.code(),
        Some(0),
        "enable failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let entries = poll_journal(&[
        "SYSLOG_IDENTIFIER=nopass-helper".to_string(),
        format!("NOPASS_UID={uid}"),
    ]);
    assert!(
        !entries.is_empty(),
        "journalctl found no SYSLOG_IDENTIFIER=nopass-helper record for uid {uid} after a successful enable"
    );
    let last = entries.last().unwrap();
    assert_eq!(field(last, "NOPASS_EVENT"), "enable");
    assert_eq!(field(last, "NOPASS_OUTCOME"), "ok");
    assert_eq!(
        field(last, "NOPASS_CONTEXT"),
        "Pkexec",
        "enable is always Pkexec-context, never SystemRoot"
    );

    let _ = run_helper(&["disable"], Some(uid));
    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
}

// --- 8.4: a root-invoked grant is journaled with SystemRoot + its target -

#[test]
fn root_invoked_grant_is_journaled_with_system_root_context_and_explicit_target() {
    skip_unless_journal_lane!(
        "root_invoked_grant_is_journaled_with_system_root_context_and_explicit_target"
    );
    let _guard = serialize();
    let layout = Layout::system();
    let user = "nopassjournal02";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);

    // No PKEXEC_UID: real uid 0 with the env var unset resolves
    // InvocationContext::SystemRoot (uid.rs), exactly the precondition
    // helper-observability's scenario names.
    let output = run_helper(
        &["grant", "--uid", &uid.to_string(), "--until-reboot"],
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "grant --uid {uid} failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let entries = poll_journal(&[
        "NOPASS_CONTEXT=SystemRoot".to_string(),
        format!("NOPASS_UID={uid}"),
    ]);
    assert!(
        !entries.is_empty(),
        "journalctl found no NOPASS_CONTEXT=SystemRoot record for uid {uid} after a successful grant"
    );
    let last = entries.last().unwrap();
    assert_eq!(field(last, "NOPASS_EVENT"), "grant");
    assert_eq!(field(last, "NOPASS_UID"), uid.to_string());
    assert_eq!(field(last, "NOPASS_OUTCOME"), "ok");

    let _ = run_helper(&["revoke", "--uid", &uid.to_string()], None);
    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
}

// --- 8.5: negative control — a context value never written for THIS uid --

/// Without this, `root_invoked_grant_is_journaled_with_system_root_
/// context_and_explicit_target` above could pass on a query that
/// matches every record regardless of `NOPASS_CONTEXT`'s actual value.
/// Scoped by uid (rather than a bare `NOPASS_CONTEXT=Pkexec` query with
/// nothing else) so this control is correct regardless of what other
/// tests in this same suite/journal have written — `cargo test`'s
/// execution order across test binaries and threads is not guaranteed,
/// and `successful_enable_is_journaled` above genuinely does write a
/// `Pkexec`-context record, just for a different uid.
#[test]
fn a_context_value_never_written_for_this_uid_matches_nothing() {
    skip_unless_journal_lane!("a_context_value_never_written_for_this_uid_matches_nothing");
    let _guard = serialize();
    let layout = Layout::system();
    let user = "nopassjournal03";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);

    let output = run_helper(
        &["grant", "--uid", &uid.to_string(), "--until-reboot"],
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "setup: grant --uid {uid} must succeed before the negative control runs"
    );

    // Positive half first: confirm journald HAS indexed a SystemRoot
    // record for this uid, so the empty result below is proven to mean
    // "this exact combination was never written", not "journald has not
    // caught up yet".
    let positive = poll_journal(&[
        "NOPASS_CONTEXT=SystemRoot".to_string(),
        format!("NOPASS_UID={uid}"),
    ]);
    assert!(
        !positive.is_empty(),
        "setup: expected an already-indexed SystemRoot record for uid {uid}"
    );

    // Negative half: the mismatched (never-written) combination for the
    // SAME uid must return zero matches.
    let mismatched = journalctl_json(&[
        "NOPASS_CONTEXT=Pkexec".to_string(),
        format!("NOPASS_UID={uid}"),
    ]);
    assert!(
        mismatched.is_empty(),
        "journalctl NOPASS_CONTEXT=Pkexec NOPASS_UID={uid} matched {} record(s); this uid was only ever \
         invoked via grant (SystemRoot) in this test, so any match proves the CONTEXT field is not being \
         discriminated on correctly",
        mismatched.len()
    );

    let _ = run_helper(&["revoke", "--uid", &uid.to_string()], None);
    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
}

// --- 8.6: revoke and inspect are journaled the same way -------------------

#[test]
fn revoke_and_inspect_are_journaled_the_same_way() {
    skip_unless_journal_lane!("revoke_and_inspect_are_journaled_the_same_way");
    let _guard = serialize();
    let layout = Layout::system();
    let user = "nopassjournal04";
    let uid = create_test_user(user);
    cleanup_rule_and_state(&layout, uid);

    let setup = run_helper(
        &["grant", "--uid", &uid.to_string(), "--until-reboot"],
        None,
    );
    assert_eq!(
        setup.status.code(),
        Some(0),
        "setup: grant --uid {uid} must succeed before revoke/inspect are tested"
    );

    let revoke_output = run_helper(&["revoke", "--uid", &uid.to_string()], None);
    assert_eq!(
        revoke_output.status.code(),
        Some(0),
        "revoke --uid {uid} failed: stderr={}",
        String::from_utf8_lossy(&revoke_output.stderr)
    );

    let inspect_output = run_helper(&["inspect", "--uid", &uid.to_string()], None);
    assert_eq!(
        inspect_output.status.code(),
        Some(0),
        "inspect --uid {uid} failed: stderr={}",
        String::from_utf8_lossy(&inspect_output.stderr)
    );

    let revoke_entries = poll_journal(&[
        "NOPASS_EVENT=revoke".to_string(),
        format!("NOPASS_UID={uid}"),
    ]);
    assert!(
        !revoke_entries.is_empty(),
        "journalctl found no NOPASS_EVENT=revoke record for uid {uid}"
    );
    let revoke_last = revoke_entries.last().unwrap();
    assert_eq!(field(revoke_last, "NOPASS_CONTEXT"), "SystemRoot");
    assert_eq!(field(revoke_last, "NOPASS_OUTCOME"), "ok");

    let inspect_entries = poll_journal(&[
        "NOPASS_EVENT=inspect".to_string(),
        format!("NOPASS_UID={uid}"),
    ]);
    assert!(
        !inspect_entries.is_empty(),
        "journalctl found no NOPASS_EVENT=inspect record for uid {uid}"
    );
    let inspect_last = inspect_entries.last().unwrap();
    assert_eq!(field(inspect_last, "NOPASS_CONTEXT"), "SystemRoot");
    assert_eq!(field(inspect_last, "NOPASS_OUTCOME"), "ok");

    cleanup_rule_and_state(&layout, uid);
    delete_test_user(user);
}

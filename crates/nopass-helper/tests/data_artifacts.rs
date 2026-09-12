//! Static assertions over the `data/` install artifacts (privilege-admission
//! §Single Polkit Action; design.md §7).
//!
//! These files are never installed by this workspace — packaging (M4) owns
//! that — so nothing else in the build exercises their content. This suite
//! is the only thing that pins the security-relevant defaults: the single
//! polkit action id and its `allow_*`/`exec.path` values, and the
//! `nopass-cleanup.service` ordering that keeps a stale grant from
//! surviving into a new login.
//!
//! Deliberately plain `str` assertions: no XML parser, no `regex` crate.
//! design.md's Architecture Decisions already rule out `regex` workspace-wide,
//! and these are four short, hand-authored files — a parsing dependency
//! would be the wrong trade for what `contains`/`matches` already proves.

use std::path::{Path, PathBuf};

fn data_file(name: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data").join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read data/{name} at {}: {e}", path.display()))
}

/// privilege-admission §Single Polkit Action, scenario "Installed policy
/// declares the single action with required defaults": exactly one
/// `<action id=` — a second action would be a second, unaudited door into
/// the privileged helper.
#[test]
fn policy_declares_exactly_one_action() {
    let policy = data_file("com.enfoquestic.nopass.policy");
    let action_count = policy.matches("<action id=").count();
    assert_eq!(action_count, 1, "expected exactly one <action id=, found {action_count}");
}

/// The action id must be the fixed `com.enfoquestic.nopass.manage` — the id
/// polkit matches the pkexec call against.
#[test]
fn policy_action_id_is_fixed() {
    let policy = data_file("com.enfoquestic.nopass.policy");
    assert!(
        policy.contains(r#"<action id="com.enfoquestic.nopass.manage">"#),
        "missing action id com.enfoquestic.nopass.manage"
    );
}

/// The three `allow_*` defaults are the entire remote-and-inactive-session
/// defence: `allow_any=no` keeps a remote (non-local) session out,
/// `allow_inactive=no` keeps a switched-away session out, and
/// `allow_active=auth_admin_keep` requires admin auth for the active
/// session, cached for the polkit-configured keep window.
#[test]
fn policy_has_required_allow_defaults() {
    let policy = data_file("com.enfoquestic.nopass.policy");
    assert!(policy.contains("<allow_any>no</allow_any>"), "allow_any must be no");
    assert!(policy.contains("<allow_inactive>no</allow_inactive>"), "allow_inactive must be no");
    assert!(
        policy.contains("<allow_active>auth_admin_keep</allow_active>"),
        "allow_active must be auth_admin_keep"
    );
}

/// `exec.path` must point at the M1 default helper path. Packaging (M4)
/// rewrites this value for Arch (`/usr/lib/nopass/nopass-helper`); a future
/// mismatch against that rewritten path is packaging's concern, not a
/// regression of this file.
#[test]
fn policy_exec_path_is_default_helper_path() {
    let policy = data_file("com.enfoquestic.nopass.policy");
    assert!(
        policy.contains(
            r#"<annotate key="org.freedesktop.policykit.exec.path">/usr/libexec/nopass-helper</annotate>"#
        ),
        "exec.path annotation must point at the default /usr/libexec/nopass-helper"
    );
}

/// design.md §4.4 / §7: the cleanup unit is a `oneshot` gated by
/// `ConditionPathExistsGlob` so it is a no-op on the overwhelmingly common
/// boot with no NoPass rule, and it runs the boot sweep via
/// `expire --boot`.
#[test]
fn cleanup_service_is_oneshot_with_condition_and_exec() {
    let unit = data_file("nopass-cleanup.service");
    assert!(unit.contains("Type=oneshot"), "cleanup unit must be Type=oneshot");
    assert!(
        unit.contains("ConditionPathExistsGlob=/etc/sudoers.d/90-nopass-*"),
        "cleanup unit must gate on ConditionPathExistsGlob=/etc/sudoers.d/90-nopass-*"
    );
    assert!(
        unit.contains("ExecStart=/usr/libexec/nopass-helper expire --boot"),
        "cleanup unit must run the boot sweep via expire --boot"
    );
}

/// `Before=systemd-user-sessions.service display-manager.service` is the
/// load-bearing ordering: no user session can begin before a `reboot`-
/// scoped or expired grant is removed. Losing this ordering would let a
/// stale sudo grant be live while a user logs in.
#[test]
fn cleanup_service_orders_before_login_and_display_manager() {
    let unit = data_file("nopass-cleanup.service");
    assert!(
        unit.contains("Before=systemd-user-sessions.service display-manager.service"),
        "cleanup unit must order Before= systemd-user-sessions.service and display-manager.service"
    );
}

/// tmpfiles.d line creating `/run/nopass` at the mode and ownership the
/// helper's state-file and lock code assume.
#[test]
fn tmpfiles_creates_run_nopass_directory() {
    let tmpfiles = data_file("nopass.tmpfiles.conf");
    assert!(
        tmpfiles.lines().any(|line| line.trim() == "d /run/nopass 0755 root root -"),
        "tmpfiles.conf must contain the line: d /run/nopass 0755 root root -"
    );
}

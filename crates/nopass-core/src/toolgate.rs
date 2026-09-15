//! The single shared "fail loudly, never skip" gate for an external tool
//! dependency (design.md §6).
//!
//! `timefmt.rs`'s `require_systemd_analyze` used to be the only copy of
//! this rule, added as the direct remedy for verify-report.md H3: an
//! earlier version of that gate `eprintln!`-ed and returned early when
//! `systemd-analyze` was absent, and `cargo test` captures the stderr of a
//! passing test — so the skip was invisible, the gate reported `ok` in
//! 0.00 s with exit 0, and the defect the gate exists to catch could have
//! shipped without a single red test anywhere. Two copies of "fail
//! loudly" is how one of them quietly grows an early return; this module
//! exists so there is exactly one copy for every hardening gate M3 adds
//! (Phase 9's Rank 1 `systemd-analyze` gate and Phase 10's Rank 2 polkit
//! gate both call this, not a rewritten version of it).

use std::process::Command;

/// Panics — never returns a "missing" value, never logs and continues —
/// when `tool` cannot be run via `<tool> --version`.
///
/// There is deliberately no `Result`, no `bool`, and no `Option` return:
/// any of those would give a caller a value it could match on and choose
/// to ignore, which is exactly the shape of the H3 defect. `why` is
/// folded into the panic message so a developer who hits it knows what
/// contract the missing tool was about to validate, not just that
/// `require` failed.
pub fn require(tool: &str, why: &str) {
    let msg = format!(
        "{tool} must be on PATH: {why}. This gate fails loudly instead of skipping \
         (verify-report.md H3) — install {tool}, or make an explicit, visible decision \
         (e.g. #[ignore]) to run without it."
    );
    let probe = Command::new(tool).arg("--version").output().expect(&msg);
    assert!(probe.status.success(), "{tool} is on PATH but `{tool} --version` did not exit 0: {why}");
}

#[cfg(test)]
mod tests {
    use super::require;

    #[test]
    fn a_known_present_tool_passes_silently() {
        // `cargo` is guaranteed present in any environment that can run
        // `cargo test` at all, so this pins the non-panicking path.
        require("cargo", "sanity check for the shared gate itself");
    }

    #[test]
    #[should_panic(expected = "must be on PATH")]
    fn a_nonexistent_binary_name_panics_rather_than_skips() {
        // This is the pin against `require` ever growing a lenient path:
        // if a future edit replaces the `.expect(...)` with an
        // `eprintln!`-and-return, this test stops panicking and fails —
        // exactly the H3 defect class, caught here instead of surviving
        // silently the way it did in `timefmt.rs` before the remedy.
        require("nopass-tool-that-does-not-exist-anywhere-on-this-path", "pin test");
    }
}

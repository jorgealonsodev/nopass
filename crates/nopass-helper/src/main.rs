#![forbid(unsafe_code)]

//! `nopass-helper`: privileged binary that owns every write to
//! `/etc/sudoers.d/`.
//!
//! This binary is a thin entry point over the `nopass_helper` library
//! (`src/lib.rs`), which owns every module. See `lib.rs` for why the
//! library exists (it is not a public-API design choice — it is the only
//! way `tests/fileops_tempdir.rs` can reach `fileops`/`lock` at all).
//!
//! Phase 4 adds the CLI surface (`cli`), typed exit-code mapping
//! (`error`), the `CommandRunner` port (`runner`), and binary resolution
//! (`bins`). Phase 5 adds invocation-context uid resolution (`uid`) and
//! uid/sudoer admission checks (`checks`). Phase 6 adds the mutation lock
//! (`lock`) and atomic sudoers-rule file operations (`fileops`). Phase 7
//! adds timer management (`timer`) and the four real
//! `ops::{enable,disable,status,expire}` transactions, and wires them in
//! here, replacing the Phase 4 stub handlers. `Layout::system()` and
//! `Binaries::system()` are each constructed exactly once, right here, per
//! design.md §2/§3.

use clap::Parser;

use nopass_core::paths::Layout;
use nopass_helper::bins::Binaries;
use nopass_helper::cli::{Cli, Cmd};
use nopass_helper::error::HelperError;
use nopass_helper::ops;
use nopass_helper::runner::SystemRunner;

fn main() {
    std::process::exit(run())
}

/// Parses the real process CLI, dispatches to a handler, and maps the
/// result to a process exit code. `Cli::parse()` itself exits 2 on a
/// parse failure before this function's body ever runs.
fn run() -> i32 {
    let cli = Cli::parse();
    let layout = Layout::system();
    let binaries = Binaries::system();
    to_exit_code(dispatch(cli.cmd, &layout, &SystemRunner, &binaries))
}

/// `Ok(())` → 0; any `HelperError` → its documented exit code.
fn to_exit_code(result: Result<(), HelperError>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(err) => err.exit_code(),
    }
}

/// Routes a parsed subcommand to its real `ops::*` transaction
/// (design.md §4).
fn dispatch(
    cmd: Cmd,
    layout: &Layout,
    runner: &nopass_helper::runner::SystemRunner,
    binaries: &Binaries,
) -> Result<(), HelperError> {
    match cmd {
        Cmd::Enable { until, until_reboot } => ops::enable(layout, runner, binaries, until, until_reboot),
        Cmd::Disable => ops::disable(layout, runner, binaries),
        Cmd::Status => ops::status(layout),
        Cmd::Expire { uid, boot } => ops::expire(layout, runner, binaries, uid, boot),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_routes_every_subcommand_and_rejects_missing_pkexec_context() {
        // No PKEXEC_UID/real-root context is available in this test
        // process, so every routed subcommand fails at context
        // resolution (exit 10) rather than mutating anything — this
        // proves `dispatch` reaches the real `ops::*` entry points
        // (which is the only thing this test can assert without a
        // privileged environment), not that it still calls Phase 4's
        // `Ok(())` stubs.
        let root = std::env::temp_dir().join(format!("nopass_test_main_dispatch_{}", std::process::id()));
        let layout = Layout::under(&root);
        let binaries = Binaries::from_candidates(&[]);
        for cmd in [
            Cmd::Enable { until: Some(100), until_reboot: false },
            Cmd::Enable { until: None, until_reboot: true },
            Cmd::Disable,
            Cmd::Status,
            Cmd::Expire { uid: Some(1000), boot: false },
            Cmd::Expire { uid: None, boot: true },
        ] {
            let err = dispatch(cmd, &layout, &SystemRunner, &binaries).unwrap_err();
            assert_eq!(err.exit_code(), 10);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn to_exit_code_maps_ok_to_zero() {
        assert_eq!(to_exit_code(Ok(())), 0);
    }

    #[test]
    fn to_exit_code_maps_err_to_its_documented_exit_code() {
        assert_eq!(to_exit_code(Err(HelperError::LockBusy)), 15);
        assert_eq!(to_exit_code(Err(HelperError::Context("missing PKEXEC_UID"))), 10);
        assert_eq!(to_exit_code(Err(HelperError::Fs("rename failed".to_string()))), 16);
    }
}

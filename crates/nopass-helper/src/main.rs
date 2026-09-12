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
//! (`bins`), and wires them together here: `Cli::parse()` → `dispatch` →
//! a stub handler per subcommand that returns `Ok(())` without touching
//! the system. Phase 5 adds invocation-context uid resolution (`uid`) and
//! uid/sudoer admission checks (`checks`). Phase 6 adds the mutation
//! lock (`lock`) and atomic sudoers-rule file operations (`fileops`).
//! None of these are wired into `dispatch` yet — that lands with the real
//! `ops::{enable,disable,status,expire}` transactions in Phase 7.

use clap::Parser;

use nopass_helper::cli::{Cli, Cmd};
use nopass_helper::error::HelperError;

fn main() {
    std::process::exit(run())
}

/// Parses the real process CLI, dispatches to a handler, and maps the
/// result to a process exit code. `Cli::parse()` itself exits 2 on a
/// parse failure before this function's body ever runs.
fn run() -> i32 {
    let cli = Cli::parse();
    to_exit_code(dispatch(cli.cmd))
}

/// `Ok(())` → 0; any `HelperError` → its documented exit code.
fn to_exit_code(result: Result<(), HelperError>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(err) => err.exit_code(),
    }
}

/// Routes a parsed subcommand to its handler. Every handler is currently
/// a stub returning `Ok(())`; Phase 7 replaces each with the real
/// `ops::*` transaction.
fn dispatch(cmd: Cmd) -> Result<(), HelperError> {
    match cmd {
        Cmd::Enable { until, until_reboot } => stub_enable(until, until_reboot),
        Cmd::Disable => stub_disable(),
        Cmd::Status => stub_status(),
        Cmd::Expire { uid, boot } => stub_expire(uid, boot),
    }
}

fn stub_enable(_until: Option<u64>, _until_reboot: bool) -> Result<(), HelperError> {
    Ok(())
}

fn stub_disable() -> Result<(), HelperError> {
    Ok(())
}

fn stub_status() -> Result<(), HelperError> {
    Ok(())
}

fn stub_expire(_uid: Option<u32>, _boot: bool) -> Result<(), HelperError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_routes_every_subcommand_to_an_ok_stub_handler() {
        assert!(dispatch(Cmd::Enable { until: Some(100), until_reboot: false }).is_ok());
        assert!(dispatch(Cmd::Enable { until: None, until_reboot: true }).is_ok());
        assert!(dispatch(Cmd::Disable).is_ok());
        assert!(dispatch(Cmd::Status).is_ok());
        assert!(dispatch(Cmd::Expire { uid: Some(1000), boot: false }).is_ok());
        assert!(dispatch(Cmd::Expire { uid: None, boot: true }).is_ok());
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

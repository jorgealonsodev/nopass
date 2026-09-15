//! Atomic user-preferences-file writes (design.md §4 D4; Threat Matrix
//! "Executable-file authoring").
//!
//! Reuses the discipline `crates/nopass-helper/src/fileops.rs::write_rule_atomic`
//! already established: `O_CREAT|O_EXCL` tmp → write → `fchmod` →
//! `fsync` → `rename` → best-effort parent `fsync`, with the tmp file
//! unlinked on every failure past the open. Two deliberate differences
//! from `fileops.rs`, both because this module writes a user-owned
//! preferences file rather than crossing the privilege boundary:
//!
//! - The caller supplies `mode` (`0600` for `config.toml`, `0644` for the
//!   `autostart` `.desktop` entry) instead of a single fixed `0440`.
//! - `write` calls `create_dir_all` on the target's parent directory.
//!   `fileops.rs` deliberately never does this for the privileged
//!   sudoers-rule directory (a real system always has it, and the helper
//!   must not be in the business of creating it) — but an XDG config
//!   directory (`~/.config/nopass`) legitimately may not exist yet, and
//!   creating it is this module's job, not its caller's.
//!
//! The code is duplicated rather than shared: `fileops.rs` also runs
//! `visudo` and `fchown`s to root, and unifying the two would drag both
//! concerns across the privilege boundary. `nopass-core` is not a home
//! for either — it has no UI or system dependencies (`openspec/config.yaml`).

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Every way [`write`] can fail. Carries the path it was operating on so
/// a caller (or a log line) does not have to guess which of `path` or its
/// tmp sibling failed.
#[derive(Debug, thiserror::Error)]
#[error("{path}: {source}")]
pub struct AtomicFileError {
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

fn fs_err(path: &Path, source: std::io::Error) -> AtomicFileError {
    AtomicFileError { path: path.to_path_buf(), source }
}

/// The tmp sibling `write` stages content in before renaming it onto
/// `path`: same directory (so the final `rename` is same-filesystem and
/// therefore atomic), a leading `.` so it never collides with a real
/// config name, and a `.tmp` suffix. `pub` so tests can target it exactly
/// the way `nopass_core::paths::Layout::rule_tmp_path` lets
/// `fileops_tempdir.rs` target the helper's tmp path.
pub fn tmp_path_for(path: &Path) -> PathBuf {
    let file_name =
        path.file_name().expect("atomicfile::write/tmp_path_for given a path with no file name").to_string_lossy();
    path.with_file_name(format!(".{file_name}.tmp"))
}

/// Writes `bytes` to `path` atomically, with permission bits `mode`
/// (e.g. `0o600` or `0o644`). On success, `path` contains exactly `bytes`
/// with exactly `mode`'s permission bits and no partially-written state
/// is ever observable at `path` — a concurrent reader sees either the
/// previous complete content or the new complete content, never a torn
/// mix of the two. On failure, nothing at `path` itself has changed, and
/// the tmp file is unlinked (best-effort) unless the failure was the
/// initial `O_CREAT|O_EXCL` open itself, in which case there was nothing
/// this call created to clean up (see `fileops.rs` for the same
/// reasoning applied to `write_rule_atomic`).
pub fn write(path: &Path, bytes: &[u8], mode: u32) -> Result<(), AtomicFileError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| fs_err(parent, e))?;
    }
    let tmp_path = tmp_path_for(path);

    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(mode)
        .open(&tmp_path)
        .map_err(|e| fs_err(&tmp_path, e))?;

    let write_result: Result<(), AtomicFileError> = (|| {
        file.write_all(bytes).map_err(|e| fs_err(&tmp_path, e))?;
        // `.mode(mode)` above is ANDed with `~umask` by the kernel at
        // creation time, so the requested bits are not guaranteed to
        // have survived a hostile umask (e.g. `0o077` would strip
        // group/other read even when the caller asked for `0o644`).
        // `set_permissions` on the still-open fd calls `fchmod`, which is
        // unaffected by umask and not subject to a symlink-following
        // TOCTOU window (`tmp_path` was just created by the `O_EXCL`
        // open above and nothing else can have replaced it since).
        file.set_permissions(std::fs::Permissions::from_mode(mode)).map_err(|e| fs_err(&tmp_path, e))?;
        file.sync_all().map_err(|e| fs_err(&tmp_path, e))?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }
    drop(file); // close before rename inspects the path

    // A pre-existing symlink at the FINAL path is adversarial evidence
    // that something is wrong with this location. POSIX `rename()` would
    // replace that directory entry atomically without ever following it,
    // so this check is defense-in-depth rather than a gap `rename()`
    // alone would leave open — but it turns a silent clobber of
    // unexplained state into a fail-closed exit with the tmp file
    // cleaned up and nothing renamed (mirrors `fileops.rs`'s identical
    // check for the same threat-matrix class).
    let final_is_symlink = std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false);
    if final_is_symlink {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(fs_err(
            path,
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "refusing to rename over a symlink"),
        ));
    }

    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(fs_err(path, e));
    }

    // Directory fsync: best-effort only, same rationale as
    // `fileops.rs::write_rule_atomic` — the rename already took effect,
    // so there is nothing to roll back if this fails.
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(())
}

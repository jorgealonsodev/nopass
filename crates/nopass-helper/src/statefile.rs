//! Atomic write of `/run/nopass/<uid>.state` (design.md §4.1 step 16, §5
//! "State-file atomic write"; helper-observability §State File Placement
//! and Permissions).
//!
//! Mirrors `fileops::write_rule_atomic`'s `O_EXCL` + `fsync` +
//! atomic-rename pattern, but at mode `0644` (world-readable — the whole
//! point of this file is that an unprivileged tray process can read it
//! despite `/etc/sudoers.d` being `0750`) rather than the rule file's
//! `0440`.
//!
//! `write`'s caller (`ops.rs`, Phase 8) is obligated by design.md's
//! rollback table (step 16) to log a write failure and NEVER let it
//! change the transaction's exit code — the sudoers grant is already
//! real by the time this call runs, so failing the operation here would
//! report a lie. This module returns a normal `Result` so the caller can
//! make that decision explicitly; it does not swallow errors itself.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use nopass_core::paths::Layout;
use nopass_core::state::HelperStatus;

use crate::error::HelperError;

fn fs_err(path: &Path, e: std::io::Error) -> HelperError {
    HelperError::Fs(format!("{}: {e}", path.display()))
}

/// Ensures `run_dir` (`/run/nopass` in production) exists at mode
/// `0755`, creating it when missing. `LockGuard::acquire` (Phase 6)
/// already creates this same directory via a plain `create_dir_all` — no
/// explicit mode, so it is whatever `~umask` leaves of `0o777` — before
/// every mutating transaction reaches this call, so in production this
/// branch fires only on a system where `nopass.tmpfiles.conf` has not
/// yet run AND the lock's own auto-creation raced ahead of a hostile
/// umask. `set_permissions` after creation (not `DirBuilder::mode`,
/// which is itself subject to the process umask at `mkdir(2)` time) is
/// what makes `0755` hold regardless of the caller's umask, mirroring
/// `write_rule_atomic`'s `fchmod`-after-open reasoning for the rule
/// file's own mode.
fn ensure_run_dir(run_dir: &Path) -> Result<(), HelperError> {
    match std::fs::metadata(run_dir) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(HelperError::Fs(format!("{}: exists and is not a directory", run_dir.display()))),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            std::fs::create_dir_all(run_dir).map_err(|e| fs_err(run_dir, e))?;
            std::fs::set_permissions(run_dir, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| fs_err(run_dir, e))?;
            Ok(())
        }
        Err(e) => Err(fs_err(run_dir, e)),
    }
}

/// Atomically writes `status` to `layout.state_path(status.uid)`:
///
/// 1. auto-create the run directory (`ensure_run_dir`, mode `0755`) when
///    missing.
/// 2. open `layout.state_tmp_path(status.uid)` with
///    `O_CREAT|O_EXCL|O_WRONLY` (`OpenOptions::create_new`) — refuses a
///    pre-existing tmp file or symlink at that path, same reasoning as
///    `fileops::write_rule_atomic` step 1.
/// 3. write the JSON line, `fchmod 0644` on the still-open fd (immune to
///    the umask — `.mode(0o644)` at open time is not), `fsync`.
/// 4. atomically `rename` the tmp file onto the final path.
/// 5. best-effort `fsync` of the containing directory.
///
/// Every failure from step 2 onward unlinks the tmp file (best-effort)
/// before returning. This function never decides whether a failure is
/// fatal to its caller's transaction — see the module doc comment.
pub fn write(layout: &Layout, status: &HelperStatus) -> Result<(), HelperError> {
    let final_path = layout.state_path(status.uid);
    let tmp_path = layout.state_tmp_path(status.uid);
    let run_dir = final_path.parent().expect("state_path always has a parent (the run directory)");

    ensure_run_dir(run_dir)?;

    let content = status.to_json_line();

    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o644)
        .open(&tmp_path)
        .map_err(|e| fs_err(&tmp_path, e))?;

    let write_result: Result<(), HelperError> = (|| {
        file.write_all(content.as_bytes()).map_err(|e| fs_err(&tmp_path, e))?;
        // `.mode(0o644)` above is ANDed with `~umask` by the kernel at
        // creation time; this `fchmod` on the already-open fd is what
        // makes 0644 hold under a hostile umask (e.g. `0o077`), mirroring
        // `write_rule_atomic`'s identical reasoning for the rule file.
        file.set_permissions(std::fs::Permissions::from_mode(0o644)).map_err(|e| fs_err(&tmp_path, e))?;
        file.sync_all().map_err(|e| fs_err(&tmp_path, e))?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }
    drop(file);

    if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(fs_err(&final_path, e));
    }

    if let Ok(dir) = std::fs::File::open(run_dir) {
        let _ = dir.sync_all();
    }

    Ok(())
}

/// Removes the state file for `uid`, if present. `Ok(())` whether or not
/// a file actually existed — idempotent, mirroring
/// `fileops::remove_rule`'s `ENOENT`-is-success convention. Not yet
/// called from `ops.rs`: design.md §4.4 names one caller for this
/// ("Stale `/run/nopass/*.state` files without a rule are removed" during
/// the boot sweep), which requires enumerating `/run/nopass/*.state`
/// against the live rule set — a second sweep loop with no RED test of
/// its own in this phase's task list (tasks.md 8.1-8.3). Deliberately
/// left unwired rather than half-built; see the Phase 8 apply-phase
/// report for the explicit reasoning, matching Phase 7's precedent for
/// `ops::resolve_username`.
pub fn remove(layout: &Layout, uid: u32) -> Result<(), HelperError> {
    let path = layout.state_path(uid);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(fs_err(&path, e)),
    }
}

/// Extracts the uid from a canonical state filename (`<uid>.state`).
/// `nopass_core::paths` has no filename parser for the state-file shape —
/// `uid_from_rule_filename` only accepts the `90-nopass-<uid>` rule-file
/// prefix — so this is a new, narrowly scoped parser living next to this
/// module's own state-file path handling, kept in the same spirit as
/// `uid_from_rule_filename`: canonical digits only, no leading zero, no
/// extra suffix.
fn uid_from_state_filename(name: &str) -> Option<u32> {
    let digits = name.strip_suffix(".state")?;
    let mut chars = digits.chars();
    let first = chars.next()?;
    if !first.is_ascii_digit() || first == '0' {
        return None;
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok()
}

/// Enumerates every uid with a canonical `<uid>.state` filename directly
/// under the run directory, sorted ascending — the boot sweep's
/// (`ops::expire_boot_inner`, design.md §4.4) enumeration point for
/// "Stale `/run/nopass/*.state` files without a rule are removed."
/// Obtains the run directory via `layout.state_path(0).parent()` (the uid
/// argument is irrelevant to the parent directory it yields), since
/// `Layout` exposes no separate run-directory accessor.
///
/// Mirrors `fileops::list_rule_uids`'s discipline exactly: requires
/// `DirEntry::file_type()` to report a regular file before trusting the
/// name, so a directory or symlink named like a state file is skipped
/// (same "Rule-file target selection" threat-matrix reasoning, applied
/// here to state files) — never resolved via `Path::metadata`, which
/// would follow a symlink.
///
/// A missing run directory yields an empty list rather than an error,
/// mirroring `list_rule_uids`'s identical "not yet created" fail-soft
/// reasoning.
pub fn list_state_uids(layout: &Layout) -> Result<Vec<u32>, HelperError> {
    let run_dir = layout
        .state_path(0)
        .parent()
        .expect("state_path always has a parent (the run directory)")
        .to_path_buf();
    let entries = match std::fs::read_dir(&run_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(fs_err(&run_dir, e)),
    };

    let mut uids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| fs_err(&run_dir, e))?;
        let file_type = entry.file_type().map_err(|e| fs_err(&run_dir, e))?;
        if !file_type.is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if let Some(uid) = uid_from_state_filename(name) {
                uids.push(uid);
            }
        }
    }
    uids.sort_unstable();
    Ok(uids)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use nopass_core::expiry::Expiry;

    fn temp_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!(
            "nopass_test_statefile_{tag}_{}_{:?}_{nanos}_{n}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    fn sample_status(uid: u32) -> HelperStatus {
        HelperStatus {
            schema: 1,
            uid,
            user: "jorge".to_string(),
            active: true,
            expires: Some(Expiry::At { epoch: 1_789_000_000 }),
            rule_path: format!("/etc/sudoers.d/90-nopass-{uid}"),
            updated_at: 1_789_000_000,
        }
    }

    #[test]
    fn write_atomically_creates_the_state_file_with_mode_0644_and_the_exact_json_line() {
        let root = temp_root("write_basic");
        let layout = Layout::under(&root);
        let status = sample_status(1000);

        write(&layout, &status).expect("write succeeds");

        let final_path = layout.state_path(1000);
        assert!(final_path.exists(), "final state file must exist");
        assert!(!layout.state_tmp_path(1000).exists(), "tmp file must not survive a successful write");
        let content = std::fs::read_to_string(&final_path).unwrap();
        assert_eq!(content, status.to_json_line());
        let mode = std::fs::metadata(&final_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_auto_creates_the_run_directory_at_mode_0755_when_missing() {
        let root = temp_root("autocreate_dir");
        let layout = Layout::under(&root);
        let run_dir = layout.state_path(1000).parent().unwrap().to_path_buf();
        assert!(!run_dir.exists(), "precondition: run dir must not exist yet");

        write(&layout, &sample_status(1000)).expect("write succeeds and creates the run dir");

        assert!(run_dir.is_dir());
        let mode = std::fs::metadata(&run_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_holds_mode_0644_under_a_hostile_umask() {
        let root = temp_root("hostile_umask");
        let layout = Layout::under(&root);
        std::fs::create_dir_all(root.join("run/nopass")).unwrap();

        let old_umask = nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o077));
        let result = write(&layout, &sample_status(1000));
        nix::sys::stat::umask(old_umask);
        result.expect("write succeeds under a hostile umask");

        let mode = std::fs::metadata(layout.state_path(1000)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "fchmod after open must defeat a 0o077 umask, exactly as write_rule_atomic does");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_fails_and_cleans_up_the_tmp_file_on_an_o_excl_collision() {
        let root = temp_root("o_excl_collision");
        let layout = Layout::under(&root);
        std::fs::create_dir_all(root.join("run/nopass")).unwrap();
        std::fs::write(layout.state_tmp_path(1000), b"stale leftover").unwrap();

        let err = write(&layout, &sample_status(1000)).unwrap_err();
        assert!(matches!(err, HelperError::Fs(_)));
        assert!(!layout.state_path(1000).exists(), "final path must never be created on failure");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_is_idempotent_when_no_state_file_exists() {
        let root = temp_root("remove_absent");
        let layout = Layout::under(&root);
        remove(&layout, 1000).expect("removing an absent state file is a no-op success");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_deletes_an_existing_state_file() {
        let root = temp_root("remove_present");
        let layout = Layout::under(&root);
        write(&layout, &sample_status(1000)).unwrap();
        assert!(layout.state_path(1000).exists());

        remove(&layout, 1000).expect("remove succeeds");
        assert!(!layout.state_path(1000).exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    // --- list_state_uids / uid_from_state_filename (boot-sweep orphan
    // cleanup, design.md §4.4) ---------------------------------------

    #[test]
    fn uid_from_state_filename_accepts_only_a_canonical_uid() {
        assert_eq!(uid_from_state_filename("1000.state"), Some(1000));
        assert_eq!(uid_from_state_filename("1.state"), Some(1));
        assert_eq!(uid_from_state_filename("01000.state"), None, "leading zero must be rejected");
        assert_eq!(uid_from_state_filename("1000.state.bak"), None, "trailing suffix must be rejected");
        assert_eq!(uid_from_state_filename("lock"), None, "wrong suffix must be rejected");
        assert_eq!(uid_from_state_filename("abc.state"), None, "non-digit body must be rejected");
    }

    #[test]
    fn list_state_uids_returns_an_empty_vec_when_the_run_directory_does_not_exist() {
        let root = temp_root("list_state_uids_missing_dir");
        let layout = Layout::under(&root);
        assert_eq!(list_state_uids(&layout).unwrap(), Vec::<u32>::new());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_state_uids_finds_canonical_state_files_and_skips_non_canonical_and_non_file_entries() {
        let root = temp_root("list_state_uids_mixed");
        let layout = Layout::under(&root);
        write(&layout, &sample_status(1000)).unwrap();
        write(&layout, &sample_status(2000)).unwrap();
        let run_dir = layout.state_path(1000).parent().unwrap().to_path_buf();
        // A non-canonical filename (wrong suffix) must never be returned.
        std::fs::write(run_dir.join("lock"), b"").unwrap();
        // A directory named like a state file must be skipped, not
        // trusted by name alone — same discipline as
        // `fileops::list_rule_uids`.
        std::fs::create_dir(run_dir.join("3000.state")).unwrap();

        let uids = list_state_uids(&layout).unwrap();

        assert_eq!(uids, vec![1000, 2000], "only the two canonical regular-file state files are returned");
        let _ = std::fs::remove_dir_all(&root);
    }
}

//! Mutation-serialization lock (design.md §3 sequence step 7; Architecture
//! Decision "`flock` on `/run/nopass/lock`, not an `O_EXCL` lock file";
//! sudoers-rule-lifecycle §Mutation Serialization).
//!
//! Every mutating operation (`enable`, `disable`, `expire`) acquires
//! [`LockGuard`] for its entire duration before touching
//! `/etc/sudoers.d/`. `LockGuard` wraps `nix::fcntl::Flock<std::fs::File>`
//! taken with `FlockArg::LockExclusiveNonblock` on `layout.lock_path()`
//! (`/run/nopass/lock` in production). Non-blocking is deliberate: a
//! second concurrent mutation fails fast with `HelperError::LockBusy`
//! (exit 15) instead of stalling a polkit dialog. `flock` is released by
//! the kernel automatically when the holding process dies (including
//! `SIGKILL`), so a crashed helper can never wedge the system — unlike an
//! `O_EXCL` sentinel file, which would leak and need stale-PID heuristics.
//!
//! `fileops`/`timer`/`statefile`/`ops` (Phases 7-8) are the modules that
//! actually call `acquire` in production. Until they land, a normal
//! (non-test) build never constructs a `LockGuard`, hence the blanket
//! allow below.

use std::fs::OpenOptions;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};

use nopass_core::paths::Layout;

use crate::error::HelperError;

/// RAII guard over an exclusive, non-blocking `flock` on
/// `layout.lock_path()`. Dropping the guard releases the lock — both
/// explicitly (`Flock`'s own `Drop` calls `LOCK_UN`) and, if the process
/// dies before that `Drop` runs, implicitly via the kernel.
pub struct LockGuard {
    _flock: Flock<std::fs::File>,
}

/// Creates `run_dir` (and any missing parents) if needed, then forces its
/// mode to `0755` regardless of the ambient umask or of who created it
/// first — see [`LockGuard::acquire`]'s doc comment (verify-report W2).
/// `create_dir_all` is a no-op success when `run_dir` already exists, so
/// this always ends with a deterministic mode, not merely on first
/// creation.
fn create_run_dir_at_0755(run_dir: &Path) -> Result<(), HelperError> {
    std::fs::create_dir_all(run_dir)
        .map_err(|e| HelperError::Fs(format!("create lock directory {}: {e}", run_dir.display())))?;
    std::fs::set_permissions(run_dir, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| HelperError::Fs(format!("set permissions on lock directory {}: {e}", run_dir.display())))?;
    Ok(())
}

impl LockGuard {
    /// Acquires the exclusive mutation lock at `layout.lock_path()`.
    ///
    /// Creates the lock's parent directory (`/run/nopass` in production)
    /// at mode `0755` when it does not yet exist: this lock is acquired
    /// before every other step of `enable`/`disable`/`expire` (design.md
    /// §4.1 step 7), including before `statefile`'s own auto-creation of
    /// that same directory (design.md §8.1, Phase 8) ever runs, so this
    /// is the first code path in the whole transaction that can depend on
    /// `/run/nopass` existing. On a real system `nopass.tmpfiles.conf`
    /// already creates it at boot; this is defense-in-depth for a boot
    /// where that unit has not yet run.
    ///
    /// verify-report W2: a bare `create_dir_all` with no explicit mode
    /// leaves the directory at `0o777 & ~umask` — `0755` only by
    /// coincidence of the ambient `0o022` umask this project develops
    /// under, not by anything the code enforces. Under a hostile umask
    /// (e.g. `0o077`) the directory would come out `0700`, and an
    /// unprivileged tray process could no longer even traverse into it to
    /// read the world-readable `0644` state file `statefile::write`
    /// places inside — defeating the whole point of that file's mode.
    /// `statefile::ensure_run_dir` cannot correct this after the fact
    /// either: it short-circuits as soon as the directory exists, which
    /// by then it already does. `set_permissions` after creation (not
    /// `DirBuilder::mode`, itself ANDed with the umask at `mkdir(2)` time)
    /// is the same defeat-the-umask pattern `statefile::ensure_run_dir`
    /// and `fileops::write_rule_atomic`'s `fchmod`-after-open already use
    /// for their own targets — applied here to the directory this
    /// function is the first in the whole transaction to create.
    ///
    /// Non-blocking: a lock already held by another process returns
    /// `HelperError::LockBusy` (exit 15) immediately, never a wait.
    pub fn acquire(layout: &Layout) -> Result<LockGuard, HelperError> {
        let lock_path = layout.lock_path();
        if let Some(parent) = lock_path.parent() {
            create_run_dir_at_0755(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&lock_path)
            .map_err(|e| HelperError::Fs(format!("open lock file {}: {e}", lock_path.display())))?;

        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(flock) => Ok(LockGuard { _flock: flock }),
            Err((_file, Errno::EWOULDBLOCK)) => Err(HelperError::LockBusy),
            Err((_file, errno)) => {
                Err(HelperError::Fs(format!("flock {}: {errno}", lock_path.display())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Unique per-call temp root, following the same collision-avoidance
    /// convention as `bins.rs`'s `temp_test_dir` (pid + thread id +
    /// nanosecond timestamp + a per-process counter).
    fn temp_layout_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!(
            "nopass_test_lock_{tag}_{}_{:?}_{nanos}_{n}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn sequential_acquire_and_release_succeed_with_no_overlap_required() {
        let root = temp_layout_root("sequential");
        let layout = Layout::under(&root);

        {
            let _first = LockGuard::acquire(&layout).expect("first acquire succeeds");
            // Dropped at the end of this block, releasing the lock.
        }
        {
            let _second = LockGuard::acquire(&layout).expect("second acquire succeeds after the first released");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn second_nonblocking_acquire_on_an_already_held_lock_is_lock_busy() {
        let root = temp_layout_root("busy");
        let layout = Layout::under(&root);

        let _held = LockGuard::acquire(&layout).expect("first acquire succeeds and is held for this scope");
        let second = LockGuard::acquire(&layout);

        match second {
            Err(err) => {
                assert!(matches!(err, HelperError::LockBusy), "expected LockBusy, got {err:?}");
                assert_eq!(err.exit_code(), 15);
            }
            Ok(_) => panic!("expected the second non-blocking acquire to fail while the first is held"),
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn acquire_creates_the_missing_run_directory() {
        let root = temp_layout_root("mkdir");
        let layout = Layout::under(&root);
        assert!(!layout.lock_path().parent().unwrap().exists());

        let _guard = LockGuard::acquire(&layout).expect("acquire creates the run directory and succeeds");
        assert!(layout.lock_path().exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// verify-report W2: before this fix, the run directory's mode was
    /// merely `0o777 & ~umask` — correct only by the ambient `0o022`
    /// umask this project happens to develop under, not by anything the
    /// code enforced. Confirmed RED first: with the pre-fix bare
    /// `create_dir_all(parent)` (no `set_permissions` after), this exact
    /// test failed under a `0o077` umask, observing mode `0700` instead
    /// of `0755` — proving the assertion is sensitive to the regression
    /// it now guards against.
    #[test]
    fn acquire_creates_the_run_directory_at_mode_0755_under_a_hostile_umask() {
        let root = temp_layout_root("hostile_umask");
        let layout = Layout::under(&root);
        let run_dir = layout.lock_path().parent().unwrap().to_path_buf();

        let old_umask = nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o077));
        let result = LockGuard::acquire(&layout);
        nix::sys::stat::umask(old_umask);
        let _guard = result.expect("acquire succeeds under a hostile umask");

        let mode = std::fs::metadata(&run_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "the run directory must be 0755 regardless of the caller's umask");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The directory-mode fix must hold even when `/run/nopass` was
    /// already created by something else first (e.g. a real
    /// `nopass.tmpfiles.conf` unit, or `statefile::ensure_run_dir`
    /// racing ahead) — `create_dir_all`'s no-op-on-existing behavior must
    /// not let a pre-existing wrong mode survive `acquire`.
    #[test]
    fn acquire_corrects_an_already_existing_run_directory_to_mode_0755() {
        let root = temp_layout_root("preexisting_wrong_mode");
        let layout = Layout::under(&root);
        let run_dir = layout.lock_path().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::set_permissions(&run_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let _guard = LockGuard::acquire(&layout).expect("acquire succeeds against a pre-existing directory");

        let mode = std::fs::metadata(&run_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "a pre-existing directory at the wrong mode must be corrected, not trusted");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Phase 6 correction (independent-verifier finding, test quality):
    /// this used to duplicate `sequential_acquire_and_release_succeed_with_no_overlap_required`
    /// — same invariant (acquire, release, acquire again succeeds),
    /// differing only in implicit vs. explicit drop. That proved nothing
    /// the other test did not already prove, through the exact same
    /// `LockGuard::acquire` code path. This test now proves something
    /// genuinely distinct: that `drop`ping the guard releases the
    /// underlying `flock` at the KERNEL level, checked via an
    /// independently opened file descriptor and a raw `nix::fcntl::Flock`
    /// call that never goes through `LockGuard::acquire` at all — so a
    /// bug where `LockGuard::acquire` always reports success without the
    /// kernel lock actually being free could not hide behind this test
    /// the way it could behind a second `LockGuard::acquire` call.
    #[test]
    fn dropping_the_guard_releases_the_kernel_level_flock_for_an_independently_opened_fd() {
        let root = temp_layout_root("kernel_level_release");
        let layout = Layout::under(&root);
        let lock_path = layout.lock_path();

        let guard = LockGuard::acquire(&layout).expect("first acquire succeeds");
        drop(guard); // explicit: releases before the raw flock attempt below

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .expect("open the lock file directly, bypassing LockGuard entirely");
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(_flock) => {} // released on drop at the end of this scope
            Err((_file, errno)) => {
                panic!("expected the kernel-level flock to be free after LockGuard was dropped, got {errno}")
            }
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}

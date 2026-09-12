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
use std::os::unix::fs::OpenOptionsExt;

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

impl LockGuard {
    /// Acquires the exclusive mutation lock at `layout.lock_path()`.
    ///
    /// Creates the lock's parent directory (`/run/nopass` in production)
    /// when it does not yet exist: this lock is acquired before every
    /// other step of `enable`/`disable`/`expire` (design.md §4.1 step 7),
    /// including before `statefile`'s own auto-creation of that same
    /// directory (design.md §8.1, Phase 8) ever runs, so this is the
    /// first code path in the whole transaction that can depend on
    /// `/run/nopass` existing. On a real system `nopass.tmpfiles.conf`
    /// already creates it at boot; this is defense-in-depth for a boot
    /// where that unit has not yet run.
    ///
    /// Non-blocking: a lock already held by another process returns
    /// `HelperError::LockBusy` (exit 15) immediately, never a wait.
    pub fn acquire(layout: &Layout) -> Result<LockGuard, HelperError> {
        let lock_path = layout.lock_path();
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| HelperError::Fs(format!("create lock directory {}: {e}", parent.display())))?;
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

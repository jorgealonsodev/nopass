//! The directory watch — never the file (design.md D8, Architecture
//! Decisions): the helper replaces `<uid>.state` with a `rename(2)` onto
//! a fresh inode. A watch registered on the file itself follows the OLD
//! inode past that rename and goes silent for the rest of the process's
//! life — silently, because the tray keeps running and simply stops
//! noticing anything. Watching the containing directory survives every
//! rename, because the directory's own inode never changes.
//!
//! `notify` runs its own OS thread and calls back synchronously from it.
//! This module debounces those raw callbacks on a second dedicated
//! thread and bridges the result into the async reactor over
//! `async_channel` — the same "bridge a foreign thread across
//! `async_channel`, never a blocking `recv` on the reactor side" pattern
//! `run_off_reactor` (design.md §4.2) uses for privileged actions. The
//! debounce thread's own `recv`/`recv_timeout` calls block, but they run
//! off the reactor entirely, exactly like `run_off_reactor`'s dedicated
//! thread; only the reactor side is required to poll `Receiver::recv`
//! asynchronously, and `app::run` (Phase 10) is the only reactor-side
//! consumer of the channel this module's `tx` half feeds.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use async_channel::Sender;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::event::Event;

/// What the raw `notify` callback (running on `notify`'s own OS thread)
/// hands to the debounce thread. `Changed` is the ordinary, debounced
/// case; `Lost` (verify-report.md H4) is not debounced at all — see
/// [`debounce_loop`].
enum RawSignal {
    Changed,
    Lost,
}

/// How long the debounce thread waits, after the most recent raw
/// filesystem event on the watched name, before emitting a single
/// [`Event::FileChanged`] (design.md §4 "inotify watcher" table).
pub const DEBOUNCE_MS: u64 = 100;

/// Every way establishing the watch can fail.
#[derive(Debug)]
pub enum WatchError {
    /// `run_dir` does not exist (or is not a directory) right now.
    /// `Watch::start` carries no state across calls — the caller (design
    /// D8; spec `tray-state-sync` "Missing run directory falls back to
    /// reconciliation only") falls back to reconciliation alone, warns,
    /// and simply calls `Watch::start` again on the next 60 s tick; a
    /// later call that finds the directory present succeeds exactly as a
    /// first attempt would; this module holds nothing that would make a
    /// retry behave differently from the first try.
    DirMissing,
    /// The underlying `notify` backend failed to register the watch for
    /// a reason other than a missing directory (for example, permission
    /// denied, or the process's inotify instance/watch-count limit).
    Io,
}

/// An established, debounced watch on `run_dir`, filtered to
/// `<uid>.state`. Dropping the `Watch` stops the `notify` backend, which
/// in turn closes the internal channel the debounce thread is blocked
/// on, so the debounce thread exits shortly after.
pub struct Watch {
    _watcher: RecommendedWatcher,
    _debounce: std::thread::JoinHandle<()>,
}

impl Watch {
    /// Establishes a debounced watch on `run_dir`, sending at most one
    /// [`Event::FileChanged`] per [`DEBOUNCE_MS`] window of activity on
    /// `<uid>.state` into `tx`. `run_dir` is watched non-recursively —
    /// never the state file path itself — so a `rename(2)` replacing the
    /// file lands exactly like a `create`/`write` would (design.md D8).
    pub fn start(run_dir: &Path, uid: u32, tx: Sender<Event>) -> Result<Watch, WatchError> {
        if !run_dir.is_dir() {
            return Err(WatchError::DirMissing);
        }

        let target_name = format!("{uid}.state");
        let watched_dir: PathBuf = run_dir.to_path_buf();
        let (raw_tx, raw_rx) = mpsc::channel::<RawSignal>();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else {
                // The backend itself reported an error rather than an
                // event — for example an inotify queue overflow, or the
                // watch descriptor being invalidated by the kernel in a
                // way that produced no `Remove` event of its own. Either
                // way this watch can no longer be trusted to keep
                // reporting, so treat it the same as an observed removal
                // (verify-report.md H4): signal loss and let the caller
                // re-establish from scratch.
                let _ = raw_tx.send(RawSignal::Lost);
                return;
            };
            // The watched directory itself being removed (verified
            // empirically against this crate's `notify` backend/version:
            // `EventKind::Remove(RemoveKind::Folder)` at `watched_dir`'s
            // own path) is "the watch is lost" (spec `tray-state-sync`
            // S4). `/run/nopass/` is an ordinary tmpfs directory — root
            // removing it, and the helper's own `statefile::
            // ensure_run_dir` recreating it, is not prevented by
            // anything, and ordinary filesystems are free to hand the
            // recreated directory the SAME inode number the old one
            // had — so this must be detected from the event stream, not
            // inferred from a `stat(2)`-based identity comparison, which
            // a fast remove+recreate can defeat.
            if matches!(event.kind, EventKind::Remove(_)) && event.paths.iter().any(|p| *p == watched_dir) {
                let _ = raw_tx.send(RawSignal::Lost);
                return;
            }
            if !is_relevant(&event.kind) {
                return;
            }
            if event.paths.iter().any(|p| matches_target(p, &target_name)) {
                // The debounce thread only stops after `raw_tx` (held by
                // this closure, owned by `_watcher`) is dropped, so a
                // send failure here only means `Watch` itself is already
                // gone — there is nothing left to notify.
                let _ = raw_tx.send(RawSignal::Changed);
            }
        })
        .map_err(|_| WatchError::Io)?;

        watcher.watch(run_dir, RecursiveMode::NonRecursive).map_err(|_| WatchError::Io)?;

        let debounce = std::thread::spawn(move || debounce_loop(raw_rx, tx));

        Ok(Watch { _watcher: watcher, _debounce: debounce })
    }

    /// Identity of this `Watch`'s debounce thread — test-only. A rebuilt
    /// `Watch` (a fresh `Watch::start` call) spawns a fresh debounce
    /// thread with a different `ThreadId`, so comparing this across two
    /// points in time is how `app.rs`'s
    /// `a_tick_over_an_unchanged_directory_does_not_rebuild_the_watch`
    /// pins `App::maybe_retry_watch`'s already-held short-circuit
    /// (verify-report.md H5) — a property no count of observed
    /// `Event::FileChanged` can distinguish, because at most one `Watch`
    /// is ever live either way (see that guard's own doc comment).
    #[cfg(test)]
    pub(crate) fn debounce_thread_id(&self) -> std::thread::ThreadId {
        self._debounce.thread().id()
    }
}

fn matches_target(path: &Path, target_name: &str) -> bool {
    path.file_name().and_then(|n| n.to_str()) == Some(target_name)
}

/// `Create`/`Modify`/`Remove` cover every way a write, an atomic
/// rename-over, or a deletion of the target surfaces; `Access`/`Other`
/// are excluded so a mere read of the file never triggers reconciliation.
fn is_relevant(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_))
}

/// Trailing-edge debounce, entirely off the reactor: block for the first
/// raw signal, then keep draining further signals that arrive within
/// [`DEBOUNCE_MS`] of the previous one, and emit exactly one
/// [`Event::FileChanged`] once that quiet window elapses with nothing
/// new. A single `rename(2)` typically raises more than one raw inotify
/// event (a paired `MOVED_FROM`/`MOVED_TO`, sometimes trailed by an
/// `ATTRIB`); this collapses all of them into the one event the spec's
/// "reacts within budget" scenario counts.
///
/// [`RawSignal::Lost`] (verify-report.md H4) is never debounced: it is
/// not a burst of related writes to collapse, and once the watched
/// directory itself is gone there is nothing further this `Watch` can
/// ever report, so the loop emits [`Event::WatchLost`] immediately and
/// exits — `App` is the one place that decides when to retry, on the
/// next `Trigger::Tick` (spec `tray-state-sync`).
fn debounce_loop(raw_rx: mpsc::Receiver<RawSignal>, tx: Sender<Event>) {
    let window = Duration::from_millis(DEBOUNCE_MS);
    while let Ok(first) = raw_rx.recv() {
        if matches!(first, RawSignal::Lost) {
            let _ = tx.send_blocking(Event::WatchLost);
            return;
        }
        // Keep absorbing signals while they keep arriving inside the
        // window; the send below only proceeds once it truly quiets
        // down. A `Lost` arriving inside the same window as a `Changed`
        // is remembered rather than silently dropped, so a
        // directory-replaced-immediately-after-a-write burst still
        // surfaces the loss.
        let mut lost_too = false;
        while let Ok(next) = raw_rx.recv_timeout(window) {
            if matches!(next, RawSignal::Lost) {
                lost_too = true;
            }
        }
        if tx.send_blocking(Event::FileChanged).is_err() {
            // The reactor side — and therefore the whole `Watch` — is
            // gone; nothing left to debounce for.
            return;
        }
        if lost_too {
            let _ = tx.send_blocking(Event::WatchLost);
            return;
        }
    }
}

// Behavioural coverage lives in `crates/nopass/tests/watch_inotify.rs` —
// a genuine Cargo integration test, exercising real inotify against a
// `TempDir` through the public `Watch`/`Event` API, per tasks.md 4.2 and
// the module's own rollback boundary (delete `watch.rs` AND that test
// file together). The retry itself — calling `Watch::start` again on the
// next 60 s tick when the first attempt failed — is owned and tested by
// `App::maybe_retry_watch` (`app.rs`), the only stateful thing that can
// hold a `Watch` handle across calls; see its own test
// `a_tick_retries_the_watch_until_established_and_never_spawns_a_second_one`
// (verify-report.md G1).

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};

    /// G8 (verify-report.md): experiment E8 made `is_relevant` return
    /// `true` for every `EventKind`, including `Access`, and every gate
    /// stayed green. `is_relevant` had no test of its own at all — this
    /// pins the filter as a pure function, directly, rather than relying
    /// on a real inotify backend to happen to emit (or not emit) an
    /// `Access` event.
    #[test]
    fn create_modify_and_remove_are_relevant() {
        assert!(is_relevant(&EventKind::Create(CreateKind::Any)));
        assert!(is_relevant(&EventKind::Modify(ModifyKind::Any)));
        assert!(is_relevant(&EventKind::Remove(RemoveKind::Any)));
    }

    #[test]
    fn access_events_are_never_relevant() {
        // design's stated intent: "Access/Other are excluded so a mere
        // read of the file never triggers reconciliation."
        assert!(!is_relevant(&EventKind::Access(AccessKind::Any)));
        assert!(!is_relevant(&EventKind::Access(AccessKind::Read)));
        assert!(!is_relevant(&EventKind::Access(AccessKind::Open(notify::event::AccessMode::Any))));
    }

    #[test]
    fn other_and_the_fully_generic_any_kind_are_never_relevant() {
        // `EventKind::Any` is notify's own imprecise-mode catch-all,
        // distinct from `EventKind::Other` — neither carries evidence of
        // a write, so neither may force a probe.
        assert!(!is_relevant(&EventKind::Other));
        assert!(!is_relevant(&EventKind::Any));
    }
}

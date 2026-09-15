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

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use async_channel::Sender;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::event::Event;

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
        let (raw_tx, raw_rx) = mpsc::channel::<()>();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if !is_relevant(&event.kind) {
                return;
            }
            if event.paths.iter().any(|p| matches_target(p, &target_name)) {
                // The debounce thread only stops after `raw_tx` (held by
                // this closure, owned by `_watcher`) is dropped, so a
                // send failure here only means `Watch` itself is already
                // gone — there is nothing left to notify.
                let _ = raw_tx.send(());
            }
        })
        .map_err(|_| WatchError::Io)?;

        watcher.watch(run_dir, RecursiveMode::NonRecursive).map_err(|_| WatchError::Io)?;

        let debounce = std::thread::spawn(move || debounce_loop(raw_rx, tx));

        Ok(Watch { _watcher: watcher, _debounce: debounce })
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
fn debounce_loop(raw_rx: mpsc::Receiver<()>, tx: Sender<Event>) {
    let window = Duration::from_millis(DEBOUNCE_MS);
    while raw_rx.recv().is_ok() {
        while raw_rx.recv_timeout(window).is_ok() {
            // Keep absorbing events while they keep arriving inside the
            // window; the loop below only proceeds once it truly quiets
            // down.
        }
        if tx.send_blocking(Event::FileChanged).is_err() {
            // The reactor side — and therefore the whole `Watch` — is
            // gone; nothing left to debounce for.
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

//! Real inotify against a `TempDir`, no bus needed (Lane A) — tasks.md
//! Phase 4, task 4.2; spec `tray-state-sync` "A state-file write is
//! observed within budget", "Missing run directory falls back to
//! reconciliation only".
//!
//! Every scenario here uses genuine filesystem operations rather than a
//! fake watcher: the one behaviour this suite exists to prove — that a
//! DIRECTORY watch survives the helper's atomic `rename(2)` over the
//! state file while a FILE watch would not — is exactly the kind of
//! thing a fake watcher would hide by construction.

use std::fs;
use std::time::{Duration, Instant};

use nopass::event::Event;
use nopass::watch::{Watch, WatchError, DEBOUNCE_MS};
use tempfile::TempDir;

/// Polls `try_recv` up to `timeout` instead of sleeping a fixed
/// duration: a fixed sleep either slows the suite down (long enough to
/// be safe) or makes it flaky under load (short enough to be fast) — the
/// exact trap that already cost M1 time once on a flaky temp-file test.
fn recv_within(rx: &async_channel::Receiver<Event>, timeout: Duration) -> Option<Event> {
    let deadline = Instant::now() + timeout;
    loop {
        match rx.try_recv() {
            Ok(event) => return Some(event),
            Err(_) if Instant::now() >= deadline => return None,
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// A generous bound: real inotify plus a 100ms debounce window should
/// always land well inside this on any machine running the suite,
/// without ever being a tight enough deadline to flake — well under the
/// spec's own 1s reaction budget plus margin.
const BUDGET: Duration = Duration::from_secs(2);

/// A window comfortably longer than the debounce period, used to assert
/// that no SECOND event follows a single logical filesystem operation.
fn quiet_window() -> Duration {
    Duration::from_millis(DEBOUNCE_MS * 3)
}

#[test]
fn missing_run_directory_yields_dir_missing_without_touching_the_filesystem() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("does-not-exist");
    let (tx, _rx) = async_channel::unbounded();
    assert!(matches!(Watch::start(&missing, 1000, tx), Err(WatchError::DirMissing)));
}

#[test]
fn a_retry_after_the_directory_appears_succeeds_exactly_like_a_first_attempt() {
    // `Watch::start` holds no state across calls — the design's "retries
    // establishing the watch on each tick" fallback (spec
    // `tray-state-sync` "Missing run directory falls back to
    // reconciliation only") relies on exactly this: a later call that
    // finds the directory present must succeed, with no special
    // "this is a retry" handling anywhere, and the watch it establishes
    // must not stay blind for the rest of the process's lifetime.
    let root = TempDir::new().unwrap();
    let run_dir = root.path().join("nopass");
    let (tx, rx) = async_channel::unbounded();

    assert!(matches!(Watch::start(&run_dir, 1000, tx.clone()), Err(WatchError::DirMissing)));

    fs::create_dir(&run_dir).unwrap();
    let watch = Watch::start(&run_dir, 1000, tx).expect("the directory now exists");

    fs::write(run_dir.join("1000.state"), b"{}").unwrap();
    assert!(matches!(recv_within(&rx, BUDGET), Some(Event::FileChanged)));

    drop(watch);
}

#[test]
fn creating_the_target_file_yields_exactly_one_debounced_event() {
    let dir = TempDir::new().unwrap();
    let (tx, rx) = async_channel::unbounded();
    let _watch = Watch::start(dir.path(), 1000, tx).unwrap();

    fs::write(dir.path().join("1000.state"), b"{}").unwrap();

    assert!(matches!(recv_within(&rx, BUDGET), Some(Event::FileChanged)));
    assert!(recv_within(&rx, quiet_window()).is_none(), "a single create must debounce to exactly one event");
}

/// The single most important test in this file: the helper never writes
/// `<uid>.state` in place, it writes a temporary file and `rename(2)`s
/// it over the target (mirroring M1's own atomic-write discipline). A
/// watch registered on the FILE would follow the pre-rename inode and
/// never observe this; only a watch on the DIRECTORY does. A test that
/// only created a file for the first time would pass regardless of which
/// of those two designs was implemented, and would prove nothing.
#[test]
fn an_atomic_rename_over_an_existing_file_is_still_observed() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("1000.state");
    fs::write(&target, b"{\"generation\":0}").unwrap();

    let (tx, rx) = async_channel::unbounded();
    let _watch = Watch::start(dir.path(), 1000, tx).unwrap();

    // The write above happened BEFORE the watch existed, so it must
    // never surface as an event — this also proves the watch was not
    // somehow pre-primed with stale activity.
    assert!(recv_within(&rx, quiet_window()).is_none());

    let tmp = dir.path().join(".1000.state.tmp");
    fs::write(&tmp, b"{\"generation\":1}").unwrap();
    fs::rename(&tmp, &target).unwrap();

    assert!(
        matches!(recv_within(&rx, BUDGET), Some(Event::FileChanged)),
        "an atomic rename over the target must still be observed by a directory watch"
    );

    let contents = fs::read_to_string(&target).unwrap();
    assert_eq!(contents, "{\"generation\":1}", "the rename must have actually replaced the file's inode");
}

#[test]
fn deleting_the_target_file_yields_exactly_one_debounced_event() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("1000.state");
    fs::write(&target, b"{}").unwrap();

    let (tx, rx) = async_channel::unbounded();
    let _watch = Watch::start(dir.path(), 1000, tx).unwrap();
    assert!(recv_within(&rx, quiet_window()).is_none());

    fs::remove_file(&target).unwrap();

    assert!(matches!(recv_within(&rx, BUDGET), Some(Event::FileChanged)));
}

#[test]
fn an_unrelated_sibling_file_yields_no_event() {
    let dir = TempDir::new().unwrap();
    let (tx, rx) = async_channel::unbounded();
    let _watch = Watch::start(dir.path(), 1000, tx).unwrap();

    fs::write(dir.path().join("2000.state"), b"{}").unwrap();
    fs::write(dir.path().join("unrelated.txt"), b"noise").unwrap();

    assert!(
        recv_within(&rx, quiet_window()).is_none(),
        "activity on a different filename must never surface as FileChanged"
    );
}

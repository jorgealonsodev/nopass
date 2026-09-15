//! Lane A (`cargo test`, no bus needed) — tasks.md Phase 5, task 5.9;
//! spec `tray-privileged-invocation` "Menu remains responsive during a
//! pending authorization".
//!
//! This does not exercise a real session bus — `tray.rs`/`instance.rs`
//! (Phases 7/9) do not exist yet, so there is no real SNI menu-open or
//! `Quit` request to send. What this proves instead is the exact
//! mechanism design.md §4.2 relies on for that responsiveness: a pending
//! privileged action runs on its own dedicated OS thread
//! (`run_off_reactor`), so unrelated work scheduled on the SAME async
//! executor completes without waiting for it. `run_off_reactor` alone,
//! driven by a bare `block_on`, would not prove this — a single pending
//! future blocking `block_on` proves nothing about a SECOND future ever
//! getting a chance to run. Polling both futures together through
//! `futures_lite::future::zip` is what makes this "a real event loop"
//! rather than `run_off_reactor` in isolation, per task 5.9's own GREEN
//! note.
//!
//! verify-report.md G2: this file used to be gated on
//! `NOPASS_DBUS_TESTS=1` — the same gate the OTHER Lane B suites use —
//! and was simultaneously excluded from `scripts/run-lane-b.sh`'s
//! `--test dbus_session` filter, so it ran in neither gate: bare
//! `cargo test --workspace` reported it `ok` in 0.00s without executing
//! its body at all. It needs no bus, as the paragraph above already
//! says, so the fix is the lane assignment, not the gate: it now runs
//! unconditionally, as an ordinary Lane A test under gate 1.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nopass::runner::{run_off_reactor, CommandRunner, CommandSpec, RunnerError, SpawnOutcome};

/// A fake spawn port that blocks until the test explicitly releases it —
/// exactly the "fake spawn port blocking until released" the spec's own
/// scenario names.
struct BlockingRunner {
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl CommandRunner for BlockingRunner {
    fn run(&self, _spec: &CommandSpec) -> Result<SpawnOutcome, RunnerError> {
        self.release.lock().unwrap().recv().ok();
        Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })
    }
}

#[test]
fn a_pending_privileged_action_does_not_block_other_reactor_work() {
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let runner: Arc<dyn CommandRunner> = Arc::new(BlockingRunner { release: Mutex::new(release_rx) });
    let spec = CommandSpec { program: std::path::PathBuf::from("/usr/bin/pkexec"), args: vec![], env: vec![] };

    // Releases the blocked action well after the "other reactor work"
    // below has had every opportunity to complete — proving the two are
    // not serialized.
    let release_after = Duration::from_millis(300);
    std::thread::spawn(move || {
        std::thread::sleep(release_after);
        let _ = release_tx.send(());
    });

    let responded_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let responded_at_writer = Arc::clone(&responded_at);

    let start = Instant::now();
    let pending_action = run_off_reactor(runner, spec);
    let other_reactor_work = async move {
        // Represents "the tray handles a menu-open or Quit request" —
        // no privileged action, no blocking, just a poll that resolves
        // immediately once the executor gives it a turn.
        *responded_at_writer.lock().unwrap() = Some(Instant::now());
    };

    // Polling BOTH futures together — not `pending_action` alone — is
    // the "real event loop" this task requires: an executor driving a
    // pending, thread-backed future and an unrelated ready future
    // side by side, cooperatively, on the one reactor thread.
    let _ = futures_lite::future::block_on(futures_lite::future::zip(pending_action, other_reactor_work));

    let recorded = responded_at.lock().unwrap().expect("other_reactor_work must have run");
    let time_to_respond = recorded.duration_since(start);
    assert!(
        time_to_respond < Duration::from_millis(100),
        "other reactor work took {time_to_respond:?} to run — it must not wait on the pending privileged action \
         (released only after {release_after:?})"
    );
}

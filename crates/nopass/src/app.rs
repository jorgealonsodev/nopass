//! `App` — the single owner of every piece of mutable state (design.md
//! §2 `app`, §6, D4). Every adapter (`watch`, the 60 s tick, the ksni
//! tray's menu/activation callbacks, the privileged-action/probe
//! threads, and `AppInterface`'s received `Activate`) only ever produces
//! an [`Event`] into one shared channel; [`run`] is the sole consumer,
//! the sole caller of `TrayPort::render`, and the sole owner of
//! [`TrayState`].
//!
//! ## Where the blocking happens (the decision Phase 7 left for this
//! phase to make explicitly)
//!
//! `TrayPort::render`/`reassert` (`tray.rs`) call `futures_lite::future::
//! block_on` internally to await `ksni::Handle::update`, and `render` is
//! called directly, synchronously, from this module's own single task —
//! which IS the reactor thread: `main::boot` drives [`run`] via
//! `futures_lite::future::block_on` on the process's own thread, and
//! nothing else ever polls it. That `block_on` inside `render` is safe
//! precisely because `ksni` runs its own service loop on its OWN,
//! already-running thread (design.md D1): the call only waits on a fast
//! in-process channel round-trip to that thread, never on a subprocess
//! and never on a human. `pkexec`/`sudo` are the invocations that can
//! block for up to 60 s on a human or a probe of the outside world, and
//! neither one EVER runs on this task: [`App`]'s own action/probe
//! launchers each spawn a dedicated OS thread (the same "OS thread +
//! `async_channel` bridge" pattern `watch.rs`'s debounce thread and
//! `runner::run_off_reactor` already use) that performs the blocking
//! work off-reactor and reports back into the shared [`Event`] channel.
//! The reactor task's own loop only ever awaits `events_rx.recv()` — an
//! `async_channel` receive with no subprocess and no human anywhere in
//! its path.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};

use crate::event::Event;
use crate::format;
use crate::invoke::{pkexec_spec, ActionGate, Locale, Ticket};
use crate::notifications::{action_notification, expiry_notification, Category, NotifyPort};
use crate::outcome::{self, Action, OutcomeKind};
use crate::preflight::Mode;
use crate::probe::{self, Probe, ProbeCache, ProbeError};
use crate::reconcile::{self, TrayState, Trigger};
use crate::runner::{CommandRunner, RunnerError, SpawnOutcome};
use crate::state::{self};
use crate::tray::{TrayPort, ViewModel};
use crate::watch::Watch;

/// Wall-clock seconds since the epoch — the same `now()` shape every
/// pure function in this crate (`merge`, `countdown`, `ProbeCache::
/// usable`, …) already takes as a plain `u64` parameter.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// The single owner of all mutable reconciliation state (design.md §2
/// `app`, D4). Built once by `main::boot`, or by a test with fake ports.
pub struct App {
    user: String,
    state_path: PathBuf,
    mode: Mode,
    tray: Arc<dyn TrayPort + Send + Sync>,
    notify: Arc<dyn NotifyPort + Send + Sync>,
    runner: Arc<dyn CommandRunner>,
    pkexec: PathBuf,
    sudo: PathBuf,
    locale: Locale,
    events_tx: Sender<Event>,
    events_rx: Receiver<Event>,
    action_gate: ActionGate,
    current_state: TrayState,
    probe_cache: Option<ProbeCache>,
    file_observed_at: u64,
    /// Set right after an `Event::ActionFinished` is classified; consumed
    /// by exactly the next `Event::ProbeFinished` this app processes
    /// (design.md §5.1 rule 3). `Event::ProbeFinished` carries no
    /// trigger/context of its own (design.md §2 `event`'s exact shape),
    /// so the escalation check is deliberately "the next probe result to
    /// arrive after this action", not a tagged round-trip.
    pending_escalation: Option<OutcomeKind>,
    /// The directory `watch` is established on — `state_path`'s parent
    /// (design.md D8).
    run_dir: PathBuf,
    uid: u32,
    /// The established inotify watch, once [`maybe_retry_watch`] first
    /// succeeds — `None` until then. Holding it here, rather than
    /// `main::boot`'s former `std::mem::forget`, is what makes both
    /// halves of spec `tray-state-sync`'s "Inotify Watch With
    /// Missing-Directory Fallback" possible: retrying on the next
    /// `Trigger::Tick` needs somewhere to remember "already established,
    /// do not try again", and `main::boot`'s own stack frame does not
    /// survive past its return — only `App`, which lives for the whole
    /// process, does (verify-report.md G1).
    ///
    /// [`maybe_retry_watch`]: App::maybe_retry_watch
    watch: Option<Watch>,
    /// Whether the "could not watch" warning has already been posted for
    /// the CURRENT degraded stretch. The retry runs on every 60 s tick,
    /// so without this the warning is level-triggered and a permanently
    /// missing run directory produces a desktop popup a minute, forever,
    /// for a condition the user was told about the first time. Cleared
    /// the moment a watch is established, so a LATER loss warns again.
    watch_warning_posted: bool,
}

#[allow(clippy::too_many_arguments)]
impl App {
    pub fn new(
        user: String,
        state_path: PathBuf,
        mode: Mode,
        tray: Arc<dyn TrayPort + Send + Sync>,
        notify: Arc<dyn NotifyPort + Send + Sync>,
        runner: Arc<dyn CommandRunner>,
        pkexec: PathBuf,
        sudo: PathBuf,
        locale: Locale,
        events_tx: Sender<Event>,
        events_rx: Receiver<Event>,
        run_dir: PathBuf,
        uid: u32,
    ) -> App {
        App {
            user,
            state_path,
            mode,
            tray,
            notify,
            runner,
            pkexec,
            sudo,
            locale,
            events_tx,
            events_rx,
            action_gate: ActionGate::new(),
            current_state: TrayState::Unknown,
            probe_cache: None,
            file_observed_at: 0,
            pending_escalation: None,
            run_dir,
            uid,
            watch: None,
            watch_warning_posted: false,
        }
    }

    /// A clone of the sender every external adapter (watch, tick,
    /// ksni's menu/activation bridge, `AppInterface`) forwards its
    /// events through — `main::boot` hands this out before calling
    /// [`run`].
    pub fn events_tx(&self) -> Sender<Event> {
        self.events_tx.clone()
    }

    /// design.md §8's `Run(Mode)` rows: announce the degraded capability
    /// exactly once at startup. `Mode::Full` announces nothing.
    fn announce_degraded_mode(&self) {
        match self.mode {
            Mode::Full => {}
            Mode::NoTrayHost => self.notify.post(
                Category::Environment,
                "No tray host found",
                "NoPass could not find a status-notifier host (for example GNOME's AppIndicator \
                 extension). The icon will appear automatically once one becomes available.",
            ),
            Mode::NoNotifications => self.notify.post(
                Category::Environment,
                "No notification service found",
                "NoPass could not find a desktop notification service. Outcomes will still be \
                 shown in the icon and its tooltip, but no toast notifications will appear.",
            ),
        }
    }

    fn render(&self, now: u64) {
        let view = ViewModel::from_state(&self.user, &self.current_state, now);
        self.tray.render(&view);
    }

    /// Establishes the inotify watch if it is not already held — called
    /// once at [`run`]'s startup and again on every `Trigger::Tick` for
    /// as long as it keeps failing, and again after a held watch is lost
    /// (design.md D8; spec `tray-state-sync` "Inotify Watch With
    /// Missing-Directory Fallback"; verify-report.md G1, H4, H5).
    ///
    /// **What `self.watch.is_some()` does NOT do**, despite an earlier
    /// version of this doc comment and of the pinning test's own
    /// comments claiming otherwise (verify-report.md H5): it does not
    /// prevent a duplicate *live* watch. `self.watch = Some(new)`
    /// already drops whatever `Watch` was previously there, on
    /// assignment, which stops that watch's `notify` backend and lets
    /// its debounce thread exit — so at most one is ever live regardless
    /// of whether this short-circuit fires. Experiment N2 (deleting the
    /// guard, but not leaking the old `Watch`) proved exactly that:
    /// every gate stayed green. What the guard actually buys is avoiding
    /// a needless teardown and rebuild of the watcher and its debounce
    /// thread — and the small window that rebuild opens in which a
    /// write could land between the old watch stopping and the new one
    /// starting — on every 60 s tick for the rest of the process's life
    /// when nothing has changed. See
    /// `a_tick_over_an_unchanged_directory_does_not_rebuild_the_watch`
    /// for the test that actually pins that.
    ///
    /// **What actually clears `self.watch` so this can retry a lost
    /// watch**: `Event::WatchLost`, handled in [`App::handle`] — raised
    /// by `watch.rs`'s own callback when it observes the watched
    /// directory itself being removed, or when the `notify` backend
    /// reports an error. `/run/nopass/` is an ordinary tmpfs directory,
    /// not a guarantee that never changes underneath a running process:
    /// nothing prevents root removing it, and the helper's own
    /// `statefile::ensure_run_dir` then recreates it — on a filesystem
    /// free to hand the new directory the very same inode number, which
    /// is why this is detected from the event stream rather than a
    /// `stat(2)`-based identity check (see `watch.rs`'s own comment on
    /// the point; verified empirically against this crate's `notify`
    /// version before relying on it).
    fn maybe_retry_watch(&mut self) {
        if self.watch.is_some() {
            return;
        }
        match Watch::start(&self.run_dir, self.uid, self.events_tx.clone()) {
            Ok(watch) => {
                self.watch = Some(watch);
                // Out of the degraded stretch: a future one warns again.
                self.watch_warning_posted = false;
            }
            Err(_) if self.watch_warning_posted => {}
            Err(_) => {
                self.watch_warning_posted = true;
                self.notify.post(
                    Category::Environment,
                    "Could not watch for changes",
                    &format!(
                        "NoPass could not set up a watch on {}. Falling back to checking every 60 \
                         seconds; it will keep retrying.",
                        self.run_dir.display()
                    ),
                );
            }
        }
    }

    /// One dedicated OS thread per probe request (design.md §4.2's
    /// off-reactor bridge) — the reactor task never spawns `sudo` itself.
    fn spawn_probe(&self) {
        let runner = Arc::clone(&self.runner);
        let sudo = self.sudo.clone();
        let tx = self.events_tx.clone();
        std::thread::spawn(move || {
            let spec = probe::spec(&sudo);
            let outcome = runner.run(&spec);
            let result = probe::interpret(outcome);
            let _ = tx.send_blocking(Event::ProbeFinished(result));
        });
    }

    /// One dedicated OS thread per privileged action (design.md §4.2) —
    /// the only place `pkexec` is spawned. `ticket` is moved into the
    /// thread and dropped only once the invocation returns, so the gate
    /// stays held for the invocation's real duration, not merely for the
    /// duration of this function call (design.md §4.4).
    fn spawn_action(&self, action: Action, ticket: Ticket) {
        let runner = Arc::clone(&self.runner);
        let spec = pkexec_spec(&self.pkexec, Path::new(nopass_core::paths::HELPER_PATH), action, &self.locale);
        let tx = self.events_tx.clone();
        std::thread::spawn(move || {
            let result = runner.run(&spec);
            drop(ticket);
            let _ = tx.send_blocking(Event::ActionFinished(action, result));
        });
    }

    /// design.md §3.4's trigger table, composed with §3.3's cache
    /// freshness for `MenuOpened` — exactly `reconcile::probe_required`
    /// plus `ProbeCache::usable`, the same composition `reconcile.rs`'s
    /// own tests already pin at the pure-function level.
    fn maybe_probe(&mut self, trigger: Trigger, now: u64) {
        let forced = reconcile::probe_required(&self.current_state, trigger);
        let cache_stale =
            trigger == Trigger::MenuOpened && self.probe_cache.and_then(|c| c.usable(self.file_observed_at, now)).is_none();
        if forced || cache_stale {
            self.spawn_probe();
        }
    }

    /// Re-reads the state file, re-merges against the current probe
    /// cache, renders on change, and requests a fresh probe per the
    /// trigger table (design.md §3.2-§3.4, §6.3).
    fn reconcile(&mut self, trigger: Trigger, now: u64) {
        let file = state::read(&self.state_path);
        if trigger == Trigger::FileEvent {
            self.file_observed_at = now;
        }
        let usable_probe = self.probe_cache.and_then(|c| c.usable(self.file_observed_at, now));
        let new_state = reconcile::merge(&file, usable_probe, now);
        if new_state != self.current_state {
            let was_active = matches!(self.current_state, TrayState::Active { .. });
            self.current_state = new_state.clone();
            self.render(now);
            // design.md §6.3: an externally-triggered transition to
            // Inactive (the expiry timer rewriting the state file, an
            // inotify event) gets its own, independent notification —
            // but only "without a pending tray-initiated action"
            // (`tray-notifications` N1). `pending_escalation` is set by
            // `handle_action_finished` and stays set until that same
            // action's own corroborating probe consumes it
            // (`handle_probe_finished`), so it is exactly the "an action
            // is in flight and not yet corroborated" signal this guard
            // needs: without it, a user-initiated Disable's own
            // FileEvent — which can win the race against that probe
            // (verify-report.md G3) — is misread as a spontaneous
            // expiry, immediately after the user was already told
            // "Passwordless sudo disabled".
            if trigger == Trigger::FileEvent
                && was_active
                && matches!(new_state, TrayState::Inactive)
                && self.pending_escalation.is_none()
            {
                let (summary, body) = expiry_notification();
                self.notify.post(Category::Expiry, &summary, &body);
            }
        }
        self.maybe_probe(trigger, now);
    }

    fn handle_toggle(&mut self, now: u64) {
        let Some(ticket) = self.action_gate.try_begin() else {
            // design.md §4.4: a pending action already owns the gate —
            // the toggle is ignored, exactly like a second SNI Activate.
            return;
        };
        let action = match &self.current_state {
            TrayState::Inactive => Action::Enable { until: now + 3600 },
            TrayState::Active { .. } => Action::Disable,
            // `toggle_label(Unknown) == None` (design.md D6): no producer
            // of `Event::ToggleRequested` can reach this arm while the
            // menu/SNI item are wired correctly, but the ticket must
            // still be released rather than leaked if it ever does.
            TrayState::Unknown => return,
        };
        self.spawn_action(action, ticket);
    }

    fn handle_action_finished(&mut self, action: Action, result: Result<SpawnOutcome, RunnerError>, now: u64) {
        let kind = match &result {
            Ok(outcome) => {
                let helper_present = outcome.status != Some(127) || helper_is_executable();
                outcome::classify(action, outcome.status, helper_present)
            }
            Err(RunnerError::NonAbsoluteProgram { .. }) | Err(RunnerError::Spawn { .. }) => OutcomeKind::SpawnFailed,
            Err(RunnerError::Signaled { .. }) => OutcomeKind::Interrupted,
        };

        let countdown = match kind {
            OutcomeKind::Granted { until } => format::countdown(nopass_core::expiry::Expiry::At { epoch: until }, now),
            _ => String::new(),
        };
        let (summary, body) = action_notification(kind, &countdown);
        self.notify.post(Category::Action, &summary, &body);

        // design.md D5/§5.1: an exit code is never evidence of state —
        // `Trigger::ActionCompleted` always forces a fresh probe, and
        // that probe (not this outcome) is what may later escalate.
        self.pending_escalation = Some(kind);
        self.reconcile(Trigger::ActionCompleted, now);
    }

    fn handle_probe_finished(&mut self, result: Result<Probe, ProbeError>, now: u64) {
        if let Some(prev_kind) = self.pending_escalation.take() {
            if let Ok(probe) = result {
                if let Some(escalated) = outcome::escalate(prev_kind, probe) {
                    let (summary, body) = escalated.text();
                    self.notify.post(Category::Action, &summary, &body);
                }
            }
        }

        if let Ok(probe) = result {
            self.probe_cache = Some(ProbeCache { value: probe, taken_at: now });
        }

        let file = state::read(&self.state_path);
        let usable_probe = self.probe_cache.and_then(|c| c.usable(self.file_observed_at, now));
        let new_state = reconcile::merge(&file, usable_probe, now);
        if new_state != self.current_state {
            self.current_state = new_state;
            self.render(now);
        }
    }

    /// The event-handling core (Lane A: fed one `Event` at a time with
    /// fake ports). Returns `false` once `Event::Quit` has been handled —
    /// [`run`]'s only exit condition.
    fn handle(&mut self, event: Event) -> bool {
        let now = now();
        match event {
            Event::FileChanged => self.reconcile(Trigger::FileEvent, now),
            Event::Tick => {
                // spec `tray-state-sync` "retries establishing the watch
                // on each 60 s tick" (verify-report.md G1).
                self.maybe_retry_watch();
                self.reconcile(Trigger::Tick, now);
            }
            Event::WatchLost => {
                // S4's "or the watch is lost" half (verify-report.md
                // H4): drop the dead `Watch` and warn through the
                // notification port — an observed warning, not an
                // `eprintln!` nothing reads. `Event::Tick`'s existing
                // retry (above) is what re-establishes it; this handler
                // does not retry immediately, so the retry cadence stays
                // exactly "on each 60 s tick" per the spec text.
                self.watch = None;
                self.notify.post(
                    Category::Environment,
                    "Lost the change watch",
                    &format!(
                        "NoPass's watch on {} was lost — the directory was replaced or removed. \
                         Falling back to checking every 60 seconds; it will retry establishing a \
                         new watch on the next tick.",
                        self.run_dir.display()
                    ),
                );
            }
            Event::MenuOpened => self.reconcile(Trigger::MenuOpened, now),
            Event::ToggleRequested => self.handle_toggle(now),
            Event::ActivateRequested => self.tray.reassert(),
            // ksni's own StatusNotifierWatcher client already re-issues
            // `RegisterStatusNotifierItem` internally whenever the
            // watcher's `NameOwnerChanged` fires (`tray.rs`'s own doc
            // comment) — `reassert` here only nudges our side of that
            // exchange back out, exactly like a received second-instance
            // Activate does.
            Event::HostAppeared => self.tray.reassert(),
            Event::HostVanished => {}
            Event::ProbeFinished(result) => self.handle_probe_finished(result, now),
            Event::ActionFinished(action, result) => self.handle_action_finished(action, result, now),
            Event::Quit => return false,
        }
        true
    }
}

/// `access(HELPER_PATH, X_OK)` (design.md §5.1's exit-127 disambiguation)
/// — unprivileged, cheap, and only ever consulted on a bare 127.
fn helper_is_executable() -> bool {
    nix::unistd::access(Path::new(nopass_core::paths::HELPER_PATH), nix::unistd::AccessFlags::X_OK).is_ok()
}

/// The reconciliation loop (design.md §2 `app`): drains `app`'s own
/// event channel until `Event::Quit`, announcing the preflight's
/// degraded mode (if any) and running the startup reconciliation
/// (design.md §6.1 step 11) first. Always returns `0` — every non-zero
/// tray exit code (3/4/5) is decided by `main::boot` before an `App`
/// value can even be built (design.md §8).
pub async fn run(mut app: App) -> i32 {
    app.announce_degraded_mode();
    // Step 9's first attempt (design.md §6.1) — retried on every
    // `Trigger::Tick` thereafter if it fails (verify-report.md G1).
    app.maybe_retry_watch();
    app.reconcile(Trigger::Startup, now());

    while let Ok(event) = app.events_rx.recv().await {
        if !app.handle(event) {
            break;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::classify;
    use crate::runner::{CommandSpec, ScriptedRunner};
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct RecordingTray {
        renders: Mutex<Vec<ViewModel>>,
        reasserts: Mutex<u32>,
    }
    impl TrayPort for RecordingTray {
        fn render(&self, view: &ViewModel) {
            self.renders.lock().unwrap().push(view.clone());
        }
        fn reassert(&self) {
            *self.reasserts.lock().unwrap() += 1;
        }
    }

    #[derive(Default)]
    struct RecordingNotify {
        posts: Mutex<Vec<(Category, String, String)>>,
    }
    impl NotifyPort for RecordingNotify {
        fn post(&self, category: Category, summary: &str, body: &str) {
            self.posts.lock().unwrap().push((category, summary.to_string(), body.to_string()));
        }
    }

    fn state_path_never_exists() -> PathBuf {
        // A path that structurally cannot exist — `state::read` maps
        // this to `FileReading::Absent`, exactly as a fresh install
        // would, without needing a real tempdir for tests that do not
        // care about file contents.
        PathBuf::from("/nonexistent/nopass-app-test/does-not-exist.state")
    }

    fn test_app(
        runner: Arc<dyn CommandRunner>,
        tray: Arc<RecordingTray>,
        notify: Arc<RecordingNotify>,
        mode: Mode,
    ) -> App {
        let (tx, rx) = async_channel::unbounded();
        App::new(
            "jorge".to_string(),
            state_path_never_exists(),
            mode,
            tray,
            notify,
            runner,
            PathBuf::from("/usr/bin/pkexec"),
            PathBuf::from("/usr/bin/sudo"),
            Locale::default(),
            tx,
            rx,
            // A path that structurally cannot exist either — every test
            // using this helper drives `App` through `handle`/`reconcile`
            // directly, never `Event::Tick`, so `maybe_retry_watch` is
            // never invoked against it. Tests exercising the watch retry
            // build `App::new` directly with a real `TempDir` instead.
            PathBuf::from("/nonexistent/nopass-app-test/run"),
            0,
        )
    }

    /// Blocks for up to `timeout` for the next event on `rx`; `None` on
    /// timeout — used to assert both "a probe/action was requested" and
    /// "no extra one was" within a bounded window.
    fn recv_within(rx: &Receiver<Event>, timeout: Duration) -> Option<Event> {
        futures_lite::future::block_on(futures_lite::future::or(
            async { rx.recv().await.ok() },
            async {
                async_io::Timer::after(timeout).await;
                None
            },
        ))
    }

    /// Receives the next event within `timeout` and feeds it through
    /// `app.handle`, exactly as `run`'s own loop would — background
    /// producers (`spawn_probe`/`spawn_action`) only ever report into the
    /// channel; nothing except `handle` is allowed to react to them.
    fn recv_and_handle(app: &mut App, timeout: Duration) -> Option<Event> {
        let rx = app.events_rx.clone();
        let event = recv_within(&rx, timeout)?;
        app.handle(event.clone());
        Some(event)
    }

    const SHORT: Duration = Duration::from_millis(500);
    const NONE_EXPECTED: Duration = Duration::from_millis(150);

    // ---- 10.4/10.8: trigger table wiring through App ----

    #[test]
    fn startup_reconciliation_always_spawns_a_probe() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![(
            probe::spec(&PathBuf::from("/usr/bin/sudo")),
            Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] }),
        )]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify, Mode::Full);
        let rx = app.events_rx.clone();

        app.reconcile(Trigger::Startup, now());

        match recv_within(&rx, SHORT) {
            Some(Event::ProbeFinished(Ok(Probe::Passwordless))) => {}
            other => panic!("Startup must always request a probe, got {other:?}"),
        }
    }

    #[test]
    fn menu_opened_with_a_fresh_cache_requests_no_probe() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify, Mode::Full);
        let rx = app.events_rx.clone();
        let now = now();
        app.probe_cache = Some(ProbeCache { value: Probe::PasswordRequired, taken_at: now });
        app.file_observed_at = 0;

        app.reconcile(Trigger::MenuOpened, now);

        assert_eq!(recv_within(&rx, NONE_EXPECTED), None, "a fresh cache must not trigger a probe on MenuOpened");
    }

    #[test]
    fn menu_opened_with_a_stale_cache_requests_a_probe() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![(
            probe::spec(&PathBuf::from("/usr/bin/sudo")),
            Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] }),
        )]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify, Mode::Full);
        let rx = app.events_rx.clone();
        let now = now();
        app.probe_cache = Some(ProbeCache { value: Probe::Passwordless, taken_at: 0 });
        app.file_observed_at = 0;

        app.reconcile(Trigger::MenuOpened, now);

        match recv_within(&rx, SHORT) {
            Some(Event::ProbeFinished(Ok(Probe::PasswordRequired))) => {}
            other => panic!("a stale cache must request a probe on MenuOpened, got {other:?}"),
        }
    }

    // ---- 10.4: the privileged toggle flow, ActionGate wiring ----

    #[test]
    fn toggle_while_inactive_enables_and_a_second_toggle_in_flight_is_ignored() {
        // `ScriptedRunner` asserts exact spec equality per invocation, and
        // the enable spec's `--until` argument is `now() + 3600` — not
        // observable before calling `handle_toggle` — so this test uses a
        // permissive fake runner that accepts any spec instead (see
        // `AnyCommandRunner` below).
        let runner: Arc<dyn CommandRunner> = Arc::new(AnyCommandRunner::new(vec![
            Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] }),
            Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] }),
        ]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify.clone(), Mode::Full);
        let rx = app.events_rx.clone();

        app.current_state = TrayState::Inactive;
        app.handle_toggle(now());
        // The gate is held: a second click while the first is in flight
        // must be ignored (design.md §4.4).
        app.handle_toggle(now());

        match recv_and_handle(&mut app, SHORT) {
            Some(Event::ActionFinished(Action::Enable { .. }, Ok(_))) => {}
            other => panic!("expected the enable action to finish, got {other:?}"),
        }
        // Processing `ActionFinished` above must itself have forced the
        // `Trigger::ActionCompleted` probe (design.md D5/§5.1).
        match recv_within(&rx, SHORT) {
            Some(Event::ProbeFinished(_)) => {}
            other => panic!("ActionCompleted must always force a probe, got {other:?}"),
        }
        assert_eq!(recv_within(&rx, NONE_EXPECTED), None, "a second in-flight toggle must never spawn a second action");

        let posts = notify.posts.lock().unwrap();
        assert!(posts.iter().any(|(c, s, _)| *c == Category::Action && s == "Passwordless sudo enabled"));
    }

    /// A `CommandRunner` fake that returns its scripted results in order
    /// without asserting anything about the exact `CommandSpec` it was
    /// called with — used only where the spec is intentionally
    /// non-deterministic (an `--until` epoch computed from `now()`).
    struct AnyCommandRunner {
        results: Mutex<std::collections::VecDeque<Result<SpawnOutcome, RunnerError>>>,
    }
    impl AnyCommandRunner {
        fn new(results: Vec<Result<SpawnOutcome, RunnerError>>) -> AnyCommandRunner {
            AnyCommandRunner { results: Mutex::new(results.into()) }
        }
    }
    impl CommandRunner for AnyCommandRunner {
        fn run(&self, _spec: &CommandSpec) -> Result<SpawnOutcome, RunnerError> {
            self.results.lock().unwrap().pop_front().unwrap_or(Err(RunnerError::Spawn {
                program: "test".to_string(),
                reason: "AnyCommandRunner script exhausted".to_string(),
            }))
        }
    }

    // ---- 10.4/§5.1: escalation wiring ----

    #[test]
    fn a_timer_unscheduled_outcome_escalates_when_the_forced_probe_returns_passwordless() {
        let runner: Arc<dyn CommandRunner> =
            Arc::new(AnyCommandRunner::new(vec![Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify.clone(), Mode::Full);

        let kind = classify(Action::Enable { until: 1 }, Some(17), true);
        assert_eq!(kind, OutcomeKind::TimerUnscheduled);
        app.pending_escalation = Some(kind);
        app.handle_probe_finished(Ok(Probe::Passwordless), now());

        let posts = notify.posts.lock().unwrap();
        assert!(
            posts.iter().any(|(_, s, _)| *s == OutcomeKind::UnexpiringGrant.text().0),
            "a Passwordless probe following TimerUnscheduled must post the UnexpiringGrant warning: {posts:?}"
        );
        assert!(app.pending_escalation.is_none(), "the pending escalation must be consumed exactly once");
    }

    #[test]
    fn a_timer_unscheduled_outcome_does_not_escalate_when_the_forced_probe_returns_password_required() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify.clone(), Mode::Full);

        app.pending_escalation = Some(OutcomeKind::TimerUnscheduled);
        app.handle_probe_finished(Ok(Probe::PasswordRequired), now());

        let posts = notify.posts.lock().unwrap();
        assert!(!posts.iter().any(|(_, s, _)| *s == OutcomeKind::UnexpiringGrant.text().0));
    }

    // ---- 10.8: degraded startup announces exactly the right thing ----

    #[test]
    fn full_mode_announces_nothing_at_startup() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let app = test_app(runner, tray, notify.clone(), Mode::Full);
        app.announce_degraded_mode();
        assert!(notify.posts.lock().unwrap().is_empty());
    }

    #[test]
    fn no_tray_host_mode_posts_exactly_one_environment_notification_and_does_not_exit() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let app = test_app(runner, tray, notify.clone(), Mode::NoTrayHost);
        app.announce_degraded_mode();
        let posts = notify.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, Category::Environment);
    }

    #[test]
    fn host_appearing_later_reasserts_the_tray_without_needing_a_restart() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray.clone(), notify, Mode::NoTrayHost);
        assert!(app.handle(Event::HostAppeared), "HostAppeared must never end the loop");
        assert_eq!(*tray.reasserts.lock().unwrap(), 1);
    }

    // ---- 10.4: the shutdown path ----

    #[test]
    fn quit_event_ends_the_handle_loop() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify, Mode::Full);
        assert!(!app.handle(Event::Quit), "Quit must be the only event that stops the loop");
    }

    #[test]
    fn every_non_quit_event_keeps_the_loop_running() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify, Mode::Full);
        assert!(app.handle(Event::ActivateRequested));
        assert!(app.handle(Event::HostVanished));
    }

    // ---- G1 (verify-report.md): the watch retry on Trigger::Tick ----

    /// Drains `rx` until nothing new arrives for `quiet`, counting how
    /// many `Event::FileChanged` were seen along the way. `Trigger::Tick`
    /// also always forces a probe (`reconcile::probe_required`), which
    /// pushes its own `Event::ProbeFinished` into the same channel; this
    /// lets the watch-retry test below prove both "the retried watch
    /// works" and "at most one watch is ever live" without coupling
    /// either assertion to that unrelated probe traffic.
    fn count_file_changed_within(rx: &Receiver<Event>, quiet: Duration) -> usize {
        let mut count = 0;
        loop {
            match recv_within(rx, quiet) {
                Some(Event::FileChanged) => count += 1,
                Some(_) => continue,
                None => return count,
            }
        }
    }

    #[test]
    fn a_tick_retries_the_watch_until_established_and_never_spawns_a_second_one() {
        // G1: `Watch::start` had exactly one production call site
        // (`main.rs`'s own startup step), which never retried, and
        // `App` had no field to retry with. This drives the real
        // `Event::Tick` path end to end — nothing here calls
        // `maybe_retry_watch` directly — so removing the retry call from
        // `Event::Tick`'s handling leaves `app.watch` `None` forever and
        // the second assertion below fails.
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("nopass");
        // Deliberately does not exist yet: a tray started before the
        // helper's first grant on this boot, per the finding's own
        // "real machine" scenario (verify-report.md G1).
        let state_path = run_dir.join("1000.state");

        let runner: Arc<dyn CommandRunner> = Arc::new(AnyCommandRunner::new(vec![
            Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] }),
            Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] }),
            Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] }),
        ]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let (tx, rx) = async_channel::unbounded();
        let mut app = App::new(
            "jorge".to_string(),
            state_path,
            Mode::Full,
            tray,
            notify,
            runner,
            PathBuf::from("/usr/bin/pkexec"),
            PathBuf::from("/usr/bin/sudo"),
            Locale::default(),
            tx,
            rx.clone(),
            run_dir.clone(),
            1000,
        );

        // Tick #1: the directory does not exist yet — the retry must
        // fail quietly and leave the app with no watch.
        assert!(app.handle(Event::Tick));
        assert!(app.watch.is_none(), "no watch can exist before the run directory does");

        std::fs::create_dir(&run_dir).unwrap();

        // Tick #2: the directory now exists — this is the behaviour
        // under test.
        assert!(app.handle(Event::Tick));
        assert!(app.watch.is_some(), "a tick after the directory appears must establish the watch");

        std::fs::write(run_dir.join("1000.state"), b"{}").unwrap();
        assert_eq!(
            count_file_changed_within(&rx, Duration::from_millis(700)),
            1,
            "the retried watch must observe a write under the run directory exactly once"
        );

        // Tick #3: a watch is already held and the directory has not
        // changed — the write below must still be observed exactly once.
        // NOTE (verify-report.md H5): this assertion alone does NOT prove
        // the guard skipped a rebuild. `self.watch = Some(new)` already
        // drops any previous `Watch` on assignment, so at most one watch
        // is ever LIVE whether or not `maybe_retry_watch` rebuilds on
        // this tick — a rebuilt watch would still be the one that
        // observes the write below, and the count would still be 1.
        // `a_tick_over_an_unchanged_directory_does_not_rebuild_the_watch`
        // is the test that actually pins "no rebuild happened", by
        // comparing the debounce thread's identity across the tick.
        assert!(app.handle(Event::Tick));
        std::fs::write(run_dir.join("1000.state"), b"{}").unwrap();
        assert_eq!(
            count_file_changed_within(&rx, Duration::from_millis(700)),
            1,
            "at most one watch is ever live, rebuilt or not"
        );
    }

    #[test]
    fn a_run_directory_that_never_appears_warns_once_not_once_per_tick() {
        // The retry loop is what makes this reachable: every 60 s tick
        // calls `maybe_retry_watch`, and on a machine where the run
        // directory stays missing every one of those calls used to post
        // its own desktop notification — a popup a minute, forever, for
        // a condition the user was already told about. The warning is
        // about ENTERING the degraded state, so it is edge-triggered.
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("nopass");
        // Never created, for the whole test.
        let state_path = run_dir.join("1000.state");

        let runner: Arc<dyn CommandRunner> = Arc::new(AnyCommandRunner::new(
            (0..10)
                .map(|_| Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] }))
                .collect(),
        ));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let (tx, rx) = async_channel::unbounded();
        let mut app = App::new(
            "jorge".to_string(),
            state_path,
            Mode::Full,
            tray,
            notify.clone(),
            runner,
            PathBuf::from("/usr/bin/pkexec"),
            PathBuf::from("/usr/bin/sudo"),
            Locale::default(),
            tx,
            rx.clone(),
            run_dir.clone(),
            1000,
        );

        for _ in 0..10 {
            assert!(app.handle(Event::Tick));
        }
        assert!(app.watch.is_none(), "the directory never appeared, so no watch can exist");

        let posts = notify.posts.lock().unwrap();
        let warnings = posts
            .iter()
            .filter(|(c, s, _)| *c == Category::Environment && s == "Could not watch for changes")
            .count();
        assert_eq!(
            warnings, 1,
            "ten ticks over a permanently missing run directory must warn exactly once, not once \
             per tick: {posts:?}"
        );
        drop(posts);

        // Edge-triggered, not warn-once-ever. Recover, then degrade a
        // second time: that is a NEW stretch and must warn again.
        std::fs::create_dir(&run_dir).unwrap();
        assert!(app.handle(Event::Tick));
        assert!(app.watch.is_some(), "a tick after the directory appears must establish the watch");

        std::fs::remove_dir_all(&run_dir).unwrap();
        // Stand in for `Event::WatchLost`'s effect without racing real
        // inotify; the loss path itself is pinned by
        // `losing_the_watch_falls_back_and_warns_and_a_later_tick_reestablishes_it`.
        app.watch = None;
        assert!(app.handle(Event::Tick));

        let posts = notify.posts.lock().unwrap();
        let warnings = posts
            .iter()
            .filter(|(c, s, _)| *c == Category::Environment && s == "Could not watch for changes")
            .count();
        assert_eq!(
            warnings, 2,
            "a second degraded stretch is a new event and must warn again — suppressing it would \
             leave the user with no notice at all: {posts:?}"
        );
    }

    #[test]
    fn a_tick_over_an_unchanged_directory_does_not_rebuild_the_watch() {
        // H5 (verify-report.md): experiment N2 — deleting the
        // `self.watch.is_some()` short-circuit from `maybe_retry_watch`,
        // without also leaking the old `Watch` — left every gate green,
        // including the G1 test above, because assigning a fresh `Watch`
        // over `self.watch` already drops the old one. No test that only
        // counts observed `FileChanged` events can tell "rebuilt every
        // tick" apart from "built once and reused". This test asserts
        // the thing the guard actually buys instead: the exact same
        // debounce thread survives a tick over a directory whose
        // identity has not changed. Removing the guard rebuilds the
        // watch — and therefore spawns a fresh debounce thread — on
        // every tick, which fails the `assert_eq!` below even though the
        // event-count test above keeps passing.
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("nopass");
        std::fs::create_dir(&run_dir).unwrap();
        let state_path = run_dir.join("1000.state");

        let runner: Arc<dyn CommandRunner> = Arc::new(AnyCommandRunner::new(vec![
            Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] }),
            Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] }),
        ]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let (tx, rx) = async_channel::unbounded();
        let mut app = App::new(
            "jorge".to_string(),
            state_path,
            Mode::Full,
            tray,
            notify,
            runner,
            PathBuf::from("/usr/bin/pkexec"),
            PathBuf::from("/usr/bin/sudo"),
            Locale::default(),
            tx,
            rx,
            run_dir.clone(),
            1000,
        );

        assert!(app.handle(Event::Tick));
        let first_thread =
            app.watch.as_ref().expect("directory exists, the watch must be established").debounce_thread_id();

        assert!(app.handle(Event::Tick));
        let second_thread = app.watch.as_ref().expect("the watch must still be held").debounce_thread_id();

        assert_eq!(
            first_thread, second_thread,
            "a tick over an unchanged directory must not rebuild the watch"
        );
    }

    /// Blocks until `Event::WatchLost` is seen, feeding every other event
    /// received along the way through `app.handle` exactly as `run`'s own
    /// loop would (the forced `Trigger::Tick` probe's own `ProbeFinished`
    /// is expected traffic here and must not be mistaken for a timeout).
    fn drain_until_watch_lost(app: &mut App, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(remaining > Duration::ZERO, "timed out waiting for Event::WatchLost");
            match recv_and_handle(app, remaining) {
                Some(Event::WatchLost) => return,
                Some(_) => continue,
                None => panic!("timed out waiting for Event::WatchLost"),
            }
        }
    }

    #[test]
    fn losing_the_watch_falls_back_and_warns_and_a_later_tick_reestablishes_it() {
        // H4 (verify-report.md): S4's "or the watch is lost" half, and
        // "show a warning". `/run/nopass/` is an ordinary tmpfs
        // directory, not a guarantee — nothing prevents it being removed
        // and recreated (exactly what the helper's own
        // `statefile::ensure_run_dir` does). This drives that scenario
        // end to end: establish the watch, remove the directory it is
        // watching, let `watch.rs`'s own loss detection raise
        // `Event::WatchLost`, assert the fallback warns through the
        // notification port (an observed warning, not an `eprintln!`
        // nothing reads), recreate the directory, and assert the next
        // tick re-establishes a working watch.
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("nopass");
        std::fs::create_dir(&run_dir).unwrap();
        let state_path = run_dir.join("1000.state");

        let runner: Arc<dyn CommandRunner> = Arc::new(AnyCommandRunner::new(vec![
            Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] }),
            Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] }),
        ]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let (tx, rx) = async_channel::unbounded();
        let mut app = App::new(
            "jorge".to_string(),
            state_path,
            Mode::Full,
            tray,
            notify.clone(),
            runner,
            PathBuf::from("/usr/bin/pkexec"),
            PathBuf::from("/usr/bin/sudo"),
            Locale::default(),
            tx,
            rx,
            run_dir.clone(),
            1000,
        );

        assert!(app.handle(Event::Tick));
        assert!(app.watch.is_some(), "the watch must be established over the original directory");

        // Simulate root removing `/run/nopass`.
        std::fs::remove_dir_all(&run_dir).unwrap();

        drain_until_watch_lost(&mut app, Duration::from_secs(2));
        assert!(app.watch.is_none(), "a lost watch must be dropped, not held onto as though still live");

        let posts = notify.posts.lock().unwrap();
        assert!(
            posts.iter().any(|(c, s, _)| *c == Category::Environment && s == "Lost the change watch"),
            "losing the watch must post an observed warning through the notification port, not just \
             an eprintln! nothing reads: {posts:?}"
        );
        drop(posts);

        // The helper recreates `/run/nopass` on demand; simulate that,
        // then let the next tick retry, exactly like the startup case.
        std::fs::create_dir(&run_dir).unwrap();
        assert!(app.handle(Event::Tick));
        assert!(app.watch.is_some(), "a later tick must re-establish the watch once the directory exists again");
    }

    // ---- G9 (verify-report.md): the MenuOpened cache-staleness clause
    // and `file_observed_at`, isolated from the unrelated `Unknown`
    // rule ----

    #[test]
    fn menu_opened_with_a_stale_cache_forces_a_probe_even_when_the_state_is_not_unknown() {
        // F3: `reconcile::probe_required(Unknown, MenuOpened)` already
        // returns `true` on its own, so a test that leaves `current_state`
        // at its default `Unknown` cannot tell `maybe_probe`'s
        // cache-staleness clause apart from that unrelated rule — which
        // is exactly why experiment F3 (deleting the clause) stayed
        // green despite `menu_opened_with_a_stale_cache_requests_a_probe`
        // existing. Forcing `current_state` to `Inactive` here isolates
        // it: `probe_required` alone now returns `false`, so only the
        // cache-staleness clause can be responsible for the probe this
        // test expects.
        let runner: Arc<dyn CommandRunner> =
            Arc::new(AnyCommandRunner::new(vec![Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify, Mode::Full);
        let rx = app.events_rx.clone();
        let now = now();

        app.current_state = TrayState::Inactive;
        app.probe_cache = Some(ProbeCache { value: Probe::Passwordless, taken_at: 0 });
        app.file_observed_at = now; // taken_at(0) < file_observed_at(now) => stale.

        app.maybe_probe(Trigger::MenuOpened, now);

        match recv_within(&rx, SHORT) {
            Some(Event::ProbeFinished(_)) => {}
            other => panic!("a stale cache must force a probe on MenuOpened even when state != Unknown, got {other:?}"),
        }
    }

    #[test]
    fn a_file_event_stamps_file_observed_at_with_the_observation_time() {
        // F5: experiment F5 deleted this one assignment and all 438
        // tests stayed green. It is what makes design §3.3's "the probe
        // wins is not the oldest probe wins" rule real — without it, a
        // probe cached before an out-of-band file rewrite would stay
        // `usable()` after that rewrite.
        let runner: Arc<dyn CommandRunner> =
            Arc::new(AnyCommandRunner::new(vec![Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify, Mode::Full);
        let now = now();
        app.file_observed_at = 0;

        app.reconcile(Trigger::FileEvent, now);

        assert_eq!(app.file_observed_at, now, "a FileEvent must stamp file_observed_at with its own observation time");
    }

    // ---- G5 (verify-report.md): App::render actually calling
    // TrayPort::render ----

    #[test]
    fn a_probe_driven_state_transition_calls_render_on_the_tray_port() {
        // F18: experiment F18 replaced `App::render`'s body so it never
        // called `TrayPort::render`, and all 438 tests stayed green —
        // nothing observed `App` pushing a `ViewModel` after a state
        // change.
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray.clone(), notify, Mode::Full);
        let now = now();

        assert!(tray.renders.lock().unwrap().is_empty(), "no render before any reconciliation");

        // Absent file + a Passwordless probe merges to Active (row 1) —
        // a genuine transition away from the default Unknown.
        app.handle_probe_finished(Ok(Probe::Passwordless), now);

        let renders = tray.renders.lock().unwrap();
        assert_eq!(renders.len(), 1, "the Unknown -> Active transition must push exactly one render");
        assert_eq!(renders[0].toggle, Some("Disable passwordless sudo"), "the pushed ViewModel must reflect the new Active state");
    }

    // ---- G3/Fix 4 (verify-report.md): the expiry-notification trigger,
    // and the "without a pending tray-initiated action" guard ----

    fn write_inactive_state_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            br#"{"schema":1,"uid":1000,"user":"jorge","active":false,"expires":null,"rule_path":"x","updated_at":1}"#,
        )
        .unwrap();
    }

    #[test]
    fn a_file_event_transitioning_active_to_inactive_with_no_pending_action_announces_expiry() {
        // E2: experiment E2 deleted the whole notification block from
        // `reconcile` and all 438 tests stayed green — the only Lane B
        // test naming this scenario drives the pure composer
        // (`expiry_notification()`) directly and posts it through the
        // notifier port itself; it never drives `App` through a state
        // transition. This does.
        let dir = tempfile::TempDir::new().unwrap();
        let state_path = dir.path().join("1000.state");
        write_inactive_state_file(&state_path);

        let runner: Arc<dyn CommandRunner> =
            Arc::new(AnyCommandRunner::new(vec![Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] })]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let (tx, rx) = async_channel::unbounded();
        let mut app = App::new(
            "jorge".to_string(),
            state_path,
            Mode::Full,
            tray,
            notify.clone(),
            runner,
            PathBuf::from("/usr/bin/pkexec"),
            PathBuf::from("/usr/bin/sudo"),
            Locale::default(),
            tx,
            rx,
            dir.path().to_path_buf(),
            1000,
        );
        app.current_state = TrayState::Active { user: Some("jorge".to_string()), expiry: None };

        app.reconcile(Trigger::FileEvent, now());

        assert_eq!(app.current_state, TrayState::Inactive);
        let posts = notify.posts.lock().unwrap();
        assert!(
            posts.iter().any(|(c, s, _)| *c == Category::Expiry && s == "Passwordless sudo expired"),
            "an out-of-band Active -> Inactive FileEvent transition must announce expiry: {posts:?}"
        );
    }

    #[test]
    fn a_file_event_during_a_pending_action_does_not_announce_expiry() {
        // Fix 4 / G3(a): reproduces the exact race the report names —
        // the helper's own state-file rewrite (a FileEvent) can beat the
        // action's own corroborating probe back to the app. Before the
        // guard, this FileEvent-triggered Active -> Inactive transition
        // was announced as an out-of-band expiry immediately after the
        // user was already told "Passwordless sudo disabled".
        let dir = tempfile::TempDir::new().unwrap();
        let state_path = dir.path().join("1000.state");
        write_inactive_state_file(&state_path);

        let runner: Arc<dyn CommandRunner> =
            Arc::new(AnyCommandRunner::new(vec![Ok(SpawnOutcome { status: Some(1), stdout: vec![], stderr: vec![] })]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let (tx, rx) = async_channel::unbounded();
        let mut app = App::new(
            "jorge".to_string(),
            state_path,
            Mode::Full,
            tray,
            notify.clone(),
            runner,
            PathBuf::from("/usr/bin/pkexec"),
            PathBuf::from("/usr/bin/sudo"),
            Locale::default(),
            tx,
            rx,
            dir.path().to_path_buf(),
            1000,
        );
        app.current_state = TrayState::Active { user: Some("jorge".to_string()), expiry: None };
        // A Disable action has just finished and is awaiting its own
        // corroborating probe (design §5.1 rule 3) — exactly the
        // "pending tray-initiated action" state the spec's guard names.
        app.pending_escalation = Some(OutcomeKind::Revoked);

        app.reconcile(Trigger::FileEvent, now());

        assert_eq!(app.current_state, TrayState::Inactive, "the file event must still be reflected in the merged state");
        let posts = notify.posts.lock().unwrap();
        assert!(
            !posts.iter().any(|(c, _, _)| *c == Category::Expiry),
            "a FileEvent transition to Inactive during a pending action must not be announced as expiry: {posts:?}"
        );
    }
}

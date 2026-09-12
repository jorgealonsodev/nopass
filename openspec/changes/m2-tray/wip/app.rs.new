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
            // inotify event) gets its own, independent notification.
            if trigger == Trigger::FileEvent && was_active && matches!(new_state, TrayState::Inactive) {
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
            Event::Tick => self.reconcile(Trigger::Tick, now),
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
}

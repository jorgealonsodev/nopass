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

use crate::autostart::{self, AutostartState};
use crate::config::{self, Config, ConfigFault};
use crate::consent::ConsentState;
use crate::duration::GrantDuration;
use crate::event::Event;
use crate::format;
use crate::invoke::{pkexec_spec, ActionGate, Locale, Ticket};
use crate::menu::MenuModel;
use crate::notifications::{action_notification, expiry_notification, Category, NotifyPort};
use crate::outcome::{self, Action, EnableRequest, OutcomeKind};
use crate::preflight::{Mode, PolkitReadiness};
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
    /// `~/.config/nopass/config.toml`'s resolved path — computed once by
    /// `main::boot` and held here so every write (consent persistence,
    /// the default-duration marker) and every `Trigger::MenuOpened`
    /// re-read (task 8.3, design.md §4 D4) goes through the same path
    /// without re-resolving `XDG_CONFIG_HOME` per call.
    config_path: PathBuf,
    /// `~/.config/autostart/nopass.desktop`'s resolved path — computed
    /// once by `main::boot`, mirroring `config_path`. Held rather than
    /// calling `autostart::path()` (a real-environment lookup) at every
    /// read/write site: a test app must never be able to touch the
    /// process's REAL `$HOME`/`$XDG_CONFIG_HOME` autostart entry.
    autostart_path: PathBuf,
    /// The resolved user preferences (design.md §4 D4; task 8.1). Read at
    /// startup and re-read at every `Trigger::MenuOpened`; every field has
    /// a safe default, so a faulted or absent file never blocks startup.
    config: Config,
    /// The consent state machine (design.md §3 D3; task 8.1) —
    /// `handle_toggle`/`handle_duration_selected`/`handle_consent_*` are
    /// the ONLY functions in this module that ever touch it, and every one
    /// of them routes through `ConsentState::grant`/`arm`/`confirm`/
    /// `cancel`, never a locally-fabricated acknowledged state.
    consent: ConsentState,
    /// The preflight polkit readiness ladder's last result (design.md §0
    /// D6, task 8.4) — the field verify-report.md H6/G6 flagged as
    /// "computed then discarded". Updated at startup and again on every
    /// `Event::PolkitReadinessChanged` (task 8.6's recovery-without-a-
    /// restart requirement).
    polkit: PolkitReadiness,
    /// Set right after a faulted `config::read` is surfaced; consumed
    /// (cleared) the moment a re-read comes back healthy — the same
    /// edge-triggered-warning discipline `watch_warning_posted` already
    /// applies to the watch (commit `29ebe32`), task 8.3's second surface
    /// for the same rule.
    last_warned_fault: Option<ConfigFault>,
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
        config_path: PathBuf,
        autostart_path: PathBuf,
        config: Config,
        polkit: PolkitReadiness,
    ) -> App {
        let consent = ConsentState::from_config(&config);
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
            config_path,
            autostart_path,
            config,
            consent,
            polkit,
            last_warned_fault: None,
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
    ///
    /// Also posts design.md §0 D6's startup `ActionMissing` notification
    /// (task 8.4; verify-report.md H6/G6's open gap — `PolkitReadiness`
    /// was computed and discarded, so the tray never said anything even
    /// though `probe_polkit_readiness` had already asked the real
    /// authority). This is independent of `self.mode`: a `Full` preflight
    /// mode says nothing about whether polkit itself is ready.
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
        if let PolkitReadiness::ActionMissing(reason) = self.polkit {
            self.notify.post(
                Category::Environment,
                format::Msg::NotifyPolkitUnavailableSummary.text(format::lang()),
                &format::Msg::NotifyPolkitUnavailableBody.text(format::lang()).replace("{}", reason),
            );
        }
    }

    fn render(&self, now: u64) {
        let view = ViewModel::from_state(&self.user, &self.current_state, now);
        self.tray.render(&view);
    }

    /// Assembles the full [`MenuModel`] `menu_tree` needs from `App`'s own
    /// state plus a fresh read of the two on-disk sources `menu.rs` itself
    /// never caches (design.md §3 data flow; task 8.3's
    /// `Trigger::MenuOpened` re-read half; §4 D5's "read at every menu
    /// open, never cached across menu opens" for autostart).
    fn build_menu_model(&self, now: u64) -> MenuModel {
        let file = state::read(&self.state_path);
        let autostart = autostart::read(&self.autostart_path);
        MenuModel {
            view: ViewModel::from_state(&self.user, &self.current_state, now),
            state: self.current_state.clone(),
            file,
            now,
            config: self.config,
            consent_branch: self.consent.branch(),
            autostart,
            polkit: self.polkit,
            action_in_flight: self.action_gate.in_flight(),
        }
    }

    /// Pushes a freshly built menu tree (task 8.7's wiring point) — called
    /// at every `Trigger::MenuOpened` and again after any event that could
    /// change what the menu would show (a duration armed/confirmed/
    /// cancelled, a default-duration or autostart change, a polkit
    /// readiness change), so the exported menu is never one click behind
    /// `App`'s own state.
    fn render_menu_now(&self, now: u64) {
        let model = self.build_menu_model(now);
        self.tray.render_menu(&model);
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
        let spec = pkexec_spec(&self.pkexec, Path::new(nopass_core::paths::HELPER_PATH), &action, &self.locale);
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

    /// The text every non-menu activation path (left click, keyboard
    /// `Activate` — both raise the same `Event::ToggleRequested` this
    /// function handles; a menu-triggered toggle click reaches this exact
    /// same function too, per spec `activation-consent`'s "written once...
    /// not duplicated per caller") posts while consent is unrecorded
    /// (design.md §3 D3 "Paths that cannot show a menu"; spec
    /// `activation-consent` "No Grant Dispatch Without Recorded Consent").
    // These were raw English constants. `format.rs` opens by promising that
    // every user-facing string the tray renders lives there, and this is the
    // most important message in the whole change: it is what a user sees the
    // first time the tray refuses to grant. Showing it in English to a
    // Spanish user, while the menu beside it speaks Spanish, defeats it.
    fn unconsented_toggle_summary() -> String {
        format::Msg::NotifyConsentNeededSummary.text(format::lang()).to_string()
    }
    fn unconsented_toggle_body() -> String {
        format::Msg::NotifyConsentNeededBody.text(format::lang()).to_string()
    }

    /// design.md §3 D3/§1's single gate every activation-requesting caller
    /// converges on (task 8.1). `Event::ToggleRequested` is shared by the
    /// menu's toggle item, an SNI left-click, and keyboard `Activate`
    /// (`tray.rs::Inner::activate`) — there is exactly one code path here,
    /// not one per caller, so a bypass cannot hide behind an unaudited
    /// second copy (spec `activation-consent` "This check MUST be
    /// enforced at the one point...").
    fn handle_toggle(&mut self, now: u64) {
        let Some(ticket) = self.action_gate.try_begin() else {
            // design.md §4.4: a pending action already owns the gate —
            // the toggle is ignored, exactly like a second SNI Activate.
            return;
        };
        match &self.current_state {
            TrayState::Inactive => match self.consent.grant() {
                Some(granted) => {
                    let action = Action::Enable(EnableRequest::new(self.config.default_duration, now, granted));
                    self.spawn_action(action, ticket);
                }
                None => {
                    // design.md §3 D3: left click, keyboard Activate, and
                    // the menu's own bare toggle item all take this exact
                    // arm while unacknowledged — none of them can render
                    // a branch (that needs a specific duration, which only
                    // `Event::DurationSelected` carries). No invocation is
                    // ever made; the ticket is released immediately.
                    drop(ticket);
                    self.notify.post(Category::Environment, &Self::unconsented_toggle_summary(), &Self::unconsented_toggle_body());
                }
            },
            TrayState::Active { .. } => self.spawn_action(Action::Disable, ticket),
            // `toggle_availability(Unknown, ..) == Unavailable(StateUnknown)`
            // (design.md D6): no producer of `Event::ToggleRequested` can
            // reach this arm while the menu/SNI item are wired correctly,
            // but the ticket must still be released rather than leaked if
            // it ever does.
            TrayState::Unknown => drop(ticket),
        }
        self.render_menu_now(now);
    }

    /// One entry inside "Activate during…" (design.md §1 "The consent
    /// branch"; spec `activation-consent` "First Activation Branches the
    /// Menu Instead of Granting", "A later activation skips the branch
    /// once consent is recorded"; task 8.1). Already-acknowledged consent
    /// dispatches the CHOSEN duration directly — never `config.
    /// default_duration`, which is `handle_toggle`'s job alone. Otherwise
    /// this arms the branch and nothing is invoked (design.md §5's
    /// sequence: "NOTHING IS INVOKED. ActionGate untouched. No pkexec. No
    /// helper.").
    fn handle_duration_selected(&mut self, duration: GrantDuration, now: u64) {
        match self.consent.grant() {
            Some(granted) => {
                let Some(ticket) = self.action_gate.try_begin() else { return };
                let action = Action::Enable(EnableRequest::new(duration, now, granted));
                self.spawn_action(action, ticket);
            }
            None => self.consent.arm(duration),
        }
        self.render_menu_now(now);
    }

    /// The consent branch's "I understand — activate[, and don't warn me
    /// again]" (spec `activation-consent` "Confirming the branch grants
    /// exactly once", "Don't-Warn-Again Persists Consent", "A Failed
    /// Consent Write Re-Warns Rather Than Silently Granting"; task 8.1).
    /// `persist`'s write goes through the SAME atomic `config::write`
    /// every other config mutation uses; `ConsentState::confirm` only
    /// acknowledges consent once that write actually succeeds (never on an
    /// unpersisted in-memory flag), and this function mirrors that back
    /// into `self.config` — never unconditionally, only when `grant()`
    /// just started succeeding.
    fn handle_consent_confirmed(&mut self, persist: bool, now: u64) {
        let Some(ticket) = self.action_gate.try_begin() else { return };
        let config_path = self.config_path.clone();
        let mut candidate = self.config;
        candidate.warning_acknowledged = true;
        let result = self.consent.confirm(persist, || config::write(&config_path, &candidate).is_ok());
        if self.consent.grant().is_some() {
            self.config.warning_acknowledged = true;
        }
        match result {
            Some((duration, granted)) => {
                let action = Action::Enable(EnableRequest::new(duration, now, granted));
                self.spawn_action(action, ticket);
            }
            // `confirm` with nothing armed (a stale/duplicate click) — stay
            // total, release the ticket rather than leak it.
            None => drop(ticket),
        }
        self.render_menu_now(now);
    }

    /// The consent branch's "Cancel" (spec `activation-consent`
    /// "Cancelling the branch grants nothing": no consent state changes,
    /// zero invocations).
    fn handle_consent_cancelled(&mut self, now: u64) {
        self.consent.cancel();
        self.render_menu_now(now);
    }

    /// A "Default duration" `RadioGroup` entry (spec `tray-menu`
    /// "Selecting a new default moves the marker and persists it"; task
    /// 8.1). Reuses `menu::select_default_duration` — this crate's one
    /// write-through helper for this exact field — rather than calling
    /// `config::write` a second, independently-composed way. A write
    /// failure leaves `self.config` unchanged: the marker stays where it
    /// was rather than showing a value the disk does not actually hold.
    fn handle_default_duration_selected(&mut self, duration: GrantDuration, now: u64) {
        if let Ok(updated) = crate::menu::select_default_duration(&self.config_path, self.config, duration) {
            self.config = updated;
        }
        self.render_menu_now(now);
    }

    /// "Start with session" (design.md §4 D5; spec `autostart-entry`).
    /// `checked`'s next value always comes from a fresh `autostart::read`
    /// inside `render_menu_now` — never cached here — so a write failure
    /// (surfaced only as "nothing changed" on the next render) can never
    /// desync the checkbox from on-disk truth.
    fn handle_autostart_toggled(&mut self, now: u64) {
        match autostart::read(&self.autostart_path) {
            AutostartState::Enabled => {
                let _ = autostart::disable(&self.autostart_path);
            }
            AutostartState::Disabled => {
                let _ = autostart::enable(&self.autostart_path);
            }
            // `toggle_label`/`menu.rs::autostart_node` already render this
            // insensitive (`enabled: autostart != Indeterminate`) — an
            // event reaching here anyway is a no-op, not a state guess.
            AutostartState::Indeterminate => {}
        }
        self.render_menu_now(now);
    }

    /// design.md §0 D6, task 8.4/8.6: re-runs the polkit readiness ladder
    /// after `org.freedesktop.PolicyKit1`'s `NameOwnerChanged` fires
    /// (`main.rs`'s bus subscription is what actually re-probes and sends
    /// this) — `polkitd` restarting on a package upgrade must recover the
    /// toggle without a tray restart, and a NEWLY-missing action must
    /// disable it the same way, without waiting for the next menu open.
    fn handle_polkit_readiness_changed(&mut self, readiness: PolkitReadiness, now: u64) {
        self.polkit = readiness;
        self.render_menu_now(now);
    }

    /// design.md §4 D4, task 8.3: `Trigger::MenuOpened` re-reads
    /// `config.toml` from disk — never cached across menu opens, mirroring
    /// `autostart`'s own re-read discipline — and warns only on a
    /// Io/Malformed TRANSITION (commit `29ebe32`'s edge-triggered-warning
    /// rule, `watch_warning_posted`'s second surface): a config that stays
    /// faulted across many menu opens must notify exactly once, and a
    /// LATER fault (even the identical kind, after a healthy read in
    /// between) must notify again.
    fn refresh_config(&mut self) {
        let (config, fault) = config::resolve(&config::read(&self.config_path));
        self.config = config;
        self.consent.sync_acknowledged(config.warning_acknowledged);
        match fault {
            Some(current) if self.last_warned_fault != Some(current) => {
                self.last_warned_fault = Some(current);
                self.notify.post(
                    Category::Environment,
                    format::Msg::NotifyConfigUnreadableSummary.text(format::lang()),
                    format::Msg::NotifyConfigUnreadableBody.text(format::lang()),
                );
            }
            Some(_) => {}
            None => self.last_warned_fault = None,
        }
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
        // The `Ticket` this action held is already dropped by the time
        // this handler runs (`spawn_action`'s own thread drops it right
        // after `runner.run` returns) — re-render so a menu left open
        // through the whole invocation clears its `ActionInFlight` row
        // without waiting for the next `Trigger::MenuOpened`.
        self.render_menu_now(now);
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
            Event::MenuOpened => {
                // task 8.3: config/autostart are re-read from disk at
                // every menu open, never cached across opens — the SAME
                // discipline design.md D4 already required, applied
                // alongside the existing state-file reconciliation rather
                // than replacing it.
                self.reconcile(Trigger::MenuOpened, now);
                self.refresh_config();
                self.render_menu_now(now);
            }
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
            Event::DurationSelected(duration) => self.handle_duration_selected(duration, now),
            Event::ConsentConfirmed { persist } => self.handle_consent_confirmed(persist, now),
            Event::ConsentCancelled => self.handle_consent_cancelled(now),
            Event::DefaultDurationSelected(duration) => self.handle_default_duration_selected(duration, now),
            Event::AutostartToggled => self.handle_autostart_toggled(now),
            Event::PolkitReadinessChanged(readiness) => self.handle_polkit_readiness_changed(readiness, now),
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
    use crate::consent::granted_for_test;
    use crate::outcome::classify;
    use crate::runner::{CommandSpec, ScriptedRunner};
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct RecordingTray {
        renders: Mutex<Vec<ViewModel>>,
        menus: Mutex<Vec<MenuModel>>,
        reasserts: Mutex<u32>,
    }
    impl TrayPort for RecordingTray {
        fn render(&self, view: &ViewModel) {
            self.renders.lock().unwrap().push(view.clone());
        }
        fn render_menu(&self, model: &MenuModel) {
            self.menus.lock().unwrap().push(model.clone());
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
            PathBuf::from("/nonexistent/nopass-app-test/config.toml"),
            PathBuf::from("/nonexistent/nopass-app-test/autostart.desktop"),
            Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true },
            PolkitReadiness::Ready,
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

    /// The subset of a just-handled [`Event`] these tests need to assert
    /// on. `Event` is deliberately not `Clone` (design.md §3 D3:
    /// `ActionFinished` carries a `Granted`-backed `Action` that must not
    /// be duplicated), so `recv_and_handle` inspects the event by
    /// reference before moving it into `app.handle` and returns this
    /// small summary instead of the event itself.
    #[derive(Debug)]
    enum HandledEvent {
        WatchLost,
        ActionFinishedEnableOk,
        Other,
    }

    /// Receives the next event within `timeout` and feeds it through
    /// `app.handle`, exactly as `run`'s own loop would — background
    /// producers (`spawn_probe`/`spawn_action`) only ever report into the
    /// channel; nothing except `handle` is allowed to react to them.
    fn recv_and_handle(app: &mut App, timeout: Duration) -> Option<HandledEvent> {
        let rx = app.events_rx.clone();
        let event = recv_within(&rx, timeout)?;
        let summary = match &event {
            Event::WatchLost => HandledEvent::WatchLost,
            Event::ActionFinished(Action::Enable(_), Ok(_)) => HandledEvent::ActionFinishedEnableOk,
            _ => HandledEvent::Other,
        };
        app.handle(event);
        Some(summary)
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

        assert!(recv_within(&rx, NONE_EXPECTED).is_none(), "a fresh cache must not trigger a probe on MenuOpened");
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
            Some(HandledEvent::ActionFinishedEnableOk) => {}
            other => panic!("expected the enable action to finish, got {other:?}"),
        }
        // Processing `ActionFinished` above must itself have forced the
        // `Trigger::ActionCompleted` probe (design.md D5/§5.1).
        match recv_within(&rx, SHORT) {
            Some(Event::ProbeFinished(_)) => {}
            other => panic!("ActionCompleted must always force a probe, got {other:?}"),
        }
        assert!(recv_within(&rx, NONE_EXPECTED).is_none(), "a second in-flight toggle must never spawn a second action");

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

    // ---- task 8.2 (RED): every activation-requesting caller dispatches
    // nothing while consent is unrecorded, and posts exactly one
    // Category::Environment notification (spec `activation-consent`
    // "A menu-triggered activation...", "A non-menu activation path...",
    // "...activation nudge never itself dispatches an enable") ----

    fn unconsented_app(runner: Arc<dyn CommandRunner>, tray: Arc<RecordingTray>, notify: Arc<RecordingNotify>) -> App {
        let mut app = test_app(runner, tray, notify, Mode::Full);
        app.config.warning_acknowledged = false;
        app.consent = ConsentState::from_config(&app.config);
        app
    }

    #[test]
    fn toggle_requested_while_unconsented_dispatches_nothing_and_notifies_once() {
        // Left click, keyboard Activate, and the menu's own toggle item
        // ALL raise this exact same `Event::ToggleRequested` — proving it
        // here proves all three at once, per spec `activation-consent`
        // "This check MUST be enforced at the one point...".
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = unconsented_app(runner, tray, notify.clone());
        app.current_state = TrayState::Inactive;

        app.handle_toggle(now());

        assert!(app.action_gate.try_begin().is_some(), "handle_toggle must never leave the gate held when it dispatches nothing");
        let posts = notify.posts.lock().unwrap();
        assert_eq!(posts.len(), 1, "exactly one notification, got {posts:?}");
        assert_eq!(posts[0].0, Category::Environment);
        assert_eq!(posts[0].2, format::Msg::NotifyConsentNeededBody.text(format::lang()));
    }

    #[test]
    fn activate_requested_the_second_instance_nudge_never_dispatches_an_enable_while_unconsented() {
        // spec `activation-consent` "A received single-instance activation
        // nudge never itself dispatches an enable": `ActivateRequested`
        // only ever reassert()s — it is not even wired to `handle_toggle`,
        // so this is provable without touching `ConsentState` at all.
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = unconsented_app(runner, tray.clone(), notify.clone());
        app.current_state = TrayState::Inactive;

        assert!(app.handle(Event::ActivateRequested), "ActivateRequested must never end the loop");

        assert_eq!(*tray.reasserts.lock().unwrap(), 1);
        assert!(notify.posts.lock().unwrap().is_empty(), "the nudge itself must never notify or dispatch");
    }

    #[test]
    fn duration_selected_while_unconsented_arms_the_branch_and_dispatches_nothing() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = unconsented_app(runner, tray.clone(), notify.clone());
        app.current_state = TrayState::Inactive;

        app.handle(Event::DurationSelected(GrantDuration::Hours4));

        assert!(app.consent.branch().is_some(), "arming must record the pending duration");
        assert!(notify.posts.lock().unwrap().is_empty(), "arming a duration must never notify — only the plain toggle path does");
        let menus = tray.menus.lock().unwrap();
        assert!(menus.last().unwrap().consent_branch.is_some(), "the re-rendered menu must reflect the armed branch");
    }

    // ---- spec `activation-consent`: confirming/cancelling the branch ----

    #[test]
    fn confirming_the_branch_grants_exactly_once() {
        let runner: Arc<dyn CommandRunner> =
            Arc::new(AnyCommandRunner::new(vec![Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = unconsented_app(runner, tray, notify);
        app.current_state = TrayState::Inactive;
        app.consent.arm(GrantDuration::Hours4);

        app.handle(Event::ConsentConfirmed { persist: false });

        match recv_and_handle(&mut app, SHORT) {
            Some(HandledEvent::ActionFinishedEnableOk) => {}
            other => panic!("expected exactly one enable dispatch, got {other:?}"),
        }
        assert!(app.consent.branch().is_none(), "confirming must clear the pending duration");
    }

    #[test]
    fn cancelling_the_branch_grants_nothing_and_changes_no_consent_state() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = unconsented_app(runner, tray, notify);
        app.consent.arm(GrantDuration::Hours4);

        app.handle(Event::ConsentCancelled);

        assert!(app.consent.branch().is_none(), "cancel must clear the pending duration");
        assert!(app.consent.grant().is_none(), "cancel must not acknowledge consent as a side effect");
    }

    #[test]
    fn confirming_with_persist_writes_the_config_and_a_later_activation_skips_the_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");

        // Two round trips (confirm's own enable + its post-action probe,
        // then the later DurationSelected's enable + its own probe) — four
        // runner calls total.
        let runner: Arc<dyn CommandRunner> = Arc::new(AnyCommandRunner::new(vec![
            Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] }),
            Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] }),
            Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] }),
            Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] }),
        ]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = unconsented_app(runner, tray, notify);
        app.config_path = config_path.clone();
        app.current_state = TrayState::Inactive;
        app.consent.arm(GrantDuration::Hours4);

        let rx = app.events_rx.clone();
        app.handle(Event::ConsentConfirmed { persist: true });
        assert!(matches!(recv_and_handle(&mut app, SHORT), Some(HandledEvent::ActionFinishedEnableOk)));
        // Drain (without re-handling) the ActionCompleted trigger's own
        // forced probe before moving on — otherwise it is still sitting
        // in the channel ahead of the next round's own ActionFinished,
        // exactly the pattern `toggle_while_inactive_enables_and_a_
        // second_toggle_in_flight_is_ignored` already established above.
        assert!(matches!(recv_within(&rx, SHORT), Some(Event::ProbeFinished(_))));

        let (reread, fault) = config::resolve(&config::read(&config_path));
        assert_eq!(fault, None);
        assert!(reread.warning_acknowledged, "persist=true must write warning_acknowledged=true (warn_before_activation=false)");

        // A later activation dispatches directly — no branch presented.
        app.handle(Event::DurationSelected(GrantDuration::Hour1));
        assert!(matches!(recv_and_handle(&mut app, SHORT), Some(HandledEvent::ActionFinishedEnableOk)));
        assert!(app.consent.branch().is_none());
    }

    #[test]
    fn a_write_failure_during_persist_re_warns_rather_than_silently_granting_the_next_activation() {
        // A config path whose PARENT does not exist: `atomicfile::write`'s
        // `O_CREAT` tmp-file step fails deterministically, with no real
        // filesystem permission trickery needed.
        let config_path = PathBuf::from("/nonexistent/nopass-app-test/does-not-exist/config.toml");

        let runner: Arc<dyn CommandRunner> =
            Arc::new(AnyCommandRunner::new(vec![Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = unconsented_app(runner, tray, notify);
        app.config_path = config_path;
        app.current_state = TrayState::Inactive;
        app.consent.arm(GrantDuration::Hours4);

        app.handle(Event::ConsentConfirmed { persist: true });
        // This one confirmed activation still proceeds...
        assert!(matches!(recv_and_handle(&mut app, SHORT), Some(HandledEvent::ActionFinishedEnableOk)));
        // ...but the write failed, so consent must still read as
        // unrecorded for the NEXT activation.
        assert!(app.consent.grant().is_none(), "a failed persist write must re-warn, never silently grant next time");
    }

    // ---- task 8.1/8.8 (RED): the full app composition — default
    // duration for a left click, the SELECTED duration for
    // DurationSelected — pins the exact end-to-end argv for all six
    // durations, through App, never by calling pkexec_spec directly
    // (tray-privileged-invocation "All six durations render their
    // documented argv with no collision") ----

    #[test]
    fn all_six_durations_render_their_documented_argv_end_to_end_through_app_with_no_shell() {
        for duration in GrantDuration::ALL {
            let now_value = now();
            let mut expected_args = vec![nopass_core::paths::HELPER_PATH.to_string(), "enable".to_string()];
            expected_args.extend(duration.args(now_value));
            assert!(
                !expected_args.iter().any(|a| a == "sh" || a == "-c"),
                "documented argv for {duration:?} must never contain sh/-c: {expected_args:?}"
            );
            let expected_spec =
                CommandSpec { program: PathBuf::from("/usr/bin/pkexec"), args: expected_args, env: Vec::new() };
            // The ActionCompleted trigger's own forced probe (design.md
            // D5/§5.1) is the second call `App` makes on this runner —
            // scripted here too, so the whole round trip (not merely the
            // pkexec argv) is proven end to end with no unrelated panic
            // on a background thread.
            let probe_spec = probe::spec(&PathBuf::from("/usr/bin/sudo"));

            let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![
                (expected_spec, Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })),
                (probe_spec, Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })),
            ]));
            let tray = Arc::new(RecordingTray::default());
            let notify = Arc::new(RecordingNotify::default());
            let mut app = test_app(runner, tray, notify, Mode::Full); // acknowledged consent
            app.current_state = TrayState::Inactive;

            let rx = app.events_rx.clone();
            app.handle(Event::DurationSelected(duration));

            match recv_and_handle(&mut app, SHORT) {
                Some(HandledEvent::ActionFinishedEnableOk) => {}
                other => panic!("duration {duration:?}: expected the enable action to finish, got {other:?}"),
            }
            // Drain (without re-handling) the ActionCompleted trigger's
            // own forced probe before this iteration's `App`/`ScriptedRunner`
            // are dropped — otherwise that probe's background thread calls
            // an exhausted single-entry script after this test has already
            // moved on, panicking on an unrelated later test that happens
            // to lock the same poisoned `Mutex`.
            assert!(matches!(recv_within(&rx, SHORT), Some(Event::ProbeFinished(_))));
            // `ScriptedRunner::drop` panics on an unexhausted script —
            // reaching here at all is itself proof the exact expected
            // spec was the one `App` actually sent.
        }
    }

    #[test]
    fn a_left_click_toggle_uses_configs_default_duration_never_a_hardcoded_one() {
        for duration in [GrantDuration::Minutes15, GrantDuration::Hours8, GrantDuration::Permanent] {
            let now_value = now();
            let mut expected_args = vec![nopass_core::paths::HELPER_PATH.to_string(), "enable".to_string()];
            expected_args.extend(duration.args(now_value));
            let expected_spec =
                CommandSpec { program: PathBuf::from("/usr/bin/pkexec"), args: expected_args, env: Vec::new() };
            let probe_spec = probe::spec(&PathBuf::from("/usr/bin/sudo"));

            let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![
                (expected_spec, Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })),
                (probe_spec, Ok(SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] })),
            ]));
            let tray = Arc::new(RecordingTray::default());
            let notify = Arc::new(RecordingNotify::default());
            let mut app = test_app(runner, tray, notify, Mode::Full);
            app.config.default_duration = duration;
            app.current_state = TrayState::Inactive;

            let rx = app.events_rx.clone();
            app.handle_toggle(now());

            match recv_and_handle(&mut app, SHORT) {
                Some(HandledEvent::ActionFinishedEnableOk) => {}
                other => panic!("default duration {duration:?}: expected the enable action to finish, got {other:?}"),
            }
            // See the sibling test above for why this drain matters.
            assert!(matches!(recv_within(&rx, SHORT), Some(Event::ProbeFinished(_))));
        }
    }

    // ---- task 8.1: DefaultDurationSelected persists and re-renders ----

    #[test]
    fn default_duration_selected_persists_and_the_next_menu_model_marks_it() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");

        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray.clone(), notify, Mode::Full);
        app.config_path = config_path.clone();

        app.handle(Event::DefaultDurationSelected(GrantDuration::Hours8));

        assert_eq!(app.config.default_duration, GrantDuration::Hours8);
        let (reread, fault) = config::resolve(&config::read(&config_path));
        assert_eq!(fault, None);
        assert_eq!(reread.default_duration, GrantDuration::Hours8);
        assert_eq!(tray.menus.lock().unwrap().last().unwrap().config.default_duration, GrantDuration::Hours8);
    }

    // ---- task 8.1/§4 D5: AutostartToggled writes through autostart.rs,
    // never the real process $HOME ----

    #[test]
    fn autostart_toggled_enables_a_disabled_entry_and_disables_an_enabled_one() {
        let tmp = tempfile::tempdir().unwrap();
        let autostart_path = tmp.path().join("autostart").join("nopass.desktop");

        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray.clone(), notify, Mode::Full);
        app.autostart_path = autostart_path.clone();
        assert_eq!(autostart::read(&autostart_path), AutostartState::Disabled);

        app.handle(Event::AutostartToggled);
        assert_eq!(autostart::read(&autostart_path), AutostartState::Enabled);
        assert_eq!(tray.menus.lock().unwrap().last().unwrap().autostart, AutostartState::Enabled);

        app.handle(Event::AutostartToggled);
        assert_eq!(autostart::read(&autostart_path), AutostartState::Disabled);
    }

    // ---- task 8.3 (RED): MenuOpened re-reads config/autostart, and a
    // config fault notifies only on transition (commit 29ebe32's rule,
    // applied to config.toml) ----

    #[test]
    fn menu_opened_re_reads_config_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        config::write(&config_path, &Config { default_duration: GrantDuration::Hours8, warning_acknowledged: true }).unwrap();

        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify, Mode::Full);
        app.config_path = config_path;
        // A fresh, usable probe cache so `Trigger::MenuOpened` does not
        // also force a probe on a runner this test never scripted one
        // for — this test's own concern is the config re-read, not the
        // probe/reconcile machinery already covered elsewhere.
        app.probe_cache = Some(ProbeCache { value: Probe::PasswordRequired, taken_at: now() });
        assert_eq!(app.config.default_duration, GrantDuration::Hour1, "precondition: App::new's own default, not yet re-read");

        app.handle(Event::MenuOpened);

        assert_eq!(app.config.default_duration, GrantDuration::Hours8, "MenuOpened must re-read config.toml from disk");
    }

    #[test]
    fn a_faulted_config_warns_only_on_transition_not_once_per_menu_open() {
        // A malformed document: valid UTF-8, invalid TOML syntax —
        // `config::read` maps this to `Faulted(Malformed)`.
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "not = [valid toml").unwrap();

        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify.clone(), Mode::Full);
        app.config_path = config_path.clone();
        app.probe_cache = Some(ProbeCache { value: Probe::PasswordRequired, taken_at: now() });

        app.handle(Event::MenuOpened);
        app.handle(Event::MenuOpened);
        app.handle(Event::MenuOpened);

        let warnings = notify
            .posts
            .lock()
            .unwrap()
            .iter()
            .filter(|(c, s, _)| *c == Category::Environment && s == format::Msg::NotifyConfigUnreadableSummary.text(format::lang()))
            .count();
        assert_eq!(warnings, 1, "a config fault that stays faulted across many menu opens must warn exactly once");

        // Fix the file: the fault clears, and last_warned_fault resets.
        config::write(&config_path, &Config::defaults()).unwrap();
        app.handle(Event::MenuOpened);
        // A LATER fault (even the identical kind) must warn again — the
        // same edge-triggered rule commit 29ebe32 established.
        std::fs::write(&config_path, "not = [valid toml again").unwrap();
        app.handle(Event::MenuOpened);

        let warnings_after_recovery = notify
            .posts
            .lock()
            .unwrap()
            .iter()
            .filter(|(c, s, _)| *c == Category::Environment && s == format::Msg::NotifyConfigUnreadableSummary.text(format::lang()))
            .count();
        assert_eq!(warnings_after_recovery, 2, "a fault after a healthy read in between must notify again");
    }

    // ---- task 8.7: render_menu is wired through App the way main.rs
    // exercises it — MenuOpened must call it, not merely a test fixture
    // driving KsniTray::render_menu directly (this crate's own recurring
    // defect shape: Phase 7 nearly shipped an empty menu because every
    // Lane B test called render_menu itself while nothing in production
    // did) ----

    #[test]
    fn menu_opened_calls_render_menu_on_the_tray_port_with_the_current_state() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray.clone(), notify, Mode::Full);
        // A fresh, usable PasswordRequired probe cache: `reconcile::merge`
        // derives `Inactive` from it regardless of the (absent) state
        // file, and `Trigger::MenuOpened` does not additionally force a
        // probe this test's runner never scripted one for.
        app.probe_cache = Some(ProbeCache { value: Probe::PasswordRequired, taken_at: now() });

        assert!(tray.menus.lock().unwrap().is_empty(), "precondition: nothing rendered before any MenuOpened");

        app.handle(Event::MenuOpened);

        let menus = tray.menus.lock().unwrap();
        assert_eq!(menus.len(), 1, "MenuOpened must call TrayPort::render_menu exactly once through App::handle");
        assert_eq!(menus[0].state, TrayState::Inactive);
    }

    // ---- task 8.4/8.5/8.6: PolkitReadinessChanged re-runs the ladder's
    // effect on the toggle and recovers without a restart ----

    #[test]
    fn action_missing_disables_the_toggle_and_a_later_polkit_readiness_changed_recovers_it() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray.clone(), notify, Mode::Full);
        app.polkit = PolkitReadiness::ActionMissing("action_not_registered");
        app.probe_cache = Some(ProbeCache { value: Probe::PasswordRequired, taken_at: now() });

        app.handle(Event::MenuOpened);
        let before = tray.menus.lock().unwrap().last().unwrap().clone();
        assert_eq!(
            crate::preflight::toggle_availability(&before.state, before.polkit, before.action_in_flight),
            crate::preflight::ToggleAvailability::Unavailable(crate::preflight::UnavailableReason::InstallationIncomplete)
        );

        app.handle(Event::PolkitReadinessChanged(PolkitReadiness::Ready));

        let after = tray.menus.lock().unwrap().last().unwrap().clone();
        assert_eq!(
            crate::preflight::toggle_availability(&after.state, after.polkit, after.action_in_flight),
            crate::preflight::ToggleAvailability::OfferEnable,
            "recovery must not require a tray restart or another MenuOpened"
        );
    }

    #[test]
    fn announce_degraded_mode_posts_once_at_startup_when_the_polkit_action_is_missing() {
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![]));
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let mut app = test_app(runner, tray, notify.clone(), Mode::Full);
        app.polkit = PolkitReadiness::ActionMissing("action_not_registered");

        app.announce_degraded_mode();

        let posts = notify.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, Category::Environment);
        assert!(
            posts[0].2.contains("action_not_registered"),
            "the posted body must include the readiness reason: {:?}",
            posts[0].2
        );
        assert!(
            !posts[0].2.contains("{}"),
            "the posted body must not ship a literal, unfilled placeholder: {:?}",
            posts[0].2
        );
    }

    // The App-level test above posts through whatever `format::lang()`
    // resolves from the real process environment, so it cannot pin a
    // specific language. This test calls the exact same interpolation
    // `announce_degraded_mode` performs — `Msg::text(lang).replace("{}",
    // reason)` — with both languages named explicitly, so a Spanish arm
    // that dropped the reason or shipped a literal `{}` fails here even
    // under this machine's es_ES.UTF-8 locale.
    #[test]
    fn notify_polkit_unavailable_body_interpolates_the_reason_in_both_languages() {
        let reason = "action_not_registered";
        for lang in [format::Lang::En, format::Lang::Es] {
            let body = format::Msg::NotifyPolkitUnavailableBody.text(lang).replace("{}", reason);
            assert!(body.contains(reason), "{lang:?} body must include the reason: {body:?}");
            assert!(!body.contains("{}"), "{lang:?} body must not leave a literal placeholder: {body:?}");
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

        let kind = classify(Action::Enable(EnableRequest::new(GrantDuration::Hour1, 1, granted_for_test())), Some(17), true);
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
            PathBuf::from("/nonexistent/nopass-app-test/config.toml"),
            PathBuf::from("/nonexistent/nopass-app-test/autostart.desktop"),
            Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true },
            PolkitReadiness::Ready,
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
            PathBuf::from("/nonexistent/nopass-app-test/config.toml"),
            PathBuf::from("/nonexistent/nopass-app-test/autostart.desktop"),
            Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true },
            PolkitReadiness::Ready,
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
            PathBuf::from("/nonexistent/nopass-app-test/config.toml"),
            PathBuf::from("/nonexistent/nopass-app-test/autostart.desktop"),
            Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true },
            PolkitReadiness::Ready,
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
                Some(HandledEvent::WatchLost) => return,
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
            PathBuf::from("/nonexistent/nopass-app-test/config.toml"),
            PathBuf::from("/nonexistent/nopass-app-test/autostart.desktop"),
            Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true },
            PolkitReadiness::Ready,
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
        // Compared against the live renderer for an Active state, NOT
        // against an English literal. What this pins is the wiring — that
        // the pushed view model is the Active one rather than the Unknown
        // one it replaced — and that property holds in every locale. The
        // literal form asserted here for two milestones and only ever ran
        // on English machines; it broke the moment the localization phase
        // met a developer whose LANG is es_ES.UTF-8. The exact English
        // text is pinned where it belongs, in format.rs's own tests,
        // through the explicit-language seam.
        let expected_active = TrayState::Active { user: Some("jorge".to_string()), expiry: None };
        assert_eq!(
            renders[0].toggle,
            crate::format::toggle_label(&expected_active),
            "the pushed ViewModel must reflect the new Active state"
        );
        assert_ne!(
            renders[0].toggle,
            crate::format::toggle_label(&TrayState::Unknown),
            "and must not still be the Unknown view model"
        );
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
            PathBuf::from("/nonexistent/nopass-app-test/config.toml"),
            PathBuf::from("/nonexistent/nopass-app-test/autostart.desktop"),
            Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true },
            PolkitReadiness::Ready,
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
            PathBuf::from("/nonexistent/nopass-app-test/config.toml"),
            PathBuf::from("/nonexistent/nopass-app-test/autostart.desktop"),
            Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true },
            PolkitReadiness::Ready,
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

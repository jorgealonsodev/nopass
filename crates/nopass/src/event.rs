//! The single channel every adapter feeds and `app::run` (Phase 10)
//! alone drains — design.md §2 `event`, §6 "one async task owns every
//! piece of mutable state".
//!
//! Phase 4 shipped `Event` partially — only the variants its own
//! adapters (`watch`, the 60 s tick) and the already-existing probe port
//! could produce without pulling in Phase 5's `Action` (`outcome.rs`) or
//! Phase 7-9's SNI/D-Bus adapters. Phase 10 completes it: `MenuOpened`,
//! `ToggleRequested`, `ActivateRequested`, `HostAppeared`, `HostVanished`,
//! `ActionFinished`, and `Quit` (design.md §2 `event`).

use std::time::Duration;

use futures_lite::{Stream, StreamExt};

use crate::duration::GrantDuration;
use crate::outcome::Action;
use crate::preflight::PolkitReadiness;
use crate::probe::{Probe, ProbeError};
use crate::runner::{RunnerError, SpawnOutcome};
use crate::tray::TrayEvent;

/// The 60 s reconciliation tick (design.md §3.4, §4 "No Periodic Wakeup
/// Beyond the 60-Second Reconciliation Tick"). `app::run` (Phase 10) is
/// the only caller of [`tick`]; tests exercise [`tick_stream`] directly
/// with a much shorter period so the suite never waits a real minute.
pub const TICK_INTERVAL_SECS: u64 = 60;

/// What the reactor's adapters hand to `app::run` (design.md §2 `event`).
///
/// Deliberately not `Clone`/`PartialEq`/`Eq`: `ActionFinished` carries an
/// [`Action`], which (design.md §3 D3) holds a `Granted` proof-of-consent
/// that cannot be duplicated — one `Granted` buys exactly one invocation.
#[derive(Debug)]
pub enum Event {
    /// `watch::Watch` observed a debounced change under `<uid>.state`
    /// (design.md D8).
    FileChanged,
    /// `watch::Watch` observed its watched directory itself being
    /// removed (or the `notify` backend otherwise reported it could no
    /// longer be trusted) — spec `tray-state-sync` S4's "or the watch is
    /// lost" (verify-report.md H4). `App::handle` drops the held `Watch`
    /// and falls back to reconciliation alone; `Event::Tick` is what
    /// retries establishing a fresh one.
    WatchLost,
    /// One firing of the sole periodic timer this design funds.
    Tick,
    /// `ksni::Tray::menu_about_to_show` — the root menu is about to be
    /// displayed (`Trigger::MenuOpened`, design.md §3.4).
    MenuOpened,
    /// Left-click activation or the menu's toggle item (design.md's
    /// "left-click toggle").
    ToggleRequested,
    /// A second instance nudged us via `org.freedesktop.Application.
    /// Activate` (design.md §6.4).
    ActivateRequested,
    /// `org.kde.StatusNotifierWatcher` gained an owner after startup
    /// (design.md §6.1 step 8).
    HostAppeared,
    /// `org.kde.StatusNotifierWatcher` lost its owner.
    HostVanished,
    /// A `sudo -kn true` probe completed, successfully or not.
    ProbeFinished(Result<Probe, ProbeError>),
    /// A privileged `pkexec` invocation completed, successfully or not
    /// (design.md §5, §6.2).
    ActionFinished(Action, Result<SpawnOutcome, RunnerError>),
    /// One entry inside "Activate during…" was clicked (design.md §1 item
    /// 3.1-3.6; task 8.1). Routed through
    /// `crate::consent::ConsentState::grant`/`arm` before any invocation —
    /// never dispatched directly by this variant's mere existence.
    DurationSelected(GrantDuration),
    /// The consent branch's "I understand — activate[, and don't warn me
    /// again]" (design.md §1 "The consent branch"; spec
    /// `activation-consent` "Confirming the branch grants exactly once").
    ConsentConfirmed { persist: bool },
    /// The consent branch's "Cancel" (spec `activation-consent`
    /// "Cancelling the branch grants nothing").
    ConsentCancelled,
    /// A "Default duration" `RadioGroup` entry was selected (design.md §1
    /// item 7.1; spec `tray-menu` "Selecting a new default moves the
    /// marker and persists it").
    DefaultDurationSelected(GrantDuration),
    /// "Start with session" was toggled (design.md §1 item 8; spec
    /// `autostart-entry`).
    AutostartToggled,
    /// The polkit readiness ladder was re-run after
    /// `org.freedesktop.PolicyKit1`'s `NameOwnerChanged` fired (design.md
    /// §0 D6, task 8.4/8.6) — polkitd restarting on a package upgrade must
    /// recover the toggle without a tray restart.
    PolkitReadinessChanged(PolkitReadiness),
    /// The menu's `Quit` item, or `AppInterface`/the SNI item requesting
    /// an orderly shutdown.
    Quit,
}

/// Folds a [`TrayEvent`] (raised by `ksni` menu/activation callbacks,
/// design.md §7.2) into the full [`Event`] enum — `tray.rs` deliberately
/// stays independent of this module (see its own `TrayEvent` doc
/// comment), so the mapping lives here instead.
impl From<TrayEvent> for Event {
    fn from(event: TrayEvent) -> Event {
        match event {
            TrayEvent::ToggleRequested => Event::ToggleRequested,
            TrayEvent::MenuOpened => Event::MenuOpened,
            TrayEvent::Quit => Event::Quit,
            // Task 8.7 finishes the mapping Phase 7 (`tray.rs`) left
            // partial: the full RF-03 menu's duration selection, consent
            // confirm/cancel, default-duration selection, and autostart
            // toggle all reach `app.rs`'s consent/config/autostart wiring
            // through here now — see `app.rs::handle` for what each one
            // does. Wiring `render_menu` (task 8.7/8.1) without this
            // change landing in the same commit would let a real click
            // reach the arm this match used to end in — an `unreachable!()`
            // panic (exit 101) on the very first menu interaction; both
            // land together.
            TrayEvent::DurationSelected(d) => Event::DurationSelected(d),
            TrayEvent::ConsentConfirmed { persist } => Event::ConsentConfirmed { persist },
            TrayEvent::ConsentCancelled => Event::ConsentCancelled,
            TrayEvent::DefaultDurationSelected(d) => Event::DefaultDurationSelected(d),
            TrayEvent::AutostartToggled => Event::AutostartToggled,
        }
    }
}

/// The production tick source: exactly one [`async_io::Timer::interval`]
/// call, at [`TICK_INTERVAL_SECS`]. `app::run` (Phase 10) is the only
/// wiring point — see `tick_stream` for the parameterised version tests
/// use.
pub fn tick() -> impl Stream<Item = Event> {
    tick_stream(Duration::from_secs(TICK_INTERVAL_SECS))
}

/// The tick source parameterised by `period`, so a test can observe
/// several firings without waiting real minutes. There is exactly one
/// call to `async_io::Timer::interval` in this crate — see this module's
/// own `only_one_timer_interval_call_exists_in_the_crate` test, which
/// pins that as a structural fact rather than a promise (spec
/// `tray-state-sync` "Only one periodic wakeup source exists").
pub fn tick_stream(period: Duration) -> impl Stream<Item = Event> {
    async_io::Timer::interval(period).map(|_| Event::Tick)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::future::block_on;
    use std::time::{Duration, Instant};

    // ---- task 8.7: every m3 TrayEvent variant maps to its Event
    // counterpart — the arm that used to be unreachable!() ----

    #[test]
    fn every_m3_tray_event_variant_maps_to_its_event_counterpart() {
        assert!(matches!(
            Event::from(TrayEvent::DurationSelected(GrantDuration::Hours4)),
            Event::DurationSelected(GrantDuration::Hours4)
        ));
        assert!(matches!(
            Event::from(TrayEvent::ConsentConfirmed { persist: true }),
            Event::ConsentConfirmed { persist: true }
        ));
        assert!(matches!(Event::from(TrayEvent::ConsentCancelled), Event::ConsentCancelled));
        assert!(matches!(
            Event::from(TrayEvent::DefaultDurationSelected(GrantDuration::Hour1)),
            Event::DefaultDurationSelected(GrantDuration::Hour1)
        ));
        assert!(matches!(Event::from(TrayEvent::AutostartToggled), Event::AutostartToggled));
    }

    #[test]
    fn tick_stream_yields_tick_events_periodically() {
        let period = Duration::from_millis(30);
        let mut stream = tick_stream(period);
        let started = Instant::now();

        for _ in 0..3 {
            let event = block_on(stream.next()).expect("the interval stream never ends");
            assert!(matches!(event, Event::Tick));
        }

        // A generous upper bound rather than a tight one: this proves the
        // stream fires repeatedly on its own period, not that it hits an
        // exact wall-clock deadline on a possibly loaded machine.
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "three 30ms ticks must not take anywhere near 5s"
        );
    }

    /// Structural half of "No Periodic Wakeup Beyond the 60-Second
    /// Reconciliation Tick": exactly one call site in the whole crate
    /// constructs a periodic timer. A second call site — a dedicated
    /// countdown timer, for example — would pass every functional test
    /// above and still violate the design; this is the check that would
    /// catch it (spec `tray-state-sync` "Only one periodic wakeup source
    /// exists"; tray-presence "No timer exists solely to refresh the
    /// tooltip").
    #[test]
    fn only_one_timer_interval_call_exists_in_the_crate() {
        // Every file's own `#[cfg(test)]` module is excluded before
        // counting: otherwise this very assertion (and its message,
        // which necessarily names the pattern it looks for) would count
        // itself as a second call site.
        let needle = ["Timer", "::", "interval", "("].concat();
        let src_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut occurrences = 0usize;
        for entry in std::fs::read_dir(src_dir).expect("crate src directory must exist") {
            let path = entry.expect("readable directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let contents = std::fs::read_to_string(&path).expect("readable source file");
            let production_code = contents.split("#[cfg(test)]").next().unwrap_or("");
            occurrences += production_code.matches(&needle).count();
        }
        assert_eq!(
            occurrences, 1,
            "exactly one production `Timer::interval(` call site must exist in crates/nopass/src — found {occurrences}"
        );
    }
}

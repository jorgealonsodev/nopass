//! The only `ksni`-aware module (design.md §2 `tray`, §7.2, D6, D7; spec
//! `tray-presence` remaining scenarios; tasks.md Phase 7).
//!
//! Every other module in this crate — `reconcile`, `format`, `outcome` —
//! stays free of `ksni`/`zbus` types so it can be unit-tested without a
//! bus. This module is the one seam where a `TrayState` and its
//! `format::*` rendering become an actual StatusNotifierItem: a
//! [`ViewModel`] carries the already-rendered content, [`TrayPort`] is
//! what `app::run` (Phase 10) sees, and [`KsniTray`] is the concrete
//! `ksni`-backed implementation.

use crate::format::{self, ToolTip};
use crate::reconcile::TrayState;

/// Everything the tray renders for one `TrayState`, already formatted —
/// composed once, here, from `format::*` alone (design.md §7.1, §7.2), so
/// `TrayPort` implementations need to know nothing about `TrayState`,
/// `merge`, or reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewModel {
    pub icon: &'static str,
    pub attention: bool,
    pub tooltip: ToolTip,
    pub status_line: String,
    pub toggle: Option<&'static str>,
}

impl ViewModel {
    /// Builds the complete `ViewModel` for one `TrayState` at `now`
    /// (design.md §7.1's icon table, §7.2's tooltip/menu-label table).
    /// `attention` mirrors design.md §7.1's `Status` column: only
    /// `Unknown` sets `NeedsAttention`.
    pub fn from_state(user: &str, state: &TrayState, now: u64) -> ViewModel {
        ViewModel {
            icon: format::icon_name(state),
            attention: matches!(state, TrayState::Unknown),
            tooltip: format::tooltip(user, state, now),
            status_line: format::status_line(user, state, now),
            toggle: format::toggle_label(state),
        }
    }
}

/// What `app::run` (Phase 10) needs from the tray adapter (design.md §2
/// `tray`). `reassert` is `instance::AppInterface::activate`'s hook
/// (Phase 9) to make the item visible again after a second-instance
/// nudge.
pub trait TrayPort {
    fn render(&self, view: &ViewModel);
    fn reassert(&self);
}

/// One event a `ksni` menu or activation callback raised. Deliberately a
/// small, tray-local enum rather than reusing [`crate::event::Event`]:
/// `Event`'s `ToggleRequested`/`MenuOpened`/`ActivateRequested`/`Quit`
/// variants are completed in Phase 10 once `app.rs` exists to drain them
/// (see `event.rs`'s own note on why it ships partial). `app::run` is
/// expected to fold a `TrayEvent` into the full `Event` enum once it
/// does; this module does not depend on that not-yet-existing wiring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Left-click activation (design.md's "left-click toggle") or the
    /// menu's toggle item.
    ToggleRequested,
    /// `ksni::Tray::menu_about_to_show` — the root menu is about to be
    /// displayed (`Trigger::MenuOpened`, design.md §3.4).
    MenuOpened,
    /// The minimal menu's `Quit` item (design.md §7.2's minimal-menu
    /// table). Exiting the process with code 0 is `app::run`'s job
    /// (Phase 10) — this module only raises the request.
    Quit,
}

/// The `ksni::Tray` implementation itself. `ksni` takes ownership of this
/// value on its own executor once spawned (design.md D1's single
/// `async-io` reactor); every interaction afterwards goes through the
/// `ksni::Handle` [`KsniTray`] wraps, never this type directly.
struct Inner {
    view: ViewModel,
    events: async_channel::Sender<TrayEvent>,
}

impl ksni::Tray for Inner {
    fn id(&self) -> String {
        "com.enfoquestic.nopass".to_string()
    }

    fn title(&self) -> String {
        "NoPass".to_string()
    }

    fn status(&self) -> ksni::Status {
        if self.view.attention { ksni::Status::NeedsAttention } else { ksni::Status::Active }
    }

    fn icon_name(&self) -> String {
        self.view.icon.to_string()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: self.view.icon.to_string(),
            icon_pixmap: Vec::new(),
            title: self.view.tooltip.title.clone(),
            description: self.view.tooltip.body.clone(),
        }
    }

    /// design.md's "left-click toggle": `Activate` raises the same
    /// request as the menu's toggle item. `MENU_ON_ACTIVATE` stays at its
    /// default `false` precisely so this fires instead of opening the
    /// menu.
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.try_send(TrayEvent::ToggleRequested);
    }

    fn menu_about_to_show(&mut self) {
        let _ = self.events.try_send(TrayEvent::MenuOpened);
    }

    /// design.md §7.2's minimal menu: an insensitive status label, the
    /// toggle (insensitive with "Checking…" when [`ViewModel::toggle`] is
    /// `None`), and `Quit`. This is the deliberate M2 subset of RF-03 —
    /// the full context menu is M3.
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            ksni::menu::StandardItem { label: self.view.status_line.clone(), enabled: false, ..Default::default() }
                .into(),
            ksni::MenuItem::Separator,
            ksni::menu::StandardItem {
                label: self.view.toggle.unwrap_or("Checking…").to_string(),
                enabled: self.view.toggle.is_some(),
                activate: Box::new(|this: &mut Inner| {
                    let _ = this.events.try_send(TrayEvent::ToggleRequested);
                }),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|this: &mut Inner| {
                    let _ = this.events.try_send(TrayEvent::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The concrete [`TrayPort`], backed by a running `ksni` service.
pub struct KsniTray {
    handle: ksni::Handle<Inner>,
}

impl KsniTray {
    /// Spawns the SNI item on `ksni`'s own `async-io` executor
    /// (design.md D1) and returns the port plus the channel every
    /// menu/activation callback feeds. Callers are expected to render
    /// `Unknown` as `initial` before any probe has run — design.md §6.1
    /// step 7's <1 s startup budget depends on this happening before, not
    /// after, the first reconciliation.
    pub async fn spawn(initial: ViewModel) -> Result<(KsniTray, async_channel::Receiver<TrayEvent>), ksni::Error> {
        let (tx, rx) = async_channel::unbounded();
        let inner = Inner { view: initial, events: tx };
        let handle = ksni::TrayMethods::spawn(inner).await?;
        Ok((KsniTray { handle }, rx))
    }
}

impl TrayPort for KsniTray {
    /// Pushes a new `ViewModel` and blocks until `ksni` has applied it —
    /// `Handle::update` is `ksni`'s own async API, but this trait's
    /// signature is synchronous per design.md's module map, so the
    /// round-trip is awaited here via `block_on`. `ksni`'s service loop
    /// runs on its own dedicated executor thread (design.md D1), never on
    /// the caller's, so this cannot self-deadlock — but a caller on the
    /// app's own single reactor thread (Phase 10's `app::run`) still pays
    /// for the round-trip synchronously. That tension is Phase 10's
    /// wiring decision to make, not this module's: it may call this
    /// off-thread the same way `run_off_reactor` does for `pkexec`, or
    /// this trait may need to grow an async variant once `app.rs` exists.
    fn render(&self, view: &ViewModel) {
        let view = view.clone();
        let _ = futures_lite::future::block_on(self.handle.update(move |inner| inner.view = view));
    }

    /// The best re-registration hook `ksni`'s public `Handle` exposes: a
    /// no-op property round-trip, which still touches the connection and
    /// re-emits the current properties. `ksni` itself re-issues the real
    /// `RegisterStatusNotifierItem` call internally whenever the watcher's
    /// `NameOwnerChanged` fires (design.md §6.1 step 8) — this module
    /// does not duplicate that internal path, only gives Phase 9's
    /// `AppInterface::activate` a way to nudge the current state back out.
    fn reassert(&self) {
        let _ = futures_lite::future::block_on(self.handle.update(|_inner| {}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nopass_core::expiry::Expiry;

    const NOW: u64 = 1_000_000;

    #[test]
    fn view_model_for_inactive_matches_format_output_exactly() {
        let v = ViewModel::from_state("jorge", &TrayState::Inactive, NOW);
        assert_eq!(v.icon, format::icon_name(&TrayState::Inactive));
        assert!(!v.attention);
        assert_eq!(v.status_line, "jorge — Inactive");
        assert_eq!(v.toggle, Some("Enable passwordless sudo"));
    }

    #[test]
    fn view_model_for_active_timed_matches_format_output_exactly() {
        let state = TrayState::Active { user: Some("jorge".to_string()), expiry: Some(Expiry::At { epoch: NOW + 60 }) };
        let v = ViewModel::from_state("jorge", &state, NOW);
        assert_eq!(v.icon, "nopass-unlocked-timed-symbolic");
        assert!(!v.attention);
        assert_eq!(v.status_line, "jorge — Active (1 min)");
        assert_eq!(v.toggle, Some("Disable passwordless sudo"));
    }

    #[test]
    fn view_model_for_unknown_requests_attention_and_disables_the_toggle() {
        let v = ViewModel::from_state("jorge", &TrayState::Unknown, NOW);
        assert_eq!(v.icon, "dialog-question-symbolic");
        assert!(v.attention, "Unknown must request attention (design.md D6)");
        assert_eq!(v.toggle, None);
        assert_eq!(v.status_line, "jorge — Checking…");
    }
}

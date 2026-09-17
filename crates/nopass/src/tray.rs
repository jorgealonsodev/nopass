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

use crate::duration::GrantDuration;
use crate::format::{self, ToolTip};
use crate::menu::{menu_tree, MenuModel, MenuNode, MenuNodeKind};
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
        ViewModel::from_state_in(format::lang(), user, state, now)
    }

    /// The same view model with the language named rather than read from
    /// the process environment.
    ///
    /// `from_state` above resolves the language from `LC_ALL`/`LC_MESSAGES`/
    /// `LANG`, which is right in production and wrong in a test: a test that
    /// asserts an exact string through `from_state` passes or fails
    /// depending on the developer's locale. Four tests in this crate did
    /// exactly that and went green for two milestones, because nobody had
    /// run them on a Spanish machine until `LANG=es_ES.UTF-8` met the
    /// localization phase. They assert through here now.
    pub(crate) fn from_state_in(lang: format::Lang, user: &str, state: &TrayState, now: u64) -> ViewModel {
        ViewModel {
            icon: format::icon_name(state),
            attention: matches!(state, TrayState::Unknown),
            tooltip: format::tooltip_in(lang, user, state, now),
            status_line: format::status_line_in(lang, user, state, now),
            toggle: format::toggle_label_in(lang, state),
        }
    }
}

/// What `app::run` (Phase 10) needs from the tray adapter (design.md §2
/// `tray`). `reassert` is `instance::AppInterface::activate`'s hook
/// (Phase 9) to make the item visible again after a second-instance
/// nudge.
pub trait TrayPort {
    fn render(&self, view: &ViewModel);
    /// Pushes a freshly built menu tree (task 8.7). `App` is the only
    /// caller in production — `crate::menu::menu_tree` is what every
    /// implementation must build the real (or recorded) menu from, so a
    /// fake in tests never needs to re-derive `ksni`/D-Bus specifics to
    /// prove `App` called this at all.
    fn render_menu(&self, model: &MenuModel);
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
    /// One entry inside "Activate during…" was clicked
    /// ([`MenuNodeKind::ActivateFor`], design.md §1 item 3.1-3.6). Routing
    /// this through [`crate::consent::ConsentState::grant`]/`arm` before
    /// any invocation is task 8.1's job — this module only raises the
    /// request, carrying nothing but the duration the user picked.
    DurationSelected(GrantDuration),
    /// The consent branch's "I understand — activate[, and don't warn me
    /// again]" ([`MenuNodeKind::ConfirmActivate`], design.md §1 "The
    /// consent branch"). `persist` is the "don't warn again" choice,
    /// carried as its own action rather than a checkbox — see
    /// [`crate::consent::ConsentState::confirm`]'s own doc comment for why.
    ConsentConfirmed { persist: bool },
    /// The consent branch's "Cancel" ([`MenuNodeKind::CancelActivate`]).
    ConsentCancelled,
    /// A "Default duration" `RadioGroup` entry was selected
    /// ([`MenuNodeKind::SelectDefaultDuration`], design.md §1 item 7.1).
    /// Persisting this as the new configured default is task 8.1's job,
    /// via [`crate::menu::select_default_duration`].
    DefaultDurationSelected(GrantDuration),
    /// "Start with session" was toggled ([`MenuNodeKind::ToggleAutostart`],
    /// design.md §1 item 8). `checked`'s new value always comes from the
    /// next `crate::autostart::read` — never cached here — so this event
    /// carries nothing but the request itself.
    AutostartToggled,
    /// The full menu's `Quit` item (design.md §1 item 11). Exiting the
    /// process with code 0 is `app::run`'s job (Phase 10) — this module
    /// only raises the request.
    Quit,
}

/// The `ksni::Tray` implementation itself. `ksni` takes ownership of this
/// value on its own executor once spawned (design.md D1's single
/// `async-io` reactor); every interaction afterwards goes through the
/// `ksni::Handle` [`KsniTray`] wraps, never this type directly.
struct Inner {
    view: ViewModel,
    /// The current menu tree, already built by [`crate::menu::menu_tree`] —
    /// `Inner::menu()` only translates it into `ksni` types, it never
    /// builds one itself (design.md §1: `menu_tree` is the single pure
    /// builder). Empty until the first [`KsniTray::render_menu`] call;
    /// `KsniTray::spawn` has only a `ViewModel` to work from, not the
    /// `Config`/`TrayState`/`FileReading`/`AutostartState` a [`MenuModel`]
    /// needs, so the menu starts empty rather than guessing at any of
    /// them.
    nodes: Vec<MenuNode>,
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

    /// The full RF-03 tree (design.md §1): translates whatever
    /// [`crate::menu::menu_tree`] last computed (`self.nodes`, pushed by
    /// [`KsniTray::render_menu`]) into real `ksni::MenuItem`s. This is the
    /// M3 replacement of M2's three-item minimal menu — see this module's
    /// own doc comment for why this translation lives here and nowhere
    /// else in the crate.
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        // A tray with no menu is worse than a tray with a small one: the
        // user loses Quit and has no way to stop the process from its own
        // icon. `nodes` is empty until something calls `render_menu`, and
        // until m3 Phase 8 wires that call, nothing in production does —
        // so mapping over it unguarded would have shipped an EMPTY menu
        // and silently withdrawn the three items M2 delivered.
        //
        // Every Lane B test passed anyway, because each one calls
        // `render_menu` itself before asserting. That is the shape of
        // defect this project keeps producing: a test that sets up state
        // the production path never reaches.
        //
        // The fallback is M2's exact menu, not an approximation, and it
        // uses only `TrayEvent` variants that are routed today.
        if self.nodes.is_empty() {
            return self.fallback_menu();
        }
        self.nodes.iter().map(node_to_menu_item).collect()
    }
}

impl Inner {
    /// M2's three-item menu, rendered when no `MenuNode` tree has been
    /// pushed yet. Localized through `Msg` like everything else, so it does
    /// not reintroduce the English literals phase 5 removed.
    fn fallback_menu(&self) -> Vec<ksni::MenuItem<Inner>> {
        let lang = format::lang();
        vec![
            ksni::menu::StandardItem { label: self.view.status_line.clone(), enabled: false, ..Default::default() }
                .into(),
            ksni::MenuItem::Separator,
            ksni::menu::StandardItem {
                label: self.view.toggle.map(str::to_string).unwrap_or_else(|| format::Msg::StatusChecking.text(lang).to_string()),
                enabled: self.view.toggle.is_some(),
                activate: Box::new(|this: &mut Inner| {
                    let _ = this.events.try_send(TrayEvent::ToggleRequested);
                }),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: format::Msg::MenuQuit.text(lang).to_string(),
                activate: Box::new(|this: &mut Inner| {
                    let _ = this.events.try_send(TrayEvent::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Converts one [`MenuNode`] into the `ksni::MenuItem` it renders as
/// (design.md §1's item-type table). The one place `MenuNode` crosses into
/// `ksni` — see this module's own doc comment for why no other module may
/// ever do so.
fn node_to_menu_item(node: &MenuNode) -> ksni::MenuItem<Inner> {
    match node.kind {
        MenuNodeKind::Toggle => standard_item(node, TrayEvent::ToggleRequested),
        MenuNodeKind::ActivateFor(duration) => standard_item(node, TrayEvent::DurationSelected(duration)),
        // Reached only if a `SelectDefaultDuration` node is ever converted
        // outside `radio_group_item`'s own construction — that function
        // builds these as `RadioItem`s directly and never recurses back
        // into this function for them (`static_item` intercepts the
        // "Default duration" shell before its children reach here). Kept
        // total, and consistent with the `RadioGroup` path's event, rather
        // than `unreachable!()`, so a future structural change degrades
        // instead of panicking mid-menu-build.
        MenuNodeKind::SelectDefaultDuration(duration) => standard_item(node, TrayEvent::DefaultDurationSelected(duration)),
        MenuNodeKind::ConfirmActivate { persist } => standard_item(node, TrayEvent::ConsentConfirmed { persist }),
        MenuNodeKind::CancelActivate => standard_item(node, TrayEvent::ConsentCancelled),
        MenuNodeKind::ToggleAutostart => checkmark_item(node),
        MenuNodeKind::Quit => standard_item(node, TrayEvent::Quit),
        MenuNodeKind::Static => static_item(node),
    }
}

/// An activatable, non-checkable leaf: every [`MenuNodeKind`] except
/// `Static`, `ToggleAutostart`, and the `SelectDefaultDuration` children a
/// `RadioGroup` renders instead (see [`radio_group_item`]).
fn standard_item(node: &MenuNode, event: TrayEvent) -> ksni::MenuItem<Inner> {
    ksni::menu::StandardItem {
        label: node.label.clone(),
        enabled: node.enabled,
        activate: Box::new(move |this: &mut Inner| {
            let _ = this.events.try_send(event);
        }),
        ..Default::default()
    }
    .into()
}

/// "Start with session" (design.md §1 item 8): `checked` comes straight
/// from `node.checked` — itself read fresh off disk by whatever built the
/// [`MenuModel`] this tree came from, never cached here.
fn checkmark_item(node: &MenuNode) -> ksni::MenuItem<Inner> {
    ksni::menu::CheckmarkItem {
        label: node.label.clone(),
        enabled: node.enabled,
        checked: node.checked.unwrap_or(false),
        activate: Box::new(|this: &mut Inner| {
            let _ = this.events.try_send(TrayEvent::AutostartToggled);
        }),
        ..Default::default()
    }
    .into()
}

/// A `Static` node (`menu.rs`'s "submenu shell, or an insensitive
/// reference/warning line with no action of its own"): an insensitive
/// `StandardItem` when it has no children, a `SubMenu` when it does — with
/// its children built specially for exactly one shell. The "Default
/// duration" shell is the only `Static` node whose children are
/// `SelectDefaultDuration` entries (`menu.rs::default_duration_node`'s the
/// sole caller of that kind), and those render as a `RadioGroup` — nested
/// one level inside the labelled `SubMenu` like any other submenu, since
/// `ksni::menu::RadioGroup` carries no label of its own and would
/// otherwise flatten its six options as siblings of whatever else sits at
/// this node's nesting level — never as individually-activatable `SubMenu`
/// children. Detected from the first child's kind, not a second,
/// separately maintained list.
fn static_item(node: &MenuNode) -> ksni::MenuItem<Inner> {
    if node.children.is_empty() {
        return ksni::menu::StandardItem { label: node.label.clone(), enabled: node.enabled, ..Default::default() }.into();
    }

    let submenu = if matches!(node.children.first().map(|child| &child.kind), Some(MenuNodeKind::SelectDefaultDuration(_))) {
        vec![radio_group_item(&node.children).into()]
    } else {
        node.children.iter().map(node_to_menu_item).collect()
    };

    ksni::menu::SubMenu { label: node.label.clone(), enabled: node.enabled, submenu, ..Default::default() }.into()
}

/// Builds the "Default duration" `RadioGroup` (design.md §1 item 7):
/// `selected` is the index of whichever child `menu.rs` already marked
/// `checked == Some(true)`, and `select`'s `index` argument is looked up
/// in [`GrantDuration::ALL`] — the SAME array `menu.rs::duration_items`
/// built these children from, never a second, independently maintained
/// ordering. That single lookup is what keeps "the user picks 4 hours"
/// and "the tray marks 4 hours as default" from ever silently disagreeing
/// (task 7.2's headline risk — a reordering or off-by-one must fail a
/// test, not mis-grant).
fn radio_group_item(children: &[MenuNode]) -> ksni::menu::RadioGroup<Inner> {
    let selected = children.iter().position(|child| child.checked == Some(true)).unwrap_or(0);
    let options = children
        .iter()
        .map(|child| ksni::menu::RadioItem { label: child.label.clone(), enabled: child.enabled, ..Default::default() })
        .collect();
    ksni::menu::RadioGroup {
        selected,
        select: Box::new(|this: &mut Inner, index: usize| {
            if let Some(duration) = GrantDuration::ALL.get(index).copied() {
                let _ = this.events.try_send(TrayEvent::DefaultDurationSelected(duration));
            }
        }),
        options,
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
        let inner = Inner { view: initial, nodes: Vec::new(), events: tx };
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

    /// Blocks until `ksni` has applied both the freshly built [`menu_tree`]
    /// AND `model.view` in the same round trip, so a caller that has just
    /// built a [`MenuModel`] never needs a second call to keep the
    /// icon/tooltip/status line in sync with the menu it came from.
    fn render_menu(&self, model: &MenuModel) {
        let nodes = menu_tree(model);
        let view = model.view.clone();
        let _ = futures_lite::future::block_on(self.handle.update(move |inner| {
            inner.view = view;
            inner.nodes = nodes;
        }));
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
        let v = ViewModel::from_state_in(format::Lang::En, "jorge", &TrayState::Inactive, NOW);
        assert_eq!(v.icon, format::icon_name(&TrayState::Inactive));
        assert!(!v.attention);
        assert_eq!(v.status_line, "jorge — Inactive");
        assert_eq!(v.toggle, Some("Enable passwordless sudo"));
    }

    #[test]
    fn view_model_for_active_timed_matches_format_output_exactly() {
        let state = TrayState::Active { user: Some("jorge".to_string()), expiry: Some(Expiry::At { epoch: NOW + 60 }) };
        let v = ViewModel::from_state_in(format::Lang::En, "jorge", &state, NOW);
        assert_eq!(v.icon, "nopass-unlocked-timed-symbolic");
        assert!(!v.attention);
        assert_eq!(v.status_line, "jorge — Active (1 min)");
        assert_eq!(v.toggle, Some("Disable passwordless sudo"));
    }

    #[test]
    fn view_model_for_unknown_requests_attention_and_disables_the_toggle() {
        let v = ViewModel::from_state_in(format::Lang::En, "jorge", &TrayState::Unknown, NOW);
        assert_eq!(v.icon, "dialog-question-symbolic");
        assert!(v.attention, "Unknown must request attention (design.md D6)");
        assert_eq!(v.toggle, None);
        assert_eq!(v.status_line, "jorge — Checking…");
    }

    // ---- task 7.1: MenuNode -> ksni::MenuItem<Inner> (Lane A, no bus) ----

    fn leaf(label: &str, enabled: bool, checked: Option<bool>, kind: MenuNodeKind) -> MenuNode {
        MenuNode { label: label.to_string(), enabled, checked, kind, children: Vec::new() }
    }

    /// A fresh [`Inner`] plus the receiving half of its event channel —
    /// enough to invoke a converted item's `activate`/`select` closure
    /// directly and observe what it sends, with no `ksni` spawn and no bus.
    fn test_inner() -> (Inner, async_channel::Receiver<TrayEvent>) {
        let (tx, rx) = async_channel::unbounded();
        let inner = Inner { view: ViewModel::from_state_in(format::Lang::En, "jorge", &TrayState::Inactive, NOW), nodes: Vec::new(), events: tx };
        (inner, rx)
    }

    #[test]
    fn toggle_node_becomes_an_enabled_standard_item_that_raises_toggle_requested() {
        let node = leaf("Enable passwordless sudo", true, None, MenuNodeKind::Toggle);
        let ksni::MenuItem::Standard(item) = node_to_menu_item(&node) else { panic!("Toggle must render as a StandardItem") };
        assert_eq!(item.label, "Enable passwordless sudo");
        assert!(item.enabled);

        let (mut inner, rx) = test_inner();
        (item.activate)(&mut inner);
        assert_eq!(rx.try_recv(), Ok(TrayEvent::ToggleRequested));
    }

    #[test]
    fn activate_for_node_becomes_a_standard_item_that_raises_duration_selected_with_its_own_duration() {
        let node = leaf("4 hours", true, None, MenuNodeKind::ActivateFor(GrantDuration::Hours4));
        let ksni::MenuItem::Standard(item) = node_to_menu_item(&node) else { panic!("ActivateFor must render as a StandardItem") };

        let (mut inner, rx) = test_inner();
        (item.activate)(&mut inner);
        assert_eq!(rx.try_recv(), Ok(TrayEvent::DurationSelected(GrantDuration::Hours4)));
    }

    #[test]
    fn confirm_activate_node_raises_consent_confirmed_carrying_the_exact_persist_flag() {
        for persist in [false, true] {
            let node = leaf("I understand — activate", true, None, MenuNodeKind::ConfirmActivate { persist });
            let ksni::MenuItem::Standard(item) = node_to_menu_item(&node) else { panic!("ConfirmActivate must render as a StandardItem") };

            let (mut inner, rx) = test_inner();
            (item.activate)(&mut inner);
            assert_eq!(rx.try_recv(), Ok(TrayEvent::ConsentConfirmed { persist }), "persist={persist} must round-trip exactly");
        }
    }

    #[test]
    fn cancel_activate_node_raises_consent_cancelled() {
        let node = leaf("Cancel", true, None, MenuNodeKind::CancelActivate);
        let ksni::MenuItem::Standard(item) = node_to_menu_item(&node) else { panic!("CancelActivate must render as a StandardItem") };

        let (mut inner, rx) = test_inner();
        (item.activate)(&mut inner);
        assert_eq!(rx.try_recv(), Ok(TrayEvent::ConsentCancelled));
    }

    #[test]
    fn quit_node_becomes_a_standard_item_that_raises_quit() {
        let node = leaf("Quit", true, None, MenuNodeKind::Quit);
        let ksni::MenuItem::Standard(item) = node_to_menu_item(&node) else { panic!("Quit must render as a StandardItem") };

        let (mut inner, rx) = test_inner();
        (item.activate)(&mut inner);
        assert_eq!(rx.try_recv(), Ok(TrayEvent::Quit));
    }

    #[test]
    fn toggle_autostart_node_becomes_a_checkmark_item_reflecting_checked_and_raises_autostart_toggled() {
        for checked in [false, true] {
            let node = leaf("Start with session", true, Some(checked), MenuNodeKind::ToggleAutostart);
            let ksni::MenuItem::Checkmark(item) = node_to_menu_item(&node) else { panic!("ToggleAutostart must render as a CheckmarkItem") };
            assert_eq!(item.checked, checked);

            let (mut inner, rx) = test_inner();
            (item.activate)(&mut inner);
            assert_eq!(rx.try_recv(), Ok(TrayEvent::AutostartToggled));
        }
    }

    #[test]
    fn a_childless_static_node_becomes_an_insensitive_standard_item_with_no_activation() {
        let node = leaf("No active rule", false, None, MenuNodeKind::Static);
        let ksni::MenuItem::Standard(item) = node_to_menu_item(&node) else { panic!("a childless Static node must render as a StandardItem") };
        assert_eq!(item.label, "No active rule");
        assert!(!item.enabled);
    }

    #[test]
    fn a_static_shell_with_ordinary_children_becomes_a_submenu_preserving_labels_and_order() {
        let node = MenuNode {
            label: "Activate during…".to_string(),
            enabled: true,
            checked: None,
            kind: MenuNodeKind::Static,
            children: vec![
                leaf("15 minutes", true, None, MenuNodeKind::ActivateFor(GrantDuration::Minutes15)),
                leaf("1 hour", true, None, MenuNodeKind::ActivateFor(GrantDuration::Hour1)),
            ],
        };

        let ksni::MenuItem::SubMenu(submenu) = node_to_menu_item(&node) else { panic!("a Static shell with children must render as a SubMenu") };
        assert_eq!(submenu.label, "Activate during…");
        assert_eq!(submenu.submenu.len(), 2);
        let mut children = submenu.submenu.into_iter();
        let ksni::MenuItem::Standard(first) = children.next().unwrap() else { panic!("expected a StandardItem") };
        assert_eq!(first.label, "15 minutes");
        let ksni::MenuItem::Standard(second) = children.next().unwrap() else { panic!("expected a StandardItem") };
        assert_eq!(second.label, "1 hour");
    }

    /// "Default duration" (design.md §1 item 7) renders as a `RadioGroup`,
    /// never a `SubMenu` of individually-activatable items — the one
    /// exception `static_item` special-cases.
    #[test]
    fn the_default_duration_shell_becomes_a_labelled_submenu_wrapping_a_radio_group() {
        let children: Vec<MenuNode> = GrantDuration::ALL
            .iter()
            .map(|&d| leaf("label", true, Some(d == GrantDuration::Hours4), MenuNodeKind::SelectDefaultDuration(d)))
            .collect();
        let node = MenuNode { label: "Default duration".to_string(), enabled: true, checked: None, kind: MenuNodeKind::Static, children };

        let ksni::MenuItem::SubMenu(submenu) = node_to_menu_item(&node) else { panic!("the Default duration shell must render as a labelled SubMenu") };
        assert_eq!(submenu.label, "Default duration");
        assert_eq!(submenu.submenu.len(), 1, "the RadioGroup itself is the shell's one and only submenu entry");
        let ksni::MenuItem::RadioGroup(group) = submenu.submenu.into_iter().next().unwrap() else {
            panic!("the Default duration shell's nested item must be a RadioGroup, never plain SubMenu children")
        };
        assert_eq!(group.options.len(), 6);
        assert_eq!(group.selected, 2, "Hours4 is index 2 in GrantDuration::ALL, and it is the only checked child");
    }

    /// Task 7.2's headline invariant: the `RadioGroup::select` callback's
    /// `usize` index must map back through [`GrantDuration::ALL`] — the
    /// SAME array the menu was built from — for every index in its domain,
    /// not merely for whichever one a shape-only test happens to exercise.
    /// A reordering or an off-by-one here silently mis-grants (the user
    /// picks "4 hours" and gets "until reboot" instead, with no error).
    #[test]
    fn radio_group_select_maps_every_index_back_through_grant_duration_all() {
        let children: Vec<MenuNode> =
            GrantDuration::ALL.iter().map(|&d| leaf("label", true, None, MenuNodeKind::SelectDefaultDuration(d))).collect();
        let group = radio_group_item(&children);

        for (index, expected) in GrantDuration::ALL.iter().enumerate() {
            let (mut inner, rx) = test_inner();
            (group.select)(&mut inner, index);
            assert_eq!(
                rx.try_recv(),
                Ok(TrayEvent::DefaultDurationSelected(*expected)),
                "index {index} must map to {expected:?} via GrantDuration::ALL, never a second ordering"
            );
        }
    }

    /// The structural half of the same invariant menu.rs's own
    /// `menu_rs_never_imports_ksni_or_the_action_enable_constructor` pins
    /// on the other side of the boundary: this module never imports the
    /// unconstructible `Granted` type or the `Action`/`EnableRequest`
    /// constructors it gates. `node_to_menu_item` only ever raises a
    /// `TrayEvent` — dispatching a confirmed action is Phase 8's job
    /// (design.md §1 "The consent branch"; spec `activation-consent` "No
    /// Grant Dispatch Without Recorded Consent").
    #[test]
    fn a_tray_that_has_never_been_given_a_node_tree_still_offers_quit() {
        // The production path, exactly: KsniTray::new sets `nodes` empty and
        // NOTHING calls render_menu until m3 Phase 8. Every Lane B test calls
        // it itself before asserting, so an empty menu passed every gate
        // while silently withdrawing the three items M2 shipped — including
        // Quit, leaving the user no way to stop the process from its icon.
        let (tx, _rx) = async_channel::unbounded();
        let inner = Inner {
            view: ViewModel::from_state_in(format::Lang::En, "jorge", &TrayState::Inactive, NOW),
            nodes: Vec::new(),
            events: tx,
        };
        let items = ksni::Tray::menu(&inner);
        assert_eq!(items.len(), 4, "M2 status, separator, toggle, Quit — an empty node tree falls back to that, not to nothing");

        // Quit specifically: it is the item whose absence strands the user.
        let lang = format::lang();
        let quit = format::Msg::MenuQuit.text(lang);
        let labels: Vec<String> = items
            .iter()
            .map(|i| match i {
                ksni::MenuItem::Standard(s) => s.label.clone(),
                _ => String::new(),
            })
            .collect();
        assert!(labels.iter().any(|l| l == quit), "Quit must always be reachable: {labels:?}");
    }

    #[test]
    fn tray_rs_never_imports_the_unconstructible_granted_type_or_the_action_enable_constructor() {
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tray.rs")).unwrap();
        let production = source.split("#[cfg(test)]").next().expect("tray.rs always has a #[cfg(test)] module");
        let code: String =
            production.lines().filter(|line| !line.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");

        assert!(!code.contains("consent::Granted"), "tray.rs must never import the unconstructible Granted type");
        assert!(!code.contains("outcome::Action"), "tray.rs must never import Action — dispatch is Phase 8's job");
        assert!(!code.contains("EnableRequest"), "tray.rs must never name the Action::Enable constructor");
    }
}

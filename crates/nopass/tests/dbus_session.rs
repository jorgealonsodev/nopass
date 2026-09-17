//! Lane B (`dbus-run-session`, gated by `NOPASS_DBUS_TESTS=1`) — tasks.md
//! Phase 7 tasks 7.4-7.6, Phase 8 tasks 8.2-8.6, Phase 9 tasks 9.3-9.6;
//! design.md §9 Lane B; spec `tray-presence` remaining scenarios,
//! `tray-notifications` (all), `tray-single-instance` (all).
//!
//! This proves the D-Bus protocol `tray.rs`/`notifications.rs`/
//! `instance.rs` speak, never rendering (design.md §9: "Lane B can prove
//! the correct name was published, never that it looked right" — that
//! half is Lane C, `tests/manual/README.md`, Phase 11).
//!
//! Every test here binds one of a fixed, small set of well-known names
//! (`org.kde.StatusNotifierWatcher`, `org.freedesktop.Notifications`,
//! `com.enfoquestic.nopass`) on whatever bus `DBUS_SESSION_BUS_ADDRESS`
//! resolves to. `cargo test` runs test functions from one binary
//! concurrently by default, and two concurrent claims of the same
//! well-known name on the same bus race. [`BUS_LOCK`] serialises every
//! test in this file for exactly that reason; it is not a workaround, it
//! is what "one shared private bus, several tests" requires.
//!
//! Skipped (this file, and why) when `NOPASS_DBUS_TESTS` is unset:
//! - `sni_properties_match_the_view_model_for_each_tray_state` (7.4)
//! - `exported_menu_matches_menu_tree_item_for_item_for_an_active_grant_including_radio_group_and_submenu_nesting`,
//!   `exported_menu_matches_menu_tree_item_for_item_while_inactive_with_no_active_rule`, and
//!   `exported_menu_reflects_the_consent_branch_and_cancel_raises_consent_cancelled_over_the_real_wire` (7.2 —
//!   the full RF-03 tree over the wire, replacing M2's minimal-menu test)
//! - `activate_over_the_real_sni_wire_raises_toggle_requested` and
//!   `about_to_show_over_the_real_dbusmenu_wire_raises_menu_opened`
//!   (verify-report.md G4/F6 — the two `ksni` callbacks Phase 7 never
//!   got a test of their own)
//! - `ticks_keep_firing_on_schedule_while_a_live_tray_holds_the_bus` (7.6)
//! - every `notifications_*`/`single_instance_*` test below (Phase 8/9)
//! - every `real_binary_*` test below (Phase 10, tasks 10.5-10.6): each
//!   spawns the actual compiled `nopass` binary as a subprocess against
//!   this file's shared bus

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_lite::StreamExt;
use zbus::zvariant::OwnedValue;

use nopass::autostart::AutostartState;
use nopass::format::ToolTip as FormatToolTip;
use nopass::instance::{self, AppInterface, Acquisition, APPLICATION_INTERFACE, OBJECT_PATH, SERVICE_NAME};
use nopass::menu::{menu_tree, MenuModel, MenuNode, MenuNodeKind};
use nopass::notifications::{action_notification, already_running_notification, expiry_notification, Category, FreedesktopNotifier, NotifyPort};
use nopass::config::Config;
use nopass::consent::ConsentState;
use nopass::duration::GrantDuration;
use nopass::outcome::{classify, Action, EnableRequest, OutcomeKind};
use nopass::reconcile::TrayState;
use nopass::state::FileReading;
use nopass::tray::{KsniTray, TrayEvent, TrayPort, ViewModel};

/// Serialises every test in this file — see the module doc for why.
static BUS_LOCK: Mutex<()> = Mutex::new(());

fn lane_b_enabled() -> bool {
    std::env::var("NOPASS_DBUS_TESTS").ok().as_deref() == Some("1")
}

macro_rules! skip_unless_lane_b {
    ($name:expr) => {
        if !lane_b_enabled() {
            eprintln!("skipping {}: set NOPASS_DBUS_TESTS=1 (under dbus-run-session) to run", $name);
            return;
        }
    };
}

/// A fake `org.kde.StatusNotifierWatcher` — the real desktop-provided
/// service `ksni` looks up unconditionally. It always reports a host
/// present (so `KsniTray::spawn` completes registration instead of
/// routing through `watcher_offline`) and records every service name it
/// is asked to register.
struct FakeWatcher {
    registered: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl FakeWatcher {
    async fn register_status_notifier_item(&self, service: &str) {
        self.registered.lock().unwrap().push(service.to_string());
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }
}

/// Binds the fake watcher on whatever bus `DBUS_SESSION_BUS_ADDRESS`
/// resolves to. The returned `Connection` must stay alive for the
/// duration of the test — dropping it releases the well-known name.
async fn spawn_fake_watcher() -> (zbus::Connection, Arc<Mutex<Vec<String>>>) {
    let registered = Arc::new(Mutex::new(Vec::new()));
    let watcher = FakeWatcher { registered: registered.clone() };
    let conn = zbus::connection::Builder::session()
        .expect("session bus address must be resolvable under dbus-run-session")
        .name("org.kde.StatusNotifierWatcher")
        .expect("well-known name must be a valid bus name")
        .serve_at("/StatusNotifierWatcher", watcher)
        .expect("watcher object path must be valid")
        .build()
        .await
        .expect("fake watcher must be able to connect and claim its name");
    (conn, registered)
}

/// Client-side mirror of `ksni::ToolTip`'s wire shape (`(sa(iiay)ss)`) —
/// `(icon_name, icon_pixmap, title, description)`. The pixmap element
/// (`a(iiay)`) is kept opaque as `Vec<OwnedValue>`; neither test here
/// inspects pixmap contents.
type RawToolTip = (String, Vec<OwnedValue>, String, String);

/// Client-side mirror of `com.canonical.dbusmenu`'s recursive layout
/// struct (`(ia{sv}av)`) — `ksni`'s own `dbus_interface::Layout`:
/// `(id, properties, children)`.
type RawLayout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

#[zbus::proxy(interface = "org.kde.StatusNotifierItem", default_path = "/StatusNotifierItem")]
trait Item {
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;

    /// Declared as a raw `OwnedValue`, not `RawToolTip` directly: a
    /// D-Bus property getter converts via `TryFrom<OwnedValue>`, which
    /// `zvariant` only implements for primitives and a few container
    /// types — never for tuples (its own `owned_value.rs` carries a
    /// `FIXME` about this gap). [`decode_tool_tip`] does the conversion
    /// by hand, the same two-step `OwnedValue -> Value -> T::downcast`
    /// route `ksni`'s own `Layout::try_from(OwnedValue)` uses internally.
    #[zbus(property, name = "ToolTip")]
    fn tool_tip(&self) -> zbus::Result<OwnedValue>;

    /// verify-report.md G4: the left-click toggle. `ksni`'s generated
    /// `activate` server method takes `(x, y)`, per
    /// `org.kde.StatusNotifierItem`'s own spec.
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "com.canonical.dbusmenu", default_path = "/MenuBar")]
trait Menu {
    fn get_layout(&self, parent_id: i32, recursion_depth: i32, property_names: Vec<&str>) -> zbus::Result<(u32, RawLayout)>;

    fn event(&self, id: i32, event_id: &str, data: OwnedValue, timestamp: u32) -> zbus::Result<()>;

    /// verify-report.md F6: `menu_about_to_show`'s wire trigger.
    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;
}

/// See [`ItemProxy::tool_tip`]'s doc comment for why this indirection
/// exists instead of a direct property return type.
fn decode_tool_tip(value: OwnedValue) -> (String, String) {
    let value: zbus::zvariant::Value<'static> = value.into();
    let (_icon_name, _pixmap, title, description): RawToolTip =
        value.downcast().expect("ToolTip must decode as (s, a(iiay), s, s)");
    (title, description)
}

async fn read_item(destination: &str) -> (String, String, (String, String)) {
    let client = zbus::Connection::session().await.expect("client must connect to the same bus");
    let proxy = ItemProxy::builder(&client)
        .destination(destination)
        .expect("destination must be a valid bus name")
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .expect("item proxy must build against a live SNI object");
    let icon_name = proxy.icon_name().await.expect("IconName must be readable");
    let status = proxy.status().await.expect("Status must be readable");
    let tool_tip = decode_tool_tip(proxy.tool_tip().await.expect("ToolTip must be readable"));
    (icon_name, status, tool_tip)
}

fn owned_str(value: &OwnedValue) -> String {
    <String as TryFrom<OwnedValue>>::try_from(value.try_clone().expect("cloneable OwnedValue")).expect("expected a string property value")
}

fn owned_bool(value: &OwnedValue) -> bool {
    <bool as TryFrom<OwnedValue>>::try_from(value.try_clone().expect("cloneable OwnedValue")).expect("expected a bool property value")
}

fn owned_i32(value: &OwnedValue) -> i32 {
    <i32 as TryFrom<OwnedValue>>::try_from(value.try_clone().expect("cloneable OwnedValue")).expect("expected an i32 property value")
}

/// One node of the DBusMenu `GetLayout` recursion, decoded into an owned
/// tree — unlike [`read_menu_children`], this keeps every level, not just
/// the immediate children, so task 7.2's "submenu nesting correct"
/// assertion (design.md §9 Lane B) can walk the whole exported tree.
struct DecodedMenuNode {
    id: i32,
    properties: HashMap<String, OwnedValue>,
    children: Vec<DecodedMenuNode>,
}

fn decode_menu_node(raw: RawLayout) -> DecodedMenuNode {
    let (id, properties, raw_children) = raw;
    let children = raw_children
        .into_iter()
        .map(|child| {
            let value: zbus::zvariant::Value<'static> = child.into();
            let raw_child: RawLayout = value.downcast().expect("every menu child must decode as (i,a{sv},av)");
            decode_menu_node(raw_child)
        })
        .collect();
    DecodedMenuNode { id, properties, children }
}

/// Reads the WHOLE exported menu tree — every level, every property (an
/// empty filter means "no filter" per `to_dbus_map`) — rather than just
/// the immediate children [`read_menu_children`] keeps.
async fn read_full_menu_tree(destination: &str) -> (DecodedMenuNode, MenuProxy<'static>) {
    let client = zbus::Connection::session().await.expect("client must connect to the same bus");
    let proxy = MenuProxy::builder(&client)
        .destination(destination.to_string())
        .expect("destination must be a valid bus name")
        .build()
        .await
        .expect("menu proxy must build against a live DBusMenu object");
    let (_revision, root) =
        proxy.get_layout(0, -1, vec![]).await.expect("GetLayout must succeed against a running tray");
    (decode_menu_node(root), proxy)
}

/// A "Default duration" `RadioGroup` shell is the one [`MenuNode`] whose
/// children are [`MenuNodeKind::SelectDefaultDuration`] — see
/// `tray.rs::static_item`'s own doc comment for why this is the single
/// detection point, never a second parallel list.
fn is_radio_group_shell(node: &MenuNode) -> bool {
    matches!(node.children.first().map(|child| &child.kind), Some(MenuNodeKind::SelectDefaultDuration(_)))
}

/// Asserts the exported DBusMenu tree matches `menu.rs`'s own [`MenuNode`]
/// tree item-for-item: label, sensitivity, and nesting at every level,
/// with the "Default duration" shell asserted as a real DBusMenu radio
/// group instead of recursing into it as an ordinary submenu.
fn assert_tree_matches(actual: &[DecodedMenuNode], expected: &[MenuNode]) {
    assert_eq!(actual.len(), expected.len(), "top-level item count must match menu_tree exactly");
    for (a, e) in actual.iter().zip(expected.iter()) {
        assert_node_matches(a, e);
    }
}

fn assert_node_matches(actual: &DecodedMenuNode, expected: &MenuNode) {
    let label = actual.properties.get("label").map(owned_str).unwrap_or_default();
    assert_eq!(label, expected.label, "label mismatch");
    assert_eq!(menu_item_enabled(&actual.properties), expected.enabled, "enabled mismatch for {label:?}");

    if is_radio_group_shell(expected) {
        assert_radio_group_matches(&actual.children, &expected.children, &label);
        return;
    }

    assert_eq!(actual.children.len(), expected.children.len(), "child count mismatch under {label:?}");
    for (a_child, e_child) in actual.children.iter().zip(expected.children.iter()) {
        assert_node_matches(a_child, e_child);
    }
}

/// The wire half of the round-trip invariant `tray.rs`'s own
/// `radio_group_select_maps_every_index_back_through_grant_duration_all`
/// pins on the pure-function side: every radio option must carry
/// `toggle-type: "radio"`, exactly one option's `toggle-state` must be `1`
/// (selected), and that one must be the `MenuNode` `menu.rs` marked
/// `checked == Some(true)` — never a second, independently maintained
/// notion of "current".
fn assert_radio_group_matches(actual: &[DecodedMenuNode], expected: &[MenuNode], parent_label: &str) {
    assert_eq!(actual.len(), expected.len(), "radio group option count mismatch under {parent_label:?}");
    let mut selected_count = 0;
    for (a, e) in actual.iter().zip(expected.iter()) {
        let label = a.properties.get("label").map(owned_str).unwrap_or_default();
        assert_eq!(label, e.label);
        let toggle_type = a.properties.get("toggle-type").map(owned_str).unwrap_or_default();
        assert_eq!(toggle_type, "radio", "{label:?} under {parent_label:?} must render as a DBusMenu radio item");
        let selected = a.properties.get("toggle-state").map(owned_i32).unwrap_or(-1) == 1;
        assert_eq!(selected, e.checked == Some(true), "{label:?}'s toggle-state must reflect menu_tree's own checked marker");
        if selected {
            selected_count += 1;
        }
    }
    assert_eq!(selected_count, 1, "exactly one radio option must be selected under {parent_label:?}");
}

/// The DBusMenu wire protocol omits a property from `GetLayout`'s map
/// entirely when it equals that property's documented default —
/// `enabled`'s default is `true` (`ksni`'s own `to_dbus_map`, mirroring
/// the `com.canonical.dbusmenu` spec) — so an absent key means enabled,
/// not missing data.
fn menu_item_enabled(properties: &HashMap<String, OwnedValue>) -> bool {
    match properties.get("enabled") {
        Some(v) => owned_bool(v),
        None => true,
    }
}

/// Task 7.4: SNI registration against a fake watcher, with `IconName`,
/// `Status`, and `ToolTip` read back matching the `ViewModel` for each of
/// the three `TrayState`s plus `Unknown` (tray-presence "Icon name
/// follows the merged state exactly").
#[test]
fn sni_properties_match_the_view_model_for_each_tray_state() {
    skip_unless_lane_b!("sni_properties_match_the_view_model_for_each_tray_state");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_watcher_conn, registered) = spawn_fake_watcher().await;

        let inactive = ViewModel::from_state("jorge", &TrayState::Inactive, 0);
        let (tray, _events) =
            KsniTray::spawn(inactive.clone()).await.expect("spawn must succeed with a fake watcher present");

        let destination = registered.lock().unwrap().last().cloned().expect("watcher must have observed a registration");

        assert_view_model_matches(&destination, &inactive).await;

        const NOW: u64 = 1_000_000;
        let active_timed = ViewModel::from_state(
            "jorge",
            &TrayState::Active { user: Some("jorge".to_string()), expiry: Some(nopass_core::expiry::Expiry::At { epoch: NOW + 60 }) },
            NOW,
        );
        tray.render(&active_timed);
        assert_view_model_matches(&destination, &active_timed).await;

        let unknown = ViewModel::from_state("jorge", &TrayState::Unknown, 0);
        tray.render(&unknown);
        assert_view_model_matches(&destination, &unknown).await;
    });
}

async fn assert_view_model_matches(destination: &str, expected: &ViewModel) {
    let (icon_name, status, (title, description)) = read_item(destination).await;
    assert_eq!(icon_name, expected.icon, "IconName must match the ViewModel");
    let expected_status = if expected.attention { "NeedsAttention" } else { "Active" };
    assert_eq!(status, expected_status, "Status must match ViewModel::attention");
    let read_tooltip = FormatToolTip { title, body: description };
    assert_eq!(read_tooltip, expected.tooltip, "ToolTip title/description must match the ViewModel");
}

/// Waits up to 2s for the next `TrayEvent`, the same bounded pattern every
/// wire-click assertion in this file already used inline.
async fn recv_or_none(events: &async_channel::Receiver<TrayEvent>) -> Option<TrayEvent> {
    futures_lite::future::or(
        async { Some(events.recv().await.expect("the events channel must still be open")) },
        async {
            async_io::Timer::after(Duration::from_secs(2)).await;
            None
        },
    )
    .await
}

/// Task 7.2: the exported DBusMenu tree over a fake `StatusNotifierWatcher`
/// matches `menu.rs`'s own `menu_tree` item-for-item — label, sensitivity,
/// and nesting at every level, including the "Default duration"
/// `RadioGroup`'s `selected` index (tray-menu "The full item tree is
/// present...", "Both submenus render the same six items...", "The marker
/// follows the configured default"). Both sides call the SAME
/// locale-resolving `menu_tree` in the SAME process, so this stays correct
/// under any `LANG` rather than asserting an English literal (this
/// crate's own hard-won lesson — `tray.rs`'s `ViewModel::from_state_in`
/// doc comment).
#[test]
fn exported_menu_matches_menu_tree_item_for_item_for_an_active_grant_including_radio_group_and_submenu_nesting() {
    skip_unless_lane_b!("exported_menu_matches_menu_tree_item_for_item_for_an_active_grant_including_radio_group_and_submenu_nesting");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_watcher_conn, registered) = spawn_fake_watcher().await;

        const NOW: u64 = 1_000_000;
        let expiry = nopass_core::expiry::Expiry::At { epoch: NOW + 60 * 42 };
        let state = TrayState::Active { user: Some("jorge".to_string()), expiry: Some(expiry) };
        let status = nopass_core::state::HelperStatus {
            schema: 1,
            uid: 1000,
            user: "jorge".to_string(),
            active: true,
            expires: Some(expiry),
            rule_path: "/etc/sudoers.d/90-nopass-1000".to_string(),
            updated_at: NOW,
        };
        let model = MenuModel {
            view: ViewModel::from_state("jorge", &state, NOW),
            state,
            file: FileReading::Parsed(status),
            now: NOW,
            config: Config { default_duration: GrantDuration::Hours4, warning_acknowledged: true },
            consent_branch: None,
            autostart: AutostartState::Enabled,
        };
        let expected = menu_tree(&model);

        let (tray, events) =
            KsniTray::spawn(model.view.clone()).await.expect("spawn must succeed with a fake watcher present");
        let destination = registered.lock().unwrap().last().cloned().expect("watcher must have observed a registration");

        tray.render_menu(&model);

        let (root, menu_proxy) = read_full_menu_tree(&destination).await;
        assert_tree_matches(&root.children, &expected);

        // ---- The RadioGroup index round-trips over the REAL wire. ----
        // `ksni`'s own `menu_flatten` maps a raw clicked item id back to a
        // local index (`id - offset`) before calling `select` — the pure
        // `radio_group_item` test in `tray.rs` cannot exercise that
        // subtraction at all, so only this proves the whole chain (click
        // -> id -> index -> GrantDuration::ALL[index]) end to end.
        let default_duration = &root.children[2];
        // Not an English literal — `assert_tree_matches` above already
        // proved this node's label matches `menu_tree`'s own output
        // (`expected[2]`) under whatever `LANG` this process runs under.
        assert_eq!(default_duration.properties.get("label").map(owned_str).unwrap_or_default(), expected[2].label);
        let hour1_id = default_duration.children[1].id; // GrantDuration::ALL[1] == Hour1
        menu_proxy.event(hour1_id, "clicked", OwnedValue::from(0u8), 0).await.expect("Event(SelectDefaultDuration, clicked) must be accepted");
        assert_eq!(
            recv_or_none(&events).await,
            Some(TrayEvent::DefaultDurationSelected(GrantDuration::Hour1)),
            "clicking the 2nd radio option must raise exactly DefaultDurationSelected(Hour1), not a mis-mapped duration"
        );

        // ---- Quit still raises TrayEvent::Quit over the real wire. ----
        let quit_id = root.children[6].id;
        menu_proxy.event(quit_id, "clicked", OwnedValue::from(0u8), 0).await.expect("Event(Quit, clicked) must be accepted");
        assert_eq!(recv_or_none(&events).await, Some(TrayEvent::Quit), "clicking Quit must raise exactly TrayEvent::Quit");
    });
}

/// Triangulates the structural comparison above against a different
/// branch: `Inactive`, no coherent file, autostart disabled — exercising
/// "Current rule"'s single insensitive "No active rule" placeholder and
/// the unchecked autostart checkbox instead of the four-detail/`Enabled`
/// paths above.
#[test]
fn exported_menu_matches_menu_tree_item_for_item_while_inactive_with_no_active_rule() {
    skip_unless_lane_b!("exported_menu_matches_menu_tree_item_for_item_while_inactive_with_no_active_rule");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_watcher_conn, registered) = spawn_fake_watcher().await;

        let model = MenuModel {
            view: ViewModel::from_state("jorge", &TrayState::Inactive, 0),
            state: TrayState::Inactive,
            file: FileReading::Absent,
            now: 0,
            config: Config { default_duration: GrantDuration::Minutes15, warning_acknowledged: true },
            consent_branch: None,
            autostart: AutostartState::Disabled,
        };
        let expected = menu_tree(&model);

        let (tray, _events) =
            KsniTray::spawn(model.view.clone()).await.expect("spawn must succeed with a fake watcher present");
        let destination = registered.lock().unwrap().last().cloned().expect("watcher must have observed a registration");

        tray.render_menu(&model);

        let (root, _menu_proxy) = read_full_menu_tree(&destination).await;
        assert_tree_matches(&root.children, &expected);
    });
}

/// The consent branch (design.md §1 "The consent branch"; spec
/// `activation-consent` "First Activation Branches the Menu Instead of
/// Granting") replaces the exported tree's first two items exactly as it
/// does bus-free in `menu.rs`, and clicking "Cancel" over the real wire
/// raises exactly `TrayEvent::ConsentCancelled` — never anything that
/// could construct an `Action::Enable` (this module's own
/// `tray_rs_never_imports_the_unconstructible_granted_type_or_the_action_enable_constructor`
/// pins the structural half of the same guarantee).
#[test]
fn exported_menu_reflects_the_consent_branch_and_cancel_raises_consent_cancelled_over_the_real_wire() {
    skip_unless_lane_b!("exported_menu_reflects_the_consent_branch_and_cancel_raises_consent_cancelled_over_the_real_wire");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_watcher_conn, registered) = spawn_fake_watcher().await;

        let mut consent = ConsentState::from_config(&Config { default_duration: GrantDuration::Hour1, warning_acknowledged: false });
        consent.arm(GrantDuration::Hours4);

        let model = MenuModel {
            view: ViewModel::from_state("jorge", &TrayState::Inactive, 0),
            state: TrayState::Inactive,
            file: FileReading::Absent,
            now: 0,
            config: Config { default_duration: GrantDuration::Hour1, warning_acknowledged: false },
            consent_branch: consent.branch(),
            autostart: AutostartState::Disabled,
        };
        assert!(model.consent_branch.is_some(), "precondition: a duration must be armed and unacknowledged");
        let expected = menu_tree(&model);
        assert_eq!(expected[0].kind, MenuNodeKind::Static, "precondition: row 0 must be the branch's insensitive warning title");

        let (tray, events) =
            KsniTray::spawn(model.view.clone()).await.expect("spawn must succeed with a fake watcher present");
        let destination = registered.lock().unwrap().last().cloned().expect("watcher must have observed a registration");

        tray.render_menu(&model);

        let (root, menu_proxy) = read_full_menu_tree(&destination).await;
        assert_tree_matches(&root.children, &expected);

        let cancel_id = root.children[4].id; // warning title, warning body, confirm-once, confirm-persist, Cancel
        menu_proxy.event(cancel_id, "clicked", OwnedValue::from(0u8), 0).await.expect("Event(CancelActivate, clicked) must be accepted");
        assert_eq!(
            recv_or_none(&events).await,
            Some(TrayEvent::ConsentCancelled),
            "clicking Cancel in the consent branch must raise exactly ConsentCancelled"
        );
    });
}

/// verify-report.md G4: experiment F7 emptied `ksni::Tray::activate` in
/// `tray.rs` — the left-click toggle, PRD RF-02's headline interaction —
/// and Lane B stayed green (15/15); the menu test above only ever drives
/// `Quit`. This calls the real `org.kde.StatusNotifierItem.Activate`
/// method over the wire and reads the resulting `TrayEvent` off the same
/// channel `KsniTray::spawn` returns.
#[test]
fn activate_over_the_real_sni_wire_raises_toggle_requested() {
    skip_unless_lane_b!("activate_over_the_real_sni_wire_raises_toggle_requested");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_watcher_conn, registered) = spawn_fake_watcher().await;
        let view = ViewModel::from_state("jorge", &TrayState::Inactive, 0);
        let (_tray, events) = KsniTray::spawn(view).await.expect("spawn must succeed with a fake watcher present");
        let destination = registered.lock().unwrap().last().cloned().expect("watcher must have observed a registration");

        let client = zbus::Connection::session().await.expect("client must connect to the same bus");
        let proxy = ItemProxy::builder(&client)
            .destination(destination)
            .expect("destination must be a valid bus name")
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .expect("item proxy must build against a live SNI object");

        proxy.activate(0, 0).await.expect("Activate must be accepted");

        let raised = futures_lite::future::or(
            async { Some(events.recv().await.expect("the events channel must still be open")) },
            async {
                async_io::Timer::after(Duration::from_secs(2)).await;
                None
            },
        )
        .await;
        assert_eq!(
            raised,
            Some(TrayEvent::ToggleRequested),
            "Activate over the real SNI wire must raise exactly TrayEvent::ToggleRequested"
        );
    });
}

/// verify-report.md F6: experiment F6 stopped `menu_about_to_show` from
/// raising `TrayEvent::MenuOpened` — one of `tray-state-sync` S3's four
/// named reconciliation triggers — and Lane B stayed green; the menu
/// test above never drives `AboutToShow`. This calls the real
/// `com.canonical.dbusmenu.AboutToShow` method and reads the resulting
/// event.
#[test]
fn about_to_show_over_the_real_dbusmenu_wire_raises_menu_opened() {
    skip_unless_lane_b!("about_to_show_over_the_real_dbusmenu_wire_raises_menu_opened");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_watcher_conn, registered) = spawn_fake_watcher().await;
        let view = ViewModel::from_state("jorge", &TrayState::Inactive, 0);
        let (_tray, events) = KsniTray::spawn(view).await.expect("spawn must succeed with a fake watcher present");
        let destination = registered.lock().unwrap().last().cloned().expect("watcher must have observed a registration");

        let client = zbus::Connection::session().await.expect("client must connect to the same bus");
        let proxy = MenuProxy::builder(&client)
            .destination(destination.to_string())
            .expect("destination must be a valid bus name")
            .build()
            .await
            .expect("menu proxy must build against a live DBusMenu object");

        proxy.about_to_show(0).await.expect("AboutToShow must be accepted");

        let raised = futures_lite::future::or(
            async { Some(events.recv().await.expect("the events channel must still be open")) },
            async {
                async_io::Timer::after(Duration::from_secs(2)).await;
                None
            },
        )
        .await;
        assert_eq!(
            raised,
            Some(TrayEvent::MenuOpened),
            "AboutToShow over the real DBusMenu wire must raise exactly TrayEvent::MenuOpened"
        );
    });
}

/// Task 7.6: re-verifies, "under the full stack" (a live `KsniTray` on a
/// real bus connection), the structural claim task 4.4 already pins at
/// the source level — no periodic wakeup exists beyond the 60 s
/// reconciliation tick. A live tray does not preempt or interfere with
/// `async_io::Timer` registration on the same executor (tray-presence "No
/// timer exists solely to refresh the tooltip").
#[test]
fn ticks_keep_firing_on_schedule_while_a_live_tray_holds_the_bus() {
    skip_unless_lane_b!("ticks_keep_firing_on_schedule_while_a_live_tray_holds_the_bus");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_watcher_conn, _registered) = spawn_fake_watcher().await;
        let initial = ViewModel::from_state("jorge", &TrayState::Inactive, 0);
        let (_tray, _events) = KsniTray::spawn(initial).await.expect("spawn must succeed with a fake watcher present");

        let period = Duration::from_millis(30);
        let mut stream = nopass::event::tick_stream(period);
        let started = std::time::Instant::now();
        for _ in 0..3 {
            let event = stream.next().await.expect("the interval stream never ends");
            assert!(matches!(event, nopass::event::Event::Tick));
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "three 30ms ticks must not take anywhere near 5s even with a live tray holding the bus"
        );
    });
}

// =====================================================================
// Phase 8 — Notifications (tasks.md 8.2-8.6; spec `tray-notifications`)
// =====================================================================

/// One captured `Notify` call — verbatim, so tests can assert exactly
/// what `FreedesktopNotifier` delivered over the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NotifyCall {
    app_name: String,
    replaces_id: u32,
    summary: String,
    body: String,
}

/// A fake `org.freedesktop.Notifications`. Returns a fresh monotonically
/// increasing id for a new notification (`replaces_id == 0`) and echoes
/// `replaces_id` back for an update — the same contract every real
/// notification daemon implements, which is what lets
/// `FreedesktopNotifier::post` retain and update one handle per
/// `Category` instead of stacking a new notification each time.
struct FakeNotifications {
    calls: Arc<Mutex<Vec<NotifyCall>>>,
    next_id: Mutex<u32>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl FakeNotifications {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let _ = (app_icon, actions, hints, expire_timeout);
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            *next
        };
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push(NotifyCall {
            app_name,
            replaces_id,
            summary,
            body,
        });
        id
    }
}

/// Binds the fake notification daemon on whatever bus
/// `DBUS_SESSION_BUS_ADDRESS` resolves to. The returned `Connection` must
/// stay alive for the duration of the test.
async fn spawn_fake_notifications() -> (zbus::Connection, Arc<Mutex<Vec<NotifyCall>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let service = FakeNotifications { calls: calls.clone(), next_id: Mutex::new(0) };
    let conn = zbus::connection::Builder::session()
        .expect("session bus address must be resolvable under dbus-run-session")
        .name("org.freedesktop.Notifications")
        .expect("well-known name must be a valid bus name")
        .serve_at("/org/freedesktop/Notifications", service)
        .expect("notifications object path must be valid")
        .build()
        .await
        .expect("fake notifications daemon must be able to connect and claim its name");
    (conn, calls)
}

/// Task 8.2: successful enable/disable emits a confirmation notification,
/// and a detected expiry emits its own, independent one (spec "Success,
/// Failure, and Expiry Notifications").
#[test]
fn successful_action_and_detected_expiry_each_produce_a_delivered_notification() {
    skip_unless_lane_b!("successful_action_and_detected_expiry_each_produce_a_delivered_notification");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_daemon_conn, calls) = spawn_fake_notifications().await;
        let notifier = FreedesktopNotifier::new();

        let consent = ConsentState::from_config(&Config {
            default_duration: GrantDuration::Hour1,
            warning_acknowledged: true,
        });
        let granted_token = consent.grant().expect("an acknowledged ConsentState always yields Granted");
        let enable_action = Action::Enable(EnableRequest::new(GrantDuration::Hour1, 1_700_000_000, granted_token));
        let granted = classify(enable_action, Some(0), true);
        let (summary, body) = action_notification(granted, "1 hour");
        notifier.post(Category::Action, &summary, &body);

        let (expiry_summary, expiry_body) = expiry_notification();
        notifier.post(Category::Expiry, &expiry_summary, &expiry_body);

        let captured = calls.lock().unwrap().clone();
        assert_eq!(captured.len(), 2, "one delivery per category: {captured:?}");
        assert_eq!(captured[0].summary, "Passwordless sudo enabled");
        assert!(captured[0].body.contains("1 hour"));
        assert_eq!(captured[1].summary, "Passwordless sudo expired");
        assert_ne!(captured[0].body, captured[1].body, "the two notifications must never share a body");
    });
}

/// Task 8.3: a non-sudoer rejection (exit 12) and a visudo rejection
/// (exit 14) render distinct summaries and bodies (spec "Notification
/// Body Is Distinct Per Outcome").
#[test]
fn not_sudoer_and_visudo_rejected_payloads_are_captured_and_distinct() {
    skip_unless_lane_b!("not_sudoer_and_visudo_rejected_payloads_are_captured_and_distinct");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_daemon_conn, calls) = spawn_fake_notifications().await;
        let notifier = FreedesktopNotifier::new();

        let not_sudoer = classify(Action::Disable, Some(12), true);
        let visudo_rejected = classify(Action::Disable, Some(14), true);
        assert_eq!(not_sudoer, OutcomeKind::NotSudoer);
        assert_eq!(visudo_rejected, OutcomeKind::VisudoRejected);

        let (s1, b1) = action_notification(not_sudoer, "");
        notifier.post(Category::Action, &s1, &b1);
        let (s2, b2) = action_notification(visudo_rejected, "");
        notifier.post(Category::Action, &s2, &b2);

        let captured = calls.lock().unwrap().clone();
        assert_eq!(captured.len(), 2);
        assert_ne!((captured[0].summary.as_str(), captured[0].body.as_str()), (captured[1].summary.as_str(), captured[1].body.as_str()));
        // Same category, so the second `Notify` call must be an update
        // (non-zero `replaces_id` echoing the first's id) — the retained-
        // per-category-handle contract (design.md §7.3).
        assert_ne!(captured[1].replaces_id, 0, "the second Action post must update the first's handle, not stack a new one");
    });
}

/// Task 8.4: with no owner for `org.freedesktop.Notifications`, `post`
/// still returns promptly (no retry loop, no hang) and logs to its error
/// sink exactly once per call — and, independently, the tray keeps
/// rendering (spec "Degraded Mode When No Notification Service Is
/// Present").
#[test]
fn no_notification_service_owner_degrades_without_a_retry_loop() {
    skip_unless_lane_b!("no_notification_service_owner_degrades_without_a_retry_loop");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        // Deliberately no `spawn_fake_notifications()` call here — no
        // owner exists for `org.freedesktop.Notifications` on this bus.
        let log = Arc::new(Mutex::new(Vec::<u8>::new()));
        let notifier = FreedesktopNotifier::with_error_sink(SharedSink(log.clone()));

        let started = Instant::now();
        notifier.post(Category::Action, "Passwordless sudo enabled", "Passwordless sudo enabled until the requested time.");
        let elapsed_first = started.elapsed();
        assert!(elapsed_first < Duration::from_secs(3), "a missing service must fail fast, not hang: {elapsed_first:?}");

        let logged_once = String::from_utf8(log.lock().unwrap().clone()).expect("log must be valid UTF-8");
        assert!(!logged_once.is_empty(), "a degraded delivery must write something to the error sink");

        // A second, independent post (a later event) still attempts
        // delivery on its own — this is "no retry loop", not "gave up
        // forever after the first failure".
        let started_again = Instant::now();
        notifier.post(Category::Action, "Passwordless sudo disabled", "Passwordless sudo disabled");
        assert!(started_again.elapsed() < Duration::from_secs(3), "a second independent post must also fail fast");
        let logged_twice = String::from_utf8(log.lock().unwrap().clone()).expect("log must be valid UTF-8");
        assert!(logged_twice.len() > logged_once.len(), "the second independent post must log its own failure too");

        // Independently, the tray (icon/tooltip) keeps working — a fake
        // watcher and a live `KsniTray` are entirely unaffected by the
        // notifier's degraded state, because nothing couples them.
        let (_watcher_conn, registered) = spawn_fake_watcher().await;
        let view = ViewModel::from_state("jorge", &TrayState::Inactive, 0);
        let (_tray, _events) = KsniTray::spawn(view.clone()).await.expect("spawn must succeed with a fake watcher present");
        let destination = registered.lock().unwrap().last().cloned().expect("watcher must have observed a registration");
        assert_view_model_matches(&destination, &view).await;
    });
}

/// A `Write` sink shared with the test via an `Arc<Mutex<Vec<u8>>>`, so
/// the exact degraded-mode message can be inspected without capturing
/// the process's real stderr.
struct SharedSink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for SharedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Task 8.5: threat matrix "Untrusted text reaching a user surface" — a
/// `VisudoRejected` outcome renders only the constant text (never any
/// stderr-shaped string), and a hostile username renders literally,
/// length-capped, with no markup, once actually delivered over the wire.
#[test]
fn visudo_rejected_and_a_hostile_username_never_leak_untrusted_text_into_the_delivered_payload() {
    skip_unless_lane_b!("visudo_rejected_and_a_hostile_username_never_leak_untrusted_text_into_the_delivered_payload");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_daemon_conn, calls) = spawn_fake_notifications().await;
        let notifier = FreedesktopNotifier::new();

        // `action_notification` has no parameter through which stderr
        // could even be passed — this delivers the VisudoRejected outcome
        // and asserts the wire payload is exactly the constant text.
        let visudo_rejected = classify(Action::Disable, Some(14), true);
        let (summary, body) = action_notification(visudo_rejected, "");
        notifier.post(Category::Action, &summary, &body);

        let hostile_user = "<script>alert(1)</script>root";
        let (env_summary, env_body) = already_running_notification(hostile_user);
        notifier.post(Category::Environment, &env_summary, &env_body);

        let captured = calls.lock().unwrap().clone();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].body, OutcomeKind::VisudoRejected.text().1);
        assert!(!captured[0].body.to_lowercase().contains("syntax error"), "must never contain helper stderr text");

        for markup_char in ['<', '>', '(', ')', '/'] {
            assert!(!captured[1].body.contains(markup_char), "delivered body must never carry {markup_char:?}: {:?}", captured[1].body);
        }
        assert!(captured[1].body.len() < 200, "delivered body must stay bounded, never grow with a hostile username");
    });
}

/// Task 8.6: threat matrix "Privilege-request initiation over D-Bus"
/// (notification leg) — a hostile fake `org.freedesktop.Notifications`
/// that returns garbage ids never changes any `TrayState`. `NotifyPort`
/// is write-only from the tray's perspective (design threat matrix row
/// 4): the return value of `post` is `()`, so there is no code path
/// through which a malicious daemon's reply could ever reach a
/// `TrayState`. This is proven structurally by `NotifyPort`'s own
/// signature; this test additionally proves a hostile daemon cannot even
/// make delivery panic or hang.
#[test]
fn a_hostile_notification_daemon_returning_garbage_ids_cannot_affect_the_tray() {
    skip_unless_lane_b!("a_hostile_notification_daemon_returning_garbage_ids_cannot_affect_the_tray");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let service = HostileNotifications { calls: calls.clone() };
        let _daemon_conn = zbus::connection::Builder::session()
            .expect("session bus address must be resolvable under dbus-run-session")
            .name("org.freedesktop.Notifications")
            .expect("well-known name must be a valid bus name")
            .serve_at("/org/freedesktop/Notifications", service)
            .expect("notifications object path must be valid")
            .build()
            .await
            .expect("hostile fake daemon must be able to connect and claim its name");

        let notifier = FreedesktopNotifier::new();
        notifier.post(Category::Action, "Passwordless sudo enabled", "Passwordless sudo enabled until the requested time.");
        notifier.post(Category::Action, "Passwordless sudo disabled", "Passwordless sudo disabled");

        assert_eq!(calls.lock().unwrap().len(), 2, "both posts must have reached the hostile daemon without panicking");

        // `TrayState`/`ViewModel` rendering is entirely unaffected —
        // proven the same way as the degraded-mode test: a fake watcher
        // and a live tray still work regardless of what the notification
        // daemon returned.
        let (_watcher_conn, registered) = spawn_fake_watcher().await;
        let view = ViewModel::from_state("jorge", &TrayState::Active { user: None, expiry: None }, 0);
        let (_tray, _events) = KsniTray::spawn(view.clone()).await.expect("spawn must succeed with a fake watcher present");
        let destination = registered.lock().unwrap().last().cloned().expect("watcher must have observed a registration");
        assert_view_model_matches(&destination, &view).await;
    });
}

/// Always replies with the fixed nonsense id `u32::MAX` regardless of
/// `replaces_id` — a hostile or badly-implemented daemon.
struct HostileNotifications {
    calls: Arc<Mutex<Vec<NotifyCall>>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl HostileNotifications {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let _ = (app_icon, actions, hints, expire_timeout);
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push(NotifyCall { app_name, replaces_id, summary, body });
        u32::MAX
    }
}

// =====================================================================
// Phase 9 — Single instance (tasks.md 9.3-9.6; spec `tray-single-instance`)
// =====================================================================

/// A `TrayPort`/`NotifyPort` double that records every call it receives,
/// so Phase 9 tests can assert precisely how many times `AppInterface`
/// reacted, independent of Phase 8's own wire-level notification proof
/// and Phase 7's own SNI-registration proof.
#[derive(Default)]
struct RecordingTray {
    reassert_count: Mutex<u32>,
}

impl TrayPort for RecordingTray {
    fn render(&self, _view: &ViewModel) {}
    fn reassert(&self) {
        *self.reassert_count.lock().unwrap() += 1;
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

/// Task 9.3: no existing owner ⇒ the first instance becomes owner; a
/// second connection on the same bus ⇒ `NameTaken` ⇒ `AlreadyRunning`
/// (spec "First instance claims the name", "Second instance exits 0
/// without a second icon" — the "no second icon" half holds structurally
/// here because this test never calls `KsniTray::spawn` on the
/// `AlreadyRunning` branch, exactly as `main`'s real dispatch will not).
#[test]
fn first_instance_claims_the_name_and_a_second_gets_name_taken() {
    skip_unless_lane_b!("first_instance_claims_the_name_and_a_second_gets_name_taken");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let conn1 = zbus::Connection::session().await.expect("first connection must connect");
        let acquisition1 = instance::acquire(&conn1).await.expect("the first request_name must not error");
        assert_eq!(acquisition1, Acquisition::Owner);

        let conn2 = zbus::Connection::session().await.expect("second connection must connect");
        let acquisition2 = instance::acquire(&conn2).await.expect("NameTaken must classify as Ok(AlreadyRunning), not Err");
        assert_eq!(acquisition2, Acquisition::AlreadyRunning);

        release_service_name(&conn1).await;
    });
}

/// Explicitly releases [`SERVICE_NAME`] before a test ends, rather than
/// relying on the connection's `Drop`/socket-close to propagate to the
/// (separate-process) `dbus-daemon` in time for the next test in this
/// file — `BUS_LOCK` only serialises this process, not the daemon's own
/// asynchronous disconnect handling, and a name left dangling would
/// misclassify the very next test's `acquire` as `AlreadyRunning`.
async fn release_service_name(conn: &zbus::Connection) {
    let _ = conn.release_name(SERVICE_NAME).await;
}

/// Task 9.4 (success half) + Task 9.5: on `NameTaken`, the second
/// instance's `nudge` calls `Activate` on the first within the bounded
/// timeout and the first instance re-asserts its tray item and emits
/// exactly one status notification (spec "Second instance nudges the
/// first before exiting", "The first instance reacts to a received
/// nudge").
#[test]
fn second_instance_nudges_the_first_and_the_first_reacts_exactly_once() {
    skip_unless_lane_b!("second_instance_nudges_the_first_and_the_first_reacts_exactly_once");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let app_interface = AppInterface::new(tray.clone(), notify.clone(), "jorge".to_string());

        let conn1 = zbus::connection::Builder::session()
            .expect("session bus address must be resolvable under dbus-run-session")
            .serve_at(OBJECT_PATH, app_interface)
            .expect("application object path must be valid")
            .build()
            .await
            .expect("first instance connection must build");
        let acquisition1 = instance::acquire(&conn1).await.expect("the first request_name must not error");
        assert_eq!(acquisition1, Acquisition::Owner);

        let conn2 = zbus::Connection::session().await.expect("second connection must connect");
        let acquisition2 = instance::acquire(&conn2).await.expect("NameTaken must classify as Ok(AlreadyRunning), not Err");
        assert_eq!(acquisition2, Acquisition::AlreadyRunning);

        let started = Instant::now();
        instance::nudge(&conn2).await;
        assert!(started.elapsed() < Duration::from_secs(2), "a real, responsive owner must reply well inside the bound");

        assert_eq!(*tray.reassert_count.lock().unwrap(), 1, "the first instance must re-assert its tray item exactly once");
        {
            let posts = notify.posts.lock().unwrap();
            assert_eq!(posts.len(), 1, "the first instance must emit exactly one status notification");
            assert_eq!(posts[0].0, Category::Environment);
            assert_eq!(posts[0], (Category::Environment, "NoPass is already running".to_string(), "NoPass is already running for jorge.".to_string()));
        }

        release_service_name(&conn1).await;
    });
}

/// Task 9.4 (failure half): an owner whose `Activate` handler never
/// replies still leaves the second instance's `nudge` returning inside
/// its bounded timeout (spec "A failed or timed-out nudge still exits
/// 0").
#[test]
fn a_nudge_to_an_owner_that_never_replies_still_returns_within_the_bounded_timeout() {
    skip_unless_lane_b!("a_nudge_to_an_owner_that_never_replies_still_returns_within_the_bounded_timeout");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let owner = HangingApplication;
        let conn1 = zbus::connection::Builder::session()
            .expect("session bus address must be resolvable under dbus-run-session")
            .name(SERVICE_NAME)
            .expect("well-known name must be a valid bus name")
            .serve_at(OBJECT_PATH, owner)
            .expect("application object path must be valid")
            .build()
            .await
            .expect("hanging owner connection must build");

        let conn2 = zbus::Connection::session().await.expect("second connection must connect");
        let acquisition2 = instance::acquire(&conn2).await.expect("NameTaken must classify as Ok(AlreadyRunning), not Err");
        assert_eq!(acquisition2, Acquisition::AlreadyRunning);

        let started = Instant::now();
        instance::nudge(&conn2).await;
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_secs(3), "nudge must never hang waiting for a reply that never comes: {elapsed:?}");

        release_service_name(&conn1).await;
    });
}

/// An `org.freedesktop.Application` implementation whose `Activate`
/// handler never completes — the "fake owner that never replies to
/// Activate" the spec names.
struct HangingApplication;

#[zbus::interface(name = "org.freedesktop.Application")]
impl HangingApplication {
    async fn activate(&self, _platform_data: HashMap<String, OwnedValue>) {
        futures_lite::future::pending::<()>().await;
    }
}

/// Task 9.6: threat matrix "Privilege-request initiation over D-Bus" —
/// ten `Activate` calls in one second produce exactly one notification
/// (the rate limiter), and zero `pkexec` spawns — trivially true here
/// because `AppInterface` has no code path to `invoke`/`runner` at all.
#[test]
fn ten_rapid_activations_produce_exactly_one_notification_and_zero_pkexec_spawns() {
    skip_unless_lane_b!("ten_rapid_activations_produce_exactly_one_notification_and_zero_pkexec_spawns");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let tray = Arc::new(RecordingTray::default());
        let notify = Arc::new(RecordingNotify::default());
        let app_interface = AppInterface::new(tray.clone(), notify.clone(), "jorge".to_string());

        let conn1 = zbus::connection::Builder::session()
            .expect("session bus address must be resolvable under dbus-run-session")
            .name(SERVICE_NAME)
            .expect("well-known name must be a valid bus name")
            .serve_at(OBJECT_PATH, app_interface)
            .expect("application object path must be valid")
            .build()
            .await
            .expect("owner connection must build");

        let client = zbus::Connection::session().await.expect("client connection must connect");
        for _ in 0..10 {
            let platform_data: HashMap<String, OwnedValue> = HashMap::new();
            client
                .call_method(Some(SERVICE_NAME), OBJECT_PATH, Some(APPLICATION_INTERFACE), "Activate", &(platform_data,))
                .await
                .expect("Activate must be accepted by the object server even when rate-limited internally");
        }

        assert_eq!(*tray.reassert_count.lock().unwrap(), 1, "ten rapid activations must re-assert the tray exactly once");
        assert_eq!(notify.posts.lock().unwrap().len(), 1, "ten rapid activations must produce exactly one notification");

        release_service_name(&conn1).await;
    });
}

// =====================================================================
// Phase 10 — Startup preflight, degraded modes, main wiring (tasks.md
// 10.5-10.6; design.md §6.1, §8; spec `tray-presence` refusal rows)
// =====================================================================

fn nopass_binary() -> std::process::Command {
    std::process::Command::new(env!("CARGO_BIN_EXE_nopass"))
}

/// Task 10.6 (the cargo-test half) + task 10.5's "no session bus at all"
/// row: with `DBUS_SESSION_BUS_ADDRESS` unset and unresolvable, the real
/// binary must fail fast with a message on stderr and a non-zero exit —
/// exactly exit 3 (design.md §8).
#[test]
fn real_binary_with_no_session_bus_address_exits_3_with_a_stderr_message() {
    skip_unless_lane_b!("real_binary_with_no_session_bus_address_exits_3_with_a_stderr_message");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // An empty environment does NOT deny a session bus, and assuming it did
    // cost this suite a leaked process. With both DBUS_SESSION_BUS_ADDRESS and
    // XDG_RUNTIME_DIR absent, zbus derives `/run/user/<uid>/bus` from getuid()
    // and connects to the developer's REAL desktop session — not the nested bus
    // this lane runs under. The binary then legitimately started, claimed the
    // well-known name on that real bus, registered a tray item and ran forever,
    // which both hung `output()` and left a tray process on the user's machine.
    //
    // Denying a bus takes an address that resolves to nothing.
    let mut cmd = nopass_binary();
    cmd.env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/nopass-test-no-bus");
    cmd.env_remove("XDG_RUNTIME_DIR");
    cmd.env_remove("DBUS_STARTER_ADDRESS");
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // Bounded wait. If the premise ever breaks again the child is killed and
    // the test says so, rather than blocking forever and leaking a tray.
    let mut child = cmd.spawn().expect("spawn the real nopass binary");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("try_wait must not itself fail") {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "the binary did not exit within 10s with an unreachable bus address; \
                     it most likely reached a real session bus and started for real"
                );
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let output = child.wait_with_output().expect("collect the child's output");

    assert_eq!(status.code(), Some(3), "no reachable session bus must exit exactly 3: {output:?}");
    assert!(!output.stderr.is_empty(), "a session-bus connection failure must be reported on stderr");
}

/// Task 10.5: with a real session bus present but neither
/// `org.kde.StatusNotifierWatcher` nor `org.freedesktop.Notifications`
/// owned by anyone, the real binary must refuse with exit 4 — distinct
/// from exit 3 and from the SNI-host-only degraded case below, which
/// does not exit at all (spec `tray-presence` "Hard Refusal With No
/// User-Visible Channel").
#[test]
fn real_binary_with_neither_host_nor_notifications_exits_4() {
    skip_unless_lane_b!("real_binary_with_neither_host_nor_notifications_exits_4");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Deliberately no fake watcher, no fake notifications daemon —
    // `DBUS_SESSION_BUS_ADDRESS` resolves (dbus-run-session provides a
    // real bus), but no one owns either well-known name.
    let output = nopass_binary().output().expect("spawn the real nopass binary");

    assert_eq!(
        output.status.code(),
        Some(4),
        "a live session bus with no possible user-visible channel must exit exactly 4: {output:?}"
    );
}

/// Task 10.5: the SNI-host-only degraded case — `org.kde.
/// StatusNotifierWatcher` present, `org.freedesktop.Notifications`
/// absent — must NOT exit at all (design.md §8's `Run(NoNotifications)`
/// row), unlike the "neither present" case above which exits 4
/// immediately. The process is killed once this is observed; nothing
/// here asserts on its rendering (Lane B proves protocol, not looks —
/// design.md §9).
#[test]
fn real_binary_with_only_the_sni_host_present_keeps_running_degraded() {
    skip_unless_lane_b!("real_binary_with_only_the_sni_host_present_keeps_running_degraded");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_watcher_conn, _registered) = spawn_fake_watcher().await;

        let mut child = nopass_binary().spawn().expect("spawn the real nopass binary");

        // Give the process time to run its full startup sequence (well
        // above the <1s NFR budget) and confirm it is still alive rather
        // than having exited (degraded, not refused).
        async_io::Timer::after(Duration::from_secs(2)).await;
        match child.try_wait().expect("try_wait must not itself fail") {
            None => {}
            Some(status) => panic!("a present SNI host with no notification service must not exit; got {status:?}"),
        }

        let _ = child.kill();
        let _ = child.wait();
    });
}

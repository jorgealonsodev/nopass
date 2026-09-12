//! Lane B (`dbus-run-session`, gated by `NOPASS_DBUS_TESTS=1`) — tasks.md
//! Phase 7, tasks 7.4-7.6; design.md §9 Lane B; spec `tray-presence`
//! remaining scenarios.
//!
//! This proves the D-Bus protocol `tray.rs` speaks, never rendering
//! (design.md §9: "Lane B can prove the correct name was published, never
//! that it looked right" — that half is Lane C, `tests/manual/README.md`,
//! Phase 11).
//!
//! Every test here binds `org.kde.StatusNotifierWatcher` — a fixed,
//! spec-mandated well-known name `ksni` looks up unconditionally — on
//! whatever bus `DBUS_SESSION_BUS_ADDRESS` resolves to. `cargo test` runs
//! test functions from one binary concurrently by default, and two
//! concurrent claims of the same well-known name on the same bus race.
//! [`BUS_LOCK`] serialises every test in this file for exactly that
//! reason; it is not a workaround, it is what "one shared private bus,
//! several tests" requires.
//!
//! Skipped (this file, and why) when `NOPASS_DBUS_TESTS` is unset:
//! - `sni_properties_match_the_view_model_for_each_tray_state` (7.4)
//! - `menu_labels_and_sensitivity_match_the_view_model_and_quit_raises_its_event` (7.5)
//! - `ticks_keep_firing_on_schedule_while_a_live_tray_holds_the_bus` (7.6)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_lite::StreamExt;
use zbus::zvariant::OwnedValue;

use nopass::format::ToolTip as FormatToolTip;
use nopass::reconcile::TrayState;
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
}

#[zbus::proxy(interface = "com.canonical.dbusmenu", default_path = "/MenuBar")]
trait Menu {
    fn get_layout(&self, parent_id: i32, recursion_depth: i32, property_names: Vec<&str>) -> zbus::Result<(u32, RawLayout)>;

    fn event(&self, id: i32, event_id: &str, data: OwnedValue, timestamp: u32) -> zbus::Result<()>;
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

async fn read_menu_children(destination: &str) -> (Vec<(i32, HashMap<String, OwnedValue>)>, MenuProxy<'static>) {
    let client = zbus::Connection::session().await.expect("client must connect to the same bus");
    let proxy = MenuProxy::builder(&client)
        .destination(destination.to_string())
        .expect("destination must be a valid bus name")
        .build()
        .await
        .expect("menu proxy must build against a live DBusMenu object");
    let (_revision, (_root_id, _root_properties, root_children)) = proxy
        .get_layout(0, -1, vec!["label", "enabled", "visible"])
        .await
        .expect("GetLayout must succeed against a running tray");
    let children: Vec<(i32, HashMap<String, OwnedValue>)> = root_children
        .into_iter()
        .map(|child| {
            let value: zbus::zvariant::Value<'static> = child.into();
            let (id, properties, _grandchildren): RawLayout =
                value.downcast().expect("every menu child must decode as (i,a{sv},av)");
            (id, properties)
        })
        .collect();
    (children, proxy)
}

fn owned_str(value: &OwnedValue) -> String {
    <String as TryFrom<OwnedValue>>::try_from(value.try_clone().expect("cloneable OwnedValue")).expect("expected a string property value")
}

fn owned_bool(value: &OwnedValue) -> bool {
    <bool as TryFrom<OwnedValue>>::try_from(value.try_clone().expect("cloneable OwnedValue")).expect("expected a bool property value")
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

/// Task 7.5: the minimal menu — `Status` insensitive label, toggle item
/// labelled per `toggle_label` (insensitive with "Checking…" when
/// `None`), and `Quit` raising its `TrayEvent` (design.md §7.2's
/// minimal-menu table). Exiting the process with code 0 is `app::run`'s
/// job (Phase 10, out of scope here) — this proves the item exists, is
/// activatable, and raises exactly the request `app::run` is expected to
/// translate into that exit.
#[test]
fn menu_labels_and_sensitivity_match_the_view_model_and_quit_raises_its_event() {
    skip_unless_lane_b!("menu_labels_and_sensitivity_match_the_view_model_and_quit_raises_its_event");
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    futures_lite::future::block_on(async {
        let (_watcher_conn, registered) = spawn_fake_watcher().await;

        let unknown = ViewModel::from_state("jorge", &TrayState::Unknown, 0);
        let (tray, events) = KsniTray::spawn(unknown.clone()).await.expect("spawn must succeed with a fake watcher present");
        let destination = registered.lock().unwrap().last().cloned().expect("watcher must have observed a registration");

        // ---- Unknown: toggle is insensitive and reads "Checking…". ----
        let (children, _menu_proxy) = read_menu_children(&destination).await;
        assert_eq!(children.len(), 4, "root menu must have exactly Status, Separator, toggle, and Quit");

        let status_item = &children[0].1;
        assert_eq!(owned_str(&status_item["label"]), unknown.status_line);
        assert!(!menu_item_enabled(status_item), "the Status label must always be insensitive");

        let toggle_item = &children[2].1;
        assert_eq!(owned_str(&toggle_item["label"]), "Checking…");
        assert!(!menu_item_enabled(toggle_item), "the toggle must be insensitive while the state is Unknown");

        // ---- Active: toggle becomes sensitive and reads the real label. ----
        let active = ViewModel::from_state("jorge", &TrayState::Active { user: None, expiry: None }, 0);
        tray.render(&active);
        let (children, menu_proxy) = read_menu_children(&destination).await;
        let toggle_item = &children[2].1;
        assert_eq!(owned_str(&toggle_item["label"]), active.toggle.unwrap());
        assert!(menu_item_enabled(toggle_item), "the toggle must be sensitive once a label can be derived");

        // ---- Quit raises TrayEvent::Quit over the real DBusMenu wire protocol. ----
        let quit_id = children[3].0;
        menu_proxy
            .event(quit_id, "clicked", OwnedValue::from(0u8), 0)
            .await
            .expect("Event(Quit, clicked) must be accepted");

        let raised = futures_lite::future::or(
            async { Some(events.recv().await.expect("the events channel must still be open")) },
            async {
                async_io::Timer::after(Duration::from_secs(2)).await;
                None
            },
        )
        .await;
        assert_eq!(raised, Some(TrayEvent::Quit), "clicking Quit must raise exactly TrayEvent::Quit");
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

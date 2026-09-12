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
//! - `menu_labels_and_sensitivity_match_the_view_model_and_quit_raises_its_event` (7.5)
//! - `ticks_keep_firing_on_schedule_while_a_live_tray_holds_the_bus` (7.6)
//! - every `notifications_*`/`single_instance_*` test below (Phase 8/9)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_lite::StreamExt;
use zbus::zvariant::OwnedValue;

use nopass::format::ToolTip as FormatToolTip;
use nopass::instance::{self, AppInterface, Acquisition, APPLICATION_INTERFACE, OBJECT_PATH, SERVICE_NAME};
use nopass::notifications::{action_notification, already_running_notification, expiry_notification, Category, FreedesktopNotifier, NotifyPort};
use nopass::outcome::{classify, Action, OutcomeKind};
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

        let granted = classify(Action::Enable { until: 1_700_003_600 }, Some(0), true);
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

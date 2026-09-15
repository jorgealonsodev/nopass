#![forbid(unsafe_code)]

//! `nopass`: the tray application, a thin entry point over the
//! `nopass` library (`src/lib.rs`), which owns every module — see
//! `lib.rs` for why the library exists.
//!
//! `boot()` is the real startup sequence design.md §6.1 describes: it
//! connects the session bus, decides single-instance ownership, runs the
//! preflight, spawns the tray with `Unknown` rendered before any probe
//! (the < 1 s NFR budget), wires every adapter into one shared
//! [`nopass::event::Event`] channel, and finally drives
//! [`nopass::app::run`] to completion. Every non-zero exit code
//! (design.md §8) is decided here, before an `App` value is ever built —
//! `app::run` itself always returns `0`.

use std::path::PathBuf;
use std::sync::Arc;

use futures_lite::StreamExt;
use zbus::Connection;

use nopass::app::{self, App};
use nopass::instance::{self, Acquisition, AppInterface, OBJECT_PATH};
use nopass::invoke::Locale;
use nopass::notifications::{FreedesktopNotifier, NotifyPort};
use nopass::preflight::{self, EnumerateOutcome, PolicyFileCheck, Preflight, ServicePresence, StartDecision};
use nopass::reconcile::TrayState;
use nopass::runner::SystemRunner;
use nopass::tray::{KsniTray, TrayPort, ViewModel};
use nopass::watch::Watch;

/// `org.kde.StatusNotifierWatcher` — the tray-host protocol; used both
/// for the KDE and the GNOME/AppIndicator-extension name (design.md §6.1
/// step 5).
const SNI_WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const NOTIFICATIONS_NAME: &str = "org.freedesktop.Notifications";
const POLKIT_AUTHORITY_NAME: &str = "org.freedesktop.PolicyKit1";
const POLKIT_ACTION_ID: &str = "com.enfoquestic.nopass.manage";
const POLKIT_POLICY_FILE: &str = "/usr/share/polkit-1/actions/com.enfoquestic.nopass.policy";

fn main() {
    std::process::exit(boot())
}

/// The real startup sequence (design.md §6.1 steps 1-11). Synchronous by
/// signature — `main()` wants a plain exit code — but everything inside
/// runs on one `futures_lite::future::block_on`, which is also the
/// process thread that ultimately becomes `app::run`'s own reactor task
/// (see `app.rs`'s module doc for why that is safe).
fn boot() -> i32 {
    futures_lite::future::block_on(boot_async())
}

async fn boot_async() -> i32 {
    let uid = nix::unistd::getuid().as_raw();
    let user = resolve_username(uid);
    let layout = nopass_core::paths::Layout::system();
    let state_path = layout.state_path(uid);
    let run_dir = state_path.parent().expect("state_path always has a parent directory").to_path_buf();

    // Step 2: connect the session bus.
    let session_conn = match Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nopass: could not connect to the session bus: {e}");
            return 3;
        }
    };

    // Step 3/4: single-instance ownership (design.md §6.4, D3). A second
    // instance nudges the first and always exits 0, regardless of what
    // the nudge did (RF-10).
    let acquisition = match instance::acquire(&session_conn).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("nopass: could not claim {}: {e}", instance::SERVICE_NAME);
            return 5;
        }
    };
    if acquisition == Acquisition::AlreadyRunning {
        instance::nudge(&session_conn).await;
        return 0;
    }

    // Step 5: preflight — session bus is already known Present here; a
    // connection failure above already returned before this point.
    let sni_host = name_has_owner(&session_conn, SNI_WATCHER_NAME).await;
    let notifications_present = name_has_owner(&session_conn, NOTIFICATIONS_NAME).await;
    let polkit = probe_polkit_readiness().await;
    let preflight = Preflight { session_bus: ServicePresence::Present, sni_host, notifications: notifications_present, polkit };

    // Step 6: decide.
    let mode = match preflight::decide(&preflight) {
        StartDecision::Run(mode) => mode,
        StartDecision::Refuse(reason) => {
            eprintln!("nopass: refusing to start: {reason:?}");
            return 4;
        }
    };

    // Step 7: render Unknown and register the SNI item BEFORE any probe
    // — the < 1 s startup budget (design.md §6.1).
    let notify: Arc<dyn NotifyPort + Send + Sync> = Arc::new(FreedesktopNotifier::new());
    let initial = ViewModel::from_state(&user, &TrayState::Unknown, app::now());
    let (ksni_tray, tray_events) = match KsniTray::spawn(initial).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("nopass: could not register the tray item: {e}");
            return 1;
        }
    };
    let tray: Arc<dyn TrayPort + Send + Sync> = Arc::new(ksni_tray);

    // `AppInterface` is served once tray/notify exist — a benign,
    // deliberate reordering relative to design.md's step 3 (see
    // `app.rs`'s module doc and this phase's report for the rationale):
    // nothing can address us via the well-known name before step 3/4
    // above already succeeded, so no real second instance can observe
    // the gap.
    let app_interface = AppInterface::new(Arc::clone(&tray), Arc::clone(&notify), user.clone());
    if let Err(e) = session_conn.object_server().at(OBJECT_PATH, app_interface).await {
        eprintln!("nopass: could not serve {OBJECT_PATH}: {e} (second-instance nudges will be a no-op)");
    }

    let (events_tx, events_rx) = async_channel::unbounded();

    // Step 8: subscribe NameOwnerChanged for the SNI watcher (late host,
    // design.md §6.1 step 8).
    spawn_host_watch(&session_conn, events_tx.clone()).await;

    // Step 9: start the inotify watch on the run directory (missing ⇒
    // reconciliation-only fallback, retried each tick — design D8).
    match Watch::start(&run_dir, uid, events_tx.clone()) {
        Ok(watch) => std::mem::forget(watch), // kept alive for the process's lifetime
        Err(e) => eprintln!("nopass: could not watch {}: {e:?} (falling back to the 60s tick alone)", run_dir.display()),
    }

    // Step 10: the sole periodic timer (design.md §3.4, event.rs's own
    // structural test).
    spawn_tick_forwarder(events_tx.clone());

    // Bridge the ksni menu/activation channel into the shared Event
    // channel (design.md §6 "adapters own no state and only send
    // Events").
    spawn_tray_event_forwarder(tray_events, events_tx.clone());

    let pkexec = resolve_first_absolute(&["/usr/bin/pkexec", "/bin/pkexec"]);
    let sudo = resolve_first_absolute(&["/usr/bin/sudo", "/bin/sudo"]);

    let app = App::new(
        user,
        state_path,
        mode,
        tray,
        notify,
        Arc::new(SystemRunner),
        pkexec,
        sudo,
        Locale::from_env(),
        events_tx,
        events_rx,
    );

    // Step 11 (reading the state file and spawning the first probe) is
    // `app::run`'s own `Trigger::Startup` reconciliation.
    app::run(app).await
}

/// `getpwuid` via `nix::unistd::User`, falling back to the bare uid as a
/// string when no passwd entry exists — a degraded but still-honest
/// display value, never a fabricated name.
fn resolve_username(uid: u32) -> String {
    match nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)) {
        Ok(Some(user)) => user.name,
        _ => uid.to_string(),
    }
}

/// The ordered-absolute-candidate discipline (design.md §4.3): first
/// existing candidate wins; falling back to the first candidate when
/// none exist lets `SystemRunner` produce a natural `RunnerError::Spawn`
/// (⇒ `OutcomeKind::SpawnFailed`) the first time it is actually needed,
/// rather than refusing to start over a binary the user may install
/// later.
fn resolve_first_absolute(candidates: &[&str]) -> PathBuf {
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if std::fs::metadata(&path).map(|m| m.is_file()).unwrap_or(false) {
            return path;
        }
    }
    PathBuf::from(candidates[0])
}

async fn name_has_owner(conn: &Connection, name: &str) -> ServicePresence {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return ServicePresence::Absent;
    };
    match dbus.name_has_owner(name.try_into().expect("well-known name literal is always valid")).await {
        Ok(true) => ServicePresence::Present,
        _ => ServicePresence::Absent,
    }
}

/// Subscribes to `NameOwnerChanged` for [`SNI_WATCHER_NAME`] and forwards
/// `Event::HostAppeared`/`HostVanished` into `tx` for as long as the
/// process runs (design.md §6.1 step 8). A subscription failure is
/// logged and otherwise harmless — the tray already rendered `Unknown`
/// and registered per step 7; a late host will not be detected without
/// this, but nothing else in the app depends on it.
async fn spawn_host_watch(conn: &Connection, tx: async_channel::Sender<nopass::event::Event>) {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        eprintln!("nopass: could not build a DBus proxy to watch for a late tray host");
        return;
    };
    let Ok(mut changes) = dbus.receive_name_owner_changed().await else {
        eprintln!("nopass: could not subscribe to NameOwnerChanged");
        return;
    };
    std::thread::spawn(move || {
        futures_lite::future::block_on(async {
            while let Some(signal) = changes.next().await {
                let Ok(args) = signal.args() else { continue };
                if args.name.as_str() != SNI_WATCHER_NAME {
                    continue;
                }
                let event = if args.new_owner.as_ref().is_some() {
                    nopass::event::Event::HostAppeared
                } else {
                    nopass::event::Event::HostVanished
                };
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
    });
}

/// Forwards `nopass::event::tick()` into `tx` from a dedicated thread —
/// `event.rs`'s own structural test still holds, since this calls the
/// crate's one `Timer::interval` call site rather than writing a second
/// one.
fn spawn_tick_forwarder(tx: async_channel::Sender<nopass::event::Event>) {
    std::thread::spawn(move || {
        futures_lite::future::block_on(async {
            let mut ticks = nopass::event::tick();
            while let Some(event) = ticks.next().await {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
    });
}

/// Bridges `ksni`'s own `TrayEvent` channel into the shared `Event`
/// channel (design.md §6 "adapters own no state and only send Events").
fn spawn_tray_event_forwarder(
    events: async_channel::Receiver<nopass::tray::TrayEvent>,
    tx: async_channel::Sender<nopass::event::Event>,
) {
    std::thread::spawn(move || {
        futures_lite::future::block_on(async {
            while let Ok(event) = events.recv().await {
                if tx.send(event.into()).await.is_err() {
                    break;
                }
            }
        });
    });
}

/// The polkit readiness ladder (design.md §0 G3), wired against a real
/// system bus connection. Never gates startup — `preflight::decide`
/// never reads `polkit` at all — and its result is currently carried by
/// `Preflight` for completeness only; see this phase's report for the
/// open gap (nothing yet consumes `PolkitReadiness::ActionMissing` to
/// disable the toggle, since no task in this phase specifies where that
/// wiring belongs).
async fn probe_polkit_readiness() -> nopass::preflight::PolkitReadiness {
    let Ok(system_conn) = Connection::system().await else {
        return nopass::preflight::PolkitReadiness::Indeterminate("system_bus_unreachable");
    };

    let Ok(dbus) = zbus::fdo::DBusProxy::new(&system_conn).await else {
        return nopass::preflight::PolkitReadiness::Indeterminate("dbus_proxy_unavailable");
    };
    let has_owner = dbus
        .name_has_owner(POLKIT_AUTHORITY_NAME.try_into().expect("well-known name literal is always valid"))
        .await
        .unwrap_or(false);
    if !has_owner {
        return preflight::polkit_ladder(false, EnumerateOutcome::ErrorOrTimeout, PolicyFileCheck::Unknown);
    }

    let enumerate = futures_lite::future::or(
        async {
            let proxy = PolicyKitAuthorityProxy::new(&system_conn).await.ok()?;
            proxy.enumerate_actions("").await.ok()
        },
        async {
            async_io::Timer::after(std::time::Duration::from_secs(2)).await;
            None
        },
    )
    .await;

    let enumerate_outcome = match enumerate {
        Some(actions) => {
            if actions.iter().any(|a| a.0 == POLKIT_ACTION_ID) {
                EnumerateOutcome::ActionFound
            } else {
                EnumerateOutcome::ActionAbsent
            }
        }
        None => EnumerateOutcome::ErrorOrTimeout,
    };

    let policy_file = match std::fs::metadata(POLKIT_POLICY_FILE) {
        Ok(m) if m.is_file() => PolicyFileCheck::Present,
        Ok(_) => PolicyFileCheck::Absent,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PolicyFileCheck::Absent,
        Err(_) => PolicyFileCheck::Unknown,
    };

    preflight::polkit_ladder(true, enumerate_outcome, policy_file)
}

/// One polkit action description, as `EnumerateActions` returns it —
/// `(action_id, description, message, vendor_name, vendor_url,
/// implicit_any, implicit_inactive, implicit_active, annotations)`,
/// following polkit's own D-Bus introspection (`sssssuuua{ss}`).
type PolkitActionDescription = (String, String, String, String, String, u32, u32, u32, std::collections::HashMap<String, String>);

/// Client proxy for `org.freedesktop.PolicyKit1.Authority` — used only
/// for the best-effort `EnumerateActions` step of the polkit ladder
/// (design.md §0 G3, step 2).
#[zbus::proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait PolicyKitAuthority {
    #[zbus(name = "EnumerateActions")]
    fn enumerate_actions(&self, locale: &str) -> zbus::Result<Vec<PolkitActionDescription>>;
}

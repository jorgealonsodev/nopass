//! Single-instance D-Bus name ownership, the `NameTaken` exit-0 path, and
//! the activation nudge (design.md §2 `instance`, §6.4, D3; spec
//! `tray-single-instance`; tasks.md Phase 9).
//!
//! `acquire` decides which of exactly two outcomes a `request_name` call
//! produced: this process is now the owner, or another instance already
//! is. Every other `request_name` failure is a real startup fault, never
//! folded into the second-instance path (spec "Non-NameTaken
//! request_name Errors Are a Real Fault"). `nudge` is the second
//! instance's side of RF-10: call `Activate` on the existing owner,
//! bounded, and exit 0 regardless of what happened. `AppInterface` is
//! the first instance's side: it serves `org.freedesktop.Application`
//! and reacts to a received `Activate` by re-asserting its tray item and
//! posting exactly one notification per rate-limit window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::zvariant::OwnedValue;
use zbus::Connection;

use crate::notifications::{already_running_notification, Category, NotifyPort};
use crate::tray::TrayPort;

/// The well-known name every instance races to own (spec
/// `tray-single-instance` "D-Bus Name Claim and NameTaken Exit").
pub const SERVICE_NAME: &str = "com.enfoquestic.nopass";

/// The object path [`AppInterface`] serves `org.freedesktop.Application`
/// at (design.md §2 `instance`).
pub const OBJECT_PATH: &str = "/com/enfoquestic/nopass";

/// The interface name `AppInterface` implements — the standard GNOME/
/// freedesktop application-activation interface, not a private protocol
/// (design.md §6.4).
pub const APPLICATION_INTERFACE: &str = "org.freedesktop.Application";

/// How long the second instance waits for its `Activate` call before
/// giving up and exiting anyway (design.md §6.4: "ignore success,
/// failure, and timeout alike").
const NUDGE_TIMEOUT: Duration = Duration::from_secs(2);

/// The nudge rate limit — at most one accepted reaction per window
/// (design threat matrix row 4: "nudges are rate-limited to one per
/// 5 s").
const NUDGE_RATE_LIMIT: Duration = Duration::from_secs(5);

/// Whether this process became the name's owner or found it already
/// taken (spec "First instance claims the name", "Second instance exits
/// 0 without a second icon").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquisition {
    Owner,
    AlreadyRunning,
}

/// Classifies a raw `request_name` result into an [`Acquisition`],
/// isolated from [`zbus::Connection`] so the non-`NameTaken` branch is
/// testable without a bus (tasks.md 9.2; spec "Non-NameTaken
/// request_name Errors Are a Real Fault").
fn classify_request_name(result: Result<(), zbus::Error>) -> Result<Acquisition, zbus::Error> {
    match result {
        Ok(()) => Ok(Acquisition::Owner),
        Err(zbus::Error::NameTaken) => Ok(Acquisition::AlreadyRunning),
        Err(other) => Err(other),
    }
}

/// Requests [`SERVICE_NAME`] on `conn`, with the [`DoNotQueue`][dnq] flag
/// and deliberately *without* `ReplaceExisting`. `Connection::
/// request_name`'s own default flags (`BitFlags::default()`, i.e. none)
/// would instead queue a second caller behind the current owner and
/// report that queueing as `Ok(())` — silently misclassifying a genuine
/// second instance as the owner. `DoNotQueue` is what turns "already
/// owned" into the immediate `Err(NameTaken)` [`classify_request_name`]
/// depends on, and omitting `ReplaceExisting` is what stops a second
/// instance from stealing the name out from under the first.
///
/// An `Err` here is always a real startup fault (spec "Non-NameTaken
/// request_name Errors Are a Real Fault") — the caller must print to
/// stderr and exit non-zero, never the RF-10 `NameTaken` exit-0 path.
///
/// [dnq]: zbus::fdo::RequestNameFlags::DoNotQueue
pub async fn acquire(conn: &Connection) -> Result<Acquisition, zbus::Error> {
    use zbus::fdo::RequestNameFlags;
    let result = conn.request_name_with_flags(SERVICE_NAME, RequestNameFlags::DoNotQueue.into()).await.map(|_| ());
    classify_request_name(result)
}

/// Calls `org.freedesktop.Application.Activate` on the existing owner of
/// [`SERVICE_NAME`], bounded by [`NUDGE_TIMEOUT`]. Success, failure, and
/// timeout are all ignored identically — infallible by design, because
/// RF-10 is satisfied by exiting 0 regardless of what the nudge did
/// (design.md §6.4).
pub async fn nudge(conn: &Connection) {
    let activate = async {
        let platform_data: HashMap<String, zbus::zvariant::Value> = HashMap::new();
        conn.call_method(Some(SERVICE_NAME), OBJECT_PATH, Some(APPLICATION_INTERFACE), "Activate", &(platform_data,))
            .await
            .ok()
            .map(|_| ())
    };
    let timeout = async {
        async_io::Timer::after(NUDGE_TIMEOUT).await;
        None
    };
    let _ = futures_lite::future::or(activate, timeout).await;
}

/// Serves `org.freedesktop.Application` for the first instance
/// (design.md §2 `instance`, §6.4). `activate` re-asserts the tray item
/// and posts exactly one `Category::Environment` notification per
/// [`NUDGE_RATE_LIMIT`] window — a flood of `Activate` calls (design
/// threat matrix row 4: "Privilege-request initiation over D-Bus")
/// produces at most one reaction and never touches `pkexec`, because
/// this interface has no code path to it at all.
pub struct AppInterface {
    tray: Arc<dyn TrayPort + Send + Sync>,
    notify: Arc<dyn NotifyPort + Send + Sync>,
    user: String,
    last_accepted: Mutex<Option<Instant>>,
}

impl AppInterface {
    pub fn new(tray: Arc<dyn TrayPort + Send + Sync>, notify: Arc<dyn NotifyPort + Send + Sync>, user: String) -> AppInterface {
        AppInterface { tray, notify, user, last_accepted: Mutex::new(None) }
    }

    /// Whether this call falls inside the rate-limit window — `true`
    /// (and records `now`) at most once per [`NUDGE_RATE_LIMIT`].
    fn accept(&self, now: Instant) -> bool {
        let mut last = self.last_accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let accepted = match *last {
            Some(prev) => now.duration_since(prev) >= NUDGE_RATE_LIMIT,
            None => true,
        };
        if accepted {
            *last = Some(now);
        }
        accepted
    }
}

#[zbus::interface(name = "org.freedesktop.Application")]
impl AppInterface {
    /// design.md §6.4: re-assert SNI registration and emit one status
    /// notification, rate-limited (design threat matrix row 4). The
    /// `platform_data` argument is part of the standard interface's
    /// signature (`a{sv}`) and carries nothing this handler needs.
    async fn activate(&self, _platform_data: HashMap<String, OwnedValue>) {
        if !self.accept(Instant::now()) {
            return;
        }
        self.tray.reassert();
        let (summary, body) = already_running_notification(&self.user);
        self.notify.post(Category::Environment, &summary, &body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- classify_request_name (Lane A, task 9.2) ----

    #[test]
    fn ok_becomes_owner() {
        assert_eq!(classify_request_name(Ok(())), Ok(Acquisition::Owner));
    }

    #[test]
    fn name_taken_becomes_already_running() {
        assert_eq!(classify_request_name(Err(zbus::Error::NameTaken)), Ok(Acquisition::AlreadyRunning));
    }

    #[test]
    fn any_other_error_is_propagated_never_treated_as_a_second_instance() {
        let result = classify_request_name(Err(zbus::Error::Unsupported));
        match result {
            Err(zbus::Error::Unsupported) => {}
            other => panic!("a non-NameTaken error must be propagated as a real fault, got {other:?}"),
        }
    }

    #[test]
    fn a_failure_error_is_also_propagated_never_treated_as_a_second_instance() {
        let result = classify_request_name(Err(zbus::Error::Failure("bus connection lost".to_string())));
        assert!(result.is_err(), "a bus connection failure must never be mistaken for NameTaken");
    }

    // ---- AppInterface rate limiting (Lane A: pure Instant arithmetic) ----

    #[test]
    fn the_first_activation_in_a_window_is_always_accepted() {
        let iface = AppInterface::new(Arc::new(NoopTray), Arc::new(NoopNotify), "jorge".to_string());
        assert!(iface.accept(Instant::now()));
    }

    #[test]
    fn a_second_activation_inside_the_window_is_rejected() {
        let iface = AppInterface::new(Arc::new(NoopTray), Arc::new(NoopNotify), "jorge".to_string());
        let now = Instant::now();
        assert!(iface.accept(now));
        assert!(!iface.accept(now + Duration::from_millis(1)));
        assert!(!iface.accept(now + NUDGE_RATE_LIMIT - Duration::from_millis(1)));
    }

    #[test]
    fn an_activation_at_or_after_the_window_boundary_is_accepted_again() {
        let iface = AppInterface::new(Arc::new(NoopTray), Arc::new(NoopNotify), "jorge".to_string());
        let now = Instant::now();
        assert!(iface.accept(now));
        assert!(iface.accept(now + NUDGE_RATE_LIMIT));
    }

    struct NoopTray;
    impl TrayPort for NoopTray {
        fn render(&self, _view: &crate::tray::ViewModel) {}
        fn reassert(&self) {}
    }

    struct NoopNotify;
    impl NotifyPort for NoopNotify {
        fn post(&self, _category: Category, _summary: &str, _body: &str) {}
    }
}

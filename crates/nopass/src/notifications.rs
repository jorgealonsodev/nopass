//! Desktop notifications — success, failure, and expiry (design.md §2
//! `notifications`, §7.3; spec `tray-notifications`; tasks.md Phase 8).
//!
//! [`FreedesktopNotifier`] is the only `notify-rust`-aware module in this
//! crate: every other module composes plain `(summary, body)` text and
//! hands it to [`NotifyPort::post`], never touching `notify-rust` or
//! `org.freedesktop.Notifications` directly.
//!
//! One [`notify_rust::NotificationHandle`] is retained per [`Category`]
//! and updated in place rather than shown again, so a flurry of events
//! cannot stack several toasts for the same channel (design.md §7.3).
//! Urgency is always [`notify_rust::Urgency::Normal`] — never `Critical`,
//! which on several daemons produces an undismissable banner.
//!
//! Degraded mode (spec "Degraded Mode When No Notification Service Is
//! Present"): if `org.freedesktop.Notifications` has no owner, showing
//! or updating a notification fails; [`FreedesktopNotifier::post`] logs
//! that failure once to its error sink and returns — it never retries in
//! a loop, and the caller (icon/tooltip rendering) is entirely
//! unaffected because nothing here can block or panic the reactor.

use std::io::Write;
use std::sync::Mutex;

use notify_rust::{Notification, NotificationHandle, Urgency};

use crate::outcome::OutcomeKind;

/// The `NoPass` application name every notification is shown under.
const APP_NAME: &str = "NoPass";

/// What `app::run` (Phase 10) needs to raise a notification (design.md
/// §2 `notifications`). Deliberately infallible: a delivery failure is
/// this trait's own implementation's problem to log, never the caller's
/// to handle.
pub trait NotifyPort {
    fn post(&self, category: Category, summary: &str, body: &str);
}

/// One independently-retained notification channel (design.md §7.3). A
/// flurry of `Action` events updates the same notification instead of
/// stacking a new one each time; `Expiry` and `Environment` get their
/// own independent slots for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Action,
    Expiry,
    Environment,
}

/// The fixed, closed set of [`Category`] values — used only to size and
/// index the retained-handle table, never matched with a wildcard so a
/// newly added category cannot silently share another one's slot.
const CATEGORY_COUNT: usize = 3;

impl Category {
    fn slot(self) -> usize {
        match self {
            Category::Action => 0,
            Category::Expiry => 1,
            Category::Environment => 2,
        }
    }
}

/// Composes the notification body for a completed action's [`OutcomeKind`]
/// (spec "Notification Body Is Distinct Per Outcome"). The only outcome
/// whose constant text (outcome.rs's own `text()`) does not yet include a
/// formatted expiry is [`OutcomeKind::Granted`] — its countdown is
/// appended here, exactly as `outcome.rs`'s own doc comment expects of
/// its future caller. No other input ever reaches this function: never
/// raw subprocess stderr (design threat matrix row 5), so a `VisudoRejected`
/// outcome renders only its constant text regardless of what the helper's
/// stderr said.
pub fn action_notification(kind: OutcomeKind, countdown: &str) -> (String, String) {
    let (summary, body) = kind.text();
    match kind {
        OutcomeKind::Granted { .. } => (summary, format!("{body} Expires: {countdown}.")),
        _ => (summary, body),
    }
}

/// Composes the expiry notification for an externally-triggered
/// transition to inactive (spec "Detected expiry produces its own
/// notification"; design.md §6.3).
pub fn expiry_notification() -> (String, String) {
    ("Passwordless sudo expired".to_string(), "Passwordless sudo has expired.".to_string())
}

/// Composes the `Environment` notification `AppInterface::activate`
/// (Phase 9, `instance.rs`) emits on a received nudge (design.md §6.4):
/// the current status this second launch is stepping on — that there is
/// already a running instance owned by `user`. `user` is re-sanitized
/// with [`nopass_core::template::sanitize_username`] here, as
/// defense-in-depth (design threat matrix row 5: "the username ... is
/// length-capped before display") — this composer never has to trust
/// that its caller pre-sanitized, even though the state pipeline
/// (M1's own write-time sanitization) already did.
pub fn already_running_notification(user: &str) -> (String, String) {
    let user = nopass_core::template::sanitize_username(user);
    ("NoPass is already running".to_string(), format!("NoPass is already running for {user}."))
}

/// One retained handle slot per [`Category`], `None` until the first
/// successful `show()` for that category.
struct Slots {
    handles: [Option<NotificationHandle>; CATEGORY_COUNT],
}

impl Slots {
    fn new() -> Slots {
        Slots { handles: [None, None, None] }
    }
}

/// The concrete [`NotifyPort`], backed by `notify-rust`'s `zbus`
/// transport. Generic over its error sink so tests can assert on the
/// exact degraded-mode message without capturing the process's real
/// stderr; production code always uses [`FreedesktopNotifier::new`],
/// which defaults to [`std::io::stderr`].
pub struct FreedesktopNotifier<W: Write + Send = std::io::Stderr> {
    slots: Mutex<Slots>,
    error_sink: Mutex<W>,
}

impl FreedesktopNotifier<std::io::Stderr> {
    /// The production constructor — errors are written to the process's
    /// real stderr (spec "a message is written to stderr").
    pub fn new() -> FreedesktopNotifier<std::io::Stderr> {
        FreedesktopNotifier { slots: Mutex::new(Slots::new()), error_sink: Mutex::new(std::io::stderr()) }
    }
}

impl Default for FreedesktopNotifier<std::io::Stderr> {
    fn default() -> Self {
        FreedesktopNotifier::new()
    }
}

impl<W: Write + Send> FreedesktopNotifier<W> {
    /// Test/diagnostic constructor: routes degraded-mode error text to
    /// `sink` instead of the process's real stderr.
    pub fn with_error_sink(sink: W) -> FreedesktopNotifier<W> {
        FreedesktopNotifier { slots: Mutex::new(Slots::new()), error_sink: Mutex::new(sink) }
    }

    fn log_delivery_failure(&self, summary: &str, err: &notify_rust::error::Error) {
        let mut sink = self.error_sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = writeln!(sink, "nopass: could not deliver notification ({summary}): {err}");
    }
}

impl<W: Write + Send> NotifyPort for FreedesktopNotifier<W> {
    /// Exactly one delivery attempt per call — an existing handle is
    /// updated in place; otherwise a new notification is shown and its
    /// handle retained. Either path attempts `notify-rust` exactly once
    /// and, on failure, logs and returns: there is no retry loop, so a
    /// missing `org.freedesktop.Notifications` owner degrades this call
    /// to "log and continue" rather than blocking or repeating (spec
    /// "Degraded Mode When No Notification Service Is Present").
    fn post(&self, category: Category, summary: &str, body: &str) {
        let mut slots = self.slots.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let slot = category.slot();

        if let Some(handle) = slots.handles[slot].as_mut() {
            handle.summary(summary);
            handle.body(body);
            if let Err(err) = handle.update() {
                self.log_delivery_failure(summary, &err);
                // The service may have vanished since the last successful
                // delivery; drop the stale handle so a future post()
                // re-attempts a fresh `show()` rather than repeatedly
                // failing to update a handle nothing owns any more. This
                // is still exactly one attempt per call, never a loop.
                slots.handles[slot] = None;
            }
            return;
        }

        match Notification::new().appname(APP_NAME).summary(summary).body(body).urgency(Urgency::Normal).show() {
            Ok(handle) => slots.handles[slot] = Some(handle),
            Err(err) => self.log_delivery_failure(summary, &err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::{classify, Action};

    // ---- action_notification / expiry_notification / already_running_notification (Lane A) ----

    #[test]
    fn granted_appends_the_countdown_to_the_constant_body() {
        let kind = classify(Action::Enable { until: 1_700_000_000 }, Some(0), true);
        let (summary, body) = action_notification(kind, "42 min");
        assert_eq!(summary, "Passwordless sudo enabled");
        assert!(body.contains("42 min"), "body must carry the countdown: {body:?}");
        assert!(body.starts_with(kind.text().1.as_str()), "body must start with the constant text");
    }

    #[test]
    fn revoked_ignores_the_countdown_argument_entirely() {
        let kind = classify(Action::Disable, Some(0), true);
        let (summary, body) = action_notification(kind, "should never appear");
        assert_eq!((summary, body.clone()), kind.text());
        assert!(!body.contains("should never appear"));
    }

    #[test]
    fn visudo_rejected_never_carries_attacker_shaped_stderr() {
        // outcome.rs's own `classify`/`text()` never accept a stderr
        // string at all — this is the structural half of threat matrix
        // row 5: there is no code path through which raw stderr could
        // reach a notification body.
        let kind = classify(Action::Disable, Some(14), true);
        let (_, body) = action_notification(kind, "1 min");
        assert_eq!(body, OutcomeKind::VisudoRejected.text().1);
        assert!(!body.to_lowercase().contains("syntax error"), "must never echo helper stderr text");
    }

    #[test]
    fn already_running_notification_renders_a_markup_username_literally_and_length_capped() {
        let hostile = "<script>alert(1)</script>evil-name-that-is-far-too-long-to-display-in-full-thirty-two";
        let (_, body) = already_running_notification(hostile);
        for markup_char in ['<', '>', '(', ')', '/', ';', '&', '\0'] {
            assert!(!body.contains(markup_char), "markup/control character {markup_char:?} must never survive: {body:?}");
        }
        let rendered_user = body.trim_start_matches("NoPass is already running for ").trim_end_matches('.');
        assert!(rendered_user.chars().count() <= 32, "the rendered username must stay length-capped: {rendered_user:?}");
    }

    #[test]
    fn already_running_notification_composes_only_the_constant_prefix_and_the_sanitized_username() {
        let (summary, body) = already_running_notification("jorge");
        assert_eq!(summary, "NoPass is already running");
        assert_eq!(body, "NoPass is already running for jorge.");
    }

    // ---- category slot table ----

    #[test]
    fn every_category_maps_to_a_distinct_slot() {
        let slots: Vec<usize> = [Category::Action, Category::Expiry, Category::Environment].iter().map(|c| c.slot()).collect();
        let mut sorted = slots.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), slots.len(), "every Category must occupy its own slot");
    }
}

//! Startup preflight: whether the tray can render at all, and in which
//! degraded mode (design.md §0 G3, §8; spec `tray-presence` refusal
//! rows, degraded rows of `tray-state-sync`/`tray-notifications`).
//!
//! [`decide`] and [`polkit_ladder`] are both pure, total functions over
//! small closed enums — every branch this module documents is exercised
//! against fakes here (Lane A); nothing in this file ever opens a bus
//! connection or reads the filesystem itself. `main::boot` (task 10.7)
//! is the only caller that feeds these functions real observations.

/// Whether a well-known service/name has an owner right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePresence {
    Present,
    Absent,
}

/// The three-valued result of the polkit readiness ladder (design.md §0
/// G3). `Indeterminate` is a first-class outcome, not an error: a probe
/// that could not be completed is not evidence the product is broken,
/// and it must never disable the toggle (only [`PolkitReadiness::
/// ActionMissing`] does that). The `&'static str` payload is a short,
/// stable reason code for the status line, never a user-facing sentence
/// on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolkitReadiness {
    Ready,
    ActionMissing(&'static str),
    Indeterminate(&'static str),
}

/// Every observation [`decide`] needs. `polkit` is carried here because
/// it is part of one preflight pass, but [`decide`] itself never reads
/// it — design.md §8: "`PolkitReadiness` never enters `decide`; it only
/// constrains the toggle."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preflight {
    pub session_bus: ServicePresence,
    pub sni_host: ServicePresence,
    pub notifications: ServicePresence,
    pub polkit: PolkitReadiness,
}

/// design.md §8's `Run(Mode)` rows — which capability, if any, the tray
/// is missing while it still runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Full,
    NoTrayHost,
    NoNotifications,
}

/// design.md §8's two hard-refusal reasons. `NoSessionBus` is unreachable
/// from `main::boot`'s real dispatch — a session-bus connection failure
/// is detected and reported (exit 3) before a `Preflight` value can even
/// be built — but [`decide`] still names it as one of its own two
/// outcomes so the function stays a complete, honest description of
/// design.md §8's table on its own, independent of where a caller
/// happens to short-circuit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NoSessionBus,
    NoUserVisibleChannel,
}

/// Whether the tray can run at all, and how (design.md §8's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartDecision {
    Run(Mode),
    Refuse(Refusal),
}

/// The pure decision table (design.md §8). No session bus at all refuses
/// unconditionally; otherwise the 4-row `sni_host` × `notifications`
/// table decides, and `polkit` never changes the answer.
pub fn decide(p: &Preflight) -> StartDecision {
    if p.session_bus == ServicePresence::Absent {
        return StartDecision::Refuse(Refusal::NoSessionBus);
    }
    match (p.sni_host, p.notifications) {
        (ServicePresence::Present, ServicePresence::Present) => StartDecision::Run(Mode::Full),
        (ServicePresence::Absent, ServicePresence::Present) => StartDecision::Run(Mode::NoTrayHost),
        (ServicePresence::Present, ServicePresence::Absent) => StartDecision::Run(Mode::NoNotifications),
        (ServicePresence::Absent, ServicePresence::Absent) => {
            StartDecision::Refuse(Refusal::NoUserVisibleChannel)
        }
    }
}

/// The single polkit action id nopass registers (spec `privilege-admission`
/// "Single Polkit Action"; design.md §0 G3). Defined here, not in `main.rs`,
/// so both the real `EnumerateActions` glue and this module's own pure
/// tests share one literal instead of two that could drift apart.
pub const POLKIT_ACTION_ID: &str = "com.enfoquestic.nopass.manage";

/// Step 2 of the polkit ladder (design.md §0 G3): what `EnumerateActions`
/// answered, abstracted away from the real D-Bus call so the ladder's
/// branching is testable against fakes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumerateOutcome {
    /// `com.enfoquestic.nopass.manage` was present in the returned list.
    ActionFound,
    /// The call succeeded and conclusively did not list the action.
    ActionAbsent,
    /// The call errored, was denied, or timed out — the fallback the
    /// proposal asked for: step 3 (the policy-file stat) decides.
    ErrorOrTimeout,
}

/// Classifies a raw `EnumerateActions` action-id list into an
/// [`EnumerateOutcome`] (spec `privilege-admission` "probe_polkit_readiness
/// consumes the enumeration result", task 8.5; verify-report.md H6/G6). A
/// real `EnumerateActions` round trip returns far more than an id per
/// action — description, message, vendor, defaults — but this ladder step
/// only ever asks one question: is [`POLKIT_ACTION_ID`] present at all. The
/// caller (`main.rs::probe_polkit_readiness`) narrows the real response to
/// just the ids before calling this, so the mapping itself is testable
/// against a plain fake list, with no `zbus` type and no real polkit
/// authority anywhere in this module.
pub fn classify_enumeration<'a>(action_ids: impl IntoIterator<Item = &'a str>) -> EnumerateOutcome {
    if action_ids.into_iter().any(|id| id == POLKIT_ACTION_ID) {
        EnumerateOutcome::ActionFound
    } else {
        EnumerateOutcome::ActionAbsent
    }
}

/// Step 3 of the ladder: the result of `stat`ing
/// `/usr/share/polkit-1/actions/com.enfoquestic.nopass.policy`,
/// abstracted the same way as [`EnumerateOutcome`]. `Unknown` covers the
/// case where even the stat itself could not be answered (for example a
/// permission error on the containing directory) — the one case step 4
/// ("anything still unresolved") actually reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyFileCheck {
    Present,
    Absent,
    Unknown,
}

/// The four-step polkit readiness ladder (design.md §0 G3), first
/// conclusive answer wins:
///
/// 1. `NameHasOwner("org.freedesktop.PolicyKit1")` absent ⇒
///    `ActionMissing("no_authority")`.
/// 2. `EnumerateActions` present ⇒ `Ready`; conclusively absent ⇒
///    `ActionMissing("action_not_registered")`.
/// 3. `EnumerateActions` errored/denied/timed out ⇒ fall back to the
///    policy-file stat: present ⇒ `Ready`; absent ⇒
///    `ActionMissing("policy_file_absent")`.
/// 4. Still unresolved (the stat itself could not be answered) ⇒
///    `Indeterminate`.
///
/// `authority_has_owner == false` short-circuits at step 1 regardless of
/// the other two arguments — a caller with no owner never even attempts
/// steps 2/3, but this function stays total so every input combination
/// (including ones no real caller would construct) has a defined answer.
pub fn polkit_ladder(
    authority_has_owner: bool,
    enumerate: EnumerateOutcome,
    policy_file: PolicyFileCheck,
) -> PolkitReadiness {
    if !authority_has_owner {
        return PolkitReadiness::ActionMissing("no_authority");
    }
    match enumerate {
        EnumerateOutcome::ActionFound => PolkitReadiness::Ready,
        EnumerateOutcome::ActionAbsent => PolkitReadiness::ActionMissing("action_not_registered"),
        EnumerateOutcome::ErrorOrTimeout => match policy_file {
            PolicyFileCheck::Present => PolkitReadiness::Ready,
            PolicyFileCheck::Absent => PolkitReadiness::ActionMissing("policy_file_absent"),
            PolicyFileCheck::Unknown => PolkitReadiness::Indeterminate("policy_file_stat_failed"),
        },
    }
}

/// design.md §0 D6: why the toggle cannot be clicked right now, if it
/// cannot. Each reason renders its own insensitive, localized label
/// (`menu.rs::unavailable_toggle_node`) — replacing
/// `format::toggle_label -> Option<&'static str>`, which had no way to
/// carry a reason at all (task 8.4; verify-report.md H6/G6's open gap:
/// "the user is not left with a dead menu and no reason").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    /// `TrayState::Unknown` — the merged state itself is not yet known.
    StateUnknown,
    /// `PolkitReadiness::ActionMissing(_)` — the polkit action is not
    /// registered, regardless of which of the ladder's three reasons
    /// produced it (design.md §0 D6 "gains a reason", task 8.6).
    InstallationIncomplete,
    /// An [`crate::invoke::ActionGate`] ticket is already held: a
    /// privileged action this same toggle started is still running.
    ActionInFlight,
}

/// design.md §0 D6: what the toggle item renders as. Replaces the old
/// binary `Option<&'static str>` label with a type that can say WHY the
/// toggle is unavailable, not merely that it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleAvailability {
    OfferEnable,
    OfferDisable,
    Unavailable(UnavailableReason),
}

/// The pure decision [`ToggleAvailability`] is computed from (design.md
/// §0 D6, task 8.4/8.6). `action_in_flight` wins over everything else —
/// a privileged action already running must never be raced by a second
/// click regardless of what `polkit`/`state` say. `polkit`'s
/// [`PolkitReadiness::ActionMissing`] wins over `state` next: an
/// incomplete installation cannot honour ANY toggle click, enable or
/// disable. [`PolkitReadiness::Indeterminate`] never disables anything
/// (M2's rule, unchanged — design.md §8's own "Indeterminate polkit
/// never changes the decision", applied here to the toggle rather than
/// `decide`) — it falls straight through to the ordinary state-driven
/// answer, same as [`PolkitReadiness::Ready`].
pub fn toggle_availability(
    state: &crate::reconcile::TrayState,
    polkit: PolkitReadiness,
    action_in_flight: bool,
) -> ToggleAvailability {
    if action_in_flight {
        return ToggleAvailability::Unavailable(UnavailableReason::ActionInFlight);
    }
    if matches!(polkit, PolkitReadiness::ActionMissing(_)) {
        return ToggleAvailability::Unavailable(UnavailableReason::InstallationIncomplete);
    }
    match state {
        crate::reconcile::TrayState::Unknown => ToggleAvailability::Unavailable(UnavailableReason::StateUnknown),
        crate::reconcile::TrayState::Active { .. } => ToggleAvailability::OfferDisable,
        crate::reconcile::TrayState::Inactive => ToggleAvailability::OfferEnable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::TrayState;

    // ---- 10.2: the 4-row sni_host × notifications table, plus
    // NoSessionBus, plus "Indeterminate polkit never changes the
    // decision". ----

    fn preflight(session_bus: ServicePresence, sni_host: ServicePresence, notifications: ServicePresence, polkit: PolkitReadiness) -> Preflight {
        Preflight { session_bus, sni_host, notifications, polkit }
    }

    #[test]
    fn both_present_runs_full() {
        let p = preflight(ServicePresence::Present, ServicePresence::Present, ServicePresence::Present, PolkitReadiness::Ready);
        assert_eq!(decide(&p), StartDecision::Run(Mode::Full));
    }

    #[test]
    fn host_absent_notifications_present_runs_no_tray_host() {
        let p = preflight(ServicePresence::Present, ServicePresence::Absent, ServicePresence::Present, PolkitReadiness::Ready);
        assert_eq!(decide(&p), StartDecision::Run(Mode::NoTrayHost));
    }

    #[test]
    fn host_present_notifications_absent_runs_no_notifications() {
        let p = preflight(ServicePresence::Present, ServicePresence::Present, ServicePresence::Absent, PolkitReadiness::Ready);
        assert_eq!(decide(&p), StartDecision::Run(Mode::NoNotifications));
    }

    #[test]
    fn both_absent_refuses_no_user_visible_channel() {
        let p = preflight(ServicePresence::Present, ServicePresence::Absent, ServicePresence::Absent, PolkitReadiness::Ready);
        assert_eq!(decide(&p), StartDecision::Refuse(Refusal::NoUserVisibleChannel));
    }

    #[test]
    fn no_session_bus_refuses_regardless_of_every_other_field() {
        for (sni, notif) in [
            (ServicePresence::Present, ServicePresence::Present),
            (ServicePresence::Absent, ServicePresence::Present),
            (ServicePresence::Present, ServicePresence::Absent),
            (ServicePresence::Absent, ServicePresence::Absent),
        ] {
            let p = preflight(ServicePresence::Absent, sni, notif, PolkitReadiness::Ready);
            assert_eq!(decide(&p), StartDecision::Refuse(Refusal::NoSessionBus));
        }
    }

    #[test]
    fn indeterminate_polkit_never_changes_any_of_the_four_rows() {
        for (sni, notif, expected) in [
            (ServicePresence::Present, ServicePresence::Present, StartDecision::Run(Mode::Full)),
            (ServicePresence::Absent, ServicePresence::Present, StartDecision::Run(Mode::NoTrayHost)),
            (ServicePresence::Present, ServicePresence::Absent, StartDecision::Run(Mode::NoNotifications)),
            (ServicePresence::Absent, ServicePresence::Absent, StartDecision::Refuse(Refusal::NoUserVisibleChannel)),
        ] {
            let ready = preflight(ServicePresence::Present, sni, notif, PolkitReadiness::Ready);
            let indeterminate = preflight(ServicePresence::Present, sni, notif, PolkitReadiness::Indeterminate("x"));
            let action_missing = preflight(ServicePresence::Present, sni, notif, PolkitReadiness::ActionMissing("x"));
            assert_eq!(decide(&ready), expected);
            assert_eq!(decide(&indeterminate), expected, "Indeterminate polkit must never change the decision");
            assert_eq!(decide(&action_missing), expected, "ActionMissing polkit must never change decide()'s own answer");
        }
    }

    // ---- 10.3: the polkit ladder ----

    #[test]
    fn name_has_owner_absent_yields_action_missing_no_authority_regardless_of_later_steps() {
        for enumerate in [EnumerateOutcome::ActionFound, EnumerateOutcome::ActionAbsent, EnumerateOutcome::ErrorOrTimeout] {
            for policy_file in [PolicyFileCheck::Present, PolicyFileCheck::Absent, PolicyFileCheck::Unknown] {
                assert_eq!(
                    polkit_ladder(false, enumerate, policy_file),
                    PolkitReadiness::ActionMissing("no_authority")
                );
            }
        }
    }

    #[test]
    fn enumerate_actions_present_is_ready() {
        assert_eq!(polkit_ladder(true, EnumerateOutcome::ActionFound, PolicyFileCheck::Unknown), PolkitReadiness::Ready);
    }

    #[test]
    fn enumerate_actions_conclusively_absent_is_action_missing() {
        assert_eq!(
            polkit_ladder(true, EnumerateOutcome::ActionAbsent, PolicyFileCheck::Present),
            PolkitReadiness::ActionMissing("action_not_registered")
        );
    }

    #[test]
    fn enumerate_actions_error_or_timeout_falls_back_to_a_present_policy_file() {
        assert_eq!(polkit_ladder(true, EnumerateOutcome::ErrorOrTimeout, PolicyFileCheck::Present), PolkitReadiness::Ready);
    }

    #[test]
    fn enumerate_actions_error_or_timeout_falls_back_to_an_absent_policy_file() {
        assert_eq!(
            polkit_ladder(true, EnumerateOutcome::ErrorOrTimeout, PolicyFileCheck::Absent),
            PolkitReadiness::ActionMissing("policy_file_absent")
        );
    }

    #[test]
    fn everything_unresolved_is_indeterminate() {
        assert_eq!(
            polkit_ladder(true, EnumerateOutcome::ErrorOrTimeout, PolicyFileCheck::Unknown),
            PolkitReadiness::Indeterminate("policy_file_stat_failed")
        );
    }

    // ---- task 8.5: probe_polkit_readiness's EnumerateActions result is
    // consumed, via a fake enumeration — spec `privilege-admission`
    // "probe_polkit_readiness consumes the enumeration result",
    // verify-report.md H6/G6 ----

    #[test]
    fn classify_enumeration_finds_the_registered_action() {
        assert_eq!(
            classify_enumeration(["some.other.action", POLKIT_ACTION_ID]),
            EnumerateOutcome::ActionFound
        );
    }

    #[test]
    fn classify_enumeration_of_a_list_missing_the_action_is_action_absent_never_assumed_ready() {
        assert_eq!(classify_enumeration(["some.other.action"]), EnumerateOutcome::ActionAbsent);
        assert_eq!(classify_enumeration(std::iter::empty()), EnumerateOutcome::ActionAbsent);
    }

    // ---- task 8.4/8.6: the toggle availability decision ----

    #[test]
    fn action_in_flight_wins_over_every_other_input() {
        for polkit in [PolkitReadiness::Ready, PolkitReadiness::ActionMissing("x"), PolkitReadiness::Indeterminate("x")] {
            for state in [TrayState::Inactive, TrayState::Active { user: None, expiry: None }, TrayState::Unknown] {
                assert_eq!(
                    toggle_availability(&state, polkit, true),
                    ToggleAvailability::Unavailable(UnavailableReason::ActionInFlight),
                    "state={state:?} polkit={polkit:?}"
                );
            }
        }
    }

    #[test]
    fn action_missing_renders_installation_incomplete_regardless_of_its_specific_reason_or_state() {
        for reason in ["no_authority", "action_not_registered", "policy_file_absent"] {
            for state in [TrayState::Inactive, TrayState::Active { user: None, expiry: None }] {
                assert_eq!(
                    toggle_availability(&state, PolkitReadiness::ActionMissing(reason), false),
                    ToggleAvailability::Unavailable(UnavailableReason::InstallationIncomplete)
                );
            }
        }
    }

    #[test]
    fn ready_or_indeterminate_polkit_never_disables_the_toggle_state_alone_decides() {
        for polkit in [PolkitReadiness::Ready, PolkitReadiness::Indeterminate("x")] {
            assert_eq!(toggle_availability(&TrayState::Inactive, polkit, false), ToggleAvailability::OfferEnable);
            assert_eq!(
                toggle_availability(&TrayState::Active { user: None, expiry: None }, polkit, false),
                ToggleAvailability::OfferDisable
            );
            assert_eq!(
                toggle_availability(&TrayState::Unknown, polkit, false),
                ToggleAvailability::Unavailable(UnavailableReason::StateUnknown)
            );
        }
    }
}

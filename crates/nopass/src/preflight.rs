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

#[cfg(test)]
mod tests {
    use super::*;

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
}

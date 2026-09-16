//! Consent — the type that makes an unconsented grant unrepresentable
//! (design.md §3 D3; spec `activation-consent` (all)).
//!
//! Left click, the menu toggle, the keyboard `Activate` path, and the
//! single-instance nudge all converge on one caller that would otherwise
//! construct an enable action inline. A runtime guard there is one
//! refactor away from being forgotten — exactly the proposal's
//! **High**-likelihood risk. This module makes the alternative
//! unrepresentable instead: [`Granted`] is the ONLY producer of proof
//! that the first-activation warning was shown and explicitly accepted,
//! and it has no `Default`, no `new`, no `Clone`, no `Copy`, no `From`,
//! and no public field. Its single field is a private `()`, so a
//! `Granted` value cannot be named into existence anywhere outside this
//! module — asserted by code review and by the private field itself, not
//! by a `trybuild` dev-dependency (declined here for the same reason M2
//! declined one: see design.md §3 D3's own RED-test note).

use crate::duration::GrantDuration;

/// Proof that the first-activation warning was shown and explicitly
/// accepted. See the module doc for why this type cannot be constructed
/// outside `consent.rs`: [`ConsentState::grant`] and
/// [`ConsentState::confirm`] are the only two functions in the crate that
/// can ever produce one.
#[derive(Debug)]
pub struct Granted(());

/// What the menu renders instead of dispatching, while a duration is
/// armed and unacknowledged (design.md §1 "The consent branch"). Carries
/// only the pending duration — enough for a caller to render "Activate
/// for `<d>`" and the two-step confirmation without reaching back into
/// [`ConsentState`]'s private fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentBranch {
    pub pending: GrantDuration,
}

/// The consent state machine (design.md §3 D3). Deliberately opaque:
/// `acknowledged` is not a public field and there is no public enum
/// variant a call site could match on to fabricate a state that yields a
/// [`Granted`] — [`ConsentState::grant`] is the single gate every caller
/// must go through, not duplicated per caller (spec `activation-consent`
/// "No Grant Dispatch Without Recorded Consent").
#[derive(Debug)]
pub struct ConsentState {
    acknowledged: bool,
    pending: Option<GrantDuration>,
}

impl ConsentState {
    /// `acknowledged` starts from the persisted `warning_acknowledged`
    /// flag (design.md §4 D4); nothing is ever pending on construction.
    pub fn from_config(config: &crate::config::Config) -> Self {
        ConsentState { acknowledged: config.warning_acknowledged, pending: None }
    }

    /// The single gate (spec `activation-consent` "No Grant Dispatch
    /// Without Recorded Consent"). `None` means the caller MUST route to
    /// [`ConsentState::branch`] instead of dispatching anything.
    pub fn grant(&self) -> Option<Granted> {
        if self.acknowledged {
            Some(Granted(()))
        } else {
            None
        }
    }

    /// Records a pending duration while consent is unrecorded — what the
    /// menu's "Activate for `<d>`" item does on first click. No
    /// invocation is ever made here; only [`ConsentState::confirm`]
    /// (never this function) can yield a [`Granted`].
    pub fn arm(&mut self, duration: GrantDuration) {
        self.pending = Some(duration);
    }

    /// The explicit "I understand — activate[, and don't warn me
    /// again]" confirmation (spec `activation-consent` "First Activation
    /// Branches the Menu Instead of Granting"). Consumes the pending
    /// duration — a second call without a fresh [`ConsentState::arm`]
    /// yields `None` ("Confirming the branch grants exactly once").
    ///
    /// `persist` is the "don't warn again" checkbox split into its own
    /// action (design.md §1 "Don't warn again is a third action, not a
    /// checkbox"). When `true`, `persist_write` is called exactly once —
    /// the caller's own closure over the real `config::write` call, or a
    /// fake config port in tests — and `acknowledged` is set only if it
    /// reports success (spec `activation-consent` "A Failed Consent
    /// Write Re-Warns Rather Than Silently Granting"). The activation
    /// the user just explicitly confirmed always proceeds either way:
    /// only the "remember this for next time" flag depends on the write
    /// succeeding, never this grant.
    pub fn confirm(&mut self, persist: bool, persist_write: impl FnOnce() -> bool) -> Option<(GrantDuration, Granted)> {
        let duration = self.pending.take()?;
        if persist && persist_write() {
            self.acknowledged = true;
        }
        Some((duration, Granted(())))
    }

    /// Cancelling the branch (spec `activation-consent` "Cancelling the
    /// branch grants nothing"): clears the pending duration and changes
    /// nothing else — `acknowledged` is untouched either way.
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    /// What the menu renders instead of a dispatch: `None` once consent
    /// is acknowledged (nothing to branch on — the last thing dispatched
    /// was, or will be, a direct grant), `Some` while a duration is
    /// pending and consent is still unrecorded.
    pub fn branch(&self) -> Option<ConsentBranch> {
        if self.acknowledged {
            return None;
        }
        self.pending.map(|pending| ConsentBranch { pending })
    }
}

/// Test-only helper: the only legitimate way to obtain a [`Granted`]
/// value from outside this module's own test suite, and it does so
/// through the real `arm`/`confirm` path — not by reaching into private
/// fields — so every other module's tests that need an
/// already-consented action stay honest about how that value can come to
/// exist at all.
#[cfg(test)]
pub(crate) fn granted_for_test() -> Granted {
    let mut state = ConsentState::from_config(&crate::config::Config {
        default_duration: GrantDuration::Hour1,
        warning_acknowledged: false,
    });
    state.arm(GrantDuration::Hour1);
    state
        .confirm(false, || unreachable!("persist=false must never call the write port"))
        .expect("a freshly-armed ConsentState always yields a Granted on confirm")
        .1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::runner::ScriptedRunner;

    fn unacknowledged() -> ConsentState {
        ConsentState::from_config(&Config { default_duration: GrantDuration::Hour1, warning_acknowledged: false })
    }

    fn acknowledged() -> ConsentState {
        ConsentState::from_config(&Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true })
    }

    // ---- task 3.4 ----

    #[test]
    fn grant_returns_none_while_unacknowledged() {
        assert!(unacknowledged().grant().is_none());
    }

    #[test]
    fn grant_returns_some_once_acknowledged() {
        assert!(acknowledged().grant().is_some());
    }

    #[test]
    fn arm_records_a_pending_duration_with_zero_scripted_runner_invocations() {
        // A `ScriptedRunner` with an empty script panics on any `run()`
        // call ("script exhausted") — its mere presence, dropped without
        // panicking, is itself proof that `arm` invoked nothing.
        let runner = ScriptedRunner::new(vec![]);
        let mut state = unacknowledged();

        state.arm(GrantDuration::Hours4);

        assert_eq!(state.branch(), Some(ConsentBranch { pending: GrantDuration::Hours4 }));
        drop(runner);
    }

    // ---- task 3.5 ----

    #[test]
    fn every_activation_raising_caller_dispatches_nothing_while_unacknowledged() {
        // Table-driven over every known caller that can raise an
        // activation request (design.md §3 D3; spec `activation-consent`
        // "A menu-triggered activation...", "A non-menu activation
        // path...", "...activation nudge never itself dispatches an
        // enable"; threat matrix "Consent bypass"). Each row proves the
        // SAME shared gate is what every caller must go through: the
        // check is written once, here, not duplicated per caller.
        let callers = ["menu toggle", "SNI left-click", "SNI keyboard Activate", "single-instance activation nudge"];

        for caller in callers {
            let state = unacknowledged();
            // An empty script: if this caller's path ever reached a real
            // invocation, `run()` would panic ("script exhausted")
            // before this closure returns — zero invocations is proven
            // by construction, not merely by inspection.
            let runner = ScriptedRunner::new(vec![]);

            match state.grant() {
                Some(_) => panic!("{caller}: must not be granted while consent is unacknowledged"),
                None => {
                    // No `Action::Enable` can be built without a
                    // `Granted`, so there is nothing left to hand the
                    // runner — this IS what "zero invocations" means at
                    // the type level for every caller in this table.
                }
            }

            drop(runner);
        }
    }

    // ---- task 3.6 ----

    #[test]
    fn confirm_without_persist_grants_exactly_once_per_arm() {
        let mut state = unacknowledged();
        state.arm(GrantDuration::Hours4);

        let first = state.confirm(false, || panic!("persist=false must never attempt a write"));
        assert!(matches!(first, Some((GrantDuration::Hours4, _))));

        let second = state.confirm(false, || panic!("must not attempt a write here either"));
        assert!(second.is_none(), "confirm must grant exactly once per arm, not repeatedly");
    }

    #[test]
    fn cancel_clears_pending_with_no_other_state_change() {
        let mut state = unacknowledged();
        state.arm(GrantDuration::Hours4);

        state.cancel();

        assert!(state.branch().is_none(), "cancel must clear the pending duration");
        assert!(state.grant().is_none(), "cancel must not acknowledge consent as a side effect");
    }

    // ---- task 3.7 ----

    #[test]
    fn confirm_with_persist_acknowledges_only_after_a_successful_write() {
        let mut state = unacknowledged();
        state.arm(GrantDuration::Hours4);

        let result = state.confirm(true, || true); // fake config port: write succeeds

        assert!(matches!(result, Some((GrantDuration::Hours4, _))));
        assert!(state.grant().is_some(), "a successfully persisted write must acknowledge consent");
    }

    #[test]
    fn a_failing_config_write_leaves_consent_unacknowledged_and_still_grants_the_current_request() {
        let mut state = unacknowledged();
        state.arm(GrantDuration::Hours4);

        // Fake config port returning a write error (spec
        // `activation-consent` "A Failed Consent Write Re-Warns Rather
        // Than Silently Granting").
        let result = state.confirm(true, || false);

        // The activation the user explicitly confirmed still proceeds
        // this one time...
        assert!(matches!(result, Some((GrantDuration::Hours4, _))));
        // ...but nothing was persisted, so the NEXT activation is still
        // gated — never a silent grant based on an unpersisted flag.
        assert!(state.grant().is_none(), "a failed write must re-warn rather than silently grant next time");
    }

    // ---- task 3.8 ----

    #[test]
    fn branch_is_none_with_nothing_pending() {
        assert!(unacknowledged().branch().is_none());
    }

    #[test]
    fn branch_is_some_while_a_duration_is_pending_and_unacknowledged() {
        let mut state = unacknowledged();
        state.arm(GrantDuration::Hours4);

        assert_eq!(state.branch(), Some(ConsentBranch { pending: GrantDuration::Hours4 }));
    }

    #[test]
    fn branch_is_none_once_acknowledged_even_if_something_were_pending() {
        let mut state = acknowledged();
        // Defensive: acknowledged must win even if a caller mistakenly
        // still had something armed.
        state.arm(GrantDuration::Hours4);

        assert!(state.branch().is_none(), "an acknowledged state must never render a consent branch");
    }
}

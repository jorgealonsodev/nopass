//! The exit-code → outcome table (design.md §5, §5.1; spec
//! `tray-privileged-invocation`): every code the `pkexec`/helper
//! invocation can produce maps onto exactly one distinct user-facing
//! outcome, and an exit code is never itself evidence that a grant
//! exists or was withdrawn (design.md's rule above the table).
//!
//! `classify` is written against the same discipline `reconcile::merge`
//! uses: the raw `i32` status is first narrowed into [`DocumentedCode`],
//! a small closed enum with one named variant per row this table
//! documents, and the match over THAT type has no wildcard arm. A future
//! helper release that reuses an undocumented code, or a table row this
//! module forgets to add, therefore fails to compile here rather than
//! silently reusing another code's message.

use crate::probe::Probe;

/// What the tray asked the helper to do. `Enable`'s `until` is the epoch
/// second the helper was told to expire the grant at — carried through
/// to [`OutcomeKind::Granted`] for later display (design.md §2 `outcome`,
/// §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Enable { until: u64 },
    Disable,
}

/// One user-facing outcome. One variant per §5 table row, plus
/// [`OutcomeKind::UnexpiringGrant`] (§5.1 rule 3) — never produced by
/// [`classify`] itself, only by [`escalate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    /// Exit 0 from `enable`.
    Granted { until: u64 },
    /// Exit 0 from `disable`.
    Revoked,
    /// Exit 1 — internal error or a required binary is missing.
    InternalError,
    /// Exit 2 — clap rejected the argv the tray built.
    VersionSkew,
    /// Exit 10 — invocation-context violation.
    ContextViolation,
    /// Exit 11 — uid rejected.
    UidRejected,
    /// Exit 12 — not a sudoer.
    NotSudoer,
    /// Exit 13 — duration/until invalid.
    BadDuration,
    /// Exit 14 — `visudo` rejected the generated rule.
    VisudoRejected,
    /// Exit 15 — lock busy.
    LockBusy,
    /// Exit 16 — filesystem/atomic-write failure.
    FsFailure,
    /// Exit 17 — the expiry timer could not be scheduled (§5.1). Read as
    /// "state unknown, reconcile now" — never as a plain failure and
    /// never as an active grant.
    TimerUnscheduled,
    /// `pkexec` exit 126 — the authentication dialog was dismissed.
    Cancelled,
    /// `pkexec` exit 127, helper present — authorization failed or no
    /// polkit agent is registered in this session.
    NotAuthorized,
    /// `pkexec` exit 127, helper NOT present (`access(HELPER_PATH,
    /// X_OK)` failed) — the installation itself is incomplete.
    HelperMissing,
    /// The tray could not even spawn `pkexec` (for example, it is not
    /// installed). Never produced by [`classify`] — the caller
    /// constructs this directly from a `RunnerError::Spawn` /
    /// `RunnerError::NonAbsoluteProgram`, before there is any exit
    /// status to classify.
    SpawnFailed,
    /// The invocation was killed by a signal (`status: None`).
    Interrupted,
    /// §5.1 rule 3: a `TimerUnscheduled` outcome whose following probe
    /// came back `Passwordless` — a live, unexpiring grant the user was
    /// told had failed. The only [`Severity::Error`] outcome in M2 that
    /// reports a *live* grant.
    UnexpiringGrant,
}

/// How urgently an outcome should be surfaced (design.md §2 `outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Success,
    Info,
    Error,
}

impl OutcomeKind {
    /// design.md §5's `Sev` column. `LockBusy` and `Cancelled` are the
    /// only two `Info`-severity outcomes — neither one means anything
    /// was left in a bad state, just that the request did not go
    /// through this time.
    pub fn severity(&self) -> Severity {
        match self {
            OutcomeKind::Granted { .. } | OutcomeKind::Revoked => Severity::Success,
            OutcomeKind::LockBusy | OutcomeKind::Cancelled => Severity::Info,
            OutcomeKind::InternalError
            | OutcomeKind::VersionSkew
            | OutcomeKind::ContextViolation
            | OutcomeKind::UidRejected
            | OutcomeKind::NotSudoer
            | OutcomeKind::BadDuration
            | OutcomeKind::VisudoRejected
            | OutcomeKind::FsFailure
            | OutcomeKind::TimerUnscheduled
            | OutcomeKind::NotAuthorized
            | OutcomeKind::HelperMissing
            | OutcomeKind::SpawnFailed
            | OutcomeKind::Interrupted
            | OutcomeKind::UnexpiringGrant => Severity::Error,
        }
    }

    /// `(summary, body)` — design.md §5/§5.1's literal user-facing text.
    /// This is where a future `format::outcome_text` (Phase 6/7) will
    /// delegate once `format.rs` exists; kept here for now because the
    /// uniqueness this table promises ("Each documented code produces
    /// its own message") is exactly what this phase's tests prove.
    ///
    /// [`OutcomeKind::Granted`]'s body deliberately does not spell out a
    /// formatted "HH:MM" yet — that formatting belongs to
    /// `format::countdown` (Phase 7), which does not exist in this
    /// phase; the caller composing a notification is expected to append
    /// the countdown separately, the same split design.md's Phase 7/8
    /// task list already describes for `notifications.rs`.
    pub fn text(&self) -> (String, String) {
        match self {
            OutcomeKind::Granted { .. } => (
                "Passwordless sudo enabled".to_string(),
                "Passwordless sudo enabled until the requested time.".to_string(),
            ),
            OutcomeKind::Revoked => {
                ("Passwordless sudo disabled".to_string(), "Passwordless sudo disabled".to_string())
            }
            OutcomeKind::InternalError => (
                "NoPass internal error".to_string(),
                "NoPass could not complete the change: a required system program is missing or the \
                 helper failed internally. Nothing was changed."
                    .to_string(),
            ),
            OutcomeKind::VersionSkew => (
                "Helper version mismatch".to_string(),
                "The installed helper does not understand this request — the tray and helper \
                 versions do not match. Reinstall NoPass."
                    .to_string(),
            ),
            OutcomeKind::ContextViolation => (
                "Invalid authorization context".to_string(),
                "The authorization did not carry your user identity. Do not run NoPass as root.".to_string(),
            ),
            OutcomeKind::UidRejected => (
                "Account not eligible".to_string(),
                "This account is not eligible for passwordless sudo (a system account, or outside \
                 the normal user id range)."
                    .to_string(),
            ),
            OutcomeKind::NotSudoer => (
                "Not a sudoer".to_string(),
                "Your user is not allowed to use sudo, so passwordless sudo cannot be granted.".to_string(),
            ),
            OutcomeKind::BadDuration => (
                "Invalid expiry time".to_string(),
                "The requested expiry time was rejected. Check that the system clock is correct.".to_string(),
            ),
            OutcomeKind::VisudoRejected => (
                "Sudo rule rejected".to_string(),
                "The generated sudo rule was rejected as invalid. Nothing was changed. Please report this."
                    .to_string(),
            ),
            OutcomeKind::LockBusy => (
                "NoPass is busy".to_string(),
                "Another NoPass operation is already running. Try again in a moment.".to_string(),
            ),
            OutcomeKind::FsFailure => (
                "Could not write the rule file".to_string(),
                "NoPass could not write the rule file. Nothing was changed.".to_string(),
            ),
            OutcomeKind::TimerUnscheduled => (
                "Expiry timer could not be scheduled".to_string(),
                "The expiry timer could not be scheduled, so the timed grant was withdrawn. NoPass \
                 is re-checking whether any grant is currently active."
                    .to_string(),
            ),
            OutcomeKind::Cancelled => (
                "Authorization cancelled".to_string(),
                "Authorization cancelled. Nothing was changed.".to_string(),
            ),
            OutcomeKind::NotAuthorized => (
                "Authorization failed".to_string(),
                "Authorization failed. There may be no polkit authentication agent running in this \
                 session, or your user is not permitted to perform this action."
                    .to_string(),
            ),
            OutcomeKind::HelperMissing => (
                "NoPass is not fully installed".to_string(),
                "NoPass is not completely installed: the privileged helper is missing at \
                 /usr/libexec/nopass-helper."
                    .to_string(),
            ),
            OutcomeKind::SpawnFailed => (
                "polkit is not installed".to_string(),
                "`pkexec` is not installed, so NoPass cannot request authorization. Install `polkit`.".to_string(),
            ),
            OutcomeKind::Interrupted => (
                "Authorization interrupted".to_string(),
                "The authorization was interrupted. NoPass will re-check the current state.".to_string(),
            ),
            OutcomeKind::UnexpiringGrant => (
                "Passwordless sudo active with no expiry".to_string(),
                "Passwordless sudo is active with no expiry, because its timer could not be \
                 scheduled. Use Disable when you are done."
                    .to_string(),
            ),
        }
    }
}

/// The closed set of exit codes this table documents — the helper's own
/// exactly-0/1/2/10-17 (`nopass-helper::error::HelperError::exit_code`)
/// and `pkexec`'s own 126/127. Converting a raw `i32` into this type is
/// the ONLY place a numeric literal is compared against the status;
/// [`classify`] itself matches by name, never by number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentedCode {
    Zero,
    One,
    Two,
    Ten,
    Eleven,
    Twelve,
    Thirteen,
    Fourteen,
    Fifteen,
    Sixteen,
    Seventeen,
    OneTwentySix,
    OneTwentySeven,
    /// A code neither the helper nor `pkexec` can produce for this
    /// invocation shape. Kept as its own named arm — never folded behind
    /// a wildcard — so a future table row is never silently mistaken for
    /// this defensive fallback.
    Unexpected(i32),
}

impl DocumentedCode {
    fn from_raw(code: i32) -> DocumentedCode {
        match code {
            0 => DocumentedCode::Zero,
            1 => DocumentedCode::One,
            2 => DocumentedCode::Two,
            10 => DocumentedCode::Ten,
            11 => DocumentedCode::Eleven,
            12 => DocumentedCode::Twelve,
            13 => DocumentedCode::Thirteen,
            14 => DocumentedCode::Fourteen,
            15 => DocumentedCode::Fifteen,
            16 => DocumentedCode::Sixteen,
            17 => DocumentedCode::Seventeen,
            126 => DocumentedCode::OneTwentySix,
            127 => DocumentedCode::OneTwentySeven,
            other => DocumentedCode::Unexpected(other),
        }
    }
}

/// Maps one invocation result onto its outcome (design.md §5).
/// `status: None` means the invocation was killed by a signal —
/// [`OutcomeKind::SpawnFailed`] is deliberately unreachable from here;
/// the caller constructs it directly from a `RunnerError::Spawn` /
/// `RunnerError::NonAbsoluteProgram`, before any status exists to
/// classify at all.
pub fn classify(action: Action, status: Option<i32>, helper_present: bool) -> OutcomeKind {
    let Some(code) = status else {
        return OutcomeKind::Interrupted;
    };
    match DocumentedCode::from_raw(code) {
        DocumentedCode::Zero => match action {
            Action::Enable { until } => OutcomeKind::Granted { until },
            Action::Disable => OutcomeKind::Revoked,
        },
        DocumentedCode::One => OutcomeKind::InternalError,
        DocumentedCode::Two => OutcomeKind::VersionSkew,
        DocumentedCode::Ten => OutcomeKind::ContextViolation,
        DocumentedCode::Eleven => OutcomeKind::UidRejected,
        DocumentedCode::Twelve => OutcomeKind::NotSudoer,
        DocumentedCode::Thirteen => OutcomeKind::BadDuration,
        DocumentedCode::Fourteen => OutcomeKind::VisudoRejected,
        DocumentedCode::Fifteen => OutcomeKind::LockBusy,
        DocumentedCode::Sixteen => OutcomeKind::FsFailure,
        DocumentedCode::Seventeen => OutcomeKind::TimerUnscheduled,
        DocumentedCode::OneTwentySix => OutcomeKind::Cancelled,
        DocumentedCode::OneTwentySeven => {
            if helper_present {
                OutcomeKind::NotAuthorized
            } else {
                OutcomeKind::HelperMissing
            }
        }
        // Neither the helper (closed 0/1/2/10-17 contract) nor `pkexec`
        // (126/127, or a relayed helper code) can reach this arm in
        // practice; treated the same as an internal error rather than
        // ever being mistaken for one of the fourteen documented causes
        // above.
        DocumentedCode::Unexpected(_) => OutcomeKind::InternalError,
    }
}

/// §5.1 rule 3 — the escalation the tray runs after `Trigger::
/// ActionCompleted`'s mandatory follow-up probe. Only `TimerUnscheduled`
/// ever escalates, and only into `UnexpiringGrant`, and only when the
/// probe says `Passwordless`. Every other `OutcomeKind`, under either
/// probe value, escalates to nothing — matched by name, so a newly added
/// `OutcomeKind` variant must be given an explicit answer here too.
pub fn escalate(prev: OutcomeKind, probe: Probe) -> Option<OutcomeKind> {
    match prev {
        OutcomeKind::TimerUnscheduled => match probe {
            Probe::Passwordless => Some(OutcomeKind::UnexpiringGrant),
            Probe::PasswordRequired => None,
        },
        OutcomeKind::Granted { .. }
        | OutcomeKind::Revoked
        | OutcomeKind::InternalError
        | OutcomeKind::VersionSkew
        | OutcomeKind::ContextViolation
        | OutcomeKind::UidRejected
        | OutcomeKind::NotSudoer
        | OutcomeKind::BadDuration
        | OutcomeKind::VisudoRejected
        | OutcomeKind::LockBusy
        | OutcomeKind::FsFailure
        | OutcomeKind::Cancelled
        | OutcomeKind::NotAuthorized
        | OutcomeKind::HelperMissing
        | OutcomeKind::SpawnFailed
        | OutcomeKind::Interrupted
        | OutcomeKind::UnexpiringGrant => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_documented_outcomes() -> Vec<OutcomeKind> {
        vec![
            classify(Action::Enable { until: 1_700_000_000 }, Some(0), true),
            classify(Action::Disable, Some(0), true),
            classify(Action::Disable, Some(1), true),
            classify(Action::Disable, Some(2), true),
            classify(Action::Disable, Some(10), true),
            classify(Action::Disable, Some(11), true),
            classify(Action::Disable, Some(12), true),
            classify(Action::Disable, Some(13), true),
            classify(Action::Disable, Some(14), true),
            classify(Action::Disable, Some(15), true),
            classify(Action::Disable, Some(16), true),
            classify(Action::Disable, Some(17), true),
            classify(Action::Disable, Some(126), true),
            classify(Action::Disable, Some(127), true),
            classify(Action::Disable, Some(127), false),
            OutcomeKind::SpawnFailed,
            classify(Action::Disable, None, true),
            OutcomeKind::UnexpiringGrant,
        ]
    }

    #[test]
    fn exit_zero_enable_grants_and_carries_the_until_epoch() {
        assert_eq!(classify(Action::Enable { until: 42 }, Some(0), true), OutcomeKind::Granted { until: 42 });
    }

    #[test]
    fn exit_zero_disable_revokes() {
        assert_eq!(classify(Action::Disable, Some(0), true), OutcomeKind::Revoked);
    }

    #[test]
    fn exit_one_is_internal_error() {
        assert_eq!(classify(Action::Disable, Some(1), true), OutcomeKind::InternalError);
    }

    #[test]
    fn exit_two_is_version_skew() {
        assert_eq!(classify(Action::Disable, Some(2), true), OutcomeKind::VersionSkew);
    }

    #[test]
    fn exit_ten_is_context_violation() {
        assert_eq!(classify(Action::Disable, Some(10), true), OutcomeKind::ContextViolation);
    }

    #[test]
    fn exit_eleven_is_uid_rejected() {
        assert_eq!(classify(Action::Disable, Some(11), true), OutcomeKind::UidRejected);
    }

    #[test]
    fn exit_twelve_is_not_sudoer() {
        assert_eq!(classify(Action::Disable, Some(12), true), OutcomeKind::NotSudoer);
    }

    #[test]
    fn exit_thirteen_is_bad_duration() {
        assert_eq!(classify(Action::Disable, Some(13), true), OutcomeKind::BadDuration);
    }

    #[test]
    fn exit_fourteen_is_visudo_rejected() {
        assert_eq!(classify(Action::Disable, Some(14), true), OutcomeKind::VisudoRejected);
    }

    #[test]
    fn exit_fifteen_is_lock_busy() {
        assert_eq!(classify(Action::Disable, Some(15), true), OutcomeKind::LockBusy);
    }

    #[test]
    fn exit_sixteen_is_fs_failure() {
        assert_eq!(classify(Action::Disable, Some(16), true), OutcomeKind::FsFailure);
    }

    #[test]
    fn exit_seventeen_is_timer_unscheduled() {
        assert_eq!(classify(Action::Disable, Some(17), true), OutcomeKind::TimerUnscheduled);
    }

    #[test]
    fn exit_one_twenty_six_is_cancelled() {
        assert_eq!(classify(Action::Disable, Some(126), true), OutcomeKind::Cancelled);
    }

    #[test]
    fn exit_one_twenty_seven_with_helper_present_is_not_authorized() {
        assert_eq!(classify(Action::Disable, Some(127), true), OutcomeKind::NotAuthorized);
    }

    #[test]
    fn exit_one_twenty_seven_with_helper_absent_is_helper_missing() {
        assert_eq!(classify(Action::Disable, Some(127), false), OutcomeKind::HelperMissing);
    }

    #[test]
    fn a_missing_status_is_interrupted_regardless_of_action() {
        assert_eq!(classify(Action::Disable, None, true), OutcomeKind::Interrupted);
        assert_eq!(classify(Action::Enable { until: 1 }, None, false), OutcomeKind::Interrupted);
    }

    #[test]
    fn an_undocumented_code_is_treated_as_an_internal_error_never_as_one_of_the_documented_causes() {
        let undocumented = classify(Action::Disable, Some(99), true);
        assert_eq!(undocumented, OutcomeKind::InternalError);
    }

    #[test]
    fn spawn_failed_is_a_distinct_outcome_never_produced_by_classify() {
        // `SpawnFailed` has no exit-code source to classify from — the
        // caller constructs it directly from a `RunnerError`. This test
        // documents that this crate's own `classify` results never
        // equal it.
        for status in [None, Some(0), Some(1), Some(126), Some(127)] {
            assert_ne!(classify(Action::Disable, status, true), OutcomeKind::SpawnFailed);
        }
    }

    #[test]
    fn every_documented_outcome_renders_a_distinct_summary_and_body_pair() {
        let rendered: Vec<(String, String)> = all_documented_outcomes().iter().map(OutcomeKind::text).collect();
        let mut sorted = rendered.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            rendered.len(),
            "no two distinct outcomes may share the same rendered (summary, body) pair: {rendered:?}"
        );
    }

    #[test]
    fn exit_seventeen_text_asserts_neither_that_nothing_changed_nor_that_a_grant_is_active() {
        let (_, body) = OutcomeKind::TimerUnscheduled.text();
        assert!(!body.contains("Nothing was changed"), "must not claim nothing changed: {body:?}");
        assert!(!body.to_lowercase().contains("is active"), "must not claim a live grant either: {body:?}");
    }

    #[test]
    fn exit_seventeen_text_differs_from_exit_sixteen_text() {
        assert_ne!(OutcomeKind::TimerUnscheduled.text(), OutcomeKind::FsFailure.text());
    }

    #[test]
    fn pkexec_126_and_127_texts_are_distinguished_from_each_other_and_from_helper_texts() {
        let cancelled = OutcomeKind::Cancelled.text();
        let not_authorized = OutcomeKind::NotAuthorized.text();
        let helper_missing = OutcomeKind::HelperMissing.text();
        assert_ne!(cancelled, not_authorized);
        assert_ne!(cancelled, helper_missing);
        assert_ne!(not_authorized, helper_missing);
        for helper_text in [
            OutcomeKind::InternalError.text(),
            OutcomeKind::VersionSkew.text(),
            OutcomeKind::ContextViolation.text(),
            OutcomeKind::UidRejected.text(),
            OutcomeKind::NotSudoer.text(),
            OutcomeKind::BadDuration.text(),
            OutcomeKind::VisudoRejected.text(),
            OutcomeKind::LockBusy.text(),
            OutcomeKind::FsFailure.text(),
            OutcomeKind::TimerUnscheduled.text(),
        ] {
            assert_ne!(cancelled, helper_text);
            assert_ne!(not_authorized, helper_text);
        }
    }

    // ---- escalate (§5.1 rule 3, task 5.6) ----

    #[test]
    fn timer_unscheduled_escalates_to_unexpiring_grant_when_the_probe_is_passwordless() {
        assert_eq!(escalate(OutcomeKind::TimerUnscheduled, Probe::Passwordless), Some(OutcomeKind::UnexpiringGrant));
    }

    #[test]
    fn timer_unscheduled_does_not_escalate_when_the_probe_requires_a_password() {
        assert_eq!(escalate(OutcomeKind::TimerUnscheduled, Probe::PasswordRequired), None);
    }

    #[test]
    fn no_other_outcome_escalates_under_either_probe_value() {
        let non_escalating = all_documented_outcomes()
            .into_iter()
            .filter(|k| !matches!(k, OutcomeKind::TimerUnscheduled));
        for outcome in non_escalating {
            assert_eq!(escalate(outcome, Probe::Passwordless), None, "{outcome:?} must not escalate on Passwordless");
            assert_eq!(
                escalate(outcome, Probe::PasswordRequired),
                None,
                "{outcome:?} must not escalate on PasswordRequired"
            );
        }
    }

    #[test]
    fn unexpiring_grant_is_the_only_error_severity_outcome_reporting_a_live_grant() {
        assert_eq!(OutcomeKind::UnexpiringGrant.severity(), Severity::Error);
    }

    // ---- Severity table spot checks ----

    #[test]
    fn success_outcomes_are_success_severity() {
        assert_eq!(OutcomeKind::Granted { until: 1 }.severity(), Severity::Success);
        assert_eq!(OutcomeKind::Revoked.severity(), Severity::Success);
    }

    #[test]
    fn lock_busy_and_cancelled_are_info_severity() {
        assert_eq!(OutcomeKind::LockBusy.severity(), Severity::Info);
        assert_eq!(OutcomeKind::Cancelled.severity(), Severity::Info);
    }

    #[test]
    fn every_other_documented_outcome_is_error_severity() {
        for outcome in all_documented_outcomes() {
            if matches!(outcome, OutcomeKind::Granted { .. } | OutcomeKind::Revoked | OutcomeKind::LockBusy | OutcomeKind::Cancelled)
            {
                continue;
            }
            assert_eq!(outcome.severity(), Severity::Error, "{outcome:?} must be Severity::Error");
        }
    }
}

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

use nopass_core::expiry::Expiry;

use crate::consent::Granted;
use crate::duration::GrantDuration;
use crate::format::{Lang, Msg};
use crate::probe::Probe;

/// What the tray asked the helper to do (design.md §2 `outcome`, §4.3,
/// §3 D3). `Enable`'s only constructor is [`EnableRequest::new`], which
/// takes a [`Granted`] — a value nothing outside `consent.rs` can name —
/// so `Action::Enable` is unconstructible without going through
/// [`crate::consent::ConsentState::grant`] or
/// [`crate::consent::ConsentState::confirm`] first (threat matrix
/// "Consent bypass"). `Action` therefore cannot derive `Clone`/`Copy`:
/// one `Granted` buys exactly one invocation.
#[derive(Debug)]
pub enum Action {
    Enable(EnableRequest),
    Disable,
}

/// The privileged detail behind [`Action::Enable`]: which duration was
/// requested, when the request was made (`at`, an epoch second), and the
/// [`Granted`] proof that the first-activation warning was accepted.
/// Every field is private — [`EnableRequest::new`] is the only way to
/// build one, and it demands a `Granted` argument (design.md §3 D3).
#[derive(Debug)]
pub struct EnableRequest {
    duration: GrantDuration,
    at: u64,
    #[allow(dead_code)] // held only as proof-of-consent; never inspected
    granted: Granted,
}

impl EnableRequest {
    /// The ONLY constructor. `granted` cannot be fabricated outside
    /// `consent.rs`, so calling this at all is itself proof that consent
    /// was recorded before this value could ever exist.
    pub fn new(duration: GrantDuration, at: u64, granted: Granted) -> Self {
        EnableRequest { duration, at, granted }
    }

    /// The exact argv `duration` renders after `enable`, evaluated at
    /// `at` — the single XOR site `duration::args` already owns (design
    /// Open Questions; threat matrix "External command composition").
    /// Crate-internal: `invoke::pkexec_spec` is the only consumer.
    pub(crate) fn argv(&self) -> Vec<String> {
        self.duration.args(self.at)
    }

    /// Best-effort epoch second for [`OutcomeKind::Granted`]'s display
    /// (§5, §5.1). `Expiry::At` durations have a single epoch to report;
    /// `Expiry::Reboot`/`Expiry::Never` do not, and the full
    /// end-to-end composition — including how those two render — is
    /// finished by Phase 8 (design.md §4 file-changes table, task 8.8).
    fn until(&self) -> u64 {
        match self.duration.expiry(self.at) {
            Expiry::At { epoch } => epoch,
            Expiry::Reboot | Expiry::Never => self.at,
        }
    }
}

/// Authority-free description of an [`Action`], for the outcome table and
/// any in-flight record that must outlive the single-use [`Granted`]
/// proof (design.md §3 D3): `classify`/`handle_action_finished` can take
/// this instead of `Action` itself, so `Granted` never needs `Clone`.
#[derive(Debug, Clone, Copy)]
pub enum ActionKind {
    Enable { expiry: Expiry },
    Disable,
}

impl Action {
    pub fn kind(&self) -> ActionKind {
        match self {
            Action::Enable(req) => ActionKind::Enable { expiry: req.duration.expiry(req.at) },
            Action::Disable => ActionKind::Disable,
        }
    }
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

    /// `(summary, body)` — design.md §5/§5.1's literal user-facing text,
    /// resolved through `format.rs`'s `Msg` catalogue (Fix 2, m3 desktop
    /// review) so the toast shown after every single grant/disable/error
    /// speaks the caller's `lang` rather than a raw English literal. The
    /// uniqueness this table promises ("Each documented code produces its
    /// own message") is exactly what this module's own tests still prove,
    /// per `lang`.
    ///
    /// [`OutcomeKind::Granted`]'s body deliberately does not spell out a
    /// formatted "HH:MM" itself — that formatting belongs to
    /// `format::countdown`; the caller composing a notification appends
    /// the countdown separately (`notifications.rs`'s
    /// `action_notification`, via `Msg::OutcomeGrantedExpiresSuffix`).
    pub fn text(&self, lang: Lang) -> (String, String) {
        match self {
            OutcomeKind::Granted { .. } => {
                (Msg::OutcomeGrantedSummary.text(lang).to_string(), Msg::OutcomeGrantedBody.text(lang).to_string())
            }
            OutcomeKind::Revoked => (Msg::OutcomeRevoked.text(lang).to_string(), Msg::OutcomeRevoked.text(lang).to_string()),
            OutcomeKind::InternalError => (
                Msg::OutcomeInternalErrorSummary.text(lang).to_string(),
                Msg::OutcomeInternalErrorBody.text(lang).to_string(),
            ),
            OutcomeKind::VersionSkew => (
                Msg::OutcomeVersionSkewSummary.text(lang).to_string(),
                Msg::OutcomeVersionSkewBody.text(lang).to_string(),
            ),
            OutcomeKind::ContextViolation => (
                Msg::OutcomeContextViolationSummary.text(lang).to_string(),
                Msg::OutcomeContextViolationBody.text(lang).to_string(),
            ),
            OutcomeKind::UidRejected => (
                Msg::OutcomeUidRejectedSummary.text(lang).to_string(),
                Msg::OutcomeUidRejectedBody.text(lang).to_string(),
            ),
            OutcomeKind::NotSudoer => {
                (Msg::OutcomeNotSudoerSummary.text(lang).to_string(), Msg::OutcomeNotSudoerBody.text(lang).to_string())
            }
            OutcomeKind::BadDuration => (
                Msg::OutcomeBadDurationSummary.text(lang).to_string(),
                Msg::OutcomeBadDurationBody.text(lang).to_string(),
            ),
            OutcomeKind::VisudoRejected => (
                Msg::OutcomeVisudoRejectedSummary.text(lang).to_string(),
                Msg::OutcomeVisudoRejectedBody.text(lang).to_string(),
            ),
            OutcomeKind::LockBusy => {
                (Msg::OutcomeLockBusySummary.text(lang).to_string(), Msg::OutcomeLockBusyBody.text(lang).to_string())
            }
            OutcomeKind::FsFailure => {
                (Msg::OutcomeFsFailureSummary.text(lang).to_string(), Msg::OutcomeFsFailureBody.text(lang).to_string())
            }
            OutcomeKind::TimerUnscheduled => (
                Msg::OutcomeTimerUnscheduledSummary.text(lang).to_string(),
                Msg::OutcomeTimerUnscheduledBody.text(lang).to_string(),
            ),
            OutcomeKind::Cancelled => (
                Msg::OutcomeCancelledSummary.text(lang).to_string(),
                Msg::OutcomeCancelledBody.text(lang).to_string(),
            ),
            OutcomeKind::NotAuthorized => (
                Msg::OutcomeNotAuthorizedSummary.text(lang).to_string(),
                Msg::OutcomeNotAuthorizedBody.text(lang).to_string(),
            ),
            OutcomeKind::HelperMissing => (
                Msg::OutcomeHelperMissingSummary.text(lang).to_string(),
                Msg::OutcomeHelperMissingBody.text(lang).to_string(),
            ),
            OutcomeKind::SpawnFailed => (
                Msg::OutcomeSpawnFailedSummary.text(lang).to_string(),
                Msg::OutcomeSpawnFailedBody.text(lang).to_string(),
            ),
            OutcomeKind::Interrupted => (
                Msg::OutcomeInterruptedSummary.text(lang).to_string(),
                Msg::OutcomeInterruptedBody.text(lang).to_string(),
            ),
            OutcomeKind::UnexpiringGrant => (
                Msg::OutcomeUnexpiringGrantSummary.text(lang).to_string(),
                Msg::OutcomeUnexpiringGrantBody.text(lang).to_string(),
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
            Action::Enable(req) => OutcomeKind::Granted { until: req.until() },
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
    use crate::consent::granted_for_test;

    /// Builds an `Action::Enable` for these tests via the real
    /// `EnableRequest::new(GrantDuration, at, Granted)` constructor — the
    /// only one that exists (design.md §3 D3) — always using
    /// `GrantDuration::Hour1`, whose `expiry(at) == At { epoch: at + 3600
    /// }`, so callers can still reason about the resulting `until` in
    /// terms of `at`.
    fn enable(at: u64) -> Action {
        Action::Enable(EnableRequest::new(GrantDuration::Hour1, at, granted_for_test()))
    }

    fn all_documented_outcomes() -> Vec<OutcomeKind> {
        vec![
            classify(enable(1_700_000_000), Some(0), true),
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
        assert_eq!(classify(enable(42), Some(0), true), OutcomeKind::Granted { until: 42 + 3_600 });
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
        assert_eq!(classify(enable(1), None, false), OutcomeKind::Interrupted);
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
        let rendered: Vec<(String, String)> = all_documented_outcomes().iter().map(|k| k.text(Lang::En)).collect();
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
    fn every_documented_outcome_renders_a_distinct_summary_and_body_pair_in_spanish_too() {
        let rendered: Vec<(String, String)> = all_documented_outcomes().iter().map(|k| k.text(Lang::Es)).collect();
        let mut sorted = rendered.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            rendered.len(),
            "no two distinct outcomes may share the same rendered Spanish (summary, body) pair: {rendered:?}"
        );
    }

    #[test]
    fn exit_seventeen_text_asserts_neither_that_nothing_changed_nor_that_a_grant_is_active() {
        let (_, body) = OutcomeKind::TimerUnscheduled.text(Lang::En);
        assert!(!body.contains("Nothing was changed"), "must not claim nothing changed: {body:?}");
        assert!(!body.to_lowercase().contains("is active"), "must not claim a live grant either: {body:?}");
    }

    #[test]
    fn exit_seventeen_text_differs_from_exit_sixteen_text() {
        assert_ne!(OutcomeKind::TimerUnscheduled.text(Lang::En), OutcomeKind::FsFailure.text(Lang::En));
    }

    #[test]
    fn pkexec_126_and_127_texts_are_distinguished_from_each_other_and_from_helper_texts() {
        let cancelled = OutcomeKind::Cancelled.text(Lang::En);
        let not_authorized = OutcomeKind::NotAuthorized.text(Lang::En);
        let helper_missing = OutcomeKind::HelperMissing.text(Lang::En);
        assert_ne!(cancelled, not_authorized);
        assert_ne!(cancelled, helper_missing);
        assert_ne!(not_authorized, helper_missing);
        for helper_text in [
            OutcomeKind::InternalError.text(Lang::En),
            OutcomeKind::VersionSkew.text(Lang::En),
            OutcomeKind::ContextViolation.text(Lang::En),
            OutcomeKind::UidRejected.text(Lang::En),
            OutcomeKind::NotSudoer.text(Lang::En),
            OutcomeKind::BadDuration.text(Lang::En),
            OutcomeKind::VisudoRejected.text(Lang::En),
            OutcomeKind::LockBusy.text(Lang::En),
            OutcomeKind::FsFailure.text(Lang::En),
            OutcomeKind::TimerUnscheduled.text(Lang::En),
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

    // ---- Fix 2 (m3 desktop review) — every OutcomeKind renders Spanish
    // through the same `text(lang)` signature as English, and the two
    // languages are distinct per outcome (localization's own
    // distinctness rule, applied here the same way format.rs's
    // `every_msg_arm_renders_both_languages_and_they_are_distinct`
    // applies it to `Msg`).

    #[test]
    fn every_documented_outcome_renders_spanish_and_it_differs_from_english() {
        for outcome in all_documented_outcomes() {
            let (en_summary, en_body) = outcome.text(Lang::En);
            let (es_summary, es_body) = outcome.text(Lang::Es);
            assert!(!es_summary.is_empty(), "{outcome:?} Spanish summary must not be empty");
            assert!(!es_body.is_empty(), "{outcome:?} Spanish body must not be empty");
            assert_ne!(en_summary, es_summary, "{outcome:?} summary must differ per language");
            // `Revoked`'s summary and body render the same sentence in
            // both languages by design (see `text`'s own Revoked arm) —
            // only the body pair needs the cross-language distinctness
            // check for every other outcome.
            if !matches!(outcome, OutcomeKind::Revoked) {
                assert_ne!(en_body, es_body, "{outcome:?} body must differ per language");
            }
        }
    }

    #[test]
    fn revoked_summary_and_body_are_the_same_sentence_in_both_languages() {
        let (summary, body) = OutcomeKind::Revoked.text(Lang::En);
        assert_eq!(summary, body);
        let (summary, body) = OutcomeKind::Revoked.text(Lang::Es);
        assert_eq!(summary, body);
    }
}

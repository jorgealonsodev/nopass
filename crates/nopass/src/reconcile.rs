//! The merge — where the cached state file and the live probe meet, and
//! the four reconciliation triggers (design.md §3.2–§3.4; spec
//! `tray-state-sync` "Live Probe Takes Precedence", "Reconciliation Runs
//! at Four Defined Triggers").
//!
//! `merge` is written against the design's 8-row table by matching every
//! [`Probe`] variant and every [`FileReading`] variant BY NAME — never
//! behind a `_` wildcard. A ninth combination (a new `Probe` or
//! `FileReading` variant) therefore fails to compile here rather than
//! silently falling into a default arm. This is the other half of Phase
//! 2's guarantee: the state reader made "absence means inactive"
//! impossible to express in its own type; this exhaustive match is what
//! makes an unreviewed new file/probe combination impossible to merge
//! silently, too.

use nopass_core::expiry::Expiry;
use nopass_core::state::HelperStatus;

use crate::probe::Probe;
use crate::state::FileReading;

/// The tray's own three-valued rendering of "is passwordless sudo active
/// for this user right now". `Unknown` is a first-class member, not an
/// error — see design.md D3/D6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayState {
    Active { user: Option<String>, expiry: Option<Expiry> },
    Inactive,
    Unknown,
}

/// The exhaustive, pure, total merge (design.md §3.2). Every arm names a
/// concrete `Probe` and `FileReading` variant; none of the two outer
/// matches carries a `_` arm.
pub fn merge(file: &FileReading, probe: Option<Probe>, now: u64) -> TrayState {
    match probe {
        // Row 1: the probe confirms a live grant. The file's user/expiry
        // are echoed only when the file itself is a coherent active
        // record — otherwise the honest rendering is "active, remaining
        // time unknown" (design.md §3.2 row 1, the milestone's headline
        // risk).
        Some(Probe::Passwordless) => match file {
            FileReading::Parsed(status) => match coherent_active(status, now) {
                Some(e) => TrayState::Active { user: Some(status.user.clone()), expiry: Some(e) },
                None => TrayState::Active { user: None, expiry: None },
            },
            FileReading::Absent => TrayState::Active { user: None, expiry: None },
            FileReading::Faulted(_) => TrayState::Active { user: None, expiry: None },
        },
        // Row 2: the probe is ground truth and it disagrees with any
        // cached "active" claim the file might make (tray-state-sync
        // "Probe overrides a stale active file"; threat matrix "Stale or
        // forged observation source").
        Some(Probe::PasswordRequired) => TrayState::Inactive,
        // Rows 3–8: no fresh probe — the file alone decides, per the
        // narrower table below.
        None => match file {
            FileReading::Parsed(status) => {
                if !status.active {
                    // Row 6.
                    TrayState::Inactive
                } else {
                    match coherent_active(status, now) {
                        // Row 3.
                        Some(e) => TrayState::Active { user: Some(status.user.clone()), expiry: Some(e) },
                        // Rows 4 (expired) and 5 (no expiry at all) — both
                        // are the file contradicting itself or a shape
                        // the helper never writes; neither is trusted.
                        None => TrayState::Unknown,
                    }
                }
            }
            // Row 7.
            FileReading::Absent => TrayState::Unknown,
            // Row 8 — every fault kind, including `schema != 1`.
            FileReading::Faulted(_) => TrayState::Unknown,
        },
    }
}

/// `Some(e)` iff `status.active` and `status.expires` is a still-live
/// `Expiry` — the one condition under which the file's own claim is
/// trusted. `Never`/`Reboot` are never expired by [`Expiry::is_expired`],
/// so they always qualify; only a past `At { epoch }` and `expires: None`
/// fail this check (design.md §3.2 rows 3–5).
fn coherent_active(status: &HelperStatus, now: u64) -> Option<Expiry> {
    if !status.active {
        return None;
    }
    match status.expires {
        Some(e) if !e.is_expired(now) => Some(e),
        _ => None,
    }
}

/// The four reconciliation triggers (design.md §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Startup,
    FileEvent,
    ActionCompleted,
    MenuOpened,
    Tick,
}

/// The trigger table's "always" half, plus `Unknown`'s "additionally
/// forces a probe on entry regardless of trigger" clause. `MenuOpened`'s
/// "only if the cached probe is unusable" half lives in the caller, which
/// combines this with [`crate::probe::ProbeCache::usable`] — this
/// function alone has no cache to consult.
pub fn probe_required(state: &TrayState, trigger: Trigger) -> bool {
    match trigger {
        Trigger::Startup | Trigger::FileEvent | Trigger::ActionCompleted | Trigger::Tick => true,
        // `Unknown` "additionally forces a probe on entry regardless of
        // trigger" — for `MenuOpened` that clause is the only thing this
        // pure function can decide; the cache-freshness half is the
        // caller's job.
        Trigger::MenuOpened => matches!(state, TrayState::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ReadFault;

    fn status(active: bool, expires: Option<Expiry>) -> HelperStatus {
        HelperStatus {
            schema: 1,
            uid: 1000,
            user: "jorge".to_string(),
            active,
            expires,
            rule_path: "/etc/sudoers.d/90-nopass-1000".to_string(),
            updated_at: 1,
        }
    }

    const NOW: u64 = 1_000_000;

    // ---- Row 1: any FileReading, probe Passwordless ⇒ Active, echoing
    // the file only when it is a coherent active record. ----

    #[test]
    fn missing_file_and_probe_passwordless_yields_active_with_no_expiry() {
        // The milestone's headline risk: the state file is gone entirely,
        // but the probe confirms a live grant.
        let state = merge(&FileReading::Absent, Some(Probe::Passwordless), NOW);
        assert_eq!(state, TrayState::Active { user: None, expiry: None });
    }

    #[test]
    fn coherent_active_file_and_probe_passwordless_echoes_user_and_expiry() {
        let s = status(true, Some(Expiry::At { epoch: NOW + 60 }));
        let file = FileReading::Parsed(s.clone());
        let state = merge(&file, Some(Probe::Passwordless), NOW);
        assert_eq!(
            state,
            TrayState::Active { user: Some("jorge".to_string()), expiry: Some(Expiry::At { epoch: NOW + 60 }) }
        );
    }

    #[test]
    fn inactive_file_and_probe_passwordless_yields_active_with_no_expiry() {
        let file = FileReading::Parsed(status(false, None));
        let state = merge(&file, Some(Probe::Passwordless), NOW);
        assert_eq!(state, TrayState::Active { user: None, expiry: None });
    }

    #[test]
    fn expired_active_file_and_probe_passwordless_yields_active_with_no_expiry() {
        let file = FileReading::Parsed(status(true, Some(Expiry::At { epoch: NOW - 1 })));
        let state = merge(&file, Some(Probe::Passwordless), NOW);
        assert_eq!(state, TrayState::Active { user: None, expiry: None });
    }

    #[test]
    fn active_file_with_no_expiry_and_probe_passwordless_yields_active_with_no_expiry() {
        let file = FileReading::Parsed(status(true, None));
        let state = merge(&file, Some(Probe::Passwordless), NOW);
        assert_eq!(state, TrayState::Active { user: None, expiry: None });
    }

    #[test]
    fn faulted_file_and_probe_passwordless_yields_active_with_no_expiry() {
        let file = FileReading::Faulted(ReadFault::Io);
        let state = merge(&file, Some(Probe::Passwordless), NOW);
        assert_eq!(state, TrayState::Active { user: None, expiry: None });
    }

    // ---- Row 2: any FileReading, probe PasswordRequired ⇒ Inactive. ----

    #[test]
    fn probe_password_required_always_yields_inactive_regardless_of_file() {
        for file in [
            FileReading::Absent,
            FileReading::Faulted(ReadFault::Malformed),
            FileReading::Parsed(status(false, None)),
            FileReading::Parsed(status(true, Some(Expiry::Never))),
        ] {
            assert_eq!(merge(&file, Some(Probe::PasswordRequired), NOW), TrayState::Inactive);
        }
    }

    /// Threat matrix "Stale or forged observation source" / "State
    /// misrepresentation": a state file claiming an active grant must
    /// never override a probe that says otherwise (design threat matrix
    /// row 6; tray-state-sync "Probe overrides a stale active file").
    ///
    /// `/run/nopass/` is `0755 root:root` (M1 tmpfiles), so an
    /// unprivileged peer cannot create or replace `<uid>.state` to force
    /// this scenario in the first place — that half of the threat matrix
    /// row is an environment invariant, not something provable by this
    /// unprivileged test suite. This test proves the half that IS
    /// testable here: even if the file's own claim disagrees with the
    /// probe, the probe wins.
    #[test]
    fn threat_matrix_probe_overrides_a_state_file_claiming_active() {
        let file = FileReading::Parsed(status(true, Some(Expiry::At { epoch: NOW + 3600 })));
        let state = merge(&file, Some(Probe::PasswordRequired), NOW);
        assert_eq!(state, TrayState::Inactive);
    }

    // ---- Rows 3–8: probe is None; the file alone decides. ----

    #[test]
    fn row_3_coherent_active_file_with_no_probe_yields_active_with_expiry() {
        let e = Expiry::At { epoch: NOW + 60 };
        let file = FileReading::Parsed(status(true, Some(e)));
        let state = merge(&file, None, NOW);
        assert_eq!(state, TrayState::Active { user: Some("jorge".to_string()), expiry: Some(e) });
    }

    #[test]
    fn row_4_active_file_with_a_passed_expiry_and_no_probe_yields_unknown() {
        let file = FileReading::Parsed(status(true, Some(Expiry::At { epoch: NOW - 1 })));
        let state = merge(&file, None, NOW);
        assert_eq!(state, TrayState::Unknown, "a self-contradicting file must never be trusted as Active");
    }

    #[test]
    fn row_5_active_file_with_no_expiry_and_no_probe_yields_unknown() {
        // The helper never writes this combination — treat as corrupt,
        // never as active-forever.
        let file = FileReading::Parsed(status(true, None));
        let state = merge(&file, None, NOW);
        assert_eq!(state, TrayState::Unknown);
    }

    #[test]
    fn row_6_inactive_file_and_no_probe_yields_inactive() {
        let file = FileReading::Parsed(status(false, None));
        assert_eq!(merge(&file, None, NOW), TrayState::Inactive);
        // `expires` is irrelevant once `active` is false — design row 6
        // makes no distinction, so both values collapse to the same row.
        let file_with_stale_expiry = FileReading::Parsed(status(false, Some(Expiry::Reboot)));
        assert_eq!(merge(&file_with_stale_expiry, None, NOW), TrayState::Inactive);
    }

    #[test]
    fn row_7_absent_file_and_no_probe_yields_unknown() {
        assert_eq!(merge(&FileReading::Absent, None, NOW), TrayState::Unknown);
    }

    #[test]
    fn row_8_every_fault_kind_and_no_probe_yields_unknown() {
        for fault in [ReadFault::Io, ReadFault::Malformed, ReadFault::UnsupportedSchema { found: 2 }] {
            assert_eq!(merge(&FileReading::Faulted(fault), None, NOW), TrayState::Unknown);
        }
    }

    #[test]
    fn never_and_reboot_expiries_under_no_probe_are_treated_as_row_3_coherent_active() {
        for e in [Expiry::Never, Expiry::Reboot] {
            let file = FileReading::Parsed(status(true, Some(e)));
            let state = merge(&file, None, NOW);
            assert_eq!(state, TrayState::Active { user: Some("jorge".to_string()), expiry: Some(e) });
        }
    }

    // ---- probe_required: the four "always" triggers, MenuOpened's
    // Unknown-forces-a-probe clause, and Unknown forcing a probe under
    // every trigger. ----

    #[test]
    fn every_trigger_except_menu_opened_always_requires_a_probe_regardless_of_state() {
        for trigger in [Trigger::Startup, Trigger::FileEvent, Trigger::ActionCompleted, Trigger::Tick] {
            for state in [
                TrayState::Unknown,
                TrayState::Inactive,
                TrayState::Active { user: None, expiry: None },
            ] {
                assert!(probe_required(&state, trigger), "{trigger:?} over {state:?} must always probe");
            }
        }
    }

    #[test]
    fn menu_opened_with_unknown_state_forces_a_probe() {
        assert!(probe_required(&TrayState::Unknown, Trigger::MenuOpened));
    }

    #[test]
    fn menu_opened_with_a_resolved_state_does_not_force_a_probe_on_its_own() {
        // The cache-freshness half of MenuOpened's rule is the caller's
        // job (composed with `ProbeCache::usable`); `probe_required`
        // alone answers only the state-forces-it half.
        assert!(!probe_required(&TrayState::Inactive, Trigger::MenuOpened));
        assert!(!probe_required(&TrayState::Active { user: None, expiry: None }, Trigger::MenuOpened));
    }

    /// Composes `probe_required` with [`crate::probe::ProbeCache::usable`]
    /// the way the app loop will (Phase 10), with a fake invocation
    /// counter, proving each of the four "always" triggers probes exactly
    /// once and `MenuOpened` probes only when the cache is unusable
    /// (tray-state-sync "Reconciliation Runs at Four Defined Triggers").
    #[test]
    fn each_trigger_invokes_the_probe_port_exactly_once_per_the_table() {
        use crate::probe::ProbeCache;
        use std::cell::Cell;

        let invocations = Cell::new(0u32);
        let probe_once = |state: &TrayState, trigger: Trigger, cache: Option<ProbeCache>, file_observed_at, now| {
            let forced = probe_required(state, trigger);
            let cache_stale = trigger == Trigger::MenuOpened
                && cache.and_then(|c| c.usable(file_observed_at, now)).is_none();
            if forced || cache_stale {
                invocations.set(invocations.get() + 1);
            }
        };

        let resolved = TrayState::Active { user: None, expiry: None };
        let fresh_cache = ProbeCache { value: Probe::Passwordless, taken_at: 100 };

        probe_once(&resolved, Trigger::Startup, Some(fresh_cache), 0, 100);
        probe_once(&resolved, Trigger::FileEvent, Some(fresh_cache), 0, 100);
        probe_once(&resolved, Trigger::ActionCompleted, Some(fresh_cache), 0, 100);
        probe_once(&resolved, Trigger::Tick, Some(fresh_cache), 0, 100);
        assert_eq!(invocations.get(), 4, "each always-trigger must probe exactly once");

        // MenuOpened with a fresh, usable cache: no extra probe.
        probe_once(&resolved, Trigger::MenuOpened, Some(fresh_cache), 0, 100);
        assert_eq!(invocations.get(), 4, "MenuOpened must not probe when the cache is still usable");

        // MenuOpened with a stale cache: one more probe.
        probe_once(&resolved, Trigger::MenuOpened, Some(fresh_cache), 0, 100 + MAX_PROBE_AGE_SECS_PLUS_ONE);
        assert_eq!(invocations.get(), 5, "MenuOpened must probe once the cache is unusable");
    }

    const MAX_PROBE_AGE_SECS_PLUS_ONE: u64 = crate::probe::MAX_PROBE_AGE_SECS + 1;
}

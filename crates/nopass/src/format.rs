//! Every user-facing string the tray renders lives here (design.md §7.1,
//! §7.2; spec `tray-presence` "Three Visual States With Distinct Icon
//! Names", "Tooltip Recomputed Only At Existing Wake Points"). M3 replaces
//! this module's body with `rust-i18n` without touching a single call
//! site outside it.
//!
//! Phase 6 adds [`icon_name`] — the theme-aware icon resolution table.
//! Phase 7 adds [`countdown`], [`tooltip`], [`status_line`], and
//! [`toggle_label`] — the tooltip/countdown/menu-label formatting that
//! `tray.rs`'s [`crate::tray::ViewModel`] is built from.

use nopass_core::expiry::Expiry;

use crate::reconcile::TrayState;

/// Whether a caller wants the monochrome panel variant (the default, per
/// design.md §7.1: panel tray icons are monochrome by convention on both
/// GNOME and KDE) or the full-colour asset. Overridden by
/// `NOPASS_ICON_STYLE=color|symbolic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconStyle {
    Color,
    Symbolic,
}

/// Reads `NOPASS_ICON_STYLE` once per call. Any value other than exactly
/// `color` — including absence — resolves to [`IconStyle::Symbolic`],
/// the documented default.
fn icon_style_from_env() -> IconStyle {
    match std::env::var("NOPASS_ICON_STYLE").ok().as_deref() {
        Some("color") => IconStyle::Color,
        _ => IconStyle::Symbolic,
    }
}

/// The theme-aware icon name for the current [`TrayState`] (design.md
/// §7.1's table; spec `tray-presence` "Icon name follows the merged state
/// exactly"). `Unknown` deliberately renders the stock
/// `dialog-question-symbolic` name regardless of style — design.md D6: a
/// padlock glyph during an unresolved state is the exact visual lie this
/// milestone exists to prevent, and a stock name costs no new asset.
pub fn icon_name(state: &TrayState) -> &'static str {
    icon_name_for_style(state, icon_style_from_env())
}

/// [`icon_name`]'s table, parameterised by an explicit [`IconStyle`]
/// rather than reading the environment — the seam this module's own
/// tests use to exercise both styles deterministically. `std::env::var`
/// reads are safe, but *setting* `NOPASS_ICON_STYLE` from a test would
/// require `std::env::set_var`, which is `unsafe` under the 2024 edition
/// and forbidden by this crate's `#![forbid(unsafe_code)]` — so the
/// table itself, not the environment lookup, is what gets exercised
/// per-style.
fn icon_name_for_style(state: &TrayState, style: IconStyle) -> &'static str {
    match state {
        TrayState::Unknown => "dialog-question-symbolic",
        TrayState::Inactive => match style {
            IconStyle::Symbolic => "nopass-locked-symbolic",
            IconStyle::Color => "nopass-locked",
        },
        TrayState::Active { expiry: Some(Expiry::At { .. }), .. } => match style {
            IconStyle::Symbolic => "nopass-unlocked-timed-symbolic",
            IconStyle::Color => "nopass-unlocked-timed",
        },
        // `Never`, `Reboot`, and `expiry: None` (design.md §3.2 row 1's
        // "active, remaining time unknown" value) all render the same
        // plain unlocked icon — none of them carries a countdown.
        TrayState::Active { .. } => match style {
            IconStyle::Symbolic => "nopass-unlocked-symbolic",
            IconStyle::Color => "nopass-unlocked",
        },
    }
}

/// The remaining-time text for a concrete [`Expiry`] (design.md §7.2, D7).
/// **Floors** to whole minutes — rounding up would advertise time the
/// user does not have, which is the wrong direction for a security
/// countdown. Worst-case staleness equals the 60 s reconciliation period,
/// which equals this function's own granularity, so the difference is
/// never observable.
pub fn countdown(expiry: Expiry, now: u64) -> String {
    match expiry {
        Expiry::Never => "no expiry".to_string(),
        Expiry::Reboot => "until reboot".to_string(),
        Expiry::At { epoch } => {
            if epoch <= now {
                // Matches `Expiry::is_expired`'s own `<=`: a countdown
                // that has reached (or passed) zero is rendered exactly
                // like a countdown for an already-past epoch.
                "expired".to_string()
            } else {
                let remaining_secs = epoch - now;
                let minutes = remaining_secs / 60;
                if minutes == 0 {
                    "less than a minute".to_string()
                } else {
                    format!("{minutes} min")
                }
            }
        }
    }
}

/// The SNI hover tooltip (design.md §2 `format`; spec `tray-presence`
/// "Tooltip Recomputed Only At Existing Wake Points"). The body is the
/// same rendered text [`status_line`] gives the minimal menu's insensitive
/// `Status` item — design.md §7.2 says the menu label "never renders a
/// state the tooltip does not", which this shared implementation makes
/// true by construction rather than by two call sites staying in sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolTip {
    pub title: String,
    pub body: String,
}

pub fn tooltip(user: &str, state: &TrayState, now: u64) -> ToolTip {
    ToolTip { title: "NoPass".to_string(), body: status_line(user, state, now) }
}

/// `<user> — <state> (<remaining>)` for `Active` (spec `tray-presence`
/// "Tooltip renders remaining time at minute granularity"); `Inactive`
/// and `Unknown` carry no remaining-time concept, so their rendering
/// omits the parenthesised remainder rather than inventing one.
pub fn status_line(user: &str, state: &TrayState, now: u64) -> String {
    match state {
        TrayState::Active { expiry, .. } => {
            let remaining = match expiry {
                Some(e) => countdown(*e, now),
                // design.md §3.2 row 1: the probe confirmed a live grant
                // but the file could not corroborate a remaining time —
                // the honest rendering is "unknown", never a guess.
                None => "remaining time unknown".to_string(),
            };
            format!("{user} — Active ({remaining})")
        }
        TrayState::Inactive => format!("{user} — Inactive"),
        TrayState::Unknown => format!("{user} — Checking…"),
    }
}

/// The minimal menu's toggle label (design.md §7.2, D6). Returns `None`
/// for `Unknown` — a type-level consequence, not a UI preference: the
/// label is state-dependent, so in `Unknown` it is underivable, and
/// offering a mislabeled privileged action is worse than offering none.
pub fn toggle_label(state: &TrayState) -> Option<&'static str> {
    match state {
        TrayState::Inactive => Some("Enable passwordless sudo"),
        TrayState::Active { .. } => Some("Disable passwordless sudo"),
        TrayState::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(expiry: Option<Expiry>) -> TrayState {
        TrayState::Active { user: Some("jorge".to_string()), expiry }
    }

    // ---- icon_name / icon_name_for_style (Phase 6, task 6.4) ----

    #[test]
    fn inactive_resolves_to_the_locked_icon_in_both_styles() {
        assert_eq!(icon_name_for_style(&TrayState::Inactive, IconStyle::Symbolic), "nopass-locked-symbolic");
        assert_eq!(icon_name_for_style(&TrayState::Inactive, IconStyle::Color), "nopass-locked");
    }

    #[test]
    fn active_with_an_at_expiry_resolves_to_the_timed_icon_in_both_styles() {
        let state = active(Some(Expiry::At { epoch: 1_000 }));
        assert_eq!(icon_name_for_style(&state, IconStyle::Symbolic), "nopass-unlocked-timed-symbolic");
        assert_eq!(icon_name_for_style(&state, IconStyle::Color), "nopass-unlocked-timed");
    }

    #[test]
    fn active_with_never_reboot_or_no_expiry_resolves_to_the_plain_unlocked_icon_in_both_styles() {
        for expiry in [Some(Expiry::Never), Some(Expiry::Reboot), None] {
            let state = active(expiry);
            assert_eq!(
                icon_name_for_style(&state, IconStyle::Symbolic),
                "nopass-unlocked-symbolic",
                "{expiry:?} must resolve to the symbolic unlocked icon"
            );
            assert_eq!(
                icon_name_for_style(&state, IconStyle::Color),
                "nopass-unlocked",
                "{expiry:?} must resolve to the colour unlocked icon"
            );
        }
    }

    #[test]
    fn unknown_always_resolves_to_the_stock_question_icon_regardless_of_style() {
        assert_eq!(icon_name_for_style(&TrayState::Unknown, IconStyle::Symbolic), "dialog-question-symbolic");
        assert_eq!(icon_name_for_style(&TrayState::Unknown, IconStyle::Color), "dialog-question-symbolic");
    }

    #[test]
    fn unknown_never_reuses_the_locked_icon() {
        // design.md D6: rendering Unknown with a closed padlock is the
        // exact visual lie this milestone exists to prevent.
        for style in [IconStyle::Symbolic, IconStyle::Color] {
            let name = icon_name_for_style(&TrayState::Unknown, style);
            assert!(!name.starts_with("nopass-locked"), "Unknown must never render {name}");
        }
    }

    #[test]
    fn the_public_icon_name_defaults_to_symbolic_without_the_env_override() {
        // This crate never calls `std::env::set_var` (forbidden by
        // `#![forbid(unsafe_code)]`), so `NOPASS_ICON_STYLE` stays absent
        // for the lifetime of this test binary and `icon_name` exercises
        // its real, non-parameterised env lookup.
        assert_eq!(icon_name(&TrayState::Inactive), "nopass-locked-symbolic");
    }

    // ---- countdown (Phase 7, task 7.1) ----

    #[test]
    fn countdown_floors_to_whole_minutes_across_the_documented_boundaries() {
        const NOW: u64 = 1_000_000;
        let cases: [(u64, &str); 8] = [
            (0, "expired"),
            (1, "less than a minute"),
            (59, "less than a minute"),
            (60, "1 min"),
            (61, "1 min"),
            (3599, "59 min"),
            (3600, "60 min"),
            (28_800, "480 min"),
        ];
        for (remaining_secs, expected) in cases {
            let epoch = NOW + remaining_secs;
            assert_eq!(
                countdown(Expiry::At { epoch }, NOW),
                expected,
                "{remaining_secs}s remaining must render {expected:?}"
            );
        }
    }

    #[test]
    fn countdown_renders_expired_for_a_past_epoch() {
        const NOW: u64 = 1_000_000;
        assert_eq!(countdown(Expiry::At { epoch: NOW - 1 }, NOW), "expired");
    }

    #[test]
    fn countdown_renders_never_and_reboot_without_any_arithmetic() {
        assert_eq!(countdown(Expiry::Never, 0), "no expiry");
        assert_eq!(countdown(Expiry::Reboot, 0), "until reboot");
    }

    // ---- tooltip / status_line / toggle_label (Phase 7, task 7.2) ----

    #[test]
    fn status_line_renders_user_state_and_remaining_for_an_active_temporary_grant() {
        const NOW: u64 = 1_000_000;
        let state = active(Some(Expiry::At { epoch: NOW + 60 * 42 }));
        assert_eq!(status_line("jorge", &state, NOW), "jorge — Active (42 min)");
    }

    #[test]
    fn status_line_reports_remaining_time_unknown_when_the_probe_confirms_activity_with_no_file_expiry() {
        let state = active(None);
        assert_eq!(status_line("jorge", &state, 0), "jorge — Active (remaining time unknown)");
    }

    #[test]
    fn status_line_renders_inactive_and_unknown_without_a_remaining_clause() {
        assert_eq!(status_line("jorge", &TrayState::Inactive, 0), "jorge — Inactive");
        assert_eq!(status_line("jorge", &TrayState::Unknown, 0), "jorge — Checking…");
    }

    #[test]
    fn tooltip_body_matches_status_line_exactly() {
        const NOW: u64 = 1_000_000;
        let state = active(Some(Expiry::At { epoch: NOW + 120 }));
        let t = tooltip("jorge", &state, NOW);
        assert_eq!(t.body, status_line("jorge", &state, NOW));
        assert_eq!(t.title, "NoPass");
    }

    #[test]
    fn toggle_label_offers_enable_when_inactive_and_disable_when_active() {
        assert_eq!(toggle_label(&TrayState::Inactive), Some("Enable passwordless sudo"));
        assert_eq!(toggle_label(&active(Some(Expiry::Never))), Some("Disable passwordless sudo"));
        assert_eq!(toggle_label(&active(None)), Some("Disable passwordless sudo"));
    }

    #[test]
    fn toggle_label_is_none_for_unknown() {
        // design.md D6: a state whose label cannot be derived must not
        // be actionable.
        assert_eq!(toggle_label(&TrayState::Unknown), None);
    }
}

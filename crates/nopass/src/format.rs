//! Every user-facing string the tray renders lives here (design.md §7.1,
//! §7.2; spec `tray-presence` "Three Visual States With Distinct Icon
//! Names", "Tooltip Recomputed Only At Existing Wake Points"). M3 replaces
//! this module's body with a hand-written `Lang`/`Msg` `match` catalogue —
//! **not** `rust-i18n` (design.md §0 D2: the language set is two and
//! closed, MSRV risk, binary size, and — the decisive reason — a `match`
//! makes an untranslated string a compile error, where a missing catalogue
//! key is a silent runtime fallback) — without touching a single call
//! site outside it.
//!
//! Phase 6 adds [`icon_name`] — the theme-aware icon resolution table.
//! Phase 7 adds [`countdown`], [`tooltip`], [`status_line`], and
//! [`toggle_label`] — the tooltip/countdown/menu-label formatting that
//! `tray.rs`'s [`crate::tray::ViewModel`] is built from. Phase 5 localizes
//! all four (localization "Catalogue Behind format.rs, Call Sites
//! Unchanged"): every public signature above is unchanged, so no caller in
//! `tray.rs`/`app.rs` needs to change to receive the localized rendering.

use std::sync::OnceLock;

use nopass_core::expiry::Expiry;

use crate::reconcile::TrayState;

/// The resolved UI language — a closed set of exactly two locales
/// (design.md §0 D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Es,
}

impl Lang {
    /// Resolves the process's UI language: `LC_ALL` > `LC_MESSAGES` >
    /// `LANG`, first non-empty value naming a Spanish locale wins
    /// (localization "Locale Resolution Order With English Fallback").
    /// Absence of all three, or any other outcome, resolves to English.
    pub fn from_env() -> Lang {
        Lang::from_env_vars(
            std::env::var("LC_ALL").ok().as_deref(),
            std::env::var("LC_MESSAGES").ok().as_deref(),
            std::env::var("LANG").ok().as_deref(),
        )
    }

    /// [`Lang::from_env`]'s precedence table, parameterised by explicit
    /// values rather than reading the environment — the seam this module's
    /// own tests drive deterministically. This crate is
    /// `#![forbid(unsafe_code)]`, so a test cannot `std::env::set_var` to
    /// exercise the three-variable precedence order directly.
    ///
    /// The first candidate that is present and non-empty decides the
    /// answer outright — Spanish if its own value names a Spanish locale,
    /// English otherwise — without falling through to the next candidate.
    /// Only an absent or empty candidate is skipped in favour of the next
    /// one (localization "LC_ALL takes precedence over LC_MESSAGES and
    /// LANG": `LC_ALL=en_US.UTF-8` must resolve English even though
    /// `LC_MESSAGES=es_ES.UTF-8` — it does not get consulted at all).
    fn from_env_vars(lc_all: Option<&str>, lc_messages: Option<&str>, lang: Option<&str>) -> Lang {
        for candidate in [lc_all, lc_messages, lang] {
            match candidate {
                Some(value) if !value.is_empty() => {
                    return Lang::from_locale_string(Some(value)).unwrap_or(Lang::En);
                }
                _ => continue,
            }
        }
        Lang::En
    }

    /// A single locale value's language subtag, taken up to the first
    /// `.`/`_`/`@` (`es_ES.UTF-8` ⇒ `es`). Exactly `es` resolves to
    /// [`Lang::Es`]; absence, empty, `C`, `POSIX`, or any other subtag
    /// resolves to `None` — the caller falls through to the next candidate,
    /// or to English if none remain.
    fn from_locale_string(raw: Option<&str>) -> Option<Lang> {
        let raw = raw?;
        let subtag = raw.split(['.', '_', '@']).next().unwrap_or(raw);
        match subtag {
            "es" => Some(Lang::Es),
            _ => None,
        }
    }
}

/// Every user-facing string this crate renders, one arm per string, both
/// languages required to compile — design.md §0 D2's decisive reason to
/// choose a `match` over an i18n crate: an arm that omits [`Lang::Es`]
/// does not compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Msg {
    EnablePasswordlessSudo,
    DisablePasswordlessSudo,
    StatusActive,
    StatusInactive,
    StatusChecking,
    RemainingTimeUnknown,
    NoExpiry,
    UntilReboot,
    Expired,
    LessThanAMinute,
    MinutesSuffix,
    // ---- Phase 6 (menu.rs) — top-level menu item labels ----
    MenuActivateDuring,
    MenuDefaultDuration,
    MenuCurrentRule,
    MenuNoActiveRule,
    MenuRuleDetailsUnavailable,
    MenuStartWithSession,
    MenuAbout,
    MenuQuit,
    // ---- Phase 6 — "Current rule" detail-item prefixes ----
    RuleUserPrefix,
    RulePathPrefix,
    RuleExpiresPrefix,
    RuleRemainingPrefix,
    // ---- Phase 6 — duration display labels (menu.rs's "Activate during…"
    // and "Default duration" submenus; distinct from the countdown-context
    // Msg::UntilReboot above) ----
    DurationLabelMinutes15,
    DurationLabelHour1,
    DurationLabelHours4,
    DurationLabelHours8,
    DurationLabelUntilReboot,
    DurationLabelPermanent,
    // ---- Phase 6 — the consent branch (design.md §1 "The consent branch") ----
    ConsentWarningTitle,
    ConsentWarningBody,
    ConsentConfirmOncePrefix,
    ConsentConfirmPersist,
    ConsentCancel,
    // ---- Phase 8 (task 8.4/8.6, design.md §0 D6) — the toggle's
    // "unavailable, with a reason" labels; `StateUnknown` reuses
    // `Msg::StatusChecking` rather than a fourth arm, since it renders
    // the exact same "still finding out" idea the status line already
    // says.
    ToggleUnavailableInstallationIncomplete,
    ToggleUnavailableActionInFlight,
}

impl Msg {
    /// `pub(crate)`, not private: `menu.rs` (Phase 6) renders its own
    /// labels through this same catalogue rather than duplicating a
    /// second `Lang`-aware table — "every user-visible label goes
    /// through `Msg`" applies crate-wide, not just within this module.
    pub(crate) fn text(self, lang: Lang) -> &'static str {
        match self {
            Msg::EnablePasswordlessSudo => match lang {
                Lang::En => "Enable passwordless sudo",
                Lang::Es => "Activar sudo sin contraseña",
            },
            Msg::DisablePasswordlessSudo => match lang {
                Lang::En => "Disable passwordless sudo",
                Lang::Es => "Desactivar sudo sin contraseña",
            },
            Msg::StatusActive => match lang {
                Lang::En => "Active",
                Lang::Es => "Activo",
            },
            Msg::StatusInactive => match lang {
                Lang::En => "Inactive",
                Lang::Es => "Inactivo",
            },
            Msg::StatusChecking => match lang {
                Lang::En => "Checking…",
                Lang::Es => "Comprobando…",
            },
            Msg::RemainingTimeUnknown => match lang {
                Lang::En => "remaining time unknown",
                Lang::Es => "tiempo restante desconocido",
            },
            Msg::NoExpiry => match lang {
                Lang::En => "no expiry",
                Lang::Es => "sin caducidad",
            },
            Msg::UntilReboot => match lang {
                Lang::En => "until reboot",
                Lang::Es => "hasta el reinicio",
            },
            Msg::Expired => match lang {
                Lang::En => "expired",
                Lang::Es => "caducado",
            },
            Msg::LessThanAMinute => match lang {
                Lang::En => "less than a minute",
                Lang::Es => "menos de un minuto",
            },
            Msg::MinutesSuffix => match lang {
                Lang::En => "min",
                // Trailing period is the ordinary Spanish typographic
                // convention for an abbreviation — also what keeps this
                // arm's rendered pair distinct from English's "min".
                Lang::Es => "min.",
            },
            Msg::MenuActivateDuring => match lang {
                Lang::En => "Activate during…",
                Lang::Es => "Activar durante…",
            },
            Msg::MenuDefaultDuration => match lang {
                Lang::En => "Default duration",
                Lang::Es => "Duración predeterminada",
            },
            Msg::MenuCurrentRule => match lang {
                Lang::En => "Current rule",
                Lang::Es => "Regla actual",
            },
            Msg::MenuNoActiveRule => match lang {
                Lang::En => "No active rule",
                Lang::Es => "Sin regla activa",
            },
            Msg::MenuRuleDetailsUnavailable => match lang {
                Lang::En => "Rule details unavailable — the state file could not be read",
                Lang::Es => "Detalles de la regla no disponibles: no se pudo leer el archivo de estado",
            },
            Msg::MenuStartWithSession => match lang {
                Lang::En => "Start with session",
                Lang::Es => "Iniciar con la sesión",
            },
            Msg::MenuAbout => match lang {
                Lang::En => "About",
                Lang::Es => "Acerca de",
            },
            Msg::MenuQuit => match lang {
                Lang::En => "Quit",
                Lang::Es => "Salir",
            },
            Msg::RuleUserPrefix => match lang {
                Lang::En => "User",
                Lang::Es => "Usuario",
            },
            Msg::RulePathPrefix => match lang {
                Lang::En => "Rule",
                Lang::Es => "Regla",
            },
            Msg::RuleExpiresPrefix => match lang {
                Lang::En => "Expires",
                Lang::Es => "Caduca",
            },
            Msg::RuleRemainingPrefix => match lang {
                Lang::En => "Remaining",
                Lang::Es => "Restante",
            },
            Msg::DurationLabelMinutes15 => match lang {
                Lang::En => "15 minutes",
                Lang::Es => "15 minutos",
            },
            Msg::DurationLabelHour1 => match lang {
                Lang::En => "1 hour",
                Lang::Es => "1 hora",
            },
            Msg::DurationLabelHours4 => match lang {
                Lang::En => "4 hours",
                Lang::Es => "4 horas",
            },
            Msg::DurationLabelHours8 => match lang {
                Lang::En => "8 hours",
                Lang::Es => "8 horas",
            },
            Msg::DurationLabelUntilReboot => match lang {
                Lang::En => "Until reboot",
                Lang::Es => "Hasta el reinicio",
            },
            Msg::DurationLabelPermanent => match lang {
                Lang::En => "Permanently",
                Lang::Es => "Permanentemente",
            },
            Msg::ConsentWarningTitle => match lang {
                Lang::En => "⚠ Read this before activating",
                Lang::Es => "⚠ Lee esto antes de activar",
            },
            Msg::ConsentWarningBody => match lang {
                Lang::En => {
                    "Any program running as you can become root without a password until this expires."
                }
                Lang::Es => {
                    "Cualquier programa que se ejecute como tú podrá convertirse en root sin contraseña \
                     hasta que esto caduque."
                }
            },
            Msg::ConsentConfirmOncePrefix => match lang {
                Lang::En => "I understand — activate for",
                Lang::Es => "Entendido: activar durante",
            },
            Msg::ConsentConfirmPersist => match lang {
                Lang::En => "I understand — activate and don't warn me again",
                Lang::Es => "Entendido: activar y no volver a advertirme",
            },
            Msg::ConsentCancel => match lang {
                Lang::En => "Cancel",
                Lang::Es => "Cancelar",
            },
            Msg::ToggleUnavailableInstallationIncomplete => match lang {
                Lang::En => "Unavailable — installation incomplete",
                Lang::Es => "No disponible: instalación incompleta",
            },
            Msg::ToggleUnavailableActionInFlight => match lang {
                Lang::En => "Unavailable — an action is already in progress",
                Lang::Es => "No disponible: ya hay una acción en curso",
            },
        }
    }
}

/// The process-wide resolved language, read from the environment exactly
/// once per process (`OnceLock` — deliberate, and only possible to test via
/// the `_in`-suffixed variants below, since this crate's
/// `#![forbid(unsafe_code)]` makes `std::env::set_var` unavailable to
/// simulate a locale change from within a test).
pub(crate) fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(Lang::from_env)
}

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
    countdown_in(lang(), expiry, now)
}

/// [`countdown`]'s implementation, parameterised by an explicit [`Lang`]
/// rather than the process-wide [`lang`] — the seam this module's own
/// tests use to exercise both languages deterministically.
pub(crate) fn countdown_in(lang: Lang, expiry: Expiry, now: u64) -> String {
    match expiry {
        Expiry::Never => Msg::NoExpiry.text(lang).to_string(),
        Expiry::Reboot => Msg::UntilReboot.text(lang).to_string(),
        Expiry::At { epoch } => {
            if epoch <= now {
                // Matches `Expiry::is_expired`'s own `<=`: a countdown
                // that has reached (or passed) zero is rendered exactly
                // like a countdown for an already-past epoch.
                Msg::Expired.text(lang).to_string()
            } else {
                let remaining_secs = epoch - now;
                let minutes = remaining_secs / 60;
                if minutes == 0 {
                    Msg::LessThanAMinute.text(lang).to_string()
                } else {
                    format!("{minutes} {}", Msg::MinutesSuffix.text(lang))
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
    tooltip_in(lang(), user, state, now)
}

/// [`tooltip`]'s implementation, parameterised by an explicit [`Lang`].
pub(crate) fn tooltip_in(lang: Lang, user: &str, state: &TrayState, now: u64) -> ToolTip {
    ToolTip { title: "NoPass".to_string(), body: status_line_in(lang, user, state, now) }
}

/// `<user> — <state> (<remaining>)` for `Active` (spec `tray-presence`
/// "Tooltip renders remaining time at minute granularity"); `Inactive`
/// and `Unknown` carry no remaining-time concept, so their rendering
/// omits the parenthesised remainder rather than inventing one.
pub fn status_line(user: &str, state: &TrayState, now: u64) -> String {
    status_line_in(lang(), user, state, now)
}

/// [`status_line`]'s implementation, parameterised by an explicit [`Lang`]
/// — the seam this module's own tests use to exercise both languages
/// deterministically (localization "A Spanish locale changes format.rs
/// output without touching callers").
pub(crate) fn status_line_in(lang: Lang, user: &str, state: &TrayState, now: u64) -> String {
    match state {
        TrayState::Active { expiry, .. } => {
            let remaining = match expiry {
                Some(e) => countdown_in(lang, *e, now),
                // design.md §3.2 row 1: the probe confirmed a live grant
                // but the file could not corroborate a remaining time —
                // the honest rendering is "unknown", never a guess.
                None => Msg::RemainingTimeUnknown.text(lang).to_string(),
            };
            format!("{user} — {} ({remaining})", Msg::StatusActive.text(lang))
        }
        TrayState::Inactive => format!("{user} — {}", Msg::StatusInactive.text(lang)),
        TrayState::Unknown => format!("{user} — {}", Msg::StatusChecking.text(lang)),
    }
}

/// The minimal menu's toggle label (design.md §7.2, D6). Returns `None`
/// for `Unknown` — a type-level consequence, not a UI preference: the
/// label is state-dependent, so in `Unknown` it is underivable, and
/// offering a mislabeled privileged action is worse than offering none.
pub fn toggle_label(state: &TrayState) -> Option<&'static str> {
    toggle_label_in(lang(), state)
}

/// [`toggle_label`]'s implementation, parameterised by an explicit
/// [`Lang`].
pub(crate) fn toggle_label_in(lang: Lang, state: &TrayState) -> Option<&'static str> {
    match state {
        TrayState::Inactive => Some(Msg::EnablePasswordlessSudo.text(lang)),
        TrayState::Active { .. } => Some(Msg::DisablePasswordlessSudo.text(lang)),
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
    //
    // These drive the `_in` variant with an explicit `Lang::En` rather
    // than the public `countdown`, so their English-literal assertions
    // stay correct regardless of this test binary's real process locale
    // (Phase 5, task 5.1 — the same reason `icon_name_for_style` above is
    // tested via its explicit-parameter seam instead of the env-reading
    // public fn).

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
                countdown_in(Lang::En, Expiry::At { epoch }, NOW),
                expected,
                "{remaining_secs}s remaining must render {expected:?}"
            );
        }
    }

    #[test]
    fn countdown_renders_expired_for_a_past_epoch() {
        const NOW: u64 = 1_000_000;
        assert_eq!(countdown_in(Lang::En, Expiry::At { epoch: NOW - 1 }, NOW), "expired");
    }

    #[test]
    fn countdown_renders_never_and_reboot_without_any_arithmetic() {
        assert_eq!(countdown_in(Lang::En, Expiry::Never, 0), "no expiry");
        assert_eq!(countdown_in(Lang::En, Expiry::Reboot, 0), "until reboot");
    }

    // ---- tooltip / status_line / toggle_label (Phase 7, task 7.2) ----
    //
    // Same rationale as countdown above: `_in` with an explicit `Lang::En`
    // keeps these English-literal assertions locale-independent.

    #[test]
    fn status_line_renders_user_state_and_remaining_for_an_active_temporary_grant() {
        const NOW: u64 = 1_000_000;
        let state = active(Some(Expiry::At { epoch: NOW + 60 * 42 }));
        assert_eq!(status_line_in(Lang::En, "jorge", &state, NOW), "jorge — Active (42 min)");
    }

    #[test]
    fn status_line_reports_remaining_time_unknown_when_the_probe_confirms_activity_with_no_file_expiry() {
        let state = active(None);
        assert_eq!(status_line_in(Lang::En, "jorge", &state, 0), "jorge — Active (remaining time unknown)");
    }

    #[test]
    fn status_line_renders_inactive_and_unknown_without_a_remaining_clause() {
        assert_eq!(status_line_in(Lang::En, "jorge", &TrayState::Inactive, 0), "jorge — Inactive");
        assert_eq!(status_line_in(Lang::En, "jorge", &TrayState::Unknown, 0), "jorge — Checking…");
    }

    #[test]
    fn tooltip_body_matches_status_line_exactly() {
        const NOW: u64 = 1_000_000;
        let state = active(Some(Expiry::At { epoch: NOW + 120 }));
        let t = tooltip_in(Lang::En, "jorge", &state, NOW);
        assert_eq!(t.body, status_line_in(Lang::En, "jorge", &state, NOW));
        assert_eq!(t.title, "NoPass");
    }

    #[test]
    fn toggle_label_offers_enable_when_inactive_and_disable_when_active() {
        assert_eq!(toggle_label_in(Lang::En, &TrayState::Inactive), Some("Enable passwordless sudo"));
        assert_eq!(toggle_label_in(Lang::En, &active(Some(Expiry::Never))), Some("Disable passwordless sudo"));
        assert_eq!(toggle_label_in(Lang::En, &active(None)), Some("Disable passwordless sudo"));
    }

    #[test]
    fn toggle_label_is_none_for_unknown() {
        // design.md D6: a state whose label cannot be derived must not
        // be actionable.
        assert_eq!(toggle_label_in(Lang::En, &TrayState::Unknown), None);
    }

    // ---- Lang::from_env_vars / from_locale_string (Phase 5, task 5.2) ----

    #[test]
    fn from_env_vars_prefers_lc_all_over_lc_messages_and_lang() {
        assert_eq!(Lang::from_env_vars(Some("es_ES.UTF-8"), None, None), Lang::Es);
        assert_eq!(
            Lang::from_env_vars(Some("en_US.UTF-8"), Some("es_ES.UTF-8"), Some("es_ES.UTF-8")),
            Lang::En,
            "LC_ALL=en_US must beat LC_MESSAGES=es_ES"
        );
    }

    #[test]
    fn from_env_vars_falls_through_lc_messages_then_lang() {
        assert_eq!(Lang::from_env_vars(None, Some("es_ES.UTF-8"), None), Lang::Es);
        assert_eq!(Lang::from_env_vars(None, None, Some("es_ES.UTF-8")), Lang::Es);
    }

    #[test]
    fn from_env_vars_treats_an_empty_candidate_as_absent() {
        // An empty (but present) LC_ALL is effectively unset in glibc
        // locale resolution — it must not itself decide English and block
        // LC_MESSAGES from being consulted.
        assert_eq!(Lang::from_env_vars(Some(""), Some("es_ES.UTF-8"), None), Lang::Es);
    }

    #[test]
    fn from_env_vars_resolves_to_english_when_all_unset() {
        assert_eq!(Lang::from_env_vars(None, None, None), Lang::En);
    }

    #[test]
    fn from_locale_string_table() {
        for s in ["C", "POSIX", "", "en_US.UTF-8", "fr_FR.UTF-8"] {
            assert_eq!(Lang::from_locale_string(Some(s)), None, "{s:?} must not resolve Spanish");
        }
        for s in ["es", "es_ES.UTF-8", "es_AR", "es_MX.UTF-8", "es@euro"] {
            assert_eq!(Lang::from_locale_string(Some(s)), Some(Lang::Es), "{s:?} must resolve Spanish");
        }
        assert_eq!(Lang::from_locale_string(None), None);
    }

    // ---- Msg table: both languages, distinct per arm (Phase 5, task 5.3) ----

    #[test]
    fn every_msg_arm_renders_both_languages_and_they_are_distinct() {
        const ALL: [Msg; 34] = [
            Msg::EnablePasswordlessSudo,
            Msg::DisablePasswordlessSudo,
            Msg::StatusActive,
            Msg::StatusInactive,
            Msg::StatusChecking,
            Msg::RemainingTimeUnknown,
            Msg::NoExpiry,
            Msg::UntilReboot,
            Msg::Expired,
            Msg::LessThanAMinute,
            Msg::MinutesSuffix,
            Msg::MenuActivateDuring,
            Msg::MenuDefaultDuration,
            Msg::MenuCurrentRule,
            Msg::MenuNoActiveRule,
            Msg::MenuRuleDetailsUnavailable,
            Msg::MenuStartWithSession,
            Msg::MenuAbout,
            Msg::MenuQuit,
            Msg::RuleUserPrefix,
            Msg::RulePathPrefix,
            Msg::RuleExpiresPrefix,
            Msg::RuleRemainingPrefix,
            Msg::DurationLabelMinutes15,
            Msg::DurationLabelHour1,
            Msg::DurationLabelHours4,
            Msg::DurationLabelHours8,
            Msg::DurationLabelUntilReboot,
            Msg::DurationLabelPermanent,
            Msg::ConsentWarningTitle,
            Msg::ConsentWarningBody,
            Msg::ConsentConfirmOncePrefix,
            Msg::ConsentConfirmPersist,
            Msg::ConsentCancel,
        ];
        for msg in ALL {
            let en = msg.text(Lang::En);
            let es = msg.text(Lang::Es);
            assert!(!en.is_empty(), "{msg:?} English text must not be empty");
            assert!(!es.is_empty(), "{msg:?} Spanish text must not be empty");
            assert_ne!(en, es, "{msg:?} must render distinct text per language (M2's distinctness rule)");
        }
    }

    // ---- status_line_in / tooltip_in / toggle_label_in render Spanish
    // under Lang::Es with the same call signature as English (Phase 5,
    // task 5.4) ----

    #[test]
    fn status_line_in_renders_spanish_under_es_with_the_same_signature() {
        const NOW: u64 = 1_000_000;
        let state = active(Some(Expiry::At { epoch: NOW + 60 * 5 }));
        assert_eq!(status_line_in(Lang::En, "jorge", &state, NOW), "jorge — Active (5 min)");
        assert_eq!(status_line_in(Lang::Es, "jorge", &state, NOW), "jorge — Activo (5 min.)");
    }

    #[test]
    fn status_line_in_renders_spanish_for_inactive_unknown_and_unknown_remaining() {
        assert_eq!(status_line_in(Lang::Es, "jorge", &TrayState::Inactive, 0), "jorge — Inactivo");
        assert_eq!(status_line_in(Lang::Es, "jorge", &TrayState::Unknown, 0), "jorge — Comprobando…");
        assert_eq!(
            status_line_in(Lang::Es, "jorge", &active(None), 0),
            "jorge — Activo (tiempo restante desconocido)"
        );
    }

    #[test]
    fn tooltip_in_body_matches_status_line_in_under_es() {
        const NOW: u64 = 1_000_000;
        let state = active(Some(Expiry::At { epoch: NOW + 120 }));
        let t = tooltip_in(Lang::Es, "jorge", &state, NOW);
        assert_eq!(t.body, status_line_in(Lang::Es, "jorge", &state, NOW));
        assert_eq!(t.title, "NoPass");
    }

    #[test]
    fn toggle_label_in_renders_spanish_under_es_with_the_same_signature() {
        assert_eq!(toggle_label_in(Lang::Es, &TrayState::Inactive), Some("Activar sudo sin contraseña"));
        assert_eq!(
            toggle_label_in(Lang::Es, &active(Some(Expiry::Never))),
            Some("Desactivar sudo sin contraseña")
        );
        assert_eq!(toggle_label_in(Lang::Es, &TrayState::Unknown), None);
    }

    #[test]
    fn countdown_in_renders_spanish_words_under_es() {
        const NOW: u64 = 1_000_000;
        assert_eq!(countdown_in(Lang::Es, Expiry::Never, NOW), "sin caducidad");
        assert_eq!(countdown_in(Lang::Es, Expiry::Reboot, NOW), "hasta el reinicio");
        assert_eq!(countdown_in(Lang::Es, Expiry::At { epoch: NOW - 1 }, NOW), "caducado");
        assert_eq!(countdown_in(Lang::Es, Expiry::At { epoch: NOW + 30 }, NOW), "menos de un minuto");
        assert_eq!(countdown_in(Lang::Es, Expiry::At { epoch: NOW + 120 }, NOW), "2 min.");
    }
}

//! The menu tree — pure logic, no bus (design.md §1, §2 `tray`; spec
//! `tray-menu` (all)).
//!
//! `Tray::menu(&self)` (`ksni` 0.3.6) rebuilds the whole tree from
//! `Inner`'s already-rendered state every time it opens — there is no
//! `about_to_show` lazy-refresh hook in this version — so the entire tree
//! is expressed here as a pure function, [`menu_tree`], over an immutable
//! snapshot, [`MenuModel`]. [`MenuNode`] is the dependency-free mirror of
//! a `ksni::MenuItem` that lets this module — and its own tests — stay
//! completely free of `ksni`: **this file has no `use ksni` and no
//! `ksni::` reference anywhere**, checked below by
//! `menu_rs_never_imports_ksni_or_the_action_enable_constructor`. Phase 7
//! is the only module allowed to translate a [`MenuNode`] into a real
//! `ksni::MenuItem`.
//!
//! **The one invariant this module exists to protect**: an unconsented
//! grant must stay unrepresentable here, not merely absent by
//! convention. `menu.rs` never imports `crate::consent::Granted`,
//! `crate::outcome::Action`, or `crate::outcome::EnableRequest` — there
//! is structurally no code path in this file that could construct an
//! `Action::Enable`. The only way this module ever renders anything
//! related to a pending activation is [`ConsentBranch`] — the value
//! [`crate::consent::ConsentState::branch`] returns while a duration is
//! armed and unacknowledged — which carries nothing but the pending
//! duration, never a `Granted`. Dispatching the confirmed action is
//! Phase 7/8's job, driven by the [`MenuNodeKind::ConfirmActivate`] a
//! click on the rendered branch reports; this module never dispatches
//! anything itself.

use std::path::Path;

use nopass_core::expiry::Expiry;

use crate::atomicfile::AtomicFileError;
use crate::autostart::AutostartState;
use crate::config::{self, Config};
use crate::consent::ConsentBranch;
use crate::duration::GrantDuration;
use crate::format::{self, Lang, Msg};
use crate::reconcile::TrayState;
use crate::state::FileReading;
use crate::tray::ViewModel;

/// The dependency-free mirror of a `ksni::MenuItem` (design.md §1):
/// enough to assert the full tree without a bus, and enough for Phase 7
/// to build the real thing from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuNode {
    pub label: String,
    pub enabled: bool,
    /// `None` for anything that is not a checkbox/radio entry;
    /// `Some(true/false)` for "Start with session" and each "Default
    /// duration" entry.
    pub checked: Option<bool>,
    pub kind: MenuNodeKind,
    pub children: Vec<MenuNode>,
}

/// What a [`MenuNode`] click means, for Phase 7/8 to dispatch — never
/// anything this module acts on itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuNodeKind {
    /// A submenu shell, or an insensitive reference/warning line with no
    /// action of its own.
    Static,
    /// The state-dependent toggle (design.md §1 rows 3/4 conflated into
    /// the single item `tray-menu`'s "state-dependent toggle item"
    /// describes): activates for `config.default_duration` while
    /// `Inactive`, disables while `Active`.
    Toggle,
    /// One entry inside "Activate during…" — a request to activate for
    /// exactly this duration. Reaching consent's gate is the caller's
    /// job (`crate::consent::ConsentState::grant`/`arm`), never this
    /// module's.
    ActivateFor(GrantDuration),
    /// One entry inside "Default duration" — changes and persists the
    /// configured default (spec `tray-menu` "Selecting a new default
    /// moves the marker and persists it"; see [`select_default_duration`]).
    SelectDefaultDuration(GrantDuration),
    /// The consent branch's "I understand — activate[, and don't warn me
    /// again]" (design.md §1 "The consent branch").
    ConfirmActivate { persist: bool },
    /// The consent branch's "Cancel".
    CancelActivate,
    /// "Start with session" — toggles the autostart entry.
    ToggleAutostart,
    Quit,
}

/// Everything [`menu_tree`] needs — an extension of M2's
/// [`crate::tray::ViewModel`] with the config/consent/autostart state
/// the menu (but not the tray icon/tooltip) additionally renders (task
/// 6.1). Assembling one — including the `autostart::read`/`state::read`
/// calls that feed [`MenuModel::autostart`]/[`MenuModel::file`] — is the
/// caller's job at every `Trigger::MenuOpened` (design.md §3 data flow);
/// this struct is an immutable snapshot, never cached inside this
/// module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuModel {
    /// M2's already-rendered tray view (icon/tooltip/status line/toggle
    /// label) — `view.toggle` is reused verbatim for the top-level
    /// [`MenuNodeKind::Toggle`] label, so this module never re-derives
    /// wording `format.rs` already owns.
    pub view: ViewModel,
    pub state: TrayState,
    /// What the state file itself said, independent of the merged
    /// [`TrayState`] — needed only to source "Current rule"'s rule path,
    /// and to tell an incoherent/absent/faulted file apart from a
    /// coherent active one when the probe alone confirms activity
    /// (design.md §3.2 row 1; tray-menu "Current Rule Renders As
    /// Insensitive Reference Items").
    pub file: FileReading,
    pub now: u64,
    pub config: Config,
    /// `Some` while an activation is armed and unacknowledged — what
    /// [`crate::consent::ConsentState::branch`] returns. `None` renders
    /// the ordinary toggle/"Activate during…" pair; `Some` replaces both
    /// with the consent branch (spec `activation-consent` "First
    /// Activation Branches the Menu Instead of Granting").
    pub consent_branch: Option<ConsentBranch>,
    pub autostart: AutostartState,
}

/// Builds the full menu tree from `model` (spec `tray-menu` "The full
/// item tree is present in the exported menu"). Resolves the process
/// language via [`format::lang`] — see [`menu_tree_in`] for the
/// explicit-`Lang` seam this module's own tests use instead, matching
/// `tray.rs::ViewModel::from_state`/`from_state_in`'s existing pattern.
pub fn menu_tree(model: &MenuModel) -> Vec<MenuNode> {
    menu_tree_in(format::lang(), model)
}

/// [`menu_tree`]'s implementation, parameterised by an explicit [`Lang`]
/// — this crate is `#![forbid(unsafe_code)]`, so a test cannot
/// `std::env::set_var` to simulate a locale change; this is the seam
/// that lets this module's own tests stay locale-independent instead
/// (mirrors `format.rs`'s `_in` convention).
pub(crate) fn menu_tree_in(lang: Lang, model: &MenuModel) -> Vec<MenuNode> {
    let mut nodes = Vec::with_capacity(7);

    match model.consent_branch {
        Some(branch) => nodes.extend(consent_branch_nodes(lang, branch)),
        None => {
            nodes.push(toggle_node(model));
            nodes.push(activate_during_node(lang));
        }
    }

    nodes.push(default_duration_node(lang, model));
    nodes.push(current_rule_node(lang, model));
    nodes.push(autostart_node(lang, model));
    nodes.push(about_node(lang));
    nodes.push(quit_node(lang));

    nodes
}

/// Task 6.4's write-through half: persists `selected` as the new
/// configured default duration via [`config::write`] — this crate's only
/// atomic-write path — and returns the updated [`Config`] for the caller
/// to build the NEXT [`MenuModel`] from. The marker itself is never
/// mutated in place; [`menu_tree`] always re-derives it from whatever
/// `Config` it is given, which is what makes "moves the marker on the
/// next build" true without any UI-side cached state (spec `tray-menu`
/// "Selecting a new default moves the marker and persists it").
pub fn select_default_duration(
    path: &Path,
    current: Config,
    selected: GrantDuration,
) -> Result<Config, AtomicFileError> {
    let updated = Config { default_duration: selected, ..current };
    config::write(path, &updated)?;
    Ok(updated)
}

fn toggle_node(model: &MenuModel) -> MenuNode {
    MenuNode {
        label: model.view.toggle.unwrap_or_default().to_string(),
        enabled: model.view.toggle.is_some(),
        checked: None,
        kind: MenuNodeKind::Toggle,
        children: Vec::new(),
    }
}

fn activate_during_node(lang: Lang) -> MenuNode {
    MenuNode {
        label: Msg::MenuActivateDuring.text(lang).to_string(),
        enabled: true,
        checked: None,
        kind: MenuNodeKind::Static,
        children: duration_items(lang, MenuNodeKind::ActivateFor, |_| None),
    }
}

fn default_duration_node(lang: Lang, model: &MenuModel) -> MenuNode {
    let configured = model.config.default_duration;
    MenuNode {
        label: Msg::MenuDefaultDuration.text(lang).to_string(),
        enabled: true,
        checked: None,
        kind: MenuNodeKind::Static,
        children: duration_items(
            lang,
            MenuNodeKind::SelectDefaultDuration,
            move |d| Some(d == configured),
        ),
    }
}

/// [`GrantDuration::ALL`] is the single canonical ordering (task 1.10):
/// both duration submenus are built from exactly this array, in exactly
/// this order, so a `RadioGroup::select` index lookup in Phase 7 can
/// never disagree with what "Activate during…" offers (tray-menu "Both
/// submenus render the same six items in the same order").
fn duration_items(
    lang: Lang,
    kind: impl Fn(GrantDuration) -> MenuNodeKind,
    checked: impl Fn(GrantDuration) -> Option<bool>,
) -> Vec<MenuNode> {
    GrantDuration::ALL
        .into_iter()
        .map(|d| MenuNode {
            label: duration_label(lang, d).to_string(),
            enabled: true,
            checked: checked(d),
            kind: kind(d),
            children: Vec::new(),
        })
        .collect()
}

fn duration_label(lang: Lang, duration: GrantDuration) -> &'static str {
    match duration {
        GrantDuration::Minutes15 => Msg::DurationLabelMinutes15.text(lang),
        GrantDuration::Hour1 => Msg::DurationLabelHour1.text(lang),
        GrantDuration::Hours4 => Msg::DurationLabelHours4.text(lang),
        GrantDuration::Hours8 => Msg::DurationLabelHours8.text(lang),
        GrantDuration::UntilReboot => Msg::DurationLabelUntilReboot.text(lang),
        GrantDuration::Permanent => Msg::DurationLabelPermanent.text(lang),
    }
}

/// "Current rule" (spec `tray-menu` "Current Rule Renders As Insensitive
/// Reference Items"; task 6.5): four insensitive detail items (path,
/// user, expiry, remaining) for a coherent active grant; a single
/// insensitive "Rule details unavailable" item when the probe confirms
/// activity but [`MenuModel::file`] cannot corroborate it (design.md
/// §3.2 row 1 — `Absent`/`Faulted`, or a `Parsed` file that failed its
/// own coherence check); a single insensitive "No active rule" item
/// otherwise.
fn current_rule_node(lang: Lang, model: &MenuModel) -> MenuNode {
    let children = match &model.state {
        TrayState::Active { user: Some(user), expiry: Some(expiry) } => match model.file.parsed() {
            Some(status) => vec![
                detail_item(lang, Msg::RuleUserPrefix, user),
                detail_item(lang, Msg::RulePathPrefix, &status.rule_path),
                detail_item(lang, Msg::RuleExpiresPrefix, &expiry_value_text(lang, *expiry)),
                detail_item(lang, Msg::RuleRemainingPrefix, &format::countdown_in(lang, *expiry, model.now)),
            ],
            // The merged state claims a coherent active grant, but the
            // file this snapshot carries cannot back it up — treated the
            // same as the no-detail case below, never trusted.
            None => vec![unavailable_item(lang)],
        },
        // Probe-confirmed active, but no coherent file detail to show
        // (design.md §3.2 row 1's `user: None, expiry: None`).
        TrayState::Active { .. } => vec![unavailable_item(lang)],
        TrayState::Inactive | TrayState::Unknown => vec![no_active_rule_item(lang)],
    };

    MenuNode { label: Msg::MenuCurrentRule.text(lang).to_string(), enabled: true, checked: None, kind: MenuNodeKind::Static, children }
}

fn expiry_value_text(lang: Lang, expiry: Expiry) -> String {
    match expiry {
        Expiry::Never => Msg::NoExpiry.text(lang).to_string(),
        Expiry::Reboot => Msg::UntilReboot.text(lang).to_string(),
        // NOT `epoch.to_string()`. This item exists so a human can read
        // when their grant ends; "1789489309" answers nothing. The
        // formatter was already here — `nopass_core::timefmt` has carried
        // `format_utc_rfc3339` since M1, pinned by its own tests — so this
        // invents no time-handling scope.
        //
        // UTC rather than local: converting to local time needs timezone
        // handling this workspace deliberately does not carry, and the
        // trailing `Z` says plainly which zone is meant rather than
        // implying a local reading that would be wrong. The "remaining"
        // item directly below renders the same instant as a relative
        // countdown, so the pair gives an absolute time and a human one.
        Expiry::At { epoch } => nopass_core::timefmt::format_utc_rfc3339(epoch),
    }
}

fn detail_item(lang: Lang, prefix: Msg, value: &str) -> MenuNode {
    MenuNode {
        label: format!("{}: {value}", prefix.text(lang)),
        enabled: false,
        checked: None,
        kind: MenuNodeKind::Static,
        children: Vec::new(),
    }
}

fn unavailable_item(lang: Lang) -> MenuNode {
    static_item(Msg::MenuRuleDetailsUnavailable.text(lang))
}

fn no_active_rule_item(lang: Lang) -> MenuNode {
    static_item(Msg::MenuNoActiveRule.text(lang))
}

fn static_item(label: &str) -> MenuNode {
    MenuNode { label: label.to_string(), enabled: false, checked: None, kind: MenuNodeKind::Static, children: Vec::new() }
}

/// "Start with session" (spec `tray-menu` "Start-With-Session Checkbox
/// Toggles the Autostart Entry"; task 6.6): `checked`/`enabled` are read
/// straight off `model.autostart` — whatever [`MenuModel`] the caller
/// built this build from, always freshly read off disk by the caller via
/// `crate::autostart::read`, never cached inside this module.
fn autostart_node(lang: Lang, model: &MenuModel) -> MenuNode {
    MenuNode {
        label: Msg::MenuStartWithSession.text(lang).to_string(),
        enabled: model.autostart != AutostartState::Indeterminate,
        checked: Some(model.autostart == AutostartState::Enabled),
        kind: MenuNodeKind::ToggleAutostart,
        children: Vec::new(),
    }
}

fn about_node(lang: Lang) -> MenuNode {
    static_item(Msg::MenuAbout.text(lang))
}

fn quit_node(lang: Lang) -> MenuNode {
    MenuNode { label: Msg::MenuQuit.text(lang).to_string(), enabled: true, checked: None, kind: MenuNodeKind::Quit, children: Vec::new() }
}

/// The consent branch (design.md §1 "The consent branch"; spec
/// `activation-consent` "First Activation Branches the Menu Instead of
/// Granting"): two insensitive warning lines, then the three sensitive
/// actions. Replaces the ordinary toggle/"Activate during…" pair
/// wholesale — see [`menu_tree_in`]'s `match` — never appended alongside
/// it.
fn consent_branch_nodes(lang: Lang, branch: ConsentBranch) -> Vec<MenuNode> {
    let duration = duration_label(lang, branch.pending);
    vec![
        static_item(Msg::ConsentWarningTitle.text(lang)),
        static_item(Msg::ConsentWarningBody.text(lang)),
        MenuNode {
            label: format!("{} {duration}", Msg::ConsentConfirmOncePrefix.text(lang)),
            enabled: true,
            checked: None,
            kind: MenuNodeKind::ConfirmActivate { persist: false },
            children: Vec::new(),
        },
        MenuNode {
            label: Msg::ConsentConfirmPersist.text(lang).to_string(),
            enabled: true,
            checked: None,
            kind: MenuNodeKind::ConfirmActivate { persist: true },
            children: Vec::new(),
        },
        MenuNode {
            label: Msg::ConsentCancel.text(lang).to_string(),
            enabled: true,
            checked: None,
            kind: MenuNodeKind::CancelActivate,
            children: Vec::new(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::consent::ConsentState;
    use crate::runner::ScriptedRunner;
    use nopass_core::state::HelperStatus;

    const NOW: u64 = 1_000_000;

    fn view_for(state: &TrayState) -> ViewModel {
        ViewModel::from_state_in(Lang::En, "jorge", state, NOW)
    }

    fn model(state: TrayState, file: FileReading, config: Config, autostart: AutostartState) -> MenuModel {
        MenuModel { view: view_for(&state), state, file, now: NOW, config, consent_branch: None, autostart }
    }

    fn default_config() -> Config {
        Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true }
    }

    fn helper_status(user: &str, expiry: Expiry, rule_path: &str) -> HelperStatus {
        HelperStatus {
            schema: 1,
            uid: 1000,
            user: user.to_string(),
            active: true,
            expires: Some(expiry),
            rule_path: rule_path.to_string(),
            updated_at: NOW,
        }
    }

    // ---- task 6.2: the full item tree in order ----

    #[test]
    fn the_full_item_tree_is_present_in_order_for_any_merged_state_other_than_unknown() {
        let expected_kinds = [
            MenuNodeKind::Toggle,
            MenuNodeKind::Static, // "Activate during…"
            MenuNodeKind::Static, // "Default duration"
            MenuNodeKind::Static, // "Current rule"
            MenuNodeKind::ToggleAutostart,
            MenuNodeKind::Static, // "About"
            MenuNodeKind::Quit,
        ];
        let expected_labels =
            ["Enable passwordless sudo", "Activate during…", "Default duration", "Current rule", "Start with session", "About", "Quit"];

        for state in [TrayState::Inactive, TrayState::Active { user: None, expiry: None }] {
            let m = model(state, FileReading::Absent, default_config(), AutostartState::Disabled);
            let tree = menu_tree_in(Lang::En, &m);

            assert_eq!(tree.len(), 7, "expected exactly 7 top-level items for {m:?}");
            for (node, expected_kind) in tree.iter().zip(expected_kinds.iter()) {
                assert_eq!(&node.kind, expected_kind);
            }
            // Only the very first label is state-dependent (Enable vs
            // Disable); the rest are checked exactly regardless of state.
            for (node, expected_label) in tree.iter().skip(1).zip(expected_labels.iter().skip(1)) {
                assert_eq!(&node.label, expected_label);
            }
        }
    }

    #[test]
    fn the_toggle_label_switches_between_enable_and_disable_with_state() {
        let inactive = model(TrayState::Inactive, FileReading::Absent, default_config(), AutostartState::Disabled);
        let active =
            model(TrayState::Active { user: None, expiry: None }, FileReading::Absent, default_config(), AutostartState::Disabled);

        assert_eq!(menu_tree_in(Lang::En, &inactive)[0].label, "Enable passwordless sudo");
        assert_eq!(menu_tree_in(Lang::En, &active)[0].label, "Disable passwordless sudo");
    }

    // ---- task 6.3: both duration submenus render all six, same order ----

    #[test]
    fn activate_during_and_default_duration_both_render_all_six_durations_in_the_same_fixed_order() {
        let m = model(TrayState::Inactive, FileReading::Absent, default_config(), AutostartState::Disabled);
        let tree = menu_tree_in(Lang::En, &m);

        let activate_during = &tree[1];
        let default_duration = &tree[2];

        let expected_order: Vec<GrantDuration> = GrantDuration::ALL.to_vec();

        let activate_kinds: Vec<GrantDuration> = activate_during
            .children
            .iter()
            .map(|c| match c.kind {
                MenuNodeKind::ActivateFor(d) => d,
                other => panic!("unexpected kind in Activate during…: {other:?}"),
            })
            .collect();
        let default_kinds: Vec<GrantDuration> = default_duration
            .children
            .iter()
            .map(|c| match c.kind {
                MenuNodeKind::SelectDefaultDuration(d) => d,
                other => panic!("unexpected kind in Default duration: {other:?}"),
            })
            .collect();

        assert_eq!(activate_during.children.len(), 6);
        assert_eq!(default_duration.children.len(), 6);
        assert_eq!(activate_kinds, expected_order);
        assert_eq!(default_kinds, expected_order);

        // Same labels in the same order too, not just the same kinds.
        let activate_labels: Vec<&str> = activate_during.children.iter().map(|c| c.label.as_str()).collect();
        let default_labels: Vec<&str> = default_duration.children.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(activate_labels, default_labels);
        assert_eq!(
            activate_labels,
            vec!["15 minutes", "1 hour", "4 hours", "8 hours", "Until reboot", "Permanently"]
        );
    }

    // ---- task 6.4: the marker follows the configured default, and persists ----

    #[test]
    fn default_duration_marks_exactly_the_entry_matching_the_configured_default() {
        let config = Config { default_duration: GrantDuration::Hours4, warning_acknowledged: true };
        let m = model(TrayState::Inactive, FileReading::Absent, config, AutostartState::Disabled);
        let tree = menu_tree_in(Lang::En, &m);
        let default_duration = &tree[2];

        let marked: Vec<GrantDuration> = default_duration
            .children
            .iter()
            .filter(|c| c.checked == Some(true))
            .map(|c| match c.kind {
                MenuNodeKind::SelectDefaultDuration(d) => d,
                other => panic!("unexpected kind: {other:?}"),
            })
            .collect();

        assert_eq!(marked, vec![GrantDuration::Hours4], "exactly the 4h entry must be marked");
        let unmarked_count = default_duration.children.iter().filter(|c| c.checked == Some(false)).count();
        assert_eq!(unmarked_count, 5, "every other entry must be explicitly unmarked");
    }

    #[test]
    fn selecting_a_new_default_moves_the_marker_on_the_next_build_and_persists_it() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let initial = Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true };

        let updated = select_default_duration(&path, initial, GrantDuration::Hours8).unwrap();
        assert_eq!(updated.default_duration, GrantDuration::Hours8);

        // Persisted: a fresh read off disk agrees, independent of the
        // in-memory `updated` value this function returned.
        let (reread, fault) = config::resolve(&config::read(&path));
        assert_eq!(fault, None);
        assert_eq!(reread.default_duration, GrantDuration::Hours8);

        // The NEXT menu build (built from the persisted config) marks
        // the new entry instead of the old one.
        let m = model(TrayState::Inactive, FileReading::Absent, reread, AutostartState::Disabled);
        let tree = menu_tree_in(Lang::En, &m);
        let marked = tree[2].children.iter().find(|c| c.checked == Some(true)).unwrap();
        assert_eq!(marked.kind, MenuNodeKind::SelectDefaultDuration(GrantDuration::Hours8));
    }

    // ---- task 6.5: "Current rule" insensitive reference items ----

    #[test]
    fn an_active_grant_with_a_coherent_file_lists_path_user_expiry_and_remaining_all_insensitive() {
        let expiry = Expiry::At { epoch: NOW + 60 * 42 };
        let status = helper_status("jorge", expiry, "/etc/sudoers.d/90-nopass-1000");
        let state = TrayState::Active { user: Some("jorge".to_string()), expiry: Some(expiry) };
        let m = model(state, FileReading::Parsed(status), default_config(), AutostartState::Disabled);

        let current_rule = &menu_tree_in(Lang::En, &m)[3];
        assert_eq!(current_rule.children.len(), 4);
        for child in &current_rule.children {
            assert!(!child.enabled, "{child:?} must be insensitive");
        }
        assert_eq!(current_rule.children[0].label, "User: jorge");
        assert_eq!(current_rule.children[1].label, "Rule: /etc/sudoers.d/90-nopass-1000");
        // The formatted instant, not the epoch. This assertion used to read
        // `format!("Expires: {}", NOW + 60 * 42)` and so agreed with the
        // code that it was a bare integer the user would be shown.
        assert_eq!(
            current_rule.children[2].label,
            format!("Expires: {}", nopass_core::timefmt::format_utc_rfc3339(NOW + 60 * 42))
        );
        assert_eq!(current_rule.children[3].label, "Remaining: 42 min");
    }

    #[test]
    fn an_inactive_state_renders_a_single_insensitive_no_active_rule_item() {
        let m = model(TrayState::Inactive, FileReading::Absent, default_config(), AutostartState::Disabled);
        let current_rule = &menu_tree_in(Lang::En, &m)[3];

        assert_eq!(current_rule.children.len(), 1);
        assert_eq!(current_rule.children[0].label, "No active rule");
        assert!(!current_rule.children[0].enabled);
    }

    #[test]
    fn probe_active_with_an_absent_or_faulted_file_renders_rule_details_unavailable() {
        for file in [FileReading::Absent, FileReading::Faulted(crate::state::ReadFault::Io)] {
            let state = TrayState::Active { user: None, expiry: None };
            let m = model(state, file.clone(), default_config(), AutostartState::Disabled);
            let current_rule = &menu_tree_in(Lang::En, &m)[3];

            assert_eq!(current_rule.children.len(), 1, "{file:?}");
            assert_eq!(current_rule.children[0].label, "Rule details unavailable — the state file could not be read");
            assert!(!current_rule.children[0].enabled);
        }
    }

    // ---- task 6.6: "Start with session" reads disk at every build ----

    #[test]
    fn start_with_session_checkbox_reflects_autostart_read_disk_truth_and_toggling_updates_the_next_build() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("autostart").join("nopass.desktop");

        let before = model(
            TrayState::Inactive,
            FileReading::Absent,
            default_config(),
            crate::autostart::read(&target),
        );
        let autostart_before = &menu_tree_in(Lang::En, &before)[4];
        assert_eq!(autostart_before.checked, Some(false));

        crate::autostart::enable(&target).unwrap();

        // A brand new MenuModel, built the same way the real caller
        // would on the NEXT `Trigger::MenuOpened` — no cache carried
        // over from `before`.
        let after = model(
            TrayState::Inactive,
            FileReading::Absent,
            default_config(),
            crate::autostart::read(&target),
        );
        let autostart_after = &menu_tree_in(Lang::En, &after)[4];
        assert_eq!(autostart_after.checked, Some(true));
        assert!(autostart_after.enabled);
    }

    #[test]
    fn indeterminate_autostart_renders_the_checkbox_insensitive() {
        let m = model(TrayState::Inactive, FileReading::Absent, default_config(), AutostartState::Indeterminate);
        let autostart_node = &menu_tree_in(Lang::En, &m)[4];
        assert!(!autostart_node.enabled);
    }

    // ---- task 6.7: the consent branch replaces rows 3-4 and dispatches nothing ----

    #[test]
    fn first_unconsented_activate_for_replaces_toggle_and_activate_during_with_the_consent_branch() {
        // An empty script: any `run()` call panics ("script exhausted")
        // before this test could ever observe a result — dropped without
        // panicking is itself proof that building this tree invoked
        // nothing (mirrors consent.rs's own zero-invocation tests).
        let runner = ScriptedRunner::new(vec![]);

        let mut consent =
            ConsentState::from_config(&Config { default_duration: GrantDuration::Hour1, warning_acknowledged: false });
        consent.arm(GrantDuration::Hours4);

        let mut m = model(TrayState::Inactive, FileReading::Absent, default_config(), AutostartState::Disabled);
        m.consent_branch = consent.branch();
        assert!(m.consent_branch.is_some(), "precondition: a duration must be armed and unacknowledged");

        let ordinary = {
            let mut acknowledged = m.clone();
            acknowledged.consent_branch = None;
            menu_tree_in(Lang::En, &acknowledged)
        };
        let branched = menu_tree_in(Lang::En, &m);

        assert_eq!(branched.len(), 10, "5 branch items replacing 2, plus the unchanged trailing 5");
        assert_eq!(&branched[5..], &ordinary[2..], "Default duration onward must be byte-identical, unchanged by the branch");

        assert!(!branched[0].enabled, "the warning title must be insensitive");
        assert_eq!(branched[0].label, "⚠ Read this before activating");
        assert!(!branched[1].enabled, "the warning body must be insensitive");
        assert_eq!(
            branched[1].label,
            "Any program running as you can become root without a password until this expires."
        );
        assert_eq!(branched[2].label, "I understand — activate for 4 hours");
        assert_eq!(branched[2].kind, MenuNodeKind::ConfirmActivate { persist: false });
        assert_eq!(branched[3].label, "I understand — activate and don't warn me again");
        assert_eq!(branched[3].kind, MenuNodeKind::ConfirmActivate { persist: true });
        assert_eq!(branched[4].label, "Cancel");
        assert_eq!(branched[4].kind, MenuNodeKind::CancelActivate);

        // No node anywhere in the branched tree is (or contains) a
        // Toggle or ActivateFor node — the two replaced slots are gone,
        // not merely relabeled.
        assert!(
            !branched.iter().any(|n| matches!(n.kind, MenuNodeKind::Toggle | MenuNodeKind::ActivateFor(_))),
            "the branch must fully replace rows 3-4, not coexist with them"
        );

        drop(runner);
    }

    #[test]
    fn the_current_rule_expiry_item_renders_a_readable_instant_not_a_raw_epoch() {
        // Regression guard. This shipped as `epoch.to_string()`, which put
        // "1789489309" in front of the user in the one panel whose whole
        // purpose is being read. A digits-only value fails here.
        let rendered = expiry_value_text(Lang::En, Expiry::At { epoch: 1_789_489_309 });
        assert_eq!(rendered, "2026-09-15T16:21:49Z");
        assert!(
            rendered.chars().any(|c| c == '-' || c == ':'),
            "an expiry a human is asked to read must not be a bare integer: {rendered}"
        );
        // Spanish renders the same instant: an absolute timestamp is not
        // translated, and must not become locale-dependent by accident.
        assert_eq!(expiry_value_text(Lang::Es, Expiry::At { epoch: 1_789_489_309 }), rendered);
    }

    #[test]
    fn menu_rs_never_imports_ksni_or_the_action_enable_constructor() {
        // The structural half of task 6.7's instruction: this is a
        // compile-time property, pinned by reading this module's own
        // source (mirrors autostart.rs's packaging-manifest pin,
        // consent.rs's `Granted(())` module-doc pin). If any of these
        // strings ever appear in this file, an unconsented grant has
        // stopped being merely absent by convention and started being
        // representable again.
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/menu.rs")).unwrap();

        // Only the PRODUCTION portion of this file is in scope — this
        // module's own doc comments legitimately discuss `ksni` (to
        // explain why it must never appear in code), and this very test
        // necessarily spells the forbidden substrings out as literals to
        // check for them. Everything from `#[cfg(test)]` onward
        // (this module's own test file, itself) is excluded from the
        // scan, and within the production portion, doc/line comments
        // (`//`, `///`, `//!`) are excluded too — this remains a real
        // structural pin over every actual production code line.
        let production = source.split("#[cfg(test)]").next().expect("menu.rs always has a #[cfg(test)] module");
        let code: String = production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!code.contains("use ksni"), "menu.rs must stay bus-free — no ksni import");
        assert!(!code.contains("ksni::"), "menu.rs must never reference ksni directly");
        assert!(!code.contains("EnableRequest"), "menu.rs must never name the Action::Enable constructor");
        assert!(!code.contains("consent::Granted"), "menu.rs must never import the unconstructible Granted type");
        assert!(!code.contains("outcome::Action"), "menu.rs must never import Action — dispatch is Phase 7/8's job");
    }
}

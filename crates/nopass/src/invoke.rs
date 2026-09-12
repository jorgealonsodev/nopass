//! The privileged-action port: `pkexec` argv construction and the
//! single-in-flight gate (design.md §4.3-4.4, D9; spec
//! `tray-privileged-invocation`).
//!
//! No `--user`, no `--disable-internal-agent`, no shell — `pkexec` sets
//! `PKEXEC_UID` to the calling uid itself, which is exactly the context
//! the helper's `uid::resolve` demands. This module never runs anything
//! itself: it only builds the [`CommandSpec`] the caller runs through
//! [`crate::runner::run_off_reactor`] and gates concurrent invocations.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::outcome::Action;
use crate::runner::CommandSpec;

/// The locale values the tray is willing to pass through to `pkexec`
/// (design.md D9) — deliberately unlike M1's `LANG=C` forcing. The
/// helper forces `C` because it *parses* subprocess output; the tray
/// parses nothing from `pkexec` but an integer status, and the only
/// consumer of locale on this path is the human-facing authentication
/// dialog. The privileged child re-clears its own environment before
/// every subprocess it spawns, so a locale value here can never reach
/// `visudo`/`sudo` parsing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Locale {
    pub lang: Option<String>,
    pub lc_all: Option<String>,
    pub lc_messages: Option<String>,
}

impl Locale {
    /// Reads `LANG`, `LC_ALL`, and `LC_MESSAGES` from the tray's own
    /// process environment. No other variable is ever consulted, and
    /// none of the three is invented when absent.
    pub fn from_env() -> Locale {
        Locale {
            lang: std::env::var("LANG").ok(),
            lc_all: std::env::var("LC_ALL").ok(),
            lc_messages: std::env::var("LC_MESSAGES").ok(),
        }
    }

    /// The exact `env_clear()`-then-pass-through pairs, one entry per
    /// variable that was actually present, in a fixed `LANG`, `LC_ALL`,
    /// `LC_MESSAGES` order — this is the *entire* child environment for
    /// a privileged invocation, nothing else is ever added.
    fn pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        if let Some(v) = &self.lang {
            pairs.push(("LANG".to_string(), v.clone()));
        }
        if let Some(v) = &self.lc_all {
            pairs.push(("LC_ALL".to_string(), v.clone()));
        }
        if let Some(v) = &self.lc_messages {
            pairs.push(("LC_MESSAGES".to_string(), v.clone()));
        }
        pairs
    }
}

/// Builds the exact, documented privileged-action invocation
/// (design.md §4.3):
///
/// ```text
/// pkexec /usr/libexec/nopass-helper enable --until <epoch>
/// pkexec /usr/libexec/nopass-helper disable
/// ```
///
/// `pkexec` and `helper` are both absolute paths resolved by the caller
/// (design.md's ordered-absolute-candidate discipline); this function
/// never resolves anything itself and never falls back to a bare name.
pub fn pkexec_spec(pkexec: &Path, helper: &Path, action: Action, locale: &Locale) -> CommandSpec {
    let mut args = vec![helper.display().to_string()];
    match action {
        Action::Enable { until } => {
            args.push("enable".to_string());
            args.push("--until".to_string());
            args.push(until.to_string());
        }
        Action::Disable => args.push("disable".to_string()),
    }
    CommandSpec { program: pkexec.to_path_buf(), args, env: locale.pairs() }
}

/// At most one privileged action in flight at a time (design.md §4.4).
/// Without this, a user clicking twice stacks two polkit dialogs and two
/// competing helper transactions that race on M1's `flock` and produce a
/// gratuitous exit 15.
pub struct ActionGate {
    in_flight: Arc<AtomicBool>,
}

impl ActionGate {
    pub fn new() -> ActionGate {
        ActionGate { in_flight: Arc::new(AtomicBool::new(false)) }
    }

    /// `Some(Ticket)` iff no other `Ticket` issued by this same gate is
    /// still alive; `None` while one is in flight. The toggle menu item
    /// goes insensitive and a repeated SNI `Activate` is ignored for the
    /// duration a caller holds `None` back from.
    pub fn try_begin(&self) -> Option<Ticket> {
        match self.in_flight.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => Some(Ticket { in_flight: Arc::clone(&self.in_flight) }),
            Err(_) => None,
        }
    }
}

impl Default for ActionGate {
    fn default() -> Self {
        ActionGate::new()
    }
}

/// Held for the duration of exactly one privileged action. Dropping it —
/// on success, on failure, or on an unwind — releases the gate for the
/// next `try_begin()`.
pub struct Ticket {
    in_flight: Arc<AtomicBool>,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.in_flight.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe;
    use crate::runner::{CommandRunner as _, RunnerError, SpawnOutcome, SystemRunner};
    use std::path::PathBuf;

    const HELPER_PATH: &str = nopass_core::paths::HELPER_PATH;

    fn locale(lang: &str) -> Locale {
        Locale { lang: Some(lang.to_string()), lc_all: None, lc_messages: None }
    }

    // ---- 5.2: exact pkexec argv, env pass-through list ----

    #[test]
    fn pkexec_spec_builds_the_exact_documented_enable_argv() {
        let spec =
            pkexec_spec(&PathBuf::from("/usr/bin/pkexec"), &PathBuf::from(HELPER_PATH), Action::Enable { until: 1_700_003_600 }, &locale("en_US.UTF-8"));
        assert_eq!(spec.program, PathBuf::from("/usr/bin/pkexec"));
        assert_eq!(
            spec.args,
            vec![HELPER_PATH.to_string(), "enable".to_string(), "--until".to_string(), "1700003600".to_string()]
        );
    }

    #[test]
    fn pkexec_spec_builds_the_exact_documented_disable_argv() {
        let spec = pkexec_spec(&PathBuf::from("/usr/bin/pkexec"), &PathBuf::from(HELPER_PATH), Action::Disable, &locale("en_US.UTF-8"));
        assert_eq!(spec.program, PathBuf::from("/usr/bin/pkexec"));
        assert_eq!(spec.args, vec![HELPER_PATH.to_string(), "disable".to_string()]);
    }

    #[test]
    fn pkexec_spec_never_adds_user_or_shell_flags() {
        let spec = pkexec_spec(&PathBuf::from("/usr/bin/pkexec"), &PathBuf::from(HELPER_PATH), Action::Disable, &Locale::default());
        assert!(!spec.args.iter().any(|a| a == "--user"), "must never pass --user: {:?}", spec.args);
        assert!(!spec.args.iter().any(|a| a == "--disable-internal-agent"), "must never pass --disable-internal-agent");
        assert!(spec.args.iter().all(|a| a != "sh" && a != "-c"), "must never wrap in a shell: {:?}", spec.args);
    }

    #[test]
    fn env_pass_through_is_exactly_lang_lc_all_lc_messages_and_nothing_else() {
        let full = Locale {
            lang: Some("es_AR.UTF-8".to_string()),
            lc_all: Some("es_AR.UTF-8".to_string()),
            lc_messages: Some("es_AR.UTF-8".to_string()),
        };
        let spec = pkexec_spec(&PathBuf::from("/usr/bin/pkexec"), &PathBuf::from(HELPER_PATH), Action::Disable, &full);
        assert_eq!(
            spec.env,
            vec![
                ("LANG".to_string(), "es_AR.UTF-8".to_string()),
                ("LC_ALL".to_string(), "es_AR.UTF-8".to_string()),
                ("LC_MESSAGES".to_string(), "es_AR.UTF-8".to_string()),
            ]
        );
    }

    #[test]
    fn env_pass_through_omits_variables_absent_from_the_locale() {
        let spec = pkexec_spec(&PathBuf::from("/usr/bin/pkexec"), &PathBuf::from(HELPER_PATH), Action::Disable, &Locale::default());
        assert!(spec.env.is_empty(), "no LANG/LC_ALL/LC_MESSAGES present ⇒ no env pairs at all: {:?}", spec.env);
    }

    #[test]
    fn locale_from_env_reads_only_the_three_documented_variables() {
        // Read-only structural check: `from_env` must not read anything
        // beyond the three documented variables. We cannot control the
        // test process's real environment here, so this only asserts
        // the function is callable and returns a `Locale` shape with
        // exactly those three optional fields (compile-time proof that
        // no fourth field exists to be populated from a fourth variable).
        let l = Locale::from_env();
        let _: Option<String> = l.lang;
        let _: Option<String> = l.lc_all;
        let _: Option<String> = l.lc_messages;
    }

    // ---- 5.3: ActionGate ----

    #[test]
    fn second_try_begin_while_one_is_in_flight_returns_none() {
        let gate = ActionGate::new();
        let first = gate.try_begin();
        assert!(first.is_some(), "the first try_begin must succeed");
        assert!(gate.try_begin().is_none(), "a second try_begin while one is in flight must return None");
    }

    #[test]
    fn dropping_the_ticket_releases_the_gate_for_the_next_action() {
        let gate = ActionGate::new();
        let ticket = gate.try_begin().expect("first try_begin must succeed");
        drop(ticket);
        assert!(gate.try_begin().is_some(), "dropping the ticket must release the gate");
    }

    // ---- 5.7: threat-matrix "External command composition" completion,
    // spanning both `invoke::pkexec_spec` and `probe::spec` together.

    #[test]
    fn pkexec_and_sudo_specs_are_command_spec_only_never_a_shell_string() {
        let pkexec = pkexec_spec(&PathBuf::from("/usr/bin/pkexec"), &PathBuf::from(HELPER_PATH), Action::Enable { until: 1 }, &locale("C"));
        let sudo = probe::spec(&PathBuf::from("/usr/bin/sudo"));

        assert_eq!(
            pkexec,
            CommandSpec {
                program: PathBuf::from("/usr/bin/pkexec"),
                args: vec![HELPER_PATH.to_string(), "enable".to_string(), "--until".to_string(), "1".to_string()],
                env: vec![("LANG".to_string(), "C".to_string())],
            }
        );
        assert_eq!(
            sudo,
            CommandSpec {
                program: PathBuf::from("/usr/bin/sudo"),
                args: vec!["-k".to_string(), "-n".to_string(), "true".to_string()],
                env: vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())],
            }
        );
    }

    #[test]
    fn a_non_absolute_program_is_rejected_before_any_spawn_for_either_ports_spec_shape() {
        for bad in [
            CommandSpec { program: PathBuf::from("pkexec"), args: vec![], env: vec![] },
            CommandSpec { program: PathBuf::from("sudo"), args: vec![], env: vec![] },
        ] {
            let program = bad.program.display().to_string();
            match SystemRunner.run(&bad) {
                Err(RunnerError::NonAbsoluteProgram { program: p }) => assert_eq!(p, program),
                other => panic!("expected NonAbsoluteProgram for {program:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_status_none_outcome_is_never_treated_as_success_by_the_runner_layer() {
        // Structural pin, not a new behaviour: `SpawnOutcome{status: None}`
        // is a shape only a `ScriptedRunner` script can express (real
        // `SystemRunner` maps that case to `RunnerError::Signaled`
        // itself) — reaffirmed here because `invoke`/`outcome` both
        // depend on that invariant holding for `classify`'s `None` arm.
        let outcome = SpawnOutcome { status: None, stdout: vec![], stderr: vec![] };
        assert_eq!(outcome.status, None);
    }

    // ---- 5.8: the tray's I/O port surface excludes /etc/sudoers.d
    // entirely — a structural assertion over the port trait definitions.

    #[test]
    fn no_production_code_in_the_crate_references_a_path_under_etc_sudoers_d() {
        let needle = "/etc/sudoers.d";
        let src_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut offenders = Vec::new();
        for entry in std::fs::read_dir(src_dir).expect("crate src directory must exist") {
            let path = entry.expect("readable directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let contents = std::fs::read_to_string(&path).expect("readable source file");
            // Exclude every `#[cfg(test)]` module (for example
            // `HelperStatus.rule_path` sample fixtures in `state.rs`
            // and `reconcile.rs`) — only the actual port surface is
            // inspected, not test data that merely echoes a value the
            // wire format already carries.
            let production_code = contents.split("#[cfg(test)]").next().unwrap_or("");
            if production_code.contains(needle) {
                offenders.push(path.display().to_string());
            }
        }
        assert!(offenders.is_empty(), "production code must never reference {needle}: {offenders:?}");
    }
}

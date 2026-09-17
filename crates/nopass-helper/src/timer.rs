//! Transient expiry-timer management via `systemd-run`/`systemctl`
//! (expiry-policy §Transient Timer Replacement; design.md §6).
//!
//! `ops::enable`/`ops::disable`/`ops::expire` (Phase 7) are the production
//! callers; `ops.rs` is the only caller wired into `main`'s `dispatch`. No
//! `#[allow(dead_code)]` is needed despite that: every item below is
//! `pub` inside this crate's `pub mod timer` (declared in `lib.rs`), which
//! makes it public library API exempt from the `dead_code` lint.

use crate::bins::Binaries;
use crate::error::HelperError;
use crate::runner::{CommandRunner, CommandSpec, Expect};

fn lang_c_env() -> Vec<(String, String)> {
    vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())]
}

/// One `systemd-run` property token, tagged with the unit section it
/// belongs to. `UNIT_PROPERTIES` below is the **only** place the five
/// `AccuracySec=1s`/`Persistent=false`/`WakeSystem=false`/
/// `RemainAfterElapse=false`/`Type=oneshot` spellings exist
/// (design.md §6.1): `property_args()` renders them into the argv
/// `schedule` sends to `systemd-run`, and `synthesize_unit()` renders the
/// SAME array into `[Timer]`/`[Service]` unit-file text that
/// `crates/nopass-helper/tests/systemd_unit_contract.rs` hands to real
/// `systemd-analyze verify`. A gate that re-spells these tokens instead of
/// reading this array would prove only that the string can be typed
/// twice, not that production argv and the gate agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitProperty {
    Timer(&'static str),
    Service(&'static str),
}

/// The five non-calendar `systemd-run` property tokens, in the fixed
/// order `schedule`'s argv has always used. Argv bytes produced by
/// [`property_args`] are unchanged from the literal list this array
/// replaces — `schedule_builds_the_exact_pinned_systemd_run_argv_in_order`
/// (below) passes unmodified as the proof of that.
pub const UNIT_PROPERTIES: [UnitProperty; 5] = [
    UnitProperty::Timer("AccuracySec=1s"),
    UnitProperty::Timer("Persistent=false"),
    UnitProperty::Timer("WakeSystem=false"),
    UnitProperty::Timer("RemainAfterElapse=false"),
    UnitProperty::Service("Type=oneshot"),
];

/// Renders [`UNIT_PROPERTIES`] into the `--timer-property=`/`--property=`
/// flags `schedule` sends to `systemd-run`, in array order. One flag per
/// entry, nothing else — pinned by
/// `every_unit_property_appears_in_the_production_argv`.
pub fn property_args() -> Vec<String> {
    UNIT_PROPERTIES
        .iter()
        .map(|property| match property {
            UnitProperty::Timer(value) => format!("--timer-property={value}"),
            UnitProperty::Service(value) => format!("--property={value}"),
        })
        .collect()
}

/// Renders `[Timer]`/`[Service]` unit-file text for `uid`'s expiry timer
/// from the SAME [`UNIT_PROPERTIES`] array [`property_args`] reads, so the
/// real-tool gate in `tests/systemd_unit_contract.rs` validates exactly
/// what `schedule` sends to `systemd-run` — not a second, hand-copied set
/// of spellings. `--unit=` becomes the caller-chosen filename
/// (`<unit_name(uid)>.timer`/`.service`, validating an illegal unit name
/// as an illegal filename); `--description=` becomes the `[Unit]`
/// `Description=` directive; `--on-calendar=` becomes `OnCalendar=`;
/// `ExecStart=` targets [`nopass_core::paths::HELPER_PATH`], the same
/// target `schedule`'s argv appends after the property flags. Returns
/// `(timer_unit_text, service_unit_text)`; the caller writes them to
/// `<unit_name(uid)>.timer`/`.service` before running `systemd-analyze
/// verify` over them.
pub fn synthesize_unit(uid: u32, epoch: u64) -> (String, String) {
    let mut timer_properties = String::new();
    let mut service_properties = String::new();
    for property in &UNIT_PROPERTIES {
        match property {
            UnitProperty::Timer(value) => timer_properties.push_str(&format!("{value}\n")),
            UnitProperty::Service(value) => service_properties.push_str(&format!("{value}\n")),
        }
    }
    let timer_unit = format!(
        "[Unit]\nDescription=NoPass expiry for uid {uid}\n\n[Timer]\nOnCalendar={}\n{timer_properties}",
        nopass_core::timefmt::format_systemd_calendar(epoch)
    );
    let service_unit = format!(
        "[Unit]\nDescription=NoPass expiry for uid {uid}\n\n[Service]\n{service_properties}ExecStart={} expire --uid {uid}\n",
        nopass_core::paths::HELPER_PATH
    );
    (timer_unit, service_unit)
}

/// The base unit name for `uid`'s expiry timer: `nopass-expire-<uid>`.
/// systemd materializes `<name>.timer` and `<name>.service` from it.
pub fn unit_name(uid: u32) -> String {
    format!("nopass-expire-{uid}")
}

/// Stops any existing `nopass-expire-<uid>.timer`, tolerating a "not
/// loaded" (or any other non-zero) result — the unit may never have
/// existed. Also tolerates a missing `systemctl` binary or a runner-level
/// failure: this call always runs first, before every `systemd-run`
/// (design.md §6, "Replacement"), and none of those outcomes may ever
/// abort `enable`/`disable`/`expire --uid`.
pub fn stop(runner: &dyn CommandRunner, binaries: &Binaries, uid: u32) {
    let Ok(systemctl) = binaries.resolve("systemctl") else {
        return;
    };
    let spec = CommandSpec {
        program: systemctl.to_path_buf(),
        args: vec!["stop".to_string(), format!("{}.timer", unit_name(uid))],
        env: lang_c_env(),
        expect: Expect::Any,
    };
    let _ = runner.run(&spec);
}

/// Schedules a new one-shot `expire --uid <uid>` timer to fire at `epoch`
/// (UTC), via `systemd-run` (design.md §6). Argv order is fixed and
/// pinned verbatim by this module's tests. Returns
/// [`HelperError::TimerFailed`] (`rolled_back: false`) on any failure —
/// resolving the binary, spawning it, or a non-zero exit — leaving the
/// caller (`ops::enable`) to perform the design.md §4.1 rollback table's
/// row-15 rule rollback and report `rolled_back: true`.
pub fn schedule(runner: &dyn CommandRunner, binaries: &Binaries, uid: u32, epoch: u64) -> Result<(), HelperError> {
    let systemd_run = binaries.resolve("systemd-run")?.to_path_buf();
    let mut args = vec![
        format!("--unit={}", unit_name(uid)),
        format!("--description=NoPass expiry for uid {uid}"),
        format!("--on-calendar={}", nopass_core::timefmt::format_systemd_calendar(epoch)),
    ];
    // The five property flags read UNIT_PROPERTIES — the same array
    // synthesize_unit() renders into unit-file text for the real-tool
    // gate — so argv and the gate can never drift apart (design.md §6.1).
    args.extend(property_args());
    args.push(nopass_core::paths::HELPER_PATH.to_string());
    args.push("expire".to_string());
    args.push("--uid".to_string());
    args.push(uid.to_string());
    let spec = CommandSpec { program: systemd_run, args, env: lang_c_env(), expect: Expect::Zero };
    runner.run(&spec).map_err(|_| HelperError::TimerFailed { rolled_back: false })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::runner::{CommandOutcome, RunnerError, ScriptedRunner};

    fn binaries_with(name: &'static str, path: &Path) -> Binaries {
        Binaries::from_candidates(&[(name, &[path])])
    }

    fn touch(path: &Path) -> PathBuf {
        std::fs::write(path, b"").unwrap();
        path.to_path_buf()
    }

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nopass_test_timer_{tag}_{}", std::process::id()))
    }

    #[test]
    fn unit_name_is_the_fixed_nopass_expire_prefix() {
        assert_eq!(unit_name(1000), "nopass-expire-1000");
        assert_eq!(unit_name(1), "nopass-expire-1");
    }

    #[test]
    fn stop_runs_the_exact_pinned_systemctl_stop_argv_and_tolerates_non_zero() {
        let systemctl = touch(&temp_path("stop_systemctl"));
        let binaries = binaries_with("systemctl", &systemctl);
        let expected = CommandSpec {
            program: systemctl.clone(),
            args: vec!["stop".to_string(), "nopass-expire-1000.timer".to_string()],
            env: lang_c_env(),
            expect: Expect::Any,
        };
        let outcome = CommandOutcome { status: Some(5), stdout: vec![], stderr: b"Unit not loaded.".to_vec() };
        let runner = ScriptedRunner::new(vec![(expected, Ok(outcome))]);
        stop(&runner, &binaries, 1000); // must not panic despite the non-zero status
        let _ = std::fs::remove_file(&systemctl);
    }

    #[test]
    fn stop_tolerates_a_missing_systemctl_binary_without_running_anything() {
        let binaries = Binaries::from_candidates(&[("systemctl", &[])]);
        let runner = ScriptedRunner::new(vec![]); // empty — proves no command is attempted
        stop(&runner, &binaries, 1000);
    }

    #[test]
    fn schedule_builds_the_exact_pinned_systemd_run_argv_in_order() {
        let systemd_run = touch(&temp_path("schedule_systemd_run"));
        let binaries = binaries_with("systemd-run", &systemd_run);
        let expected = CommandSpec {
            program: systemd_run.clone(),
            args: vec![
                "--unit=nopass-expire-1000".to_string(),
                "--description=NoPass expiry for uid 1000".to_string(),
                "--on-calendar=2026-09-12 15:00:00 UTC".to_string(),
                "--timer-property=AccuracySec=1s".to_string(),
                "--timer-property=Persistent=false".to_string(),
                "--timer-property=WakeSystem=false".to_string(),
                "--timer-property=RemainAfterElapse=false".to_string(),
                "--property=Type=oneshot".to_string(),
                "/usr/libexec/nopass-helper".to_string(),
                "expire".to_string(),
                "--uid".to_string(),
                "1000".to_string(),
            ],
            env: lang_c_env(),
            expect: Expect::Zero,
        };
        // 1789225200 == 2026-09-12T15:00:00Z, verified via
        // `date -u -d "2026-09-12T15:00:00Z" +%s`.
        let outcome = CommandOutcome { status: Some(0), stdout: vec![], stderr: vec![] };
        let runner = ScriptedRunner::new(vec![(expected, Ok(outcome))]);
        assert!(schedule(&runner, &binaries, 1000, 1_789_225_200).is_ok());
        let _ = std::fs::remove_file(&systemd_run);
    }

    #[test]
    fn schedule_maps_a_non_zero_systemd_run_exit_to_timer_failed_not_rolled_back() {
        let systemd_run = touch(&temp_path("schedule_fail"));
        let binaries = binaries_with("systemd-run", &systemd_run);
        let spec_matcher = CommandSpec {
            program: systemd_run.clone(),
            args: vec![
                "--unit=nopass-expire-2000".to_string(),
                "--description=NoPass expiry for uid 2000".to_string(),
                "--on-calendar=1970-01-01 00:01:40 UTC".to_string(),
                "--timer-property=AccuracySec=1s".to_string(),
                "--timer-property=Persistent=false".to_string(),
                "--timer-property=WakeSystem=false".to_string(),
                "--timer-property=RemainAfterElapse=false".to_string(),
                "--property=Type=oneshot".to_string(),
                "/usr/libexec/nopass-helper".to_string(),
                "expire".to_string(),
                "--uid".to_string(),
                "2000".to_string(),
            ],
            env: lang_c_env(),
            expect: Expect::Zero,
        };
        let runner = ScriptedRunner::new(vec![(
            spec_matcher,
            Err(RunnerError::NonZero { program: systemd_run.display().to_string(), status: 1 }),
        )]);
        let err = schedule(&runner, &binaries, 2000, 100).unwrap_err();
        assert!(matches!(err, HelperError::TimerFailed { rolled_back: false }));
        assert_eq!(err.exit_code(), 17);
        let _ = std::fs::remove_file(&systemd_run);
    }

    #[test]
    fn schedule_yields_binary_missing_exit_1_when_systemd_run_has_no_candidate() {
        let binaries = Binaries::from_candidates(&[("systemd-run", &[])]);
        let runner = ScriptedRunner::new(vec![]); // proves resolve() fails before any command runs
        let err = schedule(&runner, &binaries, 3000, 100).unwrap_err();
        assert!(matches!(err, HelperError::BinaryMissing { name: "systemd-run" }));
        assert_eq!(err.exit_code(), 1);
    }
}

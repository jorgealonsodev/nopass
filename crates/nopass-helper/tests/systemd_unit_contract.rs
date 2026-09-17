//! Rank 1 hardening gate — real-tool contract for `timer.rs`'s
//! `systemd-run` property tokens (design.md §6.1; helper-cli "The
//! synthesized systemd-run unit is accepted by real systemd").
//!
//! `timer.rs:125`'s `schedule_builds_the_exact_pinned_systemd_run_argv_in_order`
//! proves the argv bytes are exactly what we intend; it says nothing about
//! whether the real tool accepts them. That gap is precisely the defect
//! class that let `--on-calendar` carry rejected RFC-3339 syntax for a
//! whole milestone while every `ScriptedRunner`-only gate stayed green
//! (see `nopass_core::timefmt`'s module doc). This file closes it for the
//! seven remaining `systemd-run` arguments by handing the SAME
//! `UNIT_PROPERTIES` array `property_args()` reads to real
//! `systemd-analyze verify`.
//!
//! Every child process here runs `LC_ALL=C`/`LANG=C`: this machine's
//! locale is `es_ES.UTF-8`, and `systemd-analyze`'s own diagnostics
//! localize — matching on message text under the wrong locale is a trap
//! this suite does not fall into.

use std::path::{Path, PathBuf};
use std::process::Command;

use nopass_helper::timer::{property_args, synthesize_unit, unit_name, UnitProperty, UNIT_PROPERTIES};

/// `systemd-analyze verify` also scans the whole installed system unit set
/// for unrelated warnings, independent of the units given as arguments —
/// observed on this machine: an `/etc/systemd/system/teamviewerd.service`
/// `PIDFile=` legacy-path notice appears even for a completely unrelated
/// target unit. That foreign diagnostic always carries a foreign file's
/// path/basename, never ours (confirmed manually and by
/// `a_corrupted_property_token_is_rejected_by_systemd_analyze` below, whose
/// injected defect DOES surface under our own file's name). So filtering
/// diagnostics down to the ones whose leading path or basename names one
/// of OUR two synthesized files is exact — never lenient about our own
/// unit — not a widening of the allowlist.
fn our_diagnostics<'a>(output: &'a str, timer_path: &Path, service_path: &Path) -> Vec<&'a str> {
    let timer_path_str = timer_path.to_string_lossy().into_owned();
    let service_path_str = service_path.to_string_lossy().into_owned();
    let timer_name = timer_path.file_name().unwrap().to_string_lossy().into_owned();
    let service_name = service_path.file_name().unwrap().to_string_lossy().into_owned();
    output
        .lines()
        .filter(|line| {
            line.starts_with(&format!("{timer_path_str}:"))
                || line.starts_with(&format!("{service_path_str}:"))
                || line.starts_with(&format!("{timer_name}:"))
                || line.starts_with(&format!("{service_name}:"))
        })
        .collect()
}

/// The narrow allowlist: an `ExecStart=` diagnostic reporting that
/// `HELPER_PATH` does not exist or is not executable — this Lane-A suite
/// deliberately never installs the real privileged binary there. Every
/// other diagnostic about OUR unit is a genuine rejection.
fn is_allowlisted_execstart_diagnostic(line: &str, service_name: &str) -> bool {
    match line.strip_prefix(&format!("{service_name}: ")) {
        Some(rest) => rest.starts_with("Command ") && rest.contains(" is not executable:"),
        None => false,
    }
}

fn write_unit(dir: &Path, uid: u32, timer_text: &str, service_text: &str) -> (PathBuf, PathBuf) {
    let timer_path = dir.join(format!("{}.timer", unit_name(uid)));
    let service_path = dir.join(format!("{}.service", unit_name(uid)));
    std::fs::write(&timer_path, timer_text).expect("write synthesized .timer");
    std::fs::write(&service_path, service_text).expect("write synthesized .service");
    (timer_path, service_path)
}

/// Runs `systemd-analyze verify` over the two given unit files and returns
/// the combined stdout+stderr text (diagnostics land on either, depending
/// on systemd version).
fn run_verify(timer_path: &Path, service_path: &Path) -> String {
    let output = Command::new("systemd-analyze")
        .arg("verify")
        .arg(timer_path)
        .arg(service_path)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("systemd-analyze presence already proven by toolgate::require");
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// helper-cli "The synthesized systemd-run unit is accepted by real
/// systemd"; design.md §6.1 test 1.
#[test]
fn the_production_unit_properties_are_accepted_by_systemd_analyze() {
    nopass_core::toolgate::require(
        "systemd-analyze",
        "validates the [Timer]/[Service] property tokens timer.rs's schedule() feeds to \
         systemd-run against the real unit-file grammar, not only field-equality against a fake \
         CommandRunner (helper-cli \"The synthesized systemd-run unit is accepted by real \
         systemd\"; design.md §6.1)",
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let (timer_text, service_text) = synthesize_unit(1000, 1_789_225_200);
    let (timer_path, service_path) = write_unit(dir.path(), 1000, &timer_text, &service_text);

    let output = run_verify(&timer_path, &service_path);
    let service_name = service_path.file_name().unwrap().to_string_lossy().into_owned();
    let offenders: Vec<&str> = our_diagnostics(&output, &timer_path, &service_path)
        .into_iter()
        .filter(|line| !is_allowlisted_execstart_diagnostic(line, &service_name))
        .collect();

    assert!(
        offenders.is_empty(),
        "systemd-analyze rejected the production unit properties outside the narrow \
         ExecStart-not-installed allowlist: {offenders:?}\nfull systemd-analyze output:\n{output}"
    );
}

/// The negative control, and the reason test 1's allowlist is safe:
/// helper-cli "A corrupted property token fails the gate"; design.md §6.1
/// test 2. If the allowlist above ever widens enough to swallow this
/// mutation, this test goes green-when-it-should-be-red and the suite
/// fails.
#[test]
fn a_corrupted_property_token_is_rejected_by_systemd_analyze() {
    nopass_core::toolgate::require(
        "systemd-analyze",
        "the negative control for the ExecStart allowlist above: without a real rejection \
         here, that allowlist could silently widen to swallow a genuine defect and 9.3 would \
         still pass vacuously (helper-cli \"A corrupted property token fails the gate\")",
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let (timer_text, service_text) = synthesize_unit(2000, 100);
    // The exact mutation design.md §6.1 names verbatim.
    let corrupted_timer_text = timer_text.replace("AccuracySec=1s", "AccuracySecc=1s");
    assert_ne!(corrupted_timer_text, timer_text, "the mutation must actually change the unit text");

    let (timer_path, service_path) = write_unit(dir.path(), 2000, &corrupted_timer_text, &service_text);

    let output = run_verify(&timer_path, &service_path);
    let service_name = service_path.file_name().unwrap().to_string_lossy().into_owned();
    let offenders: Vec<&str> = our_diagnostics(&output, &timer_path, &service_path)
        .into_iter()
        .filter(|line| !is_allowlisted_execstart_diagnostic(line, &service_name))
        .collect();

    assert!(
        !offenders.is_empty(),
        "expected systemd-analyze to reject the corrupted AccuracySecc token with a \
         disallowed diagnostic, but none was reported — the allowlist may have widened enough \
         to swallow a real defect; this test's pass/fail IS the gate:\n{output}"
    );
}

/// helper-cli's field-equality obligation, extended to pin that
/// `property_args()` (production argv) and `synthesize_unit()` (the gate
/// above) read the SAME array — design.md §6.1 test 3.
#[test]
fn every_unit_property_appears_in_the_production_argv() {
    let args = property_args();
    assert_eq!(
        args.len(),
        UNIT_PROPERTIES.len(),
        "property_args() must contain exactly one flag per UNIT_PROPERTIES entry and nothing else"
    );
    for (arg, property) in args.iter().zip(UNIT_PROPERTIES.iter()) {
        let expected = match property {
            UnitProperty::Timer(value) => format!("--timer-property={value}"),
            UnitProperty::Service(value) => format!("--property={value}"),
        };
        assert_eq!(arg, &expected, "property_args() entry does not match its UNIT_PROPERTIES source");
    }
}

/// Only run by `absence_of_systemd_analyze_fails_the_gate_not_skips_it`
/// below, inside a subprocess whose PATH has been stripped of
/// `systemd-analyze` — the "controlled sub-environment" that exercises
/// this gate's own precondition without hiding the real tool from every
/// other test in this binary.
#[test]
#[ignore = "only run via a PATH-stripped subprocess, see absence_of_systemd_analyze_fails_the_gate_not_skips_it"]
fn hidden_gate_precondition_under_stripped_path() {
    nopass_core::toolgate::require(
        "systemd-analyze",
        "gate 9.3's own precondition, re-run here with PATH stripped to prove absence fails \
         loudly rather than silently skipping",
    );
}

/// helper-cli "Absence of systemd-analyze fails the gate, not skips it";
/// design.md §6.1's "Lane and tool absence" clause. Hides `systemd-analyze`
/// from `PATH` in a re-spawned copy of this exact test binary and asserts
/// the child fails (via `toolgate::require`'s panic), never silently
/// passes or skips.
#[test]
fn absence_of_systemd_analyze_fails_the_gate_not_skips_it() {
    let empty_path_dir = tempfile::tempdir().expect("tempdir standing in for an empty PATH");
    let exe = std::env::current_exe().expect("this test binary has a current_exe path");

    let output = Command::new(&exe)
        .env("PATH", empty_path_dir.path())
        .arg("--exact")
        .arg("hidden_gate_precondition_under_stripped_path")
        .arg("--ignored")
        .arg("--test-threads=1")
        .output()
        .expect("re-spawning the current test binary must succeed");

    assert!(
        !output.status.success(),
        "expected toolgate::require to panic (fail loudly) when systemd-analyze is hidden from \
         PATH, but the re-spawned subprocess reported success — this is the H3 defect class: a \
         silent skip that reports ok"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("must be on PATH"),
        "expected toolgate::require's own panic message in the subprocess output, got:\n{stdout}"
    );
}

//! Rank 2 hardening gate — the polkit policy validated by a REAL polkit
//! authority, not only our own substring assertions (design.md §6.2;
//! privilege-admission "Single Polkit Action" real-authority scenarios;
//! task 10.3-10.5).
//!
//! `crates/nopass-helper/tests/data_artifacts.rs` proves the file's own
//! text contains the right bytes; it says nothing about whether the real
//! `polkitd` accepts and enumerates it. polkitd parses action files with
//! a hand-written GMarkup parser that does not consult the DTD, so a
//! silently rejected policy would make every privileged action fail with
//! a message indistinguishable from user error — exactly the gap this
//! suite closes.
//!
//! Gated on `NOPASS_POLKIT_TESTS=1` alone, the same shape as
//! `root_journal.rs`'s journald lane: `tests/containers/Containerfile.polkit`'s
//! baked entrypoint is the only caller that ever sets it, after starting
//! a private `dbus-daemon --system` and the real `/usr/lib/polkit-1/polkitd`
//! and installing both `data/com.enfoquestic.nopass.policy` and the
//! deliberately malformed `tests/containers/fixtures/
//! com.enfoquestic.nopass.malformed.policy` into
//! `/usr/share/polkit-1/actions/`. Outside that container every test here
//! compiles and skips, printing why with `--nocapture` — this machine's
//! own `pkaction`/`polkitd` talk to the REAL host system bus, and this
//! suite must never touch that.
//!
//! Every `pkaction` invocation forces `LC_ALL=C`/`LANG=C` on the child:
//! this machine's locale is `es_ES.UTF-8`, and `pkaction`'s own field
//! labels and diagnostics localize — matching on message text under the
//! wrong locale is a trap this suite does not fall into. Assertions below
//! are on stable, non-localized identifiers only: the action id itself
//! and the `implicit active: auth_admin_keep`/`exec.path` VALUE tokens,
//! which polkit never translates.
//!
//! Absence of a reachable polkit authority (neither container runtime
//! present, or polkitd failing to start) is NOT re-proven inside this
//! file as a unit test: `scripts/run-lane-polkit.sh` exits non-zero
//! before this binary ever runs when no runtime is found (its own
//! runtime-detection path, mirroring `run-lane-root.sh`/
//! `run-lane-journal.sh`), and `Containerfile.polkit`'s entrypoint checks
//! that `polkitd` is still alive after startup before ever invoking
//! `cargo test`, exiting non-zero — never silently — if it is not
//! (privilege-admission "Absence of a polkit authority fails the gate,
//! not skips it"). Inside the container, if that liveness check ever
//! passed vacuously, the tests below would still fail loudly: every
//! assertion here requires `pkaction` to reach a real authority and
//! succeed, so an unreachable bus turns straight into a failing
//! `assert!`, never a skip.

use std::process::Command;

fn lane_active() -> bool {
    std::env::var("NOPASS_POLKIT_TESTS").ok().as_deref() == Some("1")
}

macro_rules! skip_unless_polkit_lane {
    ($name:expr) => {
        if !lane_active() {
            eprintln!(
                "skipping {}: set NOPASS_POLKIT_TESTS=1 (inside Containerfile.polkit) to run",
                $name
            );
            return;
        }
    };
}

/// Runs `pkaction` with the given extra arguments against the REAL
/// authority this container's entrypoint already started, forcing a
/// stable locale on the child (see module doc). Returns the combined
/// stdout+stderr text and whether the process exited successfully.
fn run_pkaction(args: &[&str]) -> (bool, String) {
    let output = Command::new("pkaction")
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("pkaction presence already proven by toolgate::require");
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

/// privilege-admission "The real polkit engine enumerates the installed
/// action"; design.md §6.2 test 1.
#[test]
fn the_installed_policy_is_enumerated_by_the_real_authority() {
    skip_unless_polkit_lane!("the_installed_policy_is_enumerated_by_the_real_authority");
    nopass_core::toolgate::require(
        "pkaction",
        "queries the REAL polkit authority this container started, proving \
         data/com.enfoquestic.nopass.policy was actually parsed and accepted — the thing no \
         substring assertion over the file's own text can show (privilege-admission \"The real \
         polkit engine enumerates the installed action\"; design.md §6.2)",
    );

    let (ok, text) = run_pkaction(&["--action-id", "com.enfoquestic.nopass.manage", "--verbose"]);
    assert!(ok, "pkaction --action-id com.enfoquestic.nopass.manage did not exit 0:\n{text}");
    assert!(
        text.contains("com.enfoquestic.nopass.manage:"),
        "expected the action id header in pkaction's output, got:\n{text}"
    );
    assert!(
        text.contains("implicit active:   auth_admin_keep"),
        "expected implicit active: auth_admin_keep in pkaction's output, got:\n{text}"
    );
}

/// The negative control, and the reason test 1 means anything at all:
/// privilege-admission "A malformed policy file fails enumeration";
/// design.md §6.2 test 2; task 10.4. Without this test, an authority
/// whose `pkaction` lists every action unconditionally would pass test 1
/// vacuously — this test's own pass/fail IS the gate.
#[test]
fn a_malformed_policy_is_not_enumerated() {
    skip_unless_polkit_lane!("a_malformed_policy_is_not_enumerated");
    nopass_core::toolgate::require(
        "pkaction",
        "the negative control for the real-authority enumeration test above (privilege-admission \
         \"A malformed policy file fails enumeration\"; design.md §6.2)",
    );

    // Direct query for the malformed action id must fail or omit it.
    let (ok, text) = run_pkaction(&["--action-id", "com.enfoquestic.nopass.malformed", "--verbose"]);
    assert!(
        !ok || !text.contains("com.enfoquestic.nopass.malformed:"),
        "expected the malformed action id to be rejected or omitted by the real authority, but \
         pkaction reported it present:\n{text}"
    );

    // Full, unfiltered listing must also never contain it — the exact
    // assertion design.md §6.2 names verbatim ("assert it is absent from
    // pkaction's output").
    let (list_ok, listing) = run_pkaction(&[]);
    assert!(list_ok, "unfiltered pkaction listing did not exit 0:\n{listing}");
    assert!(
        !listing.contains("com.enfoquestic.nopass.malformed"),
        "expected com.enfoquestic.nopass.malformed to be absent from the unfiltered pkaction \
         listing, but it was present — an authority that enumerates everything would pass the \
         real-authority test vacuously:\n{listing}"
    );

    // And the valid action must still be present: the malformed sibling
    // file must not have taken the whole authority down with it.
    assert!(
        listing.contains("com.enfoquestic.nopass.manage"),
        "expected com.enfoquestic.nopass.manage to still be enumerated alongside the malformed \
         sibling file, but it was missing from the listing:\n{listing}"
    );
}

/// privilege-admission's `exec.path` obligation, checked against the real
/// authority's own parsed value rather than our own text scan; design.md
/// §6.2 test 3; task 10.5.
#[test]
fn the_exec_path_annotation_matches_the_shipped_helper_path() {
    skip_unless_polkit_lane!("the_exec_path_annotation_matches_the_shipped_helper_path");
    nopass_core::toolgate::require(
        "pkaction",
        "confirms the real authority parsed org.freedesktop.policykit.exec.path as the exact \
         fixed helper path (design.md §6.2 test 3)",
    );

    let (ok, text) = run_pkaction(&["--action-id", "com.enfoquestic.nopass.manage", "--verbose"]);
    assert!(ok, "pkaction --action-id com.enfoquestic.nopass.manage did not exit 0:\n{text}");
    let expected = format!(
        "annotation:        org.freedesktop.policykit.exec.path -> {}",
        nopass_core::paths::HELPER_PATH
    );
    assert!(
        text.contains(&expected),
        "expected `{expected}` in pkaction's verbose output, got:\n{text}"
    );
}

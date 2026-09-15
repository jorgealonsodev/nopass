//! Structural guard (design.md §8 "The execution gap, named and closed"):
//! a gated test file whose env-gate name is not wired into any runner
//! script under `scripts/` executes in no lane at all — silently, because
//! `cargo test` reports every one of its gated tests as `ok` (a skip is
//! not a failure). This suite makes that class of failure impossible by
//! scanning the repository itself, instead of trusting prose in
//! `tests/containers/README.md` that a human must keep in sync by hand.
//!
//! Dependency-free, same shape as `data_artifacts.rs`'s repo-file scans:
//! plain `str` scanning, no regex crate, no XML/TOML parser.

use std::path::{Path, PathBuf};

/// Repository root, derived the same way `data_artifacts.rs` derives it:
/// two levels up from this crate's manifest directory
/// (`crates/nopass-helper` -> repo root).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Scan `text` for every `NOPASS_[A-Z_]+_TESTS` token — an env-gate name
/// such as `NOPASS_ROOT_TESTS` or `NOPASS_DBUS_TESTS` — and return the
/// distinct names found, in first-seen order. A plain `str::split` on
/// "not part of an identifier" characters is enough: these names only
/// ever appear as whole tokens inside string literals or bare
/// identifiers, never glued to other uppercase text.
fn gate_names_in(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for token in text.split(|c: char| !(c.is_ascii_uppercase() || c == '_')) {
        if token.starts_with("NOPASS_")
            && token.ends_with("_TESTS")
            && token.len() > "NOPASS_TESTS".len()
            && !names.iter().any(|n| n == token)
        {
            names.push(token.to_string());
        }
    }
    names
}

/// Every `*.rs` file directly under `<root>/crates/*/tests/`, read whole
/// — excluding this file itself, whose own doc comments discuss gate
/// names as prose and would otherwise shadow their real declaration site.
fn crate_test_files(root: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    let crates_dir = root.join("crates");
    let mut crate_entries: Vec<_> = std::fs::read_dir(&crates_dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", crates_dir.display()))
        .filter_map(|e| e.ok())
        .collect();
    crate_entries.sort_by_key(|e| e.path());
    for crate_entry in crate_entries {
        if !crate_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let tests_dir = crate_entry.path().join("tests");
        if !tests_dir.is_dir() {
            continue;
        }
        let mut test_entries: Vec<_> = std::fs::read_dir(&tests_dir)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", tests_dir.display()))
            .filter_map(|e| e.ok())
            .collect();
        test_entries.sort_by_key(|e| e.path());
        for test_entry in test_entries {
            let path = test_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("lane_wiring.rs") {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            files.push((path, content));
        }
    }
    files
}

/// Every regular file directly under `<root>/scripts/`, as (file name,
/// content) pairs.
fn runner_scripts(root: &Path) -> Vec<(String, String)> {
    let scripts_dir = root.join("scripts");
    std::fs::read_dir(&scripts_dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", scripts_dir.display()))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| {
            let path = e.path();
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, content)
        })
        .collect()
}

/// Every env-gate name (matching `NOPASS_[A-Z_]+_TESTS`) found in
/// `crates/*/tests/*.rs` must appear in at least one script under
/// `scripts/`, and that script must in turn be named in
/// `tests/containers/README.md`'s checklist. This is the test that must
/// fail on the tree as it stood before `scripts/run-lane-root.sh`
/// existed: the root-lane gate name used by `root_system.rs` was named
/// by no script under `scripts/`, reproducing the exact "15 tests report
/// ok in a lane that never executes" gap the proposal documents.
///
/// This file is excluded from its own scan (see `crate_test_files`):
/// otherwise the doc comments here, which necessarily discuss gate names
/// as prose, would shadow the real source of a gate and misattribute a
/// genuine gap to this file instead of the test file that actually
/// declares it.
#[test]
fn every_gated_test_file_is_named_by_a_runner_script() {
    let root = repo_root();

    let mut gate_names: Vec<(String, PathBuf)> = Vec::new();
    for (path, content) in crate_test_files(&root) {
        for name in gate_names_in(&content) {
            if !gate_names.iter().any(|(n, _)| *n == name) {
                gate_names.push((name, path.clone()));
            }
        }
    }
    assert!(
        !gate_names.is_empty(),
        "scan found no NOPASS_*_TESTS gate names under crates/*/tests/ — the scan itself is \
         broken, not the wiring"
    );

    let scripts = runner_scripts(&root);
    let readme_path = root.join("tests/containers/README.md");
    let readme = std::fs::read_to_string(&readme_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", readme_path.display()));

    for (gate_name, source_file) in &gate_names {
        let owning_script = scripts.iter().find(|(_, content)| content.contains(gate_name.as_str()));
        let (script_name, _) = owning_script.unwrap_or_else(|| {
            panic!(
                "{gate_name} (gated in {}) is not named by any script under scripts/ — this test \
                 file executes in no gate at all",
                source_file.display()
            )
        });
        assert!(
            readme.contains(script_name.as_str()),
            "scripts/{script_name} (which names {gate_name}) is not listed in \
             tests/containers/README.md's checklist"
        );
    }
}

/// Negative control: proves the scan above rejects an unwired
/// `NOPASS_*_TESTS` name rather than passing vacuously — without this, a
/// scan that found nothing to check would be indistinguishable from a
/// scan that found everything wired. Deliberately fixture-driven (an
/// in-memory scan input and a name guaranteed absent from the real
/// `scripts/` tree) rather than a throwaway file dropped into the
/// repository, so this guard never depends on — or risks polluting — the
/// tree it scans.
#[test]
fn a_gated_test_file_naming_no_runner_script_fails_the_scan() {
    let fixture_test_file = r#"
        // A fixture gate name, structurally identical to a real one, that
        // is guaranteed to be wired nowhere in this repository.
        let gated = std::env::var("NOPASS_FIXTURE_ONLY_TESTS").is_ok();
    "#;

    let found = gate_names_in(fixture_test_file);
    assert_eq!(
        found,
        vec!["NOPASS_FIXTURE_ONLY_TESTS".to_string()],
        "the scan must find the fixture's gate name before it can prove that name is unwired"
    );

    let root = repo_root();
    let scripts = runner_scripts(&root);
    let is_wired = scripts
        .iter()
        .any(|(_, content)| content.contains("NOPASS_FIXTURE_ONLY_TESTS"));
    assert!(
        !is_wired,
        "NOPASS_FIXTURE_ONLY_TESTS unexpectedly found in a real script under scripts/ — pick a \
         fixture name that cannot collide, this negative control must observe a real rejection"
    );

    // The same lookup `every_gated_test_file_is_named_by_a_runner_script`
    // performs must reject this fixture exactly the way it would reject
    // any genuinely unwired gate: no owning script exists.
    let owning_script = scripts
        .iter()
        .find(|(_, content)| content.contains("NOPASS_FIXTURE_ONLY_TESTS"));
    assert!(
        owning_script.is_none(),
        "the scan's lookup must fail closed for an unwired gate name"
    );
}

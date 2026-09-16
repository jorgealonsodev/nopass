//! Structural guard over `docs/**.md` — the `headless-operation` spec's two
//! "Documentation ..." scenarios (`specs/headless-operation/spec.md`), plus
//! the forgery guard from `design.md` §8 "The docs/ assertions" that
//! neither scenario names on its own: nothing stops the same document
//! later gaining a copy-pasteable `sudo PKEXEC_UID=<uid> nopass-helper
//! <cmd>` line that reads as a working invocation.
//!
//! Deliberately plain `str`/`lines()` scans over the whole `docs/` tree —
//! same shape as `data_artifacts.rs`'s scan over `data/`: no XML/Markdown
//! parser, because the tokens tested for (`grant`, `revoke`, `inspect`,
//! `--uid`, `allow_inactive=no`, `PKEXEC_UID=`) are identifiers in any
//! prose language, not translatable sentences. `docs/` itself stays
//! Spanish (matches `docs/PRD_NoPass_Linux.md`'s existing register); only
//! this test file and its identifiers are English.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.md` file directly under `docs/` — currently `PRD_NoPass_Linux.md`
/// (read-only, existing) and `headless.md` (this phase). Not recursive:
/// `docs/` has no subdirectories today.
fn docs_files() -> Vec<PathBuf> {
    let docs_dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let entries = fs::read_dir(&docs_dir)
        .unwrap_or_else(|e| panic!("failed to read docs/ at {}: {e}", docs_dir.display()));
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("md"))
        .collect()
}

/// `(path, content)` for every file `docs_files()` names.
fn docs_contents() -> Vec<(PathBuf, String)> {
    docs_files()
        .into_iter()
        .map(|path| {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            (path, content)
        })
        .collect()
}

/// The forgery guard, factored out so both the real `docs/` tree and a
/// fixture prove the exact same logic: every line containing the literal
/// `PKEXEC_UID=` must also contain `marker`. Returns the offending lines.
fn pkexec_lines_missing_marker(content: &str, marker: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| line.contains("PKEXEC_UID=") && !line.contains(marker))
        .map(|line| line.to_string())
        .collect()
}

/// headless-operation "Documentation lists the three headless subcommands
/// as the supported path": `grant`, `revoke`, `inspect`, and `--uid` must
/// each appear somewhere under `docs/`.
///
/// This test MUST fail before `docs/headless.md` exists (task 10.2) —
/// `docs/PRD_NoPass_Linux.md` (read-only reference) never mentions `grant`,
/// `revoke`, or `inspect` as subcommand names; those three tokens ship only
/// in this change's CLI surface (`cli.rs`) and, after 10.1, in this new
/// doc.
#[test]
fn docs_name_the_three_headless_subcommands_as_supported() {
    let combined: String =
        docs_contents().into_iter().map(|(_, content)| content).collect::<Vec<_>>().join("\n");
    for token in ["grant", "revoke", "inspect", "--uid"] {
        assert!(
            combined.contains(token),
            "docs/ must document the headless token `{token}` as part of the supported procedure"
        );
    }
}

/// headless-operation "Documentation states the desktop assumption made
/// elsewhere": the headless doc must name that `enable`/`disable`/`status`
/// assume a tray, an interactive polkit agent, and `allow_inactive=no` —
/// and that none of the three applies to the headless path.
///
/// This test MUST fail before `docs/headless.md` exists (task 10.3) — the
/// literal `allow_inactive=no` (as written by `enable`/`disable`/`status`'s
/// own desktop precondition) never appears in `docs/PRD_NoPass_Linux.md`,
/// which only spells the same default as XML
/// (`<allow_inactive>no</allow_inactive>`).
#[test]
fn docs_state_the_desktop_assumption_made_elsewhere() {
    let combined: String =
        docs_contents().into_iter().map(|(_, content)| content).collect::<Vec<_>>().join("\n");
    assert!(
        combined.contains("allow_inactive=no"),
        "docs/ must state the literal allow_inactive=no precondition"
    );
    assert!(combined.contains("bandeja"), "docs/ must name the tray precondition (bandeja)");
    assert!(
        combined.contains("polkit"),
        "docs/ must name the interactive polkit agent precondition"
    );
}

/// Not vacuous: `docs_name_the_three_headless_subcommands_as_supported` and
/// `docs_state_the_desktop_assumption_made_elsewhere` assert only that the
/// right tokens are PRESENT — nothing stops a future edit from also adding
/// a copy-pasteable `sudo PKEXEC_UID=1000 …` example. This test proves the
/// guard function (`pkexec_lines_missing_marker`) actually rejects such a
/// line, using a real fixture file written to and read back from disk
/// (task 10.3b), never by weakening `docs/headless.md` itself.
#[test]
fn no_doc_line_shows_a_pkexec_uid_forgery_without_marking_it_unsupported() {
    for (path, content) in docs_contents() {
        let violations = pkexec_lines_missing_marker(&content, "NO SOPORTADO");
        assert!(
            violations.is_empty(),
            "{}: line(s) contain PKEXEC_UID= without the NO SOPORTADO marker: {violations:?}",
            path.display()
        );
    }

    // Non-vacuity: the exact same guard function, run against a real
    // fixture file that deliberately omits the marker, must reject it.
    let fixture_dir = tempfile::tempdir().expect("create temp dir for forgery fixture");
    let fixture_path = fixture_dir.path().join("forged.md");
    fs::write(&fixture_path, "sudo PKEXEC_UID=1000 nopass-helper grant --uid 1000\n")
        .expect("write forgery fixture");
    let fixture_content = fs::read_to_string(&fixture_path).expect("read forgery fixture back");
    let violations = pkexec_lines_missing_marker(&fixture_content, "NO SOPORTADO");
    assert_eq!(
        violations.len(),
        1,
        "fixture line with PKEXEC_UID= and no NO SOPORTADO marker must be rejected by the guard"
    );
}

/// The forgery guard, as a rule not a keyword ban (task 10.4): every line
/// under `docs/**.md` containing the literal `PKEXEC_UID=` also contains
/// the literal `unsupported` — the English counterpart pinned in task
/// 10.4, alongside 10.3b's Spanish `NO SOPORTADO` counterpart. Proven
/// non-vacuous against the same kind of on-disk fixture as 10.3b, never by
/// weakening the real document.
#[test]
fn every_pkexec_uid_forgery_mention_is_labelled_unsupported() {
    for (path, content) in docs_contents() {
        let violations = pkexec_lines_missing_marker(&content, "unsupported");
        assert!(
            violations.is_empty(),
            "{}: line(s) contain PKEXEC_UID= without the unsupported marker: {violations:?}",
            path.display()
        );
    }

    let fixture_dir = tempfile::tempdir().expect("create temp dir for forgery fixture");
    let fixture_path = fixture_dir.path().join("forged.md");
    fs::write(&fixture_path, "sudo PKEXEC_UID=1000 nopass-helper grant --uid 1000\n")
        .expect("write forgery fixture");
    let fixture_content = fs::read_to_string(&fixture_path).expect("read forgery fixture back");
    let violations = pkexec_lines_missing_marker(&fixture_content, "unsupported");
    assert_eq!(
        violations.len(),
        1,
        "fixture line with PKEXEC_UID= and no unsupported marker must be rejected by the guard"
    );
}

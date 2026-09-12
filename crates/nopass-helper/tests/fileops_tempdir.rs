//! Unprivileged, `Layout::under(TempDir)`-rooted integration tests for
//! `nopass_helper::fileops` (sudoers-rule-lifecycle §Atomic Rule Creation,
//! §Rule Removal; design.md §3-4.1 steps 7-13; threat matrix "Rule-file
//! target selection").
//!
//! Genuine Cargo integration test (a separate crate, per Cargo
//! convention), reachable only because `nopass-helper` exposes its
//! modules through `src/lib.rs` — see that file's doc comment for why.
//!
//! **Phase 6 decision (tasks.md blockquote under "Phase 6"):** under a
//! fresh `Layout::under(root)`, neither `<root>/sudoers.d` nor
//! `<root>/run/nopass` exists yet. `/run/nopass` auto-creation is already
//! required by the helper-observability spec (and `lock::LockGuard`
//! implements it independently — see `lock.rs`). `sudoers.d`
//! auto-creation is specified nowhere. This test suite's own
//! `fresh_layout` fixture creates `<root>/sudoers.d` explicitly;
//! `fileops::write_rule_atomic` itself never calls `create_dir_all` on
//! the sudoers directory. That keeps `Layout::system()`'s production path
//! unchanged: `/etc/sudoers.d` always exists on a real system, and the
//! helper must not be in the business of creating it.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nopass_core::expiry::Expiry;
use nopass_core::paths::Layout;
use nopass_core::template::render_rule;
use nopass_helper::bins::Binaries;
use nopass_helper::error::HelperError;
use nopass_helper::fileops;
use nopass_helper::runner::SystemRunner;

fn temp_root(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!(
        "nopass_test_fileops_{tag}_{}_{:?}_{nanos}_{n}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// See the module doc comment above for the Phase 6 sudoers.d
/// auto-creation decision this fixture implements.
fn fresh_layout(tag: &str) -> (PathBuf, Layout) {
    let root = temp_root(tag);
    let layout = Layout::under(&root);
    std::fs::create_dir_all(root.join("sudoers.d")).unwrap();
    (root, layout)
}

fn write_fake_visudo(root: &Path, accept: bool) -> PathBuf {
    let path = root.join(if accept { "visudo_accept" } else { "visudo_reject" });
    let script =
        if accept { "#!/bin/sh\nexit 0\n" } else { "#!/bin/sh\necho 'syntax error near line 4' >&2\nexit 1\n" };
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn binaries_with_visudo(visudo: &Path) -> Binaries {
    Binaries::from_candidates(&[("visudo", &[visudo])])
}

// --- write_rule_atomic: successful path -------------------------------

#[test]
fn write_rule_atomic_creates_the_final_file_with_exact_content_via_visudo_accept() {
    let (root, layout) = fresh_layout("happy");
    let visudo = write_fake_visudo(&root, true);
    let binaries = binaries_with_visudo(&visudo);
    let content = render_rule(1000, "jorge", Expiry::Never);

    fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, 1000, &content).expect("write succeeds");

    let final_path = layout.rule_path(1000);
    assert!(final_path.exists(), "final rule file was not created");
    assert_eq!(std::fs::read_to_string(&final_path).unwrap(), content);
    assert!(!layout.rule_tmp_path(1000).exists(), "tmp file was not cleaned up after a successful rename");

    let _ = std::fs::remove_dir_all(&root);
}

// --- write_rule_atomic: O_EXCL collision on the tmp path ---------------

#[test]
fn preexisting_tmp_file_makes_o_excl_fail_instead_of_being_overwritten() {
    let (root, layout) = fresh_layout("tmp_collision");
    let tmp_path = layout.rule_tmp_path(2000);
    std::fs::write(&tmp_path, "attacker-controlled content").unwrap();
    let visudo = write_fake_visudo(&root, true);
    let binaries = binaries_with_visudo(&visudo);
    let content = render_rule(2000, "ana", Expiry::Never);

    let err = fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, 2000, &content).unwrap_err();
    assert_eq!(err.exit_code(), 16, "expected a filesystem-failure exit (16), got {err:?}");
    assert_eq!(
        std::fs::read_to_string(&tmp_path).unwrap(),
        "attacker-controlled content",
        "pre-existing tmp content was overwritten instead of O_EXCL failing"
    );
    assert!(!layout.rule_path(2000).exists(), "final path must never be written when O_EXCL fails");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_symlink_at_the_tmp_path_makes_o_excl_fail_without_following_it() {
    let (root, layout) = fresh_layout("tmp_symlink");
    let tmp_path = layout.rule_tmp_path(2100);
    let decoy_target = root.join("decoy_target_never_written");
    std::os::unix::fs::symlink(&decoy_target, &tmp_path).unwrap();
    let visudo = write_fake_visudo(&root, true);
    let binaries = binaries_with_visudo(&visudo);
    let content = render_rule(2100, "ana", Expiry::Never);

    let err = fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, 2100, &content).unwrap_err();
    assert_eq!(err.exit_code(), 16, "expected a filesystem-failure exit (16), got {err:?}");
    assert!(!decoy_target.exists(), "the symlink's target must never be created or written through");
    assert!(!layout.rule_path(2100).exists());

    let _ = std::fs::remove_dir_all(&root);
}

// --- write_rule_atomic: visudo -cf rejection ---------------------------

#[test]
fn visudo_reject_unlinks_tmp_and_leaves_the_final_path_untouched() {
    let (root, layout) = fresh_layout("visudo_reject");
    let visudo = write_fake_visudo(&root, false);
    let binaries = binaries_with_visudo(&visudo);
    let content = render_rule(3000, "bob", Expiry::Never);

    let err = fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, 3000, &content).unwrap_err();
    assert!(matches!(err, HelperError::VisudoRejected { .. }), "expected VisudoRejected, got {err:?}");
    assert_eq!(err.exit_code(), 14);
    assert!(!layout.rule_tmp_path(3000).exists(), "tmp file must be unlinked after visudo rejects");
    assert!(!layout.rule_path(3000).exists(), "final path must never be written when visudo rejects");

    let _ = std::fs::remove_dir_all(&root);
}

// --- write_rule_atomic: symlink at the FINAL path ----------------------

#[test]
fn a_symlink_at_the_final_path_is_not_renamed_over() {
    let (root, layout) = fresh_layout("final_symlink");
    let decoy_target = root.join("decoy_final_target");
    std::fs::write(&decoy_target, "do not touch").unwrap();
    std::os::unix::fs::symlink(&decoy_target, layout.rule_path(4000)).unwrap();
    let visudo = write_fake_visudo(&root, true);
    let binaries = binaries_with_visudo(&visudo);
    let content = render_rule(4000, "carol", Expiry::Never);

    let err = fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, 4000, &content).unwrap_err();
    assert_eq!(err.exit_code(), 16, "expected exit 16 with no write, got {err:?}");
    assert!(!layout.rule_tmp_path(4000).exists(), "tmp file must still be cleaned up");
    assert_eq!(
        std::fs::read_to_string(&decoy_target).unwrap(),
        "do not touch",
        "the symlink's target must never be written through"
    );
    assert!(
        std::fs::symlink_metadata(layout.rule_path(4000)).unwrap().file_type().is_symlink(),
        "the symlink at the final path must still be there, untouched"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// --- write_rule_atomic: mode 0440 under a hostile umask ----------------

const UMASK_TEST_FLAG: &str = "NOPASS_TEST_FILEOPS_UMASK_CHILD";

#[test]
fn mode_0440_holds_even_under_umask_0o077() {
    if std::env::var(UMASK_TEST_FLAG).is_ok() {
        // Inner, re-exec'd invocation: the outer branch below filters the
        // test harness down to exactly this one test via `--exact`, so
        // mutating this process's own umask here cannot race any other
        // test in this binary (the same process-isolation technique
        // `nopass-helper`'s own `runner.rs` uses for its ambient-env
        // test).
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o077));

        let (root, layout) = fresh_layout("umask_child");
        let visudo = write_fake_visudo(&root, true);
        let binaries = binaries_with_visudo(&visudo);
        let content = render_rule(5000, "dave", Expiry::Never);

        fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, 5000, &content)
            .expect("write succeeds under umask 0o077");

        let mode = std::fs::metadata(layout.rule_path(5000)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o440, "mode was not pinned to 0440 under umask 0o077, got {mode:o}");

        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    let exe = std::env::current_exe().expect("current test binary path");
    let output = std::process::Command::new(exe)
        .arg("mode_0440_holds_even_under_umask_0o077")
        .arg("--exact")
        .arg("--nocapture")
        .env(UMASK_TEST_FLAG, "1")
        .output()
        .expect("spawn umask-isolated child test process");
    if !output.status.success() {
        panic!(
            "umask-isolated child test failed (status {:?}):\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

// --- remove_rule: ownership check + idempotence -------------------------

#[test]
fn remove_rule_leaves_a_foreign_non_nopass_file_untouched() {
    let (root, layout) = fresh_layout("foreign_survives");
    let path = layout.rule_path(6000);
    std::fs::write(&path, "# not a nopass file\nfoo ALL=(ALL) ALL\n").unwrap();

    let removed = fileops::remove_rule(&layout, 6000).expect("remove_rule does not error on a foreign file");
    assert!(!removed, "remove_rule must report false — it must not claim to have removed a foreign file");
    assert!(path.exists(), "a foreign non-NoPass file must survive remove_rule");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# not a nopass file\nfoo ALL=(ALL) ALL\n");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn remove_rule_deletes_an_existing_nopass_owned_rule() {
    let (root, layout) = fresh_layout("remove_owned");
    let content = render_rule(8000, "erin", Expiry::Never);
    std::fs::write(layout.rule_path(8000), &content).unwrap();

    let removed = fileops::remove_rule(&layout, 8000).expect("remove_rule succeeds");
    assert!(removed, "an existing NoPass-owned rule must be reported as removed");
    assert!(!layout.rule_path(8000).exists());

    let _ = std::fs::remove_dir_all(&root);
}

/// Task 6.3 — sudoers-rule-lifecycle §Rule Removal, "Rule file externally
/// deleted before disable runs": absence is success, an idempotent
/// no-op, never an error.
#[test]
fn remove_rule_is_an_idempotent_no_op_when_the_file_was_already_externally_deleted() {
    let (root, layout) = fresh_layout("already_deleted");
    assert!(!layout.rule_path(7000).exists());

    let removed =
        fileops::remove_rule(&layout, 7000).expect("remove_rule must succeed, not error, on an absent file");
    assert!(!removed, "nothing was actually removed");

    let _ = std::fs::remove_dir_all(&root);
}

// --- read_rule ----------------------------------------------------------

#[test]
fn read_rule_returns_none_when_absent_and_some_content_when_present() {
    let (root, layout) = fresh_layout("read_rule");
    assert_eq!(fileops::read_rule(&layout, 9000).unwrap(), None);

    let content = render_rule(9000, "frank", Expiry::Never);
    std::fs::write(layout.rule_path(9000), &content).unwrap();
    assert_eq!(fileops::read_rule(&layout, 9000).unwrap(), Some(content));

    let _ = std::fs::remove_dir_all(&root);
}

// --- list_rule_uids: threat matrix "Rule-file target selection" --------

#[test]
fn list_rule_uids_never_returns_a_uid_for_a_bak_suffixed_padded_or_tmp_filename() {
    let (root, layout) = fresh_layout("list_rejects_bak");
    let dir = root.join("sudoers.d");
    let content = render_rule(1000, "jorge", Expiry::Never);
    std::fs::write(dir.join("90-nopass-1000"), &content).unwrap();
    std::fs::write(dir.join("90-nopass-1000.bak"), &content).unwrap();
    std::fs::write(dir.join("90-nopass-01000"), &content).unwrap();
    // The dot-prefixed, `.tmp`-suffixed name `write_rule_atomic` itself
    // writes to mid-transaction (`Layout::rule_tmp_path`) must never be
    // reported either — it is filtered by `uid_from_rule_filename`
    // rejecting the leading dot, the same filename-only gate this test
    // already exercises for `.bak` and the zero-padded variant.
    std::fs::write(dir.join(".90-nopass-1000.tmp"), &content).unwrap();

    let uids = fileops::list_rule_uids(&layout).expect("list_rule_uids succeeds");
    assert_eq!(uids, vec![1000], "only the canonical filename must be reported");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn list_rule_uids_returns_every_canonical_uid_sorted() {
    let (root, layout) = fresh_layout("list_multiple");
    let dir = root.join("sudoers.d");
    std::fs::write(dir.join("90-nopass-3000"), render_rule(3000, "x", Expiry::Never)).unwrap();
    std::fs::write(dir.join("90-nopass-1000"), render_rule(1000, "y", Expiry::Never)).unwrap();
    std::fs::write(dir.join("90-nopass-2000"), render_rule(2000, "z", Expiry::Never)).unwrap();

    let uids = fileops::list_rule_uids(&layout).expect("list_rule_uids succeeds");
    assert_eq!(uids, vec![1000, 2000, 3000]);

    let _ = std::fs::remove_dir_all(&root);
}

/// Phase 6 correction (independent verifier finding, security-rated):
/// `list_rule_uids` previously filtered directory entries by filename
/// only, through `uid_from_rule_filename`, and never inspected the
/// entry's file type. A directory or a symlink named like a canonical
/// rule filename was therefore returned as if it were a managed rule
/// file. This is inert today only because nothing calls `list_rule_uids`
/// yet (Phase 7 wires it into `expire --boot`, which deletes whatever
/// this function returns), so closing it now is load-bearing before that
/// wiring lands.
#[test]
fn list_rule_uids_skips_a_directory_named_like_a_rule() {
    let (root, layout) = fresh_layout("list_skips_dir");
    let dir = root.join("sudoers.d");
    std::fs::write(dir.join("90-nopass-1000"), render_rule(1000, "jorge", Expiry::Never)).unwrap();
    std::fs::create_dir(dir.join("90-nopass-9000")).unwrap();

    let uids = fileops::list_rule_uids(&layout).expect("list_rule_uids succeeds");
    assert_eq!(uids, vec![1000], "a directory named like a rule must never be reported as a uid");

    let _ = std::fs::remove_dir_all(&root);
}

/// See `list_rule_uids_skips_a_directory_named_like_a_rule` above for the
/// full finding. This is the symlink half: even a symlink that resolves
/// to a genuine, unrelated file elsewhere on disk must be excluded,
/// because `write_rule_atomic` never creates a symlink at a rule path —
/// only a regular file it created itself and then renamed into place —
/// so a symlink here is definitionally not a file this sweep created.
#[test]
fn list_rule_uids_skips_a_symlink_named_like_a_rule() {
    let (root, layout) = fresh_layout("list_skips_symlink");
    let dir = root.join("sudoers.d");
    std::fs::write(dir.join("90-nopass-2000"), render_rule(2000, "ana", Expiry::Never)).unwrap();
    let real_target = root.join("real_file_elsewhere");
    std::fs::write(&real_target, render_rule(9500, "eve", Expiry::Never)).unwrap();
    std::os::unix::fs::symlink(&real_target, dir.join("90-nopass-9500")).unwrap();

    let uids = fileops::list_rule_uids(&layout).expect("list_rule_uids succeeds");
    assert_eq!(
        uids,
        vec![2000],
        "a symlink named like a rule must never be reported as a uid, even pointing at a real file"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// --- write_rule_atomic: visudo -cf rejection with a preexisting rule ---

/// Adversarial case the suite previously missed: `visudo -cf` rejecting
/// when a LEGITIMATE rule already sits at the final path (as opposed to
/// `visudo_reject_unlinks_tmp_and_leaves_the_final_path_untouched` above,
/// which covers rejection when no destination file exists at all). Pins
/// that the existing rule survives byte-for-byte and mode-for-mode, and
/// that the tmp file is still cleaned up.
#[test]
fn visudo_reject_leaves_a_preexisting_legitimate_rule_at_the_final_path_byte_and_mode_identical() {
    let (root, layout) = fresh_layout("visudo_reject_preexisting");
    let final_path = layout.rule_path(3100);
    let existing_content = render_rule(3100, "existing", Expiry::Never);
    std::fs::write(&final_path, &existing_content).unwrap();
    let mut perms = std::fs::metadata(&final_path).unwrap().permissions();
    perms.set_mode(0o440);
    std::fs::set_permissions(&final_path, perms).unwrap();
    let existing_mode = std::fs::metadata(&final_path).unwrap().permissions().mode() & 0o777;

    let visudo = write_fake_visudo(&root, false);
    let binaries = binaries_with_visudo(&visudo);
    let replacement_content = render_rule(3100, "replacement", Expiry::Never);

    let err =
        fileops::write_rule_atomic(&layout, &SystemRunner, &binaries, 3100, &replacement_content).unwrap_err();
    assert!(matches!(err, HelperError::VisudoRejected { .. }), "expected VisudoRejected, got {err:?}");
    assert_eq!(err.exit_code(), 14);
    assert!(!layout.rule_tmp_path(3100).exists(), "tmp file must be unlinked after visudo rejects");
    assert_eq!(
        std::fs::read_to_string(&final_path).unwrap(),
        existing_content,
        "a legitimate rule already at the final path must be byte-identical after a rejected replacement"
    );
    assert_eq!(
        std::fs::metadata(&final_path).unwrap().permissions().mode() & 0o777,
        existing_mode,
        "a legitimate rule already at the final path must keep its mode after a rejected replacement"
    );

    let _ = std::fs::remove_dir_all(&root);
}

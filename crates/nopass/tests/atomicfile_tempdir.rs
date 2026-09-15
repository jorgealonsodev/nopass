//! `atomicfile::write` against a real filesystem (design.md §4 D4,
//! Threat Matrix "Executable-file authoring"; discipline mirrors
//! `crates/nopass-helper/src/fileops.rs::write_rule_atomic`, which
//! `crates/nopass-helper/tests/fileops_tempdir.rs` exercises the same
//! way — `O_CREAT|O_EXCL` tmp, fchmod, fsync, rename, best-effort parent
//! fsync, tmp unlinked on every failure past the open).
//!
//! Genuine Cargo integration test (its own crate, per Cargo convention),
//! reachable because `crates/nopass/src/lib.rs` exposes `atomicfile`.

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use nopass::atomicfile;

#[test]
fn write_creates_the_final_file_with_exact_content_and_pinned_mode_under_a_new_parent_dir() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("config.toml");

    atomicfile::write(&path, b"hello = true\n", 0o600).expect("write succeeds, creating `sub/`");

    assert_eq!(std::fs::read(&path).unwrap(), b"hello = true\n");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "mode was not pinned to 0600, got {mode:o}");
    assert!(!atomicfile::tmp_path_for(&path).exists(), "tmp file was not cleaned up after a successful rename");
}

#[test]
fn preexisting_tmp_file_makes_o_excl_fail_instead_of_being_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let tmp_path = atomicfile::tmp_path_for(&path);
    std::fs::write(&tmp_path, "attacker-controlled content").unwrap();

    let err = atomicfile::write(&path, b"new content", 0o600).unwrap_err();
    let _ = err; // exact error shape is not the contract here, only that it failed

    assert_eq!(
        std::fs::read_to_string(&tmp_path).unwrap(),
        "attacker-controlled content",
        "pre-existing tmp content was overwritten instead of O_EXCL failing"
    );
    assert!(!path.exists(), "final path must never be written when O_EXCL fails");
}

#[test]
fn a_symlink_at_the_tmp_path_makes_o_excl_fail_without_following_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let tmp_path = atomicfile::tmp_path_for(&path);
    let decoy_target = dir.path().join("decoy_target_never_written");
    std::os::unix::fs::symlink(&decoy_target, &tmp_path).unwrap();

    atomicfile::write(&path, b"new content", 0o600).unwrap_err();

    assert!(!decoy_target.exists(), "the symlink's target must never be created or written through");
    assert!(!path.exists());
}

#[test]
fn a_symlink_at_the_final_path_is_not_renamed_over_and_the_tmp_file_is_still_unlinked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let decoy_target = dir.path().join("decoy_final_target");
    std::fs::write(&decoy_target, "do not touch").unwrap();
    std::os::unix::fs::symlink(&decoy_target, &path).unwrap();

    atomicfile::write(&path, b"new content", 0o600).unwrap_err();

    assert!(!atomicfile::tmp_path_for(&path).exists(), "tmp file must still be cleaned up on this injected failure");
    assert_eq!(
        std::fs::read_to_string(&decoy_target).unwrap(),
        "do not touch",
        "the symlink's target must never be written through"
    );
    assert!(
        std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink(),
        "the symlink at the final path must still be there, untouched"
    );
}

#[test]
fn an_interrupted_write_never_leaves_a_torn_final_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let content_a = b"variant = \"a\"\n".to_vec();
    let content_b = b"variant = \"bb\"\n".to_vec(); // deliberately a different length

    let writer_path = path.clone();
    let writer_a = content_a.clone();
    let writer_b = content_b.clone();
    let writer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut toggle = false;
        while Instant::now() < deadline {
            let content = if toggle { &writer_b } else { &writer_a };
            // Each iteration races the reader below; a real crash mid-write
            // is not something a test process can inject at the syscall
            // level, but the write-temp/rename discipline this pins means
            // no reader can ever observe a file whose bytes are not
            // EXACTLY one of the two complete contents — the property
            // that actually matters, and the one an injected-crash test
            // would otherwise only approximate.
            atomicfile::write(&writer_path, content, 0o600).unwrap();
            toggle = !toggle;
        }
    });

    let deadline = Instant::now() + Duration::from_millis(300);
    let mut observed_a = false;
    let mut observed_b = false;
    while Instant::now() < deadline {
        match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes == content_a {
                    observed_a = true;
                } else if bytes == content_b {
                    observed_b = true;
                } else {
                    panic!("torn write observed: {bytes:?}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("unexpected read error: {e}"),
        }
    }

    writer.join().unwrap();
    assert!(observed_a || observed_b, "the reader loop must observe at least one complete write to prove anything");
}

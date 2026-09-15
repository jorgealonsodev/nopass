//! `config::read`/`resolve`/`write` against a real filesystem (design.md
//! §4 D4; spec `user-config`; threat matrix "Config as foreign input").
//!
//! Genuine Cargo integration test (its own crate, per Cargo convention),
//! reachable because `crates/nopass/src/lib.rs` exposes `config`.

use std::os::unix::fs::PermissionsExt;

use nopass::atomicfile;
use nopass::config::{self, Config, ConfigFault, ConfigReading};
use nopass::duration::GrantDuration;

// ---- tolerant parsing (task 2.4) ----

#[test]
fn malformed_toml_syntax_degrades_to_defaults_and_surfaces_a_fault() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, b"default_duration = \"1h [[[not valid toml").unwrap();

    let reading = config::read(&path);

    assert_eq!(reading, ConfigReading::Faulted(ConfigFault::Malformed));
    let (resolved, fault) = config::resolve(&reading);
    assert_eq!(resolved, Config::defaults());
    assert_eq!(fault, Some(ConfigFault::Malformed));
}

#[test]
fn non_utf8_bytes_degrade_to_defaults_and_surface_a_fault() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, [0xff, 0xfe, 0x00, 0xff]).unwrap();

    let reading = config::read(&path);

    assert_eq!(reading, ConfigReading::Faulted(ConfigFault::Malformed));
    let (resolved, fault) = config::resolve(&reading);
    assert_eq!(resolved, Config::defaults());
    assert_eq!(fault, Some(ConfigFault::Malformed));
}

#[test]
fn truncated_content_mid_string_degrades_to_defaults_and_surfaces_a_fault() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    // An unterminated string is a TOML syntax error, not a valid document.
    std::fs::write(&path, b"default_duration = \"8h").unwrap();

    let reading = config::read(&path);

    assert_eq!(reading, ConfigReading::Faulted(ConfigFault::Malformed));
}

#[test]
fn an_unknown_key_is_ignored_and_recognized_fields_are_still_honored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, b"default_duration = \"4h\"\nfuture_field = 42\nwarn_before_activation = false\n").unwrap();

    let reading = config::read(&path);

    assert_eq!(
        reading,
        ConfigReading::Parsed(Config { default_duration: GrantDuration::Hours4, warning_acknowledged: true })
    );
}

#[test]
fn an_unrecognized_default_duration_falls_back_while_preserving_a_valid_sibling_field() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, b"default_duration = \"3h\"\nwarn_before_activation = false\n").unwrap();

    let reading = config::read(&path);

    // The bad `default_duration` falls back to the schema default (`Hour1`),
    // but `warn_before_activation = false` from the SAME document is not
    // discarded (design.md §4 D4 "Per-field tolerance").
    assert_eq!(
        reading,
        ConfigReading::Parsed(Config { default_duration: GrantDuration::Hour1, warning_acknowledged: true })
    );
}

#[test]
fn a_wrong_typed_default_duration_falls_back_to_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, b"default_duration = 42\nwarn_before_activation = true\n").unwrap();

    let reading = config::read(&path);

    assert_eq!(
        reading,
        ConfigReading::Parsed(Config { default_duration: GrantDuration::Hour1, warning_acknowledged: false })
    );
}

// ---- atomic write (task 2.5) ----

#[test]
fn write_persists_a_config_that_reads_back_identically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let written = Config { default_duration: GrantDuration::Hours8, warning_acknowledged: true };

    config::write(&path, &written).expect("write must succeed against a fresh directory");

    let reading = config::read(&path);
    assert_eq!(reading, ConfigReading::Parsed(written));
}

#[test]
fn write_uses_atomicfile_and_leaves_no_tmp_sibling_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let written = Config { default_duration: GrantDuration::Minutes15, warning_acknowledged: false };

    config::write(&path, &written).unwrap();

    assert!(!atomicfile::tmp_path_for(&path).exists(), "atomicfile's tmp sibling must be cleaned up after a successful write");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "config.toml must be written with 0600, not atomicfile.rs's other 0644 caller mode");
}

#[test]
fn a_write_that_fails_after_the_tmp_file_is_created_leaves_the_prior_config_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    // A valid, parseable document behind the symlink so config::write's own
    // "existing file is faulted, back it up" branch (task 2.6) does not
    // fire here — this test isolates atomicfile::write's rename-over-a-
    // symlink guard (already unit-tested directly in
    // atomicfile_tempdir.rs), composed through config::write.
    let decoy_target = dir.path().join("decoy");
    std::fs::write(&decoy_target, "default_duration = \"1h\"\nwarn_before_activation = true\n").unwrap();
    std::os::unix::fs::symlink(&decoy_target, &path).unwrap();

    let attempted = Config { default_duration: GrantDuration::Permanent, warning_acknowledged: true };
    config::write(&path, &attempted).expect_err("write must fail when the final path is a symlink");

    assert_eq!(
        std::fs::read_to_string(&decoy_target).unwrap(),
        "default_duration = \"1h\"\nwarn_before_activation = true\n",
        "the symlink's target must never be written through"
    );
}

// ---- .bak rename of a faulted existing file before a fresh write (task 2.6) ----

#[test]
fn write_over_a_faulted_existing_file_renames_it_to_bak_before_writing_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let hand_edit_mistake = b"default_duration = \"1h [[[ this is the user's broken hand-edit".to_vec();
    std::fs::write(&path, &hand_edit_mistake).unwrap();
    assert_eq!(config::read(&path), ConfigReading::Faulted(ConfigFault::Malformed), "fixture must actually be faulted");

    let fresh = Config { default_duration: GrantDuration::UntilReboot, warning_acknowledged: true };
    config::write(&path, &fresh).expect("write over a faulted file must still succeed");

    let bak_path = dir.path().join("config.toml.bak");
    assert_eq!(
        std::fs::read(&bak_path).unwrap(),
        hand_edit_mistake,
        "the user's broken hand-edit must be preserved verbatim in the .bak sibling, not discarded"
    );
    assert_eq!(config::read(&path), ConfigReading::Parsed(fresh), "the fresh document must be readable at the original path");
}

#[test]
fn write_over_a_valid_existing_file_does_not_create_a_bak_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let original = Config { default_duration: GrantDuration::Hour1, warning_acknowledged: false };
    config::write(&path, &original).unwrap();

    let updated = Config { default_duration: GrantDuration::Hours4, warning_acknowledged: true };
    config::write(&path, &updated).unwrap();

    assert!(!dir.path().join("config.toml.bak").exists(), "a valid-to-valid rewrite must never create a .bak sibling");
}

#[test]
fn write_preserves_comments_and_unknown_keys_already_present_in_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        b"# a note the user left for themselves\ndefault_duration = \"1h\"\nfuture_field = \"kept\"\nwarn_before_activation = true\n",
    )
    .unwrap();

    let updated = Config { default_duration: GrantDuration::Hours8, warning_acknowledged: true };
    config::write(&path, &updated).unwrap();

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("# a note the user left for themselves"), "comment must survive a write: {contents:?}");
    assert!(contents.contains("future_field = \"kept\""), "unknown key must survive a write: {contents:?}");
    assert!(contents.contains("default_duration = \"8h\""), "the updated field must reflect the new value: {contents:?}");
}

// ---- hostile default_duration never reaches argv (task 2.7; threat matrix "Config as foreign input") ----

#[test]
fn a_hostile_default_duration_value_never_reaches_argv_only_the_default_does() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, b"default_duration = \"; rm -rf /\"\nwarn_before_activation = true\n").unwrap();

    let reading = config::read(&path);
    let (resolved, fault) = config::resolve(&reading);

    // GrantDuration::parse's closed six-arm match is the only funnel from
    // the file's string into a variant (duration.rs); an unrecognized
    // value never reaches it as anything but `None`, so the config
    // resolves to the safe default and never to a string that could ever
    // be interpolated into `args()`'s argv.
    assert_eq!(resolved.default_duration, GrantDuration::Hour1);
    assert_eq!(fault, None, "a semantically bad but syntactically valid document is Parsed, not Faulted");

    // The only place a duration ever becomes argv: composition with
    // duration.rs's `args`, pinned here so a future change to either side
    // cannot silently let a hostile string leak into a command line.
    let argv = resolved.default_duration.args(1_789_000_000);
    assert_eq!(argv, vec!["--until", "1789003600"]);
    assert!(!argv.iter().any(|a| a.contains("rm") || a.contains(';')), "hostile input must never surface in argv: {argv:?}");
}

//! `~/.config/autostart/nopass.desktop`: whether the tray starts with the
//! user's session (design.md §4 D5; spec `autostart-entry` (all)).
//!
//! **The security property this module exists to hold**: [`enable`] writes
//! [`TEMPLATE`] — an `include_str!` of `data/nopass.desktop`, a compile-time
//! constant — and NOTHING ELSE. `Exec=nopass` in that file runs at every
//! login as the user; a `.desktop` writer whose content is composed from
//! runtime input (`config.toml`, an argument, an env var) is an
//! executable-file-authoring vulnerability (threat matrix "Executable-file
//! authoring"). `enable` therefore takes no content parameter at all — only
//! the destination path — so there is no call shape through which caller
//! data could ever reach the written bytes.
//!
//! Phase 4 tasks 4.1–4.6 land here in order: `4.2` path resolution (mirrors
//! `config.rs`'s `path`/`path_from_env` XDG idiom), `4.3` `enable`'s
//! template-verbatim write via [`crate::atomicfile::write`] (never a second
//! atomic-write path), `4.4` `disable`'s unlink, `4.5` `read`'s
//! marker-aware on-disk-truth table, `4.6` a pinning assertion that
//! packaging ships no autostart entry by default.

use std::path::{Path, PathBuf};

use crate::atomicfile::{self, AtomicFileError};

/// Whether the session will actually autostart the tray, as derived from
/// on-disk truth at each read — never cached across menu opens (design.md
/// §4 D5; spec `autostart-entry` "Checkbox State Reflects On-Disk Truth").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutostartState {
    Enabled,
    Disabled,
    Indeterminate,
}

/// The exact, unmodified content [`enable`] writes. `include_str!`, not a
/// runtime read of an installed `/usr/share/applications/` copy — the tray
/// must not depend on its own package layout (design.md §4 D5) — and never
/// composed, formatted, or templated with any value from `config.toml` or
/// elsewhere (threat matrix "Executable-file authoring").
pub const TEMPLATE: &str = include_str!("../../../data/nopass.desktop");

/// Resolves the autostart entry path: `$XDG_CONFIG_HOME/autostart/nopass.desktop`
/// when `XDG_CONFIG_HOME` is set and non-empty, `~/.config/autostart/nopass.desktop`
/// otherwise, per the XDG autostart specification (spec `autostart-entry`
/// "Autostart Path Honors XDG_CONFIG_HOME").
///
/// Reads `std::env::var` directly — a plain read, not the `unsafe`
/// `std::env::set_var` this crate's `#![forbid(unsafe_code)]` forbids — so
/// production code goes through this real lookup, and [`path_from_env`] is
/// the seam tests use to exercise both branches deterministically (mirrors
/// `config::path`/`config::path_from_env`).
pub fn path() -> PathBuf {
    path_from_env(std::env::var("XDG_CONFIG_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref())
}

/// [`path`]'s table, parameterised by explicit `XDG_CONFIG_HOME`/`HOME`
/// values rather than reading the environment (spec `autostart-entry`
/// "Autostart Path Honors XDG_CONFIG_HOME").
pub fn path_from_env(xdg_config_home: Option<&str>, home: Option<&str>) -> PathBuf {
    match xdg_config_home.filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("autostart").join("nopass.desktop"),
        None => {
            let home = home.filter(|v| !v.is_empty()).expect("HOME must be set to resolve the autostart path");
            PathBuf::from(home).join(".config").join("autostart").join("nopass.desktop")
        }
    }
}

/// Creates `p`'s parent directory if absent and writes [`TEMPLATE`]
/// verbatim to `p` via [`atomicfile::write`] (mode `0644`, never `+x`) —
/// this crate never invents a second atomic-write path (spec
/// `autostart-entry` "Create Writes the Template Verbatim"; threat matrix
/// "Executable-file authoring").
pub fn enable(p: &Path) -> Result<(), AtomicFileError> {
    atomicfile::write(p, TEMPLATE.as_bytes(), 0o644)
}

/// Deletes `p`. Idempotent: an already-absent entry is treated as
/// already-disabled, not an error (spec `autostart-entry` "Remove Deletes
/// Only the NoPass Entry", "Removing an already-absent entry does not
/// error"). Never touches any other file.
pub fn disable(p: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Computes on-disk truth for `p` — presence alone is not enough, because
/// GNOME Tweaks and the freedesktop spec both disable an entry by marking
/// it rather than deleting it (design.md §4 D5; spec `autostart-entry`
/// "Checkbox State Reflects On-Disk Truth"):
///
/// | On disk | ⇒ |
/// |---|---|
/// | absent (`ENOENT`) | [`AutostartState::Disabled`] |
/// | present, contains `Hidden=true` or `X-GNOME-Autostart-enabled=false` | [`AutostartState::Disabled`] |
/// | present, otherwise | [`AutostartState::Enabled`] |
/// | any other I/O error | [`AutostartState::Indeterminate`] |
pub fn read(p: &Path) -> AutostartState {
    match std::fs::read(p) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            if text.contains("Hidden=true") || text.contains("X-GNOME-Autostart-enabled=false") {
                AutostartState::Disabled
            } else {
                AutostartState::Enabled
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => AutostartState::Disabled,
        Err(_) => AutostartState::Indeterminate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- path resolution (task 4.2) ----

    #[test]
    fn xdg_config_home_overrides_the_default_path_when_set_and_non_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let xdg = tmp.path().join("custom-xdg");
        let xdg_str = xdg.to_str().unwrap();

        let resolved = path_from_env(Some(xdg_str), Some("/home/someone"));

        assert_eq!(resolved, xdg.join("autostart").join("nopass.desktop"));
    }

    #[test]
    fn default_path_is_used_when_xdg_config_home_is_unset() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home-dir");
        let home_str = home.to_str().unwrap();

        let resolved = path_from_env(None, Some(home_str));

        assert_eq!(resolved, home.join(".config").join("autostart").join("nopass.desktop"));
    }

    #[test]
    fn empty_xdg_config_home_falls_back_to_the_home_based_default_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home-dir");
        let home_str = home.to_str().unwrap();

        let resolved = path_from_env(Some(""), Some(home_str));

        assert_eq!(resolved, home.join(".config").join("autostart").join("nopass.desktop"));
    }

    // ---- enable (task 4.3) ----

    /// The one test whose sole job is to catch a future regression of the
    /// security property this module exists for: `enable` takes exactly
    /// one parameter (the destination `Path`) and the bytes landing at
    /// that path are byte-for-byte [`TEMPLATE`] — not a string built with
    /// `format!`, not anything read from `config.toml`, not anything that
    /// could carry caller-supplied data into `Exec=`. If `enable` ever
    /// grows a second parameter or composes its content, THIS assertion
    /// (byte equality against the compile-time constant, not merely
    /// "contains Exec=") is what breaks (threat matrix "Executable-file
    /// authoring"; spec `autostart-entry` "Create Writes the Template
    /// Verbatim").
    #[test]
    fn enable_creates_a_missing_autostart_directory_and_writes_bytes_byte_equal_to_template() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("autostart").join("nopass.desktop");
        assert!(!target.parent().unwrap().exists(), "precondition: autostart/ does not exist yet");

        enable(&target).unwrap();

        let written = std::fs::read(&target).unwrap();
        assert_eq!(written, TEMPLATE.as_bytes(), "written bytes must be byte-equal to TEMPLATE, never composed content");
    }

    #[cfg(unix)]
    #[test]
    fn enable_refuses_to_follow_a_pre_planted_symlink_at_the_target() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let autostart_dir = tmp.path().join("autostart");
        std::fs::create_dir_all(&autostart_dir).unwrap();
        let target = autostart_dir.join("nopass.desktop");
        let evil_dest = tmp.path().join("evil-target");
        symlink(&evil_dest, &target).unwrap();

        let result = enable(&target);

        assert!(result.is_err(), "enable must refuse a pre-planted symlink at the target, not follow it");
        assert!(!evil_dest.exists(), "the symlink's destination must never be created/written");
        let still_symlink = std::fs::symlink_metadata(&target).unwrap().file_type().is_symlink();
        assert!(still_symlink, "the pre-planted symlink itself must be left untouched, not replaced");
    }

    // ---- disable (task 4.4) ----

    #[test]
    fn disable_deletes_the_entry_and_leaves_a_sibling_desktop_file_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let autostart_dir = tmp.path().join("autostart");
        std::fs::create_dir_all(&autostart_dir).unwrap();
        let target = autostart_dir.join("nopass.desktop");
        let sibling = autostart_dir.join("other-app.desktop");
        std::fs::write(&target, TEMPLATE).unwrap();
        std::fs::write(&sibling, "[Desktop Entry]\nExec=other-app\n").unwrap();

        disable(&target).unwrap();

        assert!(!target.exists(), "nopass.desktop must be deleted");
        assert!(sibling.exists(), "other-app.desktop must be untouched");
        let sibling_content = std::fs::read_to_string(&sibling).unwrap();
        assert_eq!(sibling_content, "[Desktop Entry]\nExec=other-app\n", "sibling content must be unchanged");
    }

    #[test]
    fn disable_of_an_already_absent_entry_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("autostart").join("nopass.desktop");
        assert!(!target.exists());

        let result = disable(&target);

        assert!(result.is_ok(), "removal of an already-absent entry must not error");
    }

    // ---- read (task 4.5) ----

    #[test]
    fn read_of_an_absent_entry_is_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("autostart").join("nopass.desktop");

        assert_eq!(read(&target), AutostartState::Disabled);
    }

    #[test]
    fn read_of_an_entry_marked_hidden_true_is_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nopass.desktop");
        std::fs::write(&target, "[Desktop Entry]\nExec=nopass\nHidden=true\n").unwrap();

        assert_eq!(read(&target), AutostartState::Disabled);
    }

    #[test]
    fn read_of_an_entry_marked_gnome_autostart_disabled_is_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nopass.desktop");
        std::fs::write(&target, "[Desktop Entry]\nExec=nopass\nX-GNOME-Autostart-enabled=false\n").unwrap();

        assert_eq!(read(&target), AutostartState::Disabled);
    }

    #[test]
    fn read_of_an_unmarked_present_entry_is_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nopass.desktop");
        std::fs::write(&target, TEMPLATE).unwrap();

        assert_eq!(read(&target), AutostartState::Enabled);
    }

    #[test]
    fn read_of_a_path_that_errors_for_a_reason_other_than_absence_is_indeterminate() {
        let tmp = tempfile::tempdir().unwrap();
        // A directory at the target path: `std::fs::read` fails with an
        // I/O error that is NOT `NotFound` (e.g. "Is a directory"), giving
        // a deterministic non-ENOENT failure without permission tricks.
        let target = tmp.path().join("nopass.desktop");
        std::fs::create_dir_all(&target).unwrap();

        assert_eq!(read(&target), AutostartState::Indeterminate);
    }

    // ---- packaging (task 4.6) — pins existing state; GREEN is "none" ----

    /// Packaging ships no autostart entry by default (spec `autostart-entry`
    /// "Packaging Ships No Autostart Entry By Default"; design.md "Migration
    /// / Rollout": "Packaging still ships no autostart entry
    /// (`crates/nopass/Cargo.toml:61-62`, unchanged)"). This is a pinning
    /// assertion, not a new behavior: it passes today because M3 has not
    /// added an autostart install anywhere, and it exists to fail loudly if
    /// that ever changes. If this test fails, the fix is to remove the
    /// accidental install, never to weaken this assertion.
    #[test]
    fn packaging_manifest_installs_no_file_under_any_autostart_directory() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();

        assert!(
            !manifest.contains("autostart/"),
            "cargo-deb `assets` must not install anything under an `autostart/` directory"
        );

        let postinst = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/debian/postinst")).unwrap();
        let prerm = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/debian/prerm")).unwrap();
        assert!(!postinst.contains("autostart/"), "postinst must not create/enable an autostart entry");
        assert!(!prerm.contains("autostart/"), "prerm must not reference an autostart entry");
    }
}

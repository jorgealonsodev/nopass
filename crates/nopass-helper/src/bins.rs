//! Absolute-path resolution for the external binaries invoked via
//! `CommandRunner` (design.md §3; helper-cli §External Command Invocation
//! Discipline). First-existing-wins, resolved lazily per name so `status`
//! and `disable` still work on a host without `systemd-run` installed.
//!
//! `checks`, `timer`, and `fileops` (Phases 5-7) are the modules that
//! actually call `resolve()` in production; `ops.rs` (Phase 7) is the
//! only caller wired into `main`'s `dispatch`. No `#[allow(dead_code)]`
//! is needed here despite that: every item below is `pub` inside this
//! crate's `pub mod bins` (declared in `lib.rs`), which makes it public
//! library API — the `dead_code` lint does not fire on `pub` items of a
//! library crate, since an external crate (including this crate's own
//! `tests/fileops_tempdir.rs`, which does exactly this) could call them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::HelperError;

/// Ordered, absolute-path candidate table for every external binary the
/// helper invokes. Construct once via [`Binaries::system()`] in
/// production, or [`Binaries::from_candidates`] to inject test paths
/// (analogous to `nopass_core::paths::Layout::under`).
pub struct Binaries {
    candidates: HashMap<&'static str, Vec<PathBuf>>,
}

impl Binaries {
    /// The real candidate table (design.md §3): ordered per distro family,
    /// first existing candidate wins.
    pub fn system() -> Self {
        Binaries::from_candidates(&[
            ("visudo", &[Path::new("/usr/sbin/visudo"), Path::new("/sbin/visudo"), Path::new("/usr/bin/visudo")]),
            ("sudo", &[Path::new("/usr/bin/sudo"), Path::new("/bin/sudo")]),
            ("systemctl", &[Path::new("/usr/bin/systemctl"), Path::new("/bin/systemctl")]),
            ("systemd-run", &[Path::new("/usr/bin/systemd-run"), Path::new("/bin/systemd-run")]),
            ("sh", &[Path::new("/bin/sh"), Path::new("/usr/bin/sh")]),
        ])
    }

    /// Builds a `Binaries` from an arbitrary candidate table — how a test
    /// points every name at paths inside a temp directory, including an
    /// **empty** candidate list for a given name.
    pub fn from_candidates(table: &[(&'static str, &[&Path])]) -> Self {
        let candidates =
            table.iter().map(|(name, paths)| (*name, paths.iter().map(|p| p.to_path_buf()).collect())).collect();
        Binaries { candidates }
    }

    /// Resolves `name` to its first absolute candidate that is a regular
    /// file (following symlinks — merged-usr distros symlink e.g.
    /// `/bin` to `/usr/bin`). Never falls back to a `PATH` lookup. A
    /// candidate that is relative, missing, a dangling symlink, or a
    /// directory is skipped rather than "resolved", so a directory never
    /// surfaces as a late raw `EISDIR` spawn error. A name with no
    /// qualifying candidates, or a name not present in the table at
    /// all, yields `BinaryMissing`. Resolution touches only `name`'s own
    /// candidates — it never attempts, and therefore never requires the
    /// existence of, any other binary's candidates.
    ///
    /// design.md §3 specifies `nix::sys::stat::stat` for this, but `nix`
    /// is not a dependency of this crate until Phase 8. `std::fs::metadata`
    /// follows symlinks exactly as `stat` does, so it is used here
    /// instead of `symlink_metadata` (which would reject a valid
    /// symlinked candidate) or `Path::exists` (which cannot distinguish
    /// a directory from a regular file).
    pub fn resolve(&self, name: &'static str) -> Result<&Path, HelperError> {
        let candidates = self.candidates.get(name).map(Vec::as_slice).unwrap_or(&[]);
        candidates
            .iter()
            .find(|p| p.is_absolute() && std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false))
            .map(PathBuf::as_path)
            .ok_or(HelperError::BinaryMissing { name })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("nopass_test_bins_{tag}_{}_{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn first_existing_candidate_wins_when_earlier_candidates_are_absent() {
        let dir = temp_test_dir("first_wins");
        let first = dir.join("usr_sbin_visudo");
        let second = dir.join("sbin_visudo");
        touch(&second); // only the second candidate exists
        let bins = Binaries::from_candidates(&[("visudo", &[first.as_path(), second.as_path()])]);
        assert_eq!(bins.resolve("visudo").unwrap(), second.as_path());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_earlier_existing_candidate_wins_over_a_later_one() {
        let dir = temp_test_dir("earlier_wins");
        let first = dir.join("usr_sbin_visudo");
        let second = dir.join("sbin_visudo");
        touch(&first);
        touch(&second); // both exist — the earlier one in the list must win
        let bins = Binaries::from_candidates(&[("visudo", &[first.as_path(), second.as_path()])]);
        assert_eq!(bins.resolve("visudo").unwrap(), first.as_path());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_candidate_list_yields_binary_missing_mapped_to_exit_1() {
        let bins = Binaries::from_candidates(&[("visudo", &[])]);
        let err = bins.resolve("visudo").unwrap_err();
        assert!(matches!(err, HelperError::BinaryMissing { name: "visudo" }));
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn a_name_absent_from_the_table_also_yields_binary_missing() {
        let bins = Binaries::from_candidates(&[("visudo", &[])]);
        let err = bins.resolve("systemd-run").unwrap_err();
        assert!(matches!(err, HelperError::BinaryMissing { name: "systemd-run" }));
    }

    #[test]
    fn resolving_one_binary_never_touches_another_binary_candidates() {
        // Proves per-name laziness: `systemd-run` has zero candidates
        // (would fail immediately if ever resolved), yet resolving
        // `visudo` succeeds untouched by that. This is the same property
        // `ops::status` (Phase 7) relies on to never attempt resolving
        // `systemd-run`.
        let dir = temp_test_dir("laziness");
        let visudo_path = dir.join("visudo");
        touch(&visudo_path);
        let bins = Binaries::from_candidates(&[("visudo", &[visudo_path.as_path()]), ("systemd-run", &[])]);
        assert_eq!(bins.resolve("visudo").unwrap(), visudo_path.as_path());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn system_binaries_reject_an_unknown_name() {
        let bins = Binaries::system();
        let err = bins.resolve("not-a-real-binary").unwrap_err();
        assert!(matches!(err, HelperError::BinaryMissing { name: "not-a-real-binary" }));
    }

    // verify-report W5: every other test in this module builds a
    // `Binaries` via `from_candidates` with synthetic, test-owned paths —
    // the REAL production table `Binaries::system()` builds is never
    // itself asserted. A typo or reordering there (the `visudo` order
    // `/usr/sbin` → `/sbin` → `/usr/bin` is distro-load-bearing per
    // design.md §3) would fail nothing. This test reads the private
    // `candidates` map directly (this `mod tests` is a child module of
    // `bins`, so it has that access) and pins both the exact name set
    // and the exact per-name candidate order, without requiring any of
    // those paths to exist on the machine running this suite — `resolve`
    // is deliberately never called here.
    #[test]
    fn system_candidate_table_matches_design_md_section_3_names_and_order() {
        let bins = Binaries::system();
        let expected: &[(&str, &[&str])] = &[
            ("visudo", &["/usr/sbin/visudo", "/sbin/visudo", "/usr/bin/visudo"]),
            ("sudo", &["/usr/bin/sudo", "/bin/sudo"]),
            ("systemctl", &["/usr/bin/systemctl", "/bin/systemctl"]),
            ("systemd-run", &["/usr/bin/systemd-run", "/bin/systemd-run"]),
            ("sh", &["/bin/sh", "/usr/bin/sh"]),
        ];

        assert_eq!(
            bins.candidates.len(),
            expected.len(),
            "Binaries::system() must declare exactly the names design.md §3 lists, no more, no fewer"
        );
        for (name, expected_paths) in expected {
            let actual = bins
                .candidates
                .get(name)
                .unwrap_or_else(|| panic!("Binaries::system() must declare a candidate list for {name:?}"));
            let actual_str: Vec<&str> = actual.iter().map(|p| p.to_str().unwrap()).collect();
            assert_eq!(
                &actual_str, expected_paths,
                "candidate order for {name:?} must match design.md §3 exactly (first-existing-wins)"
            );
        }
    }

    #[test]
    fn a_directory_candidate_is_skipped_and_a_later_valid_file_candidate_wins() {
        // `Path::exists()` returns true for a directory too; a directory
        // candidate must be SKIPPED (it is not a regular file) rather
        // than "resolved" and only failing much later as a raw `EISDIR`
        // spawn error.
        let dir = temp_test_dir("dir_candidate_skipped");
        let dir_candidate = dir.join("visudo_dir"); // a directory, not a file
        std::fs::create_dir_all(&dir_candidate).unwrap();
        let file_candidate = dir.join("visudo_file");
        touch(&file_candidate);
        let bins =
            Binaries::from_candidates(&[("visudo", &[dir_candidate.as_path(), file_candidate.as_path()])]);
        assert_eq!(bins.resolve("visudo").unwrap(), file_candidate.as_path());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_as_the_only_candidate_yields_binary_missing() {
        let dir = temp_test_dir("dir_only_candidate");
        let dir_candidate = dir.join("visudo_dir");
        std::fs::create_dir_all(&dir_candidate).unwrap();
        let bins = Binaries::from_candidates(&[("visudo", &[dir_candidate.as_path()])]);
        let err = bins.resolve("visudo").unwrap_err();
        assert!(matches!(err, HelperError::BinaryMissing { name: "visudo" }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relative_candidate_is_skipped() {
        // `Path::exists()` resolves a relative path against the process
        // working directory; a relative candidate must never be
        // "resolved" this way, only an absolute one.
        let dir = temp_test_dir("relative_candidate_skipped");
        let absolute_file = dir.join("visudo_file");
        touch(&absolute_file);
        let cwd = std::env::current_dir().unwrap();
        let relative_name = "nopass_test_bins_relative_candidate_marker";
        let relative_in_cwd = cwd.join(relative_name);
        touch(&relative_in_cwd); // exists relative to cwd, but the candidate itself is relative
        let relative_candidate = Path::new(relative_name);
        let bins =
            Binaries::from_candidates(&[("visudo", &[relative_candidate, absolute_file.as_path()])]);
        assert_eq!(bins.resolve("visudo").unwrap(), absolute_file.as_path());
        let _ = std::fs::remove_file(&relative_in_cwd);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dangling_symlink_candidate_is_skipped() {
        let dir = temp_test_dir("dangling_symlink_skipped");
        let dangling_link = dir.join("visudo_dangling");
        let missing_target = dir.join("does_not_exist");
        std::os::unix::fs::symlink(&missing_target, &dangling_link).unwrap();
        let real_file = dir.join("visudo_real");
        touch(&real_file);
        let bins = Binaries::from_candidates(&[("visudo", &[dangling_link.as_path(), real_file.as_path()])]);
        assert_eq!(bins.resolve("visudo").unwrap(), real_file.as_path());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlink_pointing_at_a_real_file_is_accepted() {
        // Merged-usr distros ship e.g. `/bin -> /usr/bin` as a symlink;
        // the real candidate table relies on symlinked directories still
        // resolving to a usable binary. `std::fs::metadata` follows
        // symlinks exactly like `stat`, unlike `symlink_metadata`.
        let dir = temp_test_dir("symlink_to_real_file_accepted");
        let real_file = dir.join("visudo_real");
        touch(&real_file);
        let link = dir.join("visudo_link");
        std::os::unix::fs::symlink(&real_file, &link).unwrap();
        let bins = Binaries::from_candidates(&[("visudo", &[link.as_path()])]);
        assert_eq!(bins.resolve("visudo").unwrap(), link.as_path());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

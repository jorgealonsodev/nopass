//! Filesystem layout: sudoers-rule paths, state-file paths, and the lock
//! path, plus the rule-filename → uid parser.
//!
//! `Layout` is the single injection point that lets every file-operation
//! test run unprivileged against a `tempfile::TempDir` via [`Layout::under`]
//! instead of the real `/etc/sudoers.d` and `/run/nopass`. Production code
//! constructs [`Layout::system`] exactly once, in `main`.

use std::path::{Path, PathBuf};

/// Fixed prefix of every NoPass-managed sudoers rule filename.
pub const RULE_PREFIX: &str = "90-nopass-";

/// Fixed absolute path the helper binary is installed at; used as the
/// `ExecStart=`/`exec.path` target for privileged invocation.
pub const HELPER_PATH: &str = "/usr/libexec/nopass-helper";

/// Root directories the helper reads and writes under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    sudoers_dir: PathBuf,
    run_dir: PathBuf,
}

impl Layout {
    /// The real system layout: `/etc/sudoers.d` and `/run/nopass`.
    pub fn system() -> Self {
        Layout {
            sudoers_dir: PathBuf::from("/etc/sudoers.d"),
            run_dir: PathBuf::from("/run/nopass"),
        }
    }

    /// A layout rooted under `root`, for unprivileged tests
    /// (`tempfile::TempDir`). Mirrors the real subpath shape so fixtures
    /// read naturally: `<root>/sudoers.d/...` and `<root>/run/nopass/...`.
    pub fn under(root: &Path) -> Self {
        Layout {
            sudoers_dir: root.join("sudoers.d"),
            run_dir: root.join("run/nopass"),
        }
    }

    /// Path of the live sudoers rule for `uid`.
    pub fn rule_path(&self, uid: u32) -> PathBuf {
        self.sudoers_dir.join(format!("{RULE_PREFIX}{uid}"))
    }

    /// Path of the temporary file the rule is written to before
    /// `visudo -cf` validation and atomic rename.
    pub fn rule_tmp_path(&self, uid: u32) -> PathBuf {
        self.sudoers_dir.join(format!(".{RULE_PREFIX}{uid}.tmp"))
    }

    /// Path of the world-readable state file for `uid`.
    pub fn state_path(&self, uid: u32) -> PathBuf {
        self.run_dir.join(format!("{uid}.state"))
    }

    /// Path of the temporary state file before atomic rename.
    pub fn state_tmp_path(&self, uid: u32) -> PathBuf {
        self.run_dir.join(format!(".{uid}.state.tmp"))
    }

    /// Path of the `flock`-serialized mutation lock.
    pub fn lock_path(&self) -> PathBuf {
        self.run_dir.join("lock")
    }
}

/// Extracts the uid from a canonical NoPass rule filename.
///
/// Accepts only `RULE_PREFIX` followed by a canonical `[1-9][0-9]*` uid —
/// no leading zero, no trailing suffix. This gates the boot sweep's
/// `read_dir` match, so a false accept would let the sweep touch a file it
/// has no business deleting (threat matrix: Rule-file target selection).
pub fn uid_from_rule_filename(name: &str) -> Option<u32> {
    let digits = name.strip_prefix(RULE_PREFIX)?;
    let mut chars = digits.chars();
    let first = chars.next()?;
    if !first.is_ascii_digit() || first == '0' {
        return None;
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_layout_produces_real_sudoers_and_run_paths() {
        let layout = Layout::system();
        assert_eq!(layout.rule_path(1000), PathBuf::from("/etc/sudoers.d/90-nopass-1000"));
        assert_eq!(layout.rule_tmp_path(1000), PathBuf::from("/etc/sudoers.d/.90-nopass-1000.tmp"));
        assert_eq!(layout.state_path(1000), PathBuf::from("/run/nopass/1000.state"));
        assert_eq!(layout.state_tmp_path(1000), PathBuf::from("/run/nopass/.1000.state.tmp"));
        assert_eq!(layout.lock_path(), PathBuf::from("/run/nopass/lock"));
    }

    #[test]
    fn under_layout_roots_every_path_at_the_given_directory() {
        let root = PathBuf::from("/tmp/nopass-test-root");
        let layout = Layout::under(&root);
        assert_eq!(layout.rule_path(2000), PathBuf::from("/tmp/nopass-test-root/sudoers.d/90-nopass-2000"));
        assert_eq!(
            layout.rule_tmp_path(2000),
            PathBuf::from("/tmp/nopass-test-root/sudoers.d/.90-nopass-2000.tmp")
        );
        assert_eq!(layout.state_path(2000), PathBuf::from("/tmp/nopass-test-root/run/nopass/2000.state"));
        assert_eq!(
            layout.state_tmp_path(2000),
            PathBuf::from("/tmp/nopass-test-root/run/nopass/.2000.state.tmp")
        );
        assert_eq!(layout.lock_path(), PathBuf::from("/tmp/nopass-test-root/run/nopass/lock"));
    }

    #[test]
    fn uid_from_rule_filename_accepts_canonical_uid() {
        assert_eq!(uid_from_rule_filename("90-nopass-1000"), Some(1000));
        assert_eq!(uid_from_rule_filename("90-nopass-1"), Some(1));
    }

    #[test]
    fn uid_from_rule_filename_rejects_leading_zero() {
        assert_eq!(uid_from_rule_filename("90-nopass-01000"), None);
    }

    #[test]
    fn uid_from_rule_filename_rejects_trailing_suffix() {
        assert_eq!(uid_from_rule_filename("90-nopass-1000.bak"), None);
    }

    #[test]
    fn uid_from_rule_filename_rejects_wrong_prefix_and_empty_digits() {
        assert_eq!(uid_from_rule_filename("91-nopass-1000"), None);
        assert_eq!(uid_from_rule_filename("90-nopass-"), None);
        assert_eq!(uid_from_rule_filename("90-nopass-abc"), None);
    }

    #[test]
    fn helper_path_and_rule_prefix_are_the_fixed_constants() {
        assert_eq!(HELPER_PATH, "/usr/libexec/nopass-helper");
        assert_eq!(RULE_PREFIX, "90-nopass-");
    }
}

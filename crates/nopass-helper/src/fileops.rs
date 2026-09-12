//! Atomic sudoers-rule file operations (design.md §3-4.1 steps 7-13;
//! sudoers-rule-lifecycle §Atomic Rule Creation, §Rule Removal,
//! §Mutation Serialization; threat matrix "Rule-file target selection").
//!
//! `ops.rs` (Phase 7) is the production caller of every function here;
//! until it lands, a normal (non-test) build never calls them, hence the
//! blanket allow below.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use nopass_core::header::is_nopass_owned;
use nopass_core::paths::{uid_from_rule_filename, Layout};

use crate::bins::Binaries;
use crate::error::HelperError;
use crate::runner::{CommandRunner, CommandSpec, Expect};

fn fs_err(path: &Path, e: std::io::Error) -> HelperError {
    HelperError::Fs(format!("{}: {e}", path.display()))
}

/// Creates the rule file for `uid` atomically at
/// `layout.rule_path(uid)`, per the transaction sequence in design.md
/// §4.1 steps 8-13:
///
/// 1. open `layout.rule_tmp_path(uid)` with `O_CREAT|O_EXCL|O_WRONLY`
///    (`OpenOptions::create_new`) — refuses to follow or overwrite
///    anything already at that path, including a pre-planted symlink
///    (POSIX: `O_CREAT|O_EXCL` against a path naming an existing symlink
///    fails `EEXIST` regardless of the link's target).
/// 2. write `content`, `fchmod 0440` (via the still-open fd — immune to
///    the umask, unlike the mode requested at step 1), `fsync`.
/// 3. validate with `visudo -cf <tmp>` via `runner`/`binaries`.
/// 4. atomically `rename` the tmp file onto the final path.
/// 5. best-effort `fsync` of the containing directory (never rolled back
///    on failure — the rename already took effect).
///
/// Every failure from step 1 onward unlinks the tmp file (best-effort)
/// before returning; nothing is ever left half-written at the final
/// path. This function never calls `create_dir_all` on
/// `layout.sudoers_dir()` — see `tests/fileops_tempdir.rs`'s module doc
/// comment for the Phase 6 decision this follows: on a real system
/// `/etc/sudoers.d` always exists, and the helper must not be in the
/// business of creating it.
pub fn write_rule_atomic(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    uid: u32,
    content: &str,
) -> Result<(), HelperError> {
    let tmp_path = layout.rule_tmp_path(uid);
    let final_path = layout.rule_path(uid);

    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o440)
        .open(&tmp_path)
        .map_err(|e| fs_err(&tmp_path, e))?;

    let write_result: Result<(), HelperError> = (|| {
        file.write_all(content.as_bytes()).map_err(|e| fs_err(&tmp_path, e))?;
        // design.md §4.1 step 9: `fchown 0:0` then `fchmod 0440`, both on
        // the still-open fd. The prior implementation only did the
        // `fchmod` half; this closes that gap. In production this
        // process reaches this call only as uid 0 — the polkit action
        // (privilege-admission spec) admits nothing else — so the file
        // this call just created via `O_CREAT|O_EXCL` is already owned
        // `root:root` before `fchown` even runs; this is defense-in-depth
        // against a future caller of this function that is not already
        // root, not a functional requirement today. `EPERM` is the one
        // errno this call can return without that being a real failure:
        // POSIX permits `fchown` to an arbitrary owner only to a process
        // that already has that privilege (`CAP_CHOWN` on Linux), so an
        // unprivileged dev/test run — the only environment that can ever
        // observe anything other than "already root:root" — deterministically
        // gets `EPERM` here and cannot meaningfully exercise this call
        // regardless of how it is written; any other errno is a genuine
        // failure and is propagated.
        if let Err(e) =
            nix::unistd::fchown(&file, Some(nix::unistd::Uid::from_raw(0)), Some(nix::unistd::Gid::from_raw(0)))
        {
            if e != nix::errno::Errno::EPERM {
                return Err(HelperError::Fs(format!("fchown {}: {e}", tmp_path.display())));
            }
        }
        // `.mode(0o440)` above is ANDed with `~umask` by the kernel at
        // creation time — e.g. under `umask(0o077)` the group-read bit
        // would otherwise be stripped, leaving 0o400. `File::set_permissions`
        // calls `fchmod` on the already-open fd: unaffected by umask, and
        // not subject to a symlink-following TOCTOU window the way a
        // path-based `chmod` would be, since `tmp_path` was just created
        // by the `O_EXCL` open above and nothing else can have replaced
        // it since.
        file.set_permissions(std::fs::Permissions::from_mode(0o440)).map_err(|e| fs_err(&tmp_path, e))?;
        file.sync_all().map_err(|e| fs_err(&tmp_path, e))?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }
    drop(file); // close before the external `visudo` process inspects the path

    let visudo = match binaries.resolve("visudo") {
        Ok(p) => p.to_path_buf(),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(err);
        }
    };
    let spec = CommandSpec {
        program: visudo,
        args: vec!["-cf".to_string(), tmp_path.display().to_string()],
        env: vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())],
        // `Any`: this call inspects the exit status itself rather than
        // treating non-zero as a runner-level failure, because a
        // rejected candidate is an expected, typed outcome
        // (`VisudoRejected`), not an internal error.
        expect: Expect::Any,
    };
    let outcome = match runner.run(&spec) {
        Ok(o) => o,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(HelperError::Internal(e.to_string()));
        }
    };
    if outcome.status != Some(0) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(HelperError::VisudoRejected { stderr: String::from_utf8_lossy(&outcome.stderr).into_owned() });
    }

    // A pre-existing symlink at the FINAL path is itself adversarial
    // evidence that something is wrong with this location. POSIX
    // `rename()` would replace that directory entry atomically without
    // ever following it (rename never writes through a destination
    // symlink's target), so this check is defense-in-depth rather than a
    // gap `rename()` alone would leave open — but it turns a silent
    // clobber of unexplained state into a fail-closed exit with the tmp
    // file cleaned up and nothing renamed (threat matrix: Rule-file
    // target selection, "symlink at the rule path").
    let final_is_symlink =
        std::fs::symlink_metadata(&final_path).map(|m| m.file_type().is_symlink()).unwrap_or(false);
    if final_is_symlink {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(HelperError::Fs(format!("refusing to rename over a symlink at {}", final_path.display())));
    }

    if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(fs_err(&final_path, e));
    }

    // Directory fsync: best-effort only, per design.md §4.1 rollback
    // table step 13 — "none: rename already took effect and the rule is
    // valid; log warn and continue". No rollback on failure here.
    if let Some(parent) = final_path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(())
}

/// Removes the rule file for `uid` if, and only if, it exists AND its
/// header content is NoPass-owned (`nopass_core::header::is_nopass_owned`
/// — the same ownership test the boot sweep uses). Returns `Ok(true)`
/// when a rule was actually deleted, `Ok(false)` for every other
/// outcome: the file was already absent (idempotent no-op — task 6.3,
/// sudoers-rule-lifecycle §Rule Removal "Rule file externally deleted
/// before disable runs"), or the file exists but is not NoPass-owned (a
/// foreign file at this path must survive untouched).
pub fn remove_rule(layout: &Layout, uid: u32) -> Result<bool, HelperError> {
    let path = layout.rule_path(uid);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(fs_err(&path, e)),
    };
    if !is_nopass_owned(&content) {
        return Ok(false);
    }
    // Recorded, deliberately not fixed (independent-verifier finding,
    // rated low): the ownership check above reads `path` by name, and
    // the `remove_file` below unlinks that same path by name again,
    // without re-verifying content in between. A file replaced at this
    // exact path in the window between the two could be unlinked without
    // a second `is_nopass_owned` check. This is accepted rather than
    // restructured into an fd-pinned check-then-delete because only root
    // can write to `/etc/sudoers.d` at all — the only actor able to win
    // this race is root racing root, which this codebase does not treat
    // as an adversarial boundary anywhere else either.
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        // Raced with an external deleter between the read above and this
        // remove: still an idempotent no-op, not an error.
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(fs_err(&path, e)),
    }
}

/// Reads the raw content of the rule file for `uid`. `Ok(None)` when the
/// file does not exist; every other I/O failure is propagated.
pub fn read_rule(layout: &Layout, uid: u32) -> Result<Option<String>, HelperError> {
    let path = layout.rule_path(uid);
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(fs_err(&path, e)),
    }
}

/// Enumerates every uid with a canonical NoPass rule filename directly
/// under `layout.sudoers_dir()`, sorted ascending. Uses
/// `nopass_core::paths::uid_from_rule_filename` as the sole filter, so a
/// name like `90-nopass-1000.bak` or `90-nopass-01000` is never returned
/// (threat matrix: Rule-file target selection) — this is what the
/// `expire --boot` sweep (Phase 7) walks.
///
/// A missing `sudoers_dir()` yields an empty list rather than an error:
/// on a real system `/etc/sudoers.d` always exists, so this branch never
/// triggers there; it only helps a test fixture that has not yet created
/// the directory fail soft instead of hard.
pub fn list_rule_uids(layout: &Layout) -> Result<Vec<u32>, HelperError> {
    let dir_path = layout.sudoers_dir();
    let entries = match std::fs::read_dir(dir_path) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(fs_err(dir_path, e)),
    };

    let mut uids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| fs_err(dir_path, e))?;
        // Phase 6 correction (independent-verifier finding, security):
        // the filename filter below is not sufficient on its own — it
        // says nothing about what KIND of entry sits at that name. A
        // directory or a symlink named like a canonical rule filename
        // used to pass straight through. `DirEntry::file_type()` does
        // not follow a symlink (unlike `Path::metadata`), so requiring
        // `is_file()` rejects directories, symlinks, sockets and FIFOs
        // in one check without ever resolving a link target.
        //
        // A symlink is excluded even when it resolves to a genuine,
        // unrelated rule file elsewhere: `write_rule_atomic` never
        // creates a symlink at a rule path, only a regular file it
        // created itself via `O_CREAT|O_EXCL` and then renamed into
        // place, so a symlink here is by construction not a file this
        // sweep created and must not be treated as one (threat matrix:
        // "Rule-file target selection"). `is_nopass_owned` would follow
        // the link if given the chance to read it, which is exactly why
        // this function decides at the directory-entry level, before any
        // content is ever read.
        let file_type = entry.file_type().map_err(|e| fs_err(dir_path, e))?;
        if !file_type.is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if let Some(uid) = uid_from_rule_filename(name) {
                uids.push(uid);
            }
        }
    }
    uids.sort_unstable();
    Ok(uids)
}

//! The four privileged transactions — `enable`, `disable`, `status`,
//! `expire` — with the exact step order and rollback rules of design.md
//! §4. This is the only module `main::dispatch` calls into for real work.
//!
//! Every `pub fn` below (`enable`, `disable`, `status`, `expire`) is a
//! thin **production wrapper**: it is the call site obligated to read the
//! live invocation context — `std::env::var("PKEXEC_UID")` and
//! `nix::unistd::getuid()` (the REAL uid, never `geteuid()`), exactly as
//! `uid::resolve`'s doc comment prescribes — then hands off to a fully
//! parameterized, deterministically testable `*_inner` function. Every
//! `*_inner` function and every pure helper takes its inputs explicitly,
//! matching the injection convention already established by
//! `uid::resolve`, `checks::admit_uid`, and `lock::LockGuard::acquire`.

use std::path::Path;

use nopass_core::expiry::Expiry;
use nopass_core::header::{self, RuleHeader};
use nopass_core::logindefs::{self, UidRange};
use nopass_core::paths::Layout;
use nopass_core::state::HelperStatus;
use nopass_core::template::{render_rule, sanitize_username};

use crate::bins::Binaries;
use crate::checks;
use crate::cli::Cmd;
use crate::error::HelperError;
use crate::fileops;
use crate::journal::{self, AuditEvent, AuditOutcome, AuditRecord};
use crate::lock::LockGuard;
use crate::runner::CommandRunner;
use crate::statefile;
use crate::subject::Subject;
use crate::timer;
use crate::uid;

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Journals a rejection or rollback outcome for `enable`/`disable`/
/// `expire`'s early-return failure paths (helper-observability §Journald
/// Audit Records: "A rejected operation is still journaled" — the MUST
/// this closes is broader than the diagrammed step-17-only audit arrow
/// in design.md §4.1-4.3, which shows only the successful-completion
/// case). `err.exit_code()`/`err.audit_reason()` supply `EXIT`/`REASON`;
/// `REASON` is always a short stable token, never raw subprocess text
/// (design.md §9).
///
/// A `TimerFailed { rolled_back: true }` is reported as `RolledBack`, not
/// `Rejected` — design.md §4.1's rollback table names this outcome
/// distinctly ("unlink(rule) + fsync(dir) + audit `rolled_back`"), since
/// the sudoers rule genuinely existed for a moment before being undone,
/// unlike every other rejection here where nothing was ever created.
///
/// A `TimerFailed { rolled_back: false }` is reported as `Error`, not
/// `Rejected` — this is a strictly worse outcome than either of those:
/// the rule file is still on disk (the grant is live) and nothing is
/// scheduled to revoke it, because the thing that failed was the expiry
/// timer itself. `Rejected` describes "nothing was created"; `RolledBack`
/// describes "something was created and then genuinely undone". Neither
/// is true here, so folding this case into `Rejected` would tell an
/// operator grepping the journal that the grant is gone when it is not.
/// `Error` is design.md §9's documented outcome value for exactly this
/// shape of failure and was otherwise never wired into a real call site.
///
/// Phase 7 correction: `event`/`uid`/`context` are no longer separate
/// interim-literal parameters — `subject` (design.md §2's seam) supplies
/// all three via `subject.event()`, `subject.uid()`, `subject.source()`.
/// This is what finally corrects `expire`'s rejection path from the
/// hardcoded `Pkexec` literal to the real `SystemRoot` context it has
/// always actually run under.
fn audit_rejection(subject: Subject, user: &str, expires: Expiry, err: &HelperError) {
    let outcome = match err {
        HelperError::TimerFailed { rolled_back: true } => AuditOutcome::RolledBack,
        HelperError::TimerFailed { rolled_back: false } => AuditOutcome::Error,
        _ => AuditOutcome::Rejected,
    };
    journal::audit(&AuditRecord {
        event: subject.event(),
        context: subject.source(),
        uid: subject.uid(),
        user,
        outcome,
        expires,
        exit: err.exit_code(),
        reason: err.audit_reason(),
    });
}

/// The one shared wrapper-level admission point for `grant`/`revoke`/
/// `inspect` (design.md §3 "the one real tension"; privilege-admission
/// "SystemRoot Context Can Target Any Admitted UID"). `disable_inner` and
/// `status_inner` deliberately run no admission of their own — "removing
/// a privilege must never be blocked" — so a `SystemRoot`-context
/// `revoke`/`inspect` is bounded HERE, before the shared transaction is
/// ever entered, rather than inside it. `grant` reuses `enable_inner`
/// unchanged, whose own step 5 (`checks::admit_uid`) still runs a second,
/// deliberately redundant time (design.md §3; task 7.12) — this function
/// and that step can only ever agree, since both call the same pure
/// `checks::admit_uid` over the same `(uid, range)` pair.
///
/// Audits its own rejection with `subject`'s own event
/// (`Grant`/`Revoke`/`Inspect`) and `SystemRoot` context — never a
/// hardcoded event, so a rejected `revoke` is never misreported as a
/// rejected `grant`.
pub fn admit_root_target(subject: Subject, range: &UidRange) -> Result<(), HelperError> {
    if let Err(rejection) = checks::admit_uid(subject.uid(), range) {
        let err = HelperError::UidRejected(rejection);
        let user = checks::lookup_user(subject.uid()).unwrap_or_default();
        audit_rejection(subject, &user, Expiry::Never, &err);
        return Err(err);
    }
    Ok(())
}

// --- enable -----------------------------------------------------------

/// Production `enable` entry point (helper-cli, sudoers-rule-lifecycle,
/// expiry-policy §Temporary Duration Validation; design.md §4.1). Reads
/// the live invocation context and the target's real username, then
/// hands off to [`enable_inner`].
pub fn enable(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    until: Option<u64>,
    until_reboot: bool,
) -> Result<(), HelperError> {
    let cmd = Cmd::Enable { until, until_reboot };
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    // Deliberately unaudited: no target uid is resolvable yet on this
    // branch, so there is nothing genuine to attach to a journald
    // record. Inventing an "unknown uid" sentinel to fill one in would
    // misrepresent the audit trail worse than honestly omitting it —
    // unlike step 4's duration validation just below, which runs AFTER
    // `uid` is real and therefore IS audited (see
    // `resolve_expiry_audited`).
    let ctx = uid::resolve(&cmd, pkexec_uid.as_deref(), real_uid)?;
    // `Subject::pkexec` (design.md §2, the seam) replaces the old
    // `let InvocationContext::Pkexec(uid) = ctx else { unreachable!() }`
    // — `Cmd::Enable` always resolves to `InvocationContext::Pkexec`, but
    // failing closed with `HelperError::Context` (exit 10) on a future
    // mispairing is strictly better than a panic with an unlistable exit
    // code.
    let subject = Subject::pkexec(ctx, AuditEvent::Enable)?;
    let uid = subject.uid();

    let now = unix_now();
    let expiry = resolve_expiry_audited(subject, until, until_reboot, now)?;

    // Step 5's getpwuid half. verify-report C1: this used to be a bare
    // `checks::lookup_user(uid)?`, which maps a missing passwd entry to
    // `HelperError::Internal` (exit 1) — the same code a genuine syscall
    // failure produces, and it ran BEFORE `enable_inner` ever reaches
    // `checks::admit_uid`, the only place `UidRejection::Unknown` (exit
    // 11) is otherwise produced. That made privilege-admission §UID
    // Range Admission's "uid not present in getpwuid → exit 11" scenario
    // unreachable in production. `lookup_user_optional` distinguishes
    // "no entry" from a genuine syscall error so this call site can map
    // the two differently: a genuinely missing uid is admission
    // rejection, not an internal failure.
    let raw_user = match checks::lookup_user_optional(uid) {
        Ok(Some(name)) => name,
        Ok(None) => {
            let err = HelperError::UidRejected(checks::UidRejection::Unknown);
            audit_rejection(subject, "", expiry, &err);
            return Err(err);
        }
        Err(err) => return Err(err),
    };
    let login_defs = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let uid_range = logindefs::parse(&login_defs);

    enable_inner(layout, runner, binaries, subject, &raw_user, &uid_range, expiry)
}

/// Resolves `--until`/`--until-reboot` into an [`Expiry`], validating any
/// `--until` duration against `nopass_core::expiry::validate_until`
/// (expiry-policy §Temporary Duration Validation; design.md §4.1 step 4).
/// Pure and directly testable — this is where exit 13 originates, always
/// BEFORE `enable_inner` acquires the lock or touches the filesystem
/// (design.md §4.1 rollback table rows 1-7: "nothing was created").
fn resolve_expiry(until: Option<u64>, until_reboot: bool, now: u64) -> Result<Expiry, HelperError> {
    if until_reboot {
        return Ok(Expiry::Reboot);
    }
    match until {
        None => Ok(Expiry::Never),
        Some(epoch) => {
            nopass_core::expiry::validate_until(now, epoch)?;
            Ok(Expiry::At { epoch })
        }
    }
}

/// Phase 8 correction: wraps [`resolve_expiry`] with the journald audit
/// obligation (helper-observability §Journald Audit Records: "a rejected
/// operation is still journaled"). A duration rejection (exit 13) is NOT
/// the same situation as an exit-10 context-resolution failure — by the
/// time `enable` reaches step 4, step 3 has already resolved `uid`
/// (design.md §4.1: step 3 PKEXEC_UID context precedes step 4 duration
/// validate), so `uid` IS genuinely known here and omitting the audit
/// record would be an oversight, not an honest limitation.
///
/// `raw_user` has not been looked up yet at this point in `enable`'s step
/// order (step 5 `getpwuid` follows step 4), so the audit record's `user`
/// field goes through the same [`resolve_username`] fallback chain the
/// other early rejection paths in this module already use, and `expires`
/// is reported as `Expiry::Never` — a placeholder, since no expiry was
/// ever successfully resolved on this path (matching the convention
/// `disable_inner`'s and `expire_uid_inner`'s own pre-admission failure
/// branches already use).
///
/// Fully parameterized — unlike [`enable`]'s public wrapper, this reads
/// no live environment variable, so it is directly unit-testable without
/// a real `PKEXEC_UID`.
///
/// Phase 7 retype: takes `subject` (already resolved by the caller —
/// `enable`'s `Subject::pkexec` or `grant`'s `Subject::root_target`)
/// instead of a bare `uid`, so a `grant`'s duration rejection is audited
/// under its own `AuditEvent::Grant`/`SystemRoot` context rather than the
/// hardcoded `AuditEvent::Enable`/`Pkexec` this function used before.
fn resolve_expiry_audited(subject: Subject, until: Option<u64>, until_reboot: bool, now: u64) -> Result<Expiry, HelperError> {
    match resolve_expiry(until, until_reboot, now) {
        Ok(expiry) => Ok(expiry),
        Err(err) => {
            let user = resolve_username(checks::lookup_user(subject.uid()), None);
            audit_rejection(subject, &user, Expiry::Never, &err);
            Err(err)
        }
    }
}

/// The full `enable` transaction (design.md §4.1 steps 5-15), fully
/// parameterized for deterministic testing: `uid`/`raw_user` are already
/// resolved by [`enable`]'s live wrapper, and `expiry` is already
/// validated by [`resolve_expiry`]. `raw_user` is deliberately NOT
/// pre-sanitized by the caller — sanitizing it here, once, before it
/// reaches the sudoer probe is exactly the obligation `tasks.md`'s Phase
/// 7 blockquote records against this module: `checks::is_sudoer` does not
/// sanitize its `user` argument.
///
/// Phase 7 retype (design.md §2, the `ops.rs` reuse seam): `uid: u32`
/// becomes `subject: Subject`. This is the transaction `grant` reuses
/// unchanged — `subject` carries either a `Pkexec`-sourced identity (from
/// [`enable`]'s own wrapper) or a `SystemRoot`-sourced one (from
/// `ops::grant`), and every step below reads the target uid from
/// `subject.uid()`: `fileops`, `timer`, and `statefile` remain uid
/// consumers, not authority consumers, and keep taking a bare `u32`.
fn enable_inner(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    subject: Subject,
    raw_user: &str,
    uid_range: &UidRange,
    expiry: Expiry,
) -> Result<(), HelperError> {
    let uid = subject.uid();

    // Sanitized once, up front, so a rejection audit record on ANY
    // failure path below (including the admission check, which precedes
    // the sudoer probe that used to be the first place `user` existed)
    // never carries the raw, unsanitized value (helper-observability
    // §Journald Audit Records; design.md §9 "never raw subprocess text"
    // extends naturally to never raw, unsanitized input either). Moving
    // this pure, side-effect-free computation earlier does not change
    // the step order design.md §4.1 documents — it only makes `user`
    // available to every rejection branch.
    let user = sanitize_username(raw_user);

    // Step 5: getpwuid + UID range admission (exit 11). For `grant`, this
    // is the deliberate SECOND `admit_uid` call over the same `(uid,
    // range)` pair `admit_root_target` already checked at the wrapper
    // level (design.md §3, task 7.12) — kept so the bound stays inside
    // the shared transaction, not merely at its entrance.
    if let Err(rejection) = checks::admit_uid(uid, uid_range) {
        let err = HelperError::UidRejected(rejection);
        audit_rejection(subject, &user, expiry, &err);
        return Err(err);
    }

    // Step 6: sudoer probe.
    if let Err(err) = checks::is_sudoer(runner, binaries, &user) {
        audit_rejection(subject, &user, expiry, &err);
        return Err(err);
    }

    // Step 7: acquire the mutation lock. Steps 8+ all happen inside it.
    let _guard = match LockGuard::acquire(layout) {
        Ok(guard) => guard,
        Err(err) => {
            audit_rejection(subject, &user, expiry, &err);
            return Err(err);
        }
    };

    // Steps 8-13: atomic write + visudo validation + rename. Tmp cleanup
    // on any failure is handled internally by `write_rule_atomic`.
    let content = render_rule(uid, &user, expiry);
    if let Err(err) = fileops::write_rule_atomic(layout, runner, binaries, uid, &content) {
        audit_rejection(subject, &user, expiry, &err);
        return Err(err);
    }

    // Step 14: always stop a stale timer, regardless of the new expiry
    // kind — clears a leftover timer from a previous `At` activation even
    // when this `enable` is `Never`/`Reboot`.
    timer::stop(runner, binaries, uid);

    // Step 15: schedule a new timer only for `At`. Failure rolls the rule
    // back and reports exit 17 (design.md §4.1 rollback table). `rolled_back`
    // is set from what `rollback_rule` actually observed, never assumed:
    // a failed unlink here would otherwise report a rollback that never
    // happened, while the rule file — and the live, now-unscheduled
    // grant it represents — stays on disk.
    if let Expiry::At { epoch } = expiry {
        if timer::schedule(runner, binaries, uid, epoch).is_err() {
            let rolled_back = rollback_rule(layout, uid);
            let err = HelperError::TimerFailed { rolled_back };
            audit_rejection(subject, &user, expiry, &err);
            return Err(err);
        }
    }

    // Step 16: state-file write. A failure here is logged and MUST NOT
    // change the exit code — the grant is already real (the rule file is
    // on disk and validated) by the time this call runs, so returning an
    // error here would report a lie (design.md §4.1 rollback table, row
    // 16; helper-observability §State File Placement and Permissions).
    let rule_header = RuleHeader { user: user.clone(), expires: expiry };
    let rule_path = layout.rule_path(uid).display().to_string();
    let status = HelperStatus::active_from(uid, &rule_header, rule_path, unix_now());
    if let Err(err) = statefile::write(layout, &status) {
        tracing::error!(uid, error = %err, "failed to write state file after enable");
    }

    // Step 17: journald audit record. `event`/`context` now come straight
    // from `subject` — `grant` reusing this transaction is what makes the
    // event/context vary here for the first time.
    journal::audit(&AuditRecord {
        event: subject.event(),
        context: subject.source(),
        uid,
        user: &user,
        outcome: AuditOutcome::Ok,
        expires: expiry,
        exit: 0,
        reason: "",
    });

    Ok(())
    // Step 18: `_guard` drops here, releasing the lock.
}

/// Rollback for a step-15 `systemd-run` failure: unlink the rule just
/// written and best-effort `fsync` the containing directory (design.md
/// §4.1 rollback table, row 15).
///
/// Returns whether the rule file is confirmed absent once this call
/// returns — `true` on a successful unlink, and also `true` when the
/// file is already gone (`NotFound`, the same idempotence
/// `fileops::remove_rule` already applies: a file that does not exist is
/// success, not failure). Returns `false` only for a genuine removal
/// failure (e.g. no write permission on the containing directory),
/// meaning the rule file — and the grant it represents — is still live.
/// The caller reports this value verbatim as `TimerFailed.rolled_back`
/// rather than assuming success: a hardcoded `true` here would tell the
/// caller of `enable` that a time-boxed grant was undone when it was
/// not, leaving a permanent grant with nothing scheduled to revoke it.
/// The `fsync` is still attempted unconditionally and remains
/// best-effort — this function runs only from inside an error path and
/// has nothing further to roll back to.
fn rollback_rule(layout: &Layout, uid: u32) -> bool {
    let path = layout.rule_path(uid);
    let removed = match std::fs::remove_file(&path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    };
    fsync_parent(&path);
    removed
}

fn fsync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

// --- disable ------------------------------------------------------------

/// Production `disable` entry point (sudoers-rule-lifecycle §Rule
/// Removal; design.md §4.2).
pub fn disable(layout: &Layout, runner: &dyn CommandRunner, binaries: &Binaries) -> Result<(), HelperError> {
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    let ctx = uid::resolve(&Cmd::Disable, pkexec_uid.as_deref(), real_uid)?;
    let subject = Subject::pkexec(ctx, AuditEvent::Disable)?;
    disable_inner(layout, runner, binaries, subject)
}

/// design.md §4.2: no UID-range or sudoer admission is required —
/// removing a privilege must never be blocked, and a deleted account must
/// still be revocable. Unlink precedes timer stop, deliberately inverted
/// from the naive order: if the unlink failed after the timer was already
/// stopped, the grant would become permanent with no scheduled
/// revocation; in this order the worst case is an orphan timer that later
/// fires `expire --uid` and finds nothing — a no-op.
///
/// Phase 7 retype: `uid: u32` becomes `subject: Subject`. Deliberately
/// UNCHANGED in every other respect — `revoke`'s own admission bound
/// lives at its wrapper (`ops::admit_root_target`), never here, so
/// `disable`'s existing "removal must never be blocked" guarantee and
/// every test pinning it survive byte-for-byte (design.md §3, "the one
/// real tension").
fn disable_inner(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    subject: Subject,
) -> Result<(), HelperError> {
    let uid = subject.uid();
    let _guard = match LockGuard::acquire(layout) {
        Ok(guard) => guard,
        Err(err) => {
            let user = resolve_username(checks::lookup_user(uid), None);
            audit_rejection(subject, &user, Expiry::Never, &err);
            return Err(err);
        }
    };

    // Step 2: read + parse the rule BEFORE it is removed, for the audit
    // record only — a missing/unparseable file is not an error and does
    // not block the removal below (design.md §4.2). `.ok().flatten()`
    // collapses a genuine I/O error the same way a missing file already
    // is: this read exists only to feed `resolve_username`'s fallback
    // chain and the audit's `expires` field, not to gate anything.
    let header = fileops::read_rule(layout, uid).ok().flatten().and_then(|content| header::parse(&content).ok());

    if let Err(err) = fileops::remove_rule(layout, uid) {
        let user = resolve_username(checks::lookup_user(uid), header.as_ref());
        let expires = header.as_ref().map(|h| h.expires).unwrap_or(Expiry::Never);
        audit_rejection(subject, &user, expires, &err);
        return Err(err);
    }
    timer::stop(runner, binaries, uid);

    // Steps 6-7: state-file write (`active: false`, error logged, exit
    // stays 0 — same rule as enable's step 16) and journald audit. This
    // is `resolve_username`'s first production caller (tasks.md's Phase
    // 7 blockquote note: no audit record existed to consume it until
    // now).
    let user = resolve_username(checks::lookup_user(uid), header.as_ref());
    let rule_path = layout.rule_path(uid).display().to_string();
    let status = HelperStatus::inactive(uid, user.clone(), rule_path, unix_now());
    if let Err(err) = statefile::write(layout, &status) {
        tracing::error!(uid, error = %err, "failed to write state file after disable");
    }
    journal::audit(&AuditRecord {
        event: subject.event(),
        context: subject.source(),
        uid,
        user: &user,
        outcome: AuditOutcome::Ok,
        expires: header.map(|h| h.expires).unwrap_or(Expiry::Never),
        exit: 0,
        reason: "",
    });

    Ok(())
}

/// Username fallback chain for the disable audit record (design.md §4.2:
/// `getpwuid` failure falls back to the header's `nopass-user`, then to
/// `""`). Pure and independently tested here; wired into
/// `disable_inner` above as of Phase 8 — its first production caller.
pub fn resolve_username(passwd_lookup: Result<String, HelperError>, header: Option<&RuleHeader>) -> String {
    match passwd_lookup {
        Ok(name) => name,
        Err(_) => header.map(|h| h.user.clone()).unwrap_or_default(),
    }
}

// --- status ---------------------------------------------------------------

/// Production `status` entry point (helper-observability §HelperStatus
/// JSON Contract; design.md §4.5).
///
/// **Behaviour change, flagged explicitly (m3a-headless-grant tasks.md
/// 2.3) — not silently resolved.** `AuditEvent::Status` has existed
/// since M1 and, until this call, was emitted from nowhere: `status`
/// took no lock and performed no write, so it was never routed through
/// `journal::audit`. The modified helper-observability "Journald Audit
/// Records" requirement now binds by "every outcome-producing
/// subcommand", not by an enumerated name list — `status` is one such
/// subcommand, so every invocation is audited starting with this
/// change. This IS a change to already-shipped behaviour: every
/// `status` call now writes one journald record where before it wrote
/// none. Volume was checked, not assumed, before this call was added:
/// the tray's 60-second reconciliation tick reads
/// `/run/nopass/<uid>.state` directly via `std::fs::read`
/// (`crates/nopass/src/state.rs::read`/`crates/nopass/src/app.rs::reconcile`)
/// and probes liveness with `sudo -k -n true`
/// (`crates/nopass/src/probe.rs`); its `Action` enum
/// (`crates/nopass/src/outcome.rs`) has only `Enable`/`Disable` variants
/// and `pkexec_spec` (`crates/nopass/src/invoke.rs`) never builds a
/// `status` argv. The tray never invokes `nopass-helper status` through
/// `pkexec` at all, on a cadence or otherwise, so this wiring does not
/// create a periodic journald write from the tray's own reconciliation
/// loop — only a direct, manual `nopass-helper status` invocation is
/// audited.
pub fn status(layout: &Layout) -> Result<(), HelperError> {
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    let ctx = uid::resolve(&Cmd::Status, pkexec_uid.as_deref(), real_uid)?;
    let subject = Subject::pkexec(ctx, AuditEvent::Status)?;
    let status = status_inner(layout, subject, unix_now());
    audit_status(&status, subject);
    print!("{}", status.to_json_line());
    Ok(())
}

/// Emits the one journald/stderr `AuditEvent::Status` record a
/// successful `status` call now produces (helper-observability
/// "Journald Audit Records"), extracted as its own function — like
/// `resolve_username` above — so it is unit-testable independently of
/// `status`'s live wrapper, whose uid resolution depends on
/// `PKEXEC_UID`/`nix::unistd::getuid()` and therefore cannot be driven
/// deterministically from a test in a crate that forbids `unsafe`
/// (`uid.rs`'s module doc comment). `status_inner` itself stays free of
/// this call — design.md §5 has `inspect` reuse the exact same
/// `status_inner`, and `inspect` audits `AuditEvent::Inspect`, not
/// `AuditEvent::Status`; wiring the audit here, in the caller, keeps the
/// event tied to which subcommand asked, not to which function ran.
///
/// Phase 7 retype: takes `subject` instead of hardcoding
/// `AuditEvent::Status`/`UidSource::Pkexec`. `ops::inspect` calls this
/// exact function too, with its own `SystemRoot`-sourced
/// `AuditEvent::Inspect` subject — the two now share this one audit call
/// site rather than each hand-rolling their own `journal::audit`.
fn audit_status(status: &HelperStatus, subject: Subject) {
    journal::audit(&AuditRecord {
        event: subject.event(),
        context: subject.source(),
        uid: status.uid,
        user: &status.user,
        outcome: AuditOutcome::Ok,
        expires: status.expires.unwrap_or(Expiry::Never),
        exit: 0,
        reason: "",
    });
}

/// design.md §4.5: no lock, no write, reads the authoritative rule file
/// directly (not the state-file cache). Never returns an error — even a
/// genuine I/O failure (not just a missing file) degrades to
/// `active: false` rather than an exit 15/16, matching "`status` can
/// therefore never return 15 or 16."
///
/// Phase 7 retype: `uid: u32` becomes `subject: Subject`, so `inspect`
/// (design.md §5, "the same `status_inner`") can share this exact
/// function with `status` and produce a byte-identical `HelperStatus`
/// shape — the invocation context, admission, and audit event differ
/// only at the two callers, never here.
fn status_inner(layout: &Layout, subject: Subject, now: u64) -> HelperStatus {
    let uid = subject.uid();
    let rule_path = layout.rule_path(uid).display().to_string();
    let content = std::fs::read_to_string(layout.rule_path(uid)).ok();
    let parsed = content.as_deref().and_then(|c| header::parse(c).ok());
    match parsed {
        Some(header) => HelperStatus::active_from(uid, &header, rule_path, now),
        None => {
            let user = checks::lookup_user(uid).unwrap_or_default();
            HelperStatus::inactive(uid, user, rule_path, now)
        }
    }
}

// --- expire -----------------------------------------------------------

/// Production `expire` entry point (expiry-policy §Expiry Re-validation
/// Before Deletion, §Boot-Time Cleanup Sweep; design.md §4.3-4.4). Routes
/// to the boot-sweep path when `--boot` is set (restricted to `uid` when
/// both flags are given, per design.md §4.4), otherwise the single-uid
/// path — `clap`'s `ArgGroup` on `Cmd::Expire` guarantees `uid` is `Some`
/// whenever `boot` is `false`.
pub fn expire(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    uid: Option<u32>,
    boot: bool,
) -> Result<(), HelperError> {
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    let ctx = uid::resolve(&Cmd::Expire { uid, boot }, pkexec_uid.as_deref(), real_uid)?;

    let now = unix_now();
    if boot {
        expire_boot_inner(layout, uid, now)
    } else {
        let target = uid.expect("clap's ArgGroup guarantees --uid or --boot is present");
        // Phase 7 (task 7.5): routes through the same `Subject` seam
        // every other SystemRoot-context subcommand uses, rather than a
        // second path beside it — `expire_uid_inner` no longer hardcodes
        // `context: UidSource::SystemRoot`, it reads it from `subject`.
        let subject = Subject::root_target(ctx, target, AuditEvent::Expire)?;
        expire_uid_inner(layout, runner, binaries, subject, now)
    }
}

/// design.md §4.3: re-reads the header under the lock and deletes only
/// when genuinely expired — the "Stale revocation" threat-matrix row. A
/// missing rule is treated as already-expired (exit 0, no-op); a file
/// that is not NoPass-owned is never deleted; `Never`, `Reboot`, and a
/// still-future `At` are all left intact.
///
/// Phase 7 retype (task 7.3, 7.5): `uid: u32` becomes `subject: Subject`
/// — [`expire`]'s wrapper now builds it via `Subject::root_target`, so
/// this function's audit records finally read `context: subject.source()`
/// instead of the interim `UidSource::SystemRoot` literal. `expire` is
/// the seam's other `SystemRoot` consumer alongside `grant`/`revoke`/
/// `inspect`, not a second path beside it.
fn expire_uid_inner(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    subject: Subject,
    now: u64,
) -> Result<(), HelperError> {
    let uid = subject.uid();
    let _guard = match LockGuard::acquire(layout) {
        Ok(guard) => guard,
        Err(err) => {
            let user = checks::lookup_user(uid).unwrap_or_default();
            audit_rejection(subject, &user, Expiry::Never, &err);
            return Err(err);
        }
    };

    let content = match fileops::read_rule(layout, uid) {
        Ok(Some(content)) => content,
        Ok(None) => return Ok(()), // absent rule — already-expired, exit 0 no-op
        Err(err) => {
            let user = checks::lookup_user(uid).unwrap_or_default();
            audit_rejection(subject, &user, Expiry::Never, &err);
            return Err(err);
        }
    };
    let Ok(header) = header::parse(&content) else {
        return Ok(()); // foreign/corrupted file — never delete it
    };
    if !header.expires.is_expired(now) {
        // design.md §4.3's diagrammed audit point: Never, Reboot, or a
        // still-future At is a no-op, but it is still journaled.
        journal::audit(&AuditRecord {
            event: subject.event(),
            context: subject.source(),
            uid,
            user: &header.user,
            outcome: AuditOutcome::SkippedNotExpired,
            expires: header.expires,
            exit: 0,
            reason: "",
        });
        return Ok(());
    }

    let path = layout.rule_path(uid);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // raced externally — still success
        Err(e) => {
            let err = HelperError::Fs(format!("{}: {e}", path.display()));
            audit_rejection(subject, &header.user, header.expires, &err);
            return Err(err);
        }
    }
    fsync_parent(&path);
    timer::stop(runner, binaries, uid);

    // design.md §4.3's final arrow: "statefile active:false → audit".
    let rule_path = layout.rule_path(uid).display().to_string();
    let status = HelperStatus::inactive(uid, header.user.clone(), rule_path, now);
    if let Err(err) = statefile::write(layout, &status) {
        tracing::error!(uid, error = %err, "failed to write state file after expire");
    }
    journal::audit(&AuditRecord {
        event: subject.event(),
        context: subject.source(),
        uid,
        user: &header.user,
        outcome: AuditOutcome::Ok,
        expires: header.expires,
        exit: 0,
        reason: "",
    });

    Ok(())
}

/// design.md §4.4: sweeps every canonical rule file directly under
/// `layout.sudoers_dir()` (restricted to `only_uid` when given), removing
/// each whose header indicates `Reboot` or a past epoch, and leaving
/// `Never`/future-`At` rules untouched. No `systemctl`/`systemd-run` call
/// is ever made in boot mode — this function does not even take a
/// `CommandRunner`/`Binaries` parameter, so that invariant holds
/// structurally, not merely by test assertion. A per-file failure is
/// counted but never aborts the sweep.
fn expire_boot_inner(layout: &Layout, only_uid: Option<u32>, now: u64) -> Result<(), HelperError> {
    let _guard = LockGuard::acquire(layout)?;

    let mut any_failed = false;
    for candidate_uid in fileops::list_rule_uids(layout)? {
        if only_uid.is_some_and(|restrict| restrict != candidate_uid) {
            continue;
        }
        if sweep_one(layout, candidate_uid, now).is_err() {
            any_failed = true;
        }
    }

    if sweep_orphan_state_files(layout, only_uid) {
        any_failed = true;
    }

    if any_failed {
        return Err(HelperError::Fs("one or more rule files failed to sweep".to_string()));
    }
    Ok(())
}

/// design.md §4.4's remaining, previously-unwired sentence: "Stale
/// `/run/nopass/*.state` files without a rule are removed." Runs AFTER
/// the rule sweep above so a state file is judged against the
/// POST-sweep set of live rules (a rule deleted earlier in this same
/// call must not save its own now-orphaned state file). Restricted to
/// `only_uid` exactly like the rule sweep, for the same `--boot --uid`
/// scoping. A per-file `statefile::remove` failure is counted but never
/// aborts the sweep, matching `sweep_one`'s own per-file failure
/// tolerance.
fn sweep_orphan_state_files(layout: &Layout, only_uid: Option<u32>) -> bool {
    let live_rule_uids: std::collections::HashSet<u32> = match fileops::list_rule_uids(layout) {
        Ok(uids) => uids.into_iter().collect(),
        Err(_) => return true,
    };
    let state_uids = match statefile::list_state_uids(layout) {
        Ok(uids) => uids,
        Err(_) => return true,
    };

    let mut any_failed = false;
    for state_uid in state_uids {
        if only_uid.is_some_and(|restrict| restrict != state_uid) {
            continue;
        }
        if live_rule_uids.contains(&state_uid) {
            continue;
        }
        if statefile::remove(layout, state_uid).is_err() {
            any_failed = true;
        }
    }
    any_failed
}

/// Boot-sweep handling for a single uid: `Expiry::is_expired_at_boot`
/// (unlike `expire --uid`'s `is_expired`) treats `Reboot` as expired too.
fn sweep_one(layout: &Layout, uid: u32, now: u64) -> Result<(), HelperError> {
    let path = layout.rule_path(uid);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()), // raced away externally
        Err(e) => return Err(HelperError::Fs(format!("{}: {e}", path.display()))),
    };
    let Ok(header) = header::parse(&content) else {
        return Ok(()); // not NoPass-owned — leave untouched
    };
    if !header.expires.is_expired_at_boot(now) {
        return Ok(()); // Never, or a still-future At — leave untouched
    }
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(HelperError::Fs(format!("{}: {e}", path.display()))),
    }
    fsync_parent(&path);
    Ok(())
}

// --- grant / revoke / inspect (m3a-headless-grant Phase 7) -------------
//
// design.md §2 "No new transaction code at all": each of the three is a
// thin wrapper that resolves the `SystemRoot` context, builds a
// `Subject::root_target`, admits the explicit `--uid` via
// `admit_root_target`, and enters the exact same transaction
// `enable`/`disable`/`status` already use. Nothing about WHAT is done
// changes; only WHO may ask, and for WHOM.

/// Production `grant` entry point — the root-context counterpart to
/// [`enable`], reusing [`enable_inner`] unchanged (design.md §2, §7
/// sequence). Order matches the design sequence exactly: context
/// resolution (exit 10) → `Subject::root_target` (the seam) →
/// `resolve_expiry_audited` (exit 13) → `admit_root_target` (exit 11) →
/// `enable_inner`, whose own step 5 admits the same uid a deliberate
/// second time (task 7.12).
pub fn grant(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    uid_flag: u32,
    until: Option<u64>,
    until_reboot: bool,
) -> Result<(), HelperError> {
    let cmd = Cmd::Grant { uid: uid_flag, until, until_reboot };
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    let ctx = uid::resolve(&cmd, pkexec_uid.as_deref(), real_uid)?;
    // The target uid comes from `uid_flag` (the explicit `--uid`), never
    // from `real_uid` — this is the whole widening §3 bounds: a
    // SystemRoot-context grant may target ANY admitted uid, not only the
    // invoking process's own (privilege-admission "A root-invoked grant
    // targets a uid other than the caller's own").
    let subject = Subject::root_target(ctx, uid_flag, AuditEvent::Grant)?;

    let now = unix_now();
    let expiry = resolve_expiry_audited(subject, until, until_reboot, now)?;

    let login_defs = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let uid_range = logindefs::parse(&login_defs);
    admit_root_target(subject, &uid_range)?;

    // Same "no passwd entry -> exit 11, never exit 1" mapping [`enable`]
    // already uses (verify-report C1) — by construction this branch is
    // unreachable in practice, since `admit_root_target` just above
    // already confirmed a passwd entry exists via the same `admit_uid`;
    // kept anyway as defense in depth rather than an `unwrap`.
    let raw_user = match checks::lookup_user_optional(subject.uid()) {
        Ok(Some(name)) => name,
        Ok(None) => {
            let err = HelperError::UidRejected(checks::UidRejection::Unknown);
            audit_rejection(subject, "", expiry, &err);
            return Err(err);
        }
        Err(err) => return Err(err),
    };

    enable_inner(layout, runner, binaries, subject, &raw_user, &uid_range, expiry)
}

/// Production `revoke` entry point — the root-context counterpart to
/// [`disable`], reusing [`disable_inner`] unchanged. Unlike `disable`'s
/// pkexec path (deliberately admission-free, design.md §3 "removing a
/// privilege must never be blocked"), `revoke`'s SystemRoot-context
/// target still passes `admit_root_target` FIRST — the one real tension
/// design.md §3 documents, bounded here at the wrapper rather than inside
/// `disable_inner` itself, so `disable`'s existing untouched behaviour
/// and every test pinning it survive exactly as written.
pub fn revoke(
    layout: &Layout,
    runner: &dyn CommandRunner,
    binaries: &Binaries,
    uid_flag: u32,
) -> Result<(), HelperError> {
    let cmd = Cmd::Revoke { uid: uid_flag };
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    let ctx = uid::resolve(&cmd, pkexec_uid.as_deref(), real_uid)?;
    let subject = Subject::root_target(ctx, uid_flag, AuditEvent::Revoke)?;

    let login_defs = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let uid_range = logindefs::parse(&login_defs);
    admit_root_target(subject, &uid_range)?;

    disable_inner(layout, runner, binaries, subject)
}

/// Production `inspect` entry point — shares [`status_inner`] exactly
/// with [`status`], so the printed `HelperStatus` JSON is byte-identical
/// (design.md §5: "identical JSON, one implementation"). Only the
/// invocation context, admission, and audit event differ: `inspect`
/// passes `admit_root_target` first (unlike `status`'s admission-free
/// pkexec path) and audits `AuditEvent::Inspect`, never
/// `AuditEvent::Status`.
pub fn inspect(layout: &Layout, uid_flag: u32) -> Result<(), HelperError> {
    let cmd = Cmd::Inspect { uid: uid_flag };
    let pkexec_uid = std::env::var("PKEXEC_UID").ok();
    let real_uid = nix::unistd::getuid().as_raw();
    let ctx = uid::resolve(&cmd, pkexec_uid.as_deref(), real_uid)?;
    let subject = Subject::root_target(ctx, uid_flag, AuditEvent::Inspect)?;

    let login_defs = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let uid_range = logindefs::parse(&login_defs);
    admit_root_target(subject, &uid_range)?;

    let status = status_inner(layout, subject, unix_now());
    audit_status(&status, subject);
    print!("{}", status.to_json_line());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;
    use crate::runner::{CommandOutcome, CommandSpec, RunnerError, ScriptedRunner};
    use crate::uid::InvocationContext;

    // --- audit capture (same technique as `journal.rs`'s own tests):
    // scopes a `tracing_subscriber::fmt` layer writing into a shared
    // buffer to one closure via `tracing::subscriber::with_default`, so
    // it never touches global state and is safe under parallel test
    // execution. This proves a rejection-path `journal::audit` call was
    // actually made, without a journald socket.

    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedBuf {
        type Writer = SharedBuf;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture_audit<F: FnOnce()>(f: F) -> String {
        let buf = SharedBuf::default();
        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_writer(buf.clone()).with_ansi(false));
        tracing::subscriber::with_default(subscriber, f);
        String::from_utf8(buf.0.lock().unwrap().clone()).unwrap()
    }

    fn temp_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!(
            "nopass_test_ops_{tag}_{}_{:?}_{nanos}_{n}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    fn fresh_layout(tag: &str) -> (PathBuf, Layout) {
        let root = temp_root(tag);
        let layout = Layout::under(&root);
        std::fs::create_dir_all(root.join("sudoers.d")).unwrap();
        (root, layout)
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"").unwrap();
    }

    fn fake_binaries(root: &Path) -> Binaries {
        let sudo = root.join("sudo");
        let sh = root.join("sh");
        let visudo = root.join("visudo");
        let systemctl = root.join("systemctl");
        let systemd_run = root.join("systemd-run");
        for p in [&sudo, &sh, &visudo, &systemctl, &systemd_run] {
            touch(p);
        }
        Binaries::from_candidates(&[
            ("sudo", &[sudo.as_path()]),
            ("sh", &[sh.as_path()]),
            ("visudo", &[visudo.as_path()]),
            ("systemctl", &[systemctl.as_path()]),
            ("systemd-run", &[systemd_run.as_path()]),
        ])
    }

    fn lang_c() -> Vec<(String, String)> {
        vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())]
    }

    fn sudo_probe_spec(binaries: &Binaries, user: &str) -> CommandSpec {
        CommandSpec {
            program: binaries.resolve("sudo").unwrap().to_path_buf(),
            args: vec![
                "-n".to_string(),
                "-l".to_string(),
                "-U".to_string(),
                user.to_string(),
                binaries.resolve("sh").unwrap().display().to_string(),
            ],
            env: lang_c(),
            expect: crate::runner::Expect::Any,
        }
    }

    fn visudo_spec(binaries: &Binaries, layout: &Layout, uid: u32) -> CommandSpec {
        CommandSpec {
            program: binaries.resolve("visudo").unwrap().to_path_buf(),
            args: vec!["-cf".to_string(), layout.rule_tmp_path(uid).display().to_string()],
            env: lang_c(),
            expect: crate::runner::Expect::Any,
        }
    }

    fn systemctl_stop_spec(binaries: &Binaries, uid: u32) -> CommandSpec {
        CommandSpec {
            program: binaries.resolve("systemctl").unwrap().to_path_buf(),
            args: vec!["stop".to_string(), format!("{}.timer", timer::unit_name(uid))],
            env: lang_c(),
            expect: crate::runner::Expect::Any,
        }
    }

    fn systemd_run_spec(binaries: &Binaries, uid: u32, epoch: u64) -> CommandSpec {
        CommandSpec {
            program: binaries.resolve("systemd-run").unwrap().to_path_buf(),
            args: vec![
                format!("--unit={}", timer::unit_name(uid)),
                format!("--description=NoPass expiry for uid {uid}"),
                format!("--on-calendar={}", nopass_core::timefmt::format_systemd_calendar(epoch)),
                "--timer-property=AccuracySec=1s".to_string(),
                "--timer-property=Persistent=false".to_string(),
                "--timer-property=WakeSystem=false".to_string(),
                "--timer-property=RemainAfterElapse=false".to_string(),
                "--property=Type=oneshot".to_string(),
                nopass_core::paths::HELPER_PATH.to_string(),
                "expire".to_string(),
                "--uid".to_string(),
                uid.to_string(),
            ],
            env: lang_c(),
            expect: crate::runner::Expect::Zero,
        }
    }

    fn ok(status: i32) -> CommandOutcome {
        CommandOutcome { status: Some(status), stdout: vec![], stderr: vec![] }
    }

    // uid 1 (`daemon` on essentially every Linux distro) is used for every
    // admission-success scenario below — this crate's test suite already
    // targets "a real Linux dev/CI machine, not a hermetic filesystem"
    // (see `checks.rs`), and `checks::admit_uid`/`checks::lookup_user`
    // call the real `getpwuid` syscall with no injection point.
    const REAL_UID: u32 = 1;
    fn wide_range() -> UidRange {
        UidRange { min: 1, max: 60_000 }
    }

    // --- Phase 7 test helpers: build a `Subject` directly, bypassing the
    // live `uid::resolve`/`PKEXEC_UID` wrapper, exactly like every other
    // `*_inner`-level test in this module already bypasses its own public
    // wrapper's live context resolution.
    fn subj_pkexec(uid: u32, event: AuditEvent) -> Subject {
        Subject::pkexec(InvocationContext::Pkexec(uid), event).expect("Pkexec context must build a Subject")
    }

    fn subj_root(uid: u32, event: AuditEvent) -> Subject {
        Subject::root_target(InvocationContext::SystemRoot, uid, event)
            .expect("SystemRoot context must build a Subject")
    }

    // --- enable: context wiring (obligation 2) -----------------------

    #[test]
    fn enable_live_wrapper_rejects_context_when_pkexec_uid_is_absent() {
        // No PKEXEC_UID is set in this test process's real environment —
        // proves `enable`'s call site genuinely reads the live variable
        // rather than an injected stub.
        let (root, layout) = fresh_layout("enable_live_ctx");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]); // nothing must run
        let err = enable(&layout, &runner, &binaries, None, false).unwrap_err();
        assert_eq!(err.exit_code(), 10);
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- resolve_expiry (pure, duration validation, exit 13) ---------

    #[test]
    fn resolve_expiry_defaults_to_never_with_no_flags() {
        assert_eq!(resolve_expiry(None, false, 1_000_000).unwrap(), Expiry::Never);
    }

    #[test]
    fn resolve_expiry_returns_reboot_when_until_reboot_is_set() {
        assert_eq!(resolve_expiry(None, true, 1_000_000).unwrap(), Expiry::Reboot);
    }

    #[test]
    fn resolve_expiry_accepts_an_in_range_until_as_at() {
        let now = 1_000_000;
        assert_eq!(resolve_expiry(Some(now + 3600), false, now).unwrap(), Expiry::At { epoch: now + 3600 });
    }

    #[test]
    fn resolve_expiry_rejects_until_below_the_minimum_with_exit_13() {
        let now = 1_000_000;
        let err = resolve_expiry(Some(now + 30), false, now).unwrap_err();
        assert_eq!(err.exit_code(), 13);
    }

    #[test]
    fn resolve_expiry_rejects_until_above_the_maximum_with_exit_13() {
        let now = 1_000_000;
        let err = resolve_expiry(Some(now + 30_000), false, now).unwrap_err();
        assert_eq!(err.exit_code(), 13);
    }

    #[test]
    fn resolve_expiry_rejects_until_in_the_past_with_exit_13() {
        let now = 1_000_000;
        let err = resolve_expiry(Some(now - 1), false, now).unwrap_err();
        assert_eq!(err.exit_code(), 13);
    }

    // --- resolve_expiry_audited: Phase 8 correction — a duration
    // rejection (exit 13) is audited with the real target uid, because
    // by the time `enable`'s step 4 (duration validate) runs, step 3
    // (PKEXEC_UID context) has already resolved `uid` (design.md §4.1).
    // Unlike exit 10 context-resolution failures, no uid is genuinely
    // unknown here.

    #[test]
    fn duration_rejection_is_audited_with_the_real_uid() {
        let now = 1_000_000;
        let text = capture_audit(|| {
            let err = resolve_expiry_audited(subj_pkexec(REAL_UID, AuditEvent::Enable), Some(now - 1), false, now)
                .unwrap_err();
            assert_eq!(err.exit_code(), 13);
        });
        assert!(text.contains("EVENT=\"enable\""), "audit text: {text}");
        assert!(text.contains(&format!("UID={REAL_UID}")), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"rejected\""), "audit text: {text}");
        assert!(text.contains("REASON=\"invalid_duration\""), "audit text: {text}");
    }

    #[test]
    fn duration_rejection_audit_uses_a_different_uid_than_the_uid_admission_audit() {
        // Triangulation: a second uid produces a distinct UID field in the
        // audit record, proving the value comes from the real `uid`
        // parameter rather than a hardcoded/fake value.
        let now = 1_000_000;
        const OTHER_UID: u32 = 2; // "bin" on essentially every Linux distro
        let text = capture_audit(|| {
            let err = resolve_expiry_audited(subj_pkexec(OTHER_UID, AuditEvent::Enable), Some(now + 30), false, now)
                .unwrap_err();
            assert_eq!(err.exit_code(), 13);
        });
        assert!(text.contains(&format!("UID={OTHER_UID}")), "audit text: {text}");
        assert!(!text.contains(&format!("UID={REAL_UID}")), "audit text: {text}");
        assert!(text.contains("REASON=\"invalid_duration\""), "audit text: {text}");
    }

    #[test]
    fn resolve_expiry_audited_still_returns_ok_and_audits_nothing_on_a_valid_duration() {
        let now = 1_000_000;
        let text = capture_audit(|| {
            let expiry = resolve_expiry_audited(subj_pkexec(REAL_UID, AuditEvent::Enable), Some(now + 3600), false, now)
                .unwrap();
            assert_eq!(expiry, Expiry::At { epoch: now + 3600 });
        });
        assert!(text.is_empty(), "no audit record must be emitted on a successful resolution: {text}");
    }

    // --- enable_inner: admission chain (exit 11/12) -------------------

    #[test]
    fn enable_inner_uid_0_is_rejected_with_exit_11_before_any_command_runs() {
        let (root, layout) = fresh_layout("enable_uid0");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]); // empty script — proves zero mutation
        let err = enable_inner(&layout, &runner, &binaries, subj_pkexec(0, AuditEvent::Enable), "jorge", &wide_range(), Expiry::Never).unwrap_err();
        assert_eq!(err.exit_code(), 11);
        assert!(!layout.rule_path(0).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enable_inner_sudoer_probe_rejection_maps_to_exit_12_with_nothing_created() {
        let (root, layout) = fresh_layout("enable_notsudoer");
        let binaries = fake_binaries(&root);
        let mut outcome = ok(1);
        outcome.stderr = b"jorge is not allowed to run sudo".to_vec();
        let runner = ScriptedRunner::new(vec![(sudo_probe_spec(&binaries, "jorge"), Ok(outcome))]);
        let err =
            enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::Never).unwrap_err();
        assert_eq!(err.exit_code(), 12);
        assert!(!layout.rule_path(REAL_UID).exists());
        assert!(!layout.rule_tmp_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: rejection paths are still journaled -------------
    // (helper-observability §Journald Audit Records, "A rejected
    // operation is still journaled")

    #[test]
    fn enable_inner_audits_a_uid_rejection_with_the_rejected_outcome() {
        let (root, layout) = fresh_layout("enable_audit_uid_rejected");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]);
        let text = capture_audit(|| {
            let err =
                enable_inner(&layout, &runner, &binaries, subj_pkexec(0, AuditEvent::Enable), "jorge", &wide_range(), Expiry::Never).unwrap_err();
            assert_eq!(err.exit_code(), 11);
        });
        assert!(text.contains("EVENT=\"enable\""), "audit text: {text}");
        assert!(text.contains("UID=0"), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"rejected\""), "audit text: {text}");
        assert!(text.contains("REASON=\"uid_rejected_root\""), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enable_inner_audits_a_not_sudoer_rejection() {
        let (root, layout) = fresh_layout("enable_audit_not_sudoer");
        let binaries = fake_binaries(&root);
        let mut outcome = ok(1);
        outcome.stderr = b"jorge is not allowed to run sudo".to_vec();
        let runner = ScriptedRunner::new(vec![(sudo_probe_spec(&binaries, "jorge"), Ok(outcome))]);
        let text = capture_audit(|| {
            let err = enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::Never)
                .unwrap_err();
            assert_eq!(err.exit_code(), 12);
        });
        assert!(text.contains("OUTCOME=\"rejected\""), "audit text: {text}");
        assert!(text.contains("REASON=\"not_sudoer\""), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enable_inner_audits_a_rolled_back_timer_failure_with_the_rolled_back_outcome() {
        let (root, layout) = fresh_layout("enable_audit_timer_failed");
        let binaries = fake_binaries(&root);
        let epoch = unix_now() + 3600;
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(5))),
            (
                systemd_run_spec(&binaries, REAL_UID, epoch),
                Err(RunnerError::NonZero { program: "systemd-run".to_string(), status: 1 }),
            ),
        ]);
        let text = capture_audit(|| {
            let err = enable_inner(
                &layout,
                &runner,
                &binaries,
                subj_pkexec(REAL_UID, AuditEvent::Enable),
                "jorge",
                &wide_range(),
                Expiry::At { epoch },
            )
            .unwrap_err();
            assert!(matches!(err, HelperError::TimerFailed { rolled_back: true }));
        });
        assert!(text.contains("EVENT=\"enable\""), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"rolled_back\""), "the rollback outcome must be distinct from a plain rejection: {text}");
        assert!(text.contains("REASON=\"timer_failed\""), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: obligation 1, sanitized username in probe argv --

    #[test]
    fn enable_inner_sanitizes_the_raw_username_before_the_sudo_probe() {
        let (root, layout) = fresh_layout("enable_sanitize");
        let binaries = fake_binaries(&root);
        // Matches `template.rs`'s own fixture: "a;rm -rf /" sanitizes to
        // "arm-rf" ([A-Za-z0-9._-] survives; ';', ' ', '/' are dropped).
        let raw_user = "a;rm -rf /";
        let sanitized = "arm-rf";
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, sanitized), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
        ]);
        enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), raw_user, &wide_range(), Expiry::Never).unwrap();
        let content = std::fs::read_to_string(layout.rule_path(REAL_UID)).unwrap();
        assert!(content.contains("# nopass-user: arm-rf\n"));
        assert!(!content.contains(';'));
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: visudo rejection -> exit 14, tmp unlinked -------

    #[test]
    fn enable_inner_visudo_rejection_unlinks_tmp_and_leaves_final_path_untouched() {
        let (root, layout) = fresh_layout("enable_visudo_reject");
        let binaries = fake_binaries(&root);
        let mut reject = ok(1);
        reject.stderr = b"syntax error near line 4".to_vec();
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(reject)),
        ]);
        let err =
            enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::Never).unwrap_err();
        assert_eq!(err.exit_code(), 14);
        assert!(!layout.rule_tmp_path(REAL_UID).exists());
        assert!(!layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: rename failure -> exit 16 ------------------------

    #[test]
    fn enable_inner_rename_failure_over_a_symlink_yields_exit_16() {
        let (root, layout) = fresh_layout("enable_rename_fail");
        let binaries = fake_binaries(&root);
        let elsewhere = root.join("elsewhere");
        touch(&elsewhere);
        std::os::unix::fs::symlink(&elsewhere, layout.rule_path(REAL_UID)).unwrap();
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
        ]);
        let err =
            enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::Never).unwrap_err();
        assert_eq!(err.exit_code(), 16);
        assert!(!layout.rule_tmp_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: systemd-run failure rolls back, exit 17 ----------

    #[test]
    fn enable_inner_systemd_run_failure_rolls_back_the_rule_and_exits_17() {
        let (root, layout) = fresh_layout("enable_timer_fail");
        let binaries = fake_binaries(&root);
        let epoch = unix_now() + 3600;
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(5))),
            (
                systemd_run_spec(&binaries, REAL_UID, epoch),
                Err(RunnerError::NonZero { program: "systemd-run".to_string(), status: 1 }),
            ),
        ]);
        let err = enable_inner(
            &layout,
            &runner,
            &binaries,
            subj_pkexec(REAL_UID, AuditEvent::Enable),
            "jorge",
            &wide_range(),
            Expiry::At { epoch },
        )
        .unwrap_err();
        assert!(matches!(err, HelperError::TimerFailed { rolled_back: true }));
        assert_eq!(err.exit_code(), 17);
        assert!(!layout.rule_path(REAL_UID).exists(), "the rule must be rolled back on systemd-run failure");
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: systemd-run failure whose OWN rollback unlink
    // fails too — the defect this fix closes. Before this fix,
    // `enable_inner` hardcoded `TimerFailed { rolled_back: true }`
    // regardless of what the unlink actually did: a grant reported as
    // rolled back while its rule file is still on disk has nothing
    // scheduled to revoke it (the thing that failed was the expiry
    // timer), so the user is told the operation failed while actually
    // holding a permanent grant. `rolled_back` must reflect the unlink's
    // real outcome, and the rule file must genuinely survive.
    //
    // Wraps `ScriptedRunner` to strip write permission from the sudoers
    // directory at the exact moment the `systemd-run` call fails — AFTER
    // `write_rule_atomic` (steps 8-13) has already renamed the rule file
    // into place (which itself needs the directory writable), but BEFORE
    // `rollback_rule`'s unlink runs. POSIX requires write permission on
    // the CONTAINING directory to unlink an entry, regardless of the
    // entry's own permissions, so this induces a genuine, non-`NotFound`
    // `remove_file` failure inside `rollback_rule`.
    struct RollbackUnlinkFailureRunner {
        inner: ScriptedRunner,
        sudoers_dir: PathBuf,
    }

    impl crate::runner::CommandRunner for RollbackUnlinkFailureRunner {
        fn run(&self, spec: &CommandSpec) -> Result<CommandOutcome, RunnerError> {
            if spec.program.file_name().and_then(|n| n.to_str()) == Some("systemd-run") {
                std::fs::set_permissions(&self.sudoers_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
            }
            self.inner.run(spec)
        }
    }

    #[test]
    fn enable_inner_reports_rolled_back_false_and_keeps_the_file_when_the_rollback_unlink_fails() {
        if nix::unistd::geteuid().is_root() {
            // root bypasses the DAC permission check this test relies on
            // to simulate an unlink failure — same convention as
            // `disable_inner_removal_failure_propagates_and_never_stops_the_timer`.
            return;
        }
        let (root, layout) = fresh_layout("enable_timer_fail_rollback_fails");
        let binaries = fake_binaries(&root);
        let epoch = unix_now() + 3600;
        let scripted = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(5))),
            (
                systemd_run_spec(&binaries, REAL_UID, epoch),
                Err(RunnerError::NonZero { program: "systemd-run".to_string(), status: 1 }),
            ),
        ]);
        let runner = RollbackUnlinkFailureRunner { inner: scripted, sudoers_dir: layout.sudoers_dir().to_path_buf() };

        let result = enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::At { epoch });

        // Restore write permission before any assertion can fail this
        // test early and skip cleanup, leaving a stuck temp directory.
        std::fs::set_permissions(layout.sudoers_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();

        let err = result.unwrap_err();
        assert!(
            matches!(err, HelperError::TimerFailed { rolled_back: false }),
            "rolled_back must be false when the rollback unlink itself fails, got {err:?}"
        );
        assert_eq!(err.exit_code(), 17);
        assert!(
            layout.rule_path(REAL_UID).exists(),
            "the rule file must survive a failed rollback — it is still a live, unscheduled grant"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enable_inner_audits_a_failed_rollback_with_the_error_outcome_not_rolled_back_or_rejected() {
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let (root, layout) = fresh_layout("enable_audit_rollback_fails");
        let binaries = fake_binaries(&root);
        let epoch = unix_now() + 3600;
        let scripted = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(5))),
            (
                systemd_run_spec(&binaries, REAL_UID, epoch),
                Err(RunnerError::NonZero { program: "systemd-run".to_string(), status: 1 }),
            ),
        ]);
        let runner = RollbackUnlinkFailureRunner { inner: scripted, sudoers_dir: layout.sudoers_dir().to_path_buf() };

        let text = capture_audit(|| {
            let err = enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::At { epoch })
                .unwrap_err();
            std::fs::set_permissions(layout.sudoers_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(matches!(err, HelperError::TimerFailed { rolled_back: false }));
        });

        assert!(text.contains("EVENT=\"enable\""), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"error\""), "a failed rollback must be distinct from both a plain rejection and a successful rollback: {text}");
        assert!(!text.contains("OUTCOME=\"rolled_back\""), "audit text: {text}");
        assert!(!text.contains("OUTCOME=\"rejected\""), "audit text: {text}");
        assert!(text.contains("REASON=\"timer_failed\""), "audit text: {text}");
        assert!(text.contains("EXIT=17"), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: Never/Reboot never call systemd-run --------------

    #[test]
    fn enable_inner_never_and_reboot_never_call_systemd_run() {
        for expiry in [Expiry::Never, Expiry::Reboot] {
            let (root, layout) = fresh_layout("enable_never_reboot");
            let binaries = fake_binaries(&root);
            let runner = ScriptedRunner::new(vec![
                (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
                (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
                (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
            ]);
            enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), expiry).unwrap();
            assert!(layout.rule_path(REAL_UID).exists());
            let _ = std::fs::remove_dir_all(&root);
            // `ScriptedRunner`'s `Drop` would panic here if a fourth
            // (systemd-run) call had been attempted against a
            // three-entry script — proof by construction, not merely by
            // assertion.
        }
    }

    // --- enable_inner: full At success, exact systemd-run argv ---------

    #[test]
    fn enable_inner_at_expiry_schedules_the_timer_with_the_exact_pinned_argv() {
        let (root, layout) = fresh_layout("enable_at_happy");
        let binaries = fake_binaries(&root);
        let epoch = unix_now() + 3600;
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
            (systemd_run_spec(&binaries, REAL_UID, epoch), Ok(ok(0))),
        ]);
        enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::At { epoch }).unwrap();
        let content = std::fs::read_to_string(layout.rule_path(REAL_UID)).unwrap();
        assert!(content.contains(&format!("# nopass-expires: {epoch}\n")));
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- enable_inner: Phase 8 statefile/journal wiring (steps 16-17) ----

    #[test]
    fn enable_inner_writes_the_state_file_with_the_active_grant_after_a_successful_enable() {
        let (root, layout) = fresh_layout("enable_writes_state");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
        ]);
        enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::Never).unwrap();

        let raw = std::fs::read_to_string(layout.state_path(REAL_UID)).expect("state file must exist");
        let status: HelperStatus = serde_json::from_str(raw.trim_end()).unwrap();
        assert_eq!(status.uid, REAL_UID);
        assert_eq!(status.user, "jorge");
        assert!(status.active);
        assert_eq!(status.expires, Some(Expiry::Never));
        let mode = std::fs::metadata(layout.state_path(REAL_UID)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
        let _ = std::fs::remove_dir_all(&root);
    }

    // verify-report W1: all eight `capture_audit` call sites elsewhere in
    // this module cover a rejection, rollback, or duration failure. None
    // of them ever asserted that a SUCCESSFUL transaction is audited —
    // deleting the terminal `journal::audit(&AuditRecord { outcome:
    // AuditOutcome::Ok, .. })` calls in `enable_inner`/`disable_inner`/
    // `expire_uid_inner` left the whole suite green. The three tests
    // below (this one, `disable_inner_audits_a_successful_disable_with_
    // the_ok_outcome`, and `expire_uid_inner_audits_a_successful_
    // deletion_with_the_ok_outcome`) close that gap.
    #[test]
    fn enable_inner_audits_a_successful_grant_with_the_ok_outcome() {
        let (root, layout) = fresh_layout("enable_audit_ok");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
        ]);
        let text = capture_audit(|| {
            enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::Never).unwrap();
        });
        assert!(text.contains("EVENT=\"enable\""), "audit text: {text}");
        assert!(text.contains(&format!("UID={REAL_UID}")), "audit text: {text}");
        assert!(text.contains("USER=\"jorge\""), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"ok\""), "audit text: {text}");
        assert!(text.contains("EXIT=0"), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    // Commissioned by the Phase 8 apply-phase brief: design.md §4.1's
    // rollback table row 16 says a state-file write failure is "logged,
    // exit stays 0", and Phase 7's own verification flagged this as the
    // one rollback row with no test — `statefile.rs` did not exist yet.
    // Confirmed RED first via a reverted mutation: temporarily changing
    // `enable_inner`'s `if let Err(err) = statefile::write(...) {
    // tracing::error!(...) }` to `statefile::write(layout, &status)?;`
    // made this test fail with `enable_inner` returning
    // `Err(HelperError::Fs(_))` (exit 16) instead of `Ok(())` — proving
    // the test is sensitive to the exact regression it guards against —
    // then the mutation was reverted byte-for-byte before this comment
    // was written.
    #[test]
    fn enable_inner_statefile_write_failure_is_logged_but_never_changes_the_exit_code() {
        let (root, layout) = fresh_layout("enable_statefile_write_fails");
        let binaries = fake_binaries(&root);
        // Pre-create the run dir and plant a colliding tmp file at the
        // exact path `statefile::write` opens with `O_CREAT|O_EXCL` —
        // the same unprivileged failure-injection technique
        // `write_fails_and_cleans_up_the_tmp_file_on_an_o_excl_collision`
        // uses in `statefile.rs` directly, applied here at the `ops.rs`
        // call-site level.
        std::fs::create_dir_all(layout.state_path(REAL_UID).parent().unwrap()).unwrap();
        std::fs::write(layout.state_tmp_path(REAL_UID), b"stale leftover").unwrap();

        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, "jorge"), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
        ]);
        let result = enable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Enable), "jorge", &wide_range(), Expiry::Never);

        assert!(result.is_ok(), "a state-file write failure must never change enable's exit code, got {result:?}");
        assert!(layout.rule_path(REAL_UID).exists(), "the sudoers rule itself must still be written and valid");
        assert!(!layout.state_path(REAL_UID).exists(), "the final state file must not exist when its write failed");
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- disable ----------------------------------------------------------

    #[test]
    fn disable_live_wrapper_rejects_context_when_pkexec_uid_is_absent() {
        let (root, layout) = fresh_layout("disable_live_ctx");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]);
        let err = disable(&layout, &runner, &binaries).unwrap_err();
        assert_eq!(err.exit_code(), 10);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disable_inner_unlink_precedes_timer_stop_and_removes_an_existing_rule() {
        let (root, layout) = fresh_layout("disable_removes");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::Never);
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        disable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Disable)).unwrap();
        assert!(!layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // Phase 8: `resolve_username`'s first production caller. The rule
    // header carries "jorge", but `checks::lookup_user(REAL_UID)` (uid 1,
    // `daemon` on every Linux distribution this project targets) is a
    // real `getpwuid` lookup that succeeds with a DIFFERENT name — this
    // test pins that the successful passwd lookup wins over the header,
    // exactly as `resolve_username`'s own fallback-chain tests already
    // prove in isolation, now proven end-to-end through `disable_inner`.
    #[test]
    fn disable_inner_writes_the_state_file_with_active_false_after_a_successful_disable() {
        let (root, layout) = fresh_layout("disable_writes_state");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 1_789_000_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);

        disable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Disable)).unwrap();

        let raw = std::fs::read_to_string(layout.state_path(REAL_UID)).expect("state file must exist");
        let status: HelperStatus = serde_json::from_str(raw.trim_end()).unwrap();
        assert_eq!(status.uid, REAL_UID);
        assert!(!status.active);
        assert_eq!(status.expires, None);
        // Portable across hosts where uid 1 does/does not resolve via
        // `getpwuid`: compute the expected value through the exact same
        // `resolve_username` fallback chain `disable_inner` uses
        // internally, rather than assuming a specific account name.
        let header = RuleHeader { user: "jorge".to_string(), expires: Expiry::At { epoch: 1_789_000_000 } };
        let expected_user = resolve_username(checks::lookup_user(REAL_UID), Some(&header));
        assert_eq!(status.user, expected_user);
        let _ = std::fs::remove_dir_all(&root);
    }

    // verify-report W1: see `enable_inner_audits_a_successful_grant_with_
    // the_ok_outcome` above for the full rationale.
    #[test]
    fn disable_inner_audits_a_successful_disable_with_the_ok_outcome() {
        let (root, layout) = fresh_layout("disable_audit_ok");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::Never);
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        let text = capture_audit(|| {
            disable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Disable)).unwrap();
        });
        assert!(text.contains("EVENT=\"disable\""), "audit text: {text}");
        assert!(text.contains(&format!("UID={REAL_UID}")), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"ok\""), "audit text: {text}");
        assert!(text.contains("EXIT=0"), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disable_inner_is_idempotent_when_the_rule_was_already_externally_deleted() {
        let (root, layout) = fresh_layout("disable_idempotent");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        disable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Disable)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disable_inner_leaves_a_foreign_non_nopass_file_untouched() {
        let (root, layout) = fresh_layout("disable_foreign");
        let binaries = fake_binaries(&root);
        std::fs::write(layout.rule_path(REAL_UID), "foo ALL=(ALL) ALL\n").unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        disable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Disable)).unwrap();
        assert!(layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disable_inner_lock_busy_yields_exit_15() {
        let (root, layout) = fresh_layout("disable_lock_busy");
        let binaries = fake_binaries(&root);
        let _held = LockGuard::acquire(&layout).unwrap();
        let runner = ScriptedRunner::new(vec![]); // no command may run before the lock is even acquired
        let err = disable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Disable)).unwrap_err();
        assert_eq!(err.exit_code(), 15);
        let _ = std::fs::remove_dir_all(&root);
    }

    // helper-observability §Journald Audit Records, "A rejected operation
    // is still journaled" — disable's own rejection path.
    #[test]
    fn disable_inner_audits_a_lock_busy_rejection() {
        let (root, layout) = fresh_layout("disable_audit_lock_busy");
        let binaries = fake_binaries(&root);
        let _held = LockGuard::acquire(&layout).unwrap();
        let runner = ScriptedRunner::new(vec![]);
        let text = capture_audit(|| {
            let err = disable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Disable)).unwrap_err();
            assert_eq!(err.exit_code(), 15);
        });
        assert!(text.contains("EVENT=\"disable\""), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"rejected\""), "audit text: {text}");
        assert!(text.contains("REASON=\"lock_busy\""), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    // Independent-verifier finding: design.md §4.2 orders the unlink
    // before the timer stop specifically so that a genuine unlink
    // failure never leaves a permanent grant with nothing scheduled to
    // revoke it. Every other `disable_inner` test above only exercises
    // the already-absent or successful-removal paths — nothing proved
    // that a real, non-`ENOENT` `remove_rule` failure actually prevents
    // `timer::stop` from running until this test.
    #[test]
    fn disable_inner_removal_failure_propagates_and_never_stops_the_timer() {
        if nix::unistd::geteuid().is_root() {
            // root bypasses the DAC permission check this test relies on
            // to simulate an unlink failure — same convention as
            // `expire_boot_inner_a_per_file_failure_does_not_abort_the_sweep`.
            return;
        }
        let (root, layout) = fresh_layout("disable_removal_fails");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::Never);
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        // POSIX requires write permission on the CONTAINING directory to
        // unlink an entry, regardless of the entry's own permissions —
        // stripping it here induces a genuine, non-ENOENT `remove_file`
        // failure while leaving the rule file itself fully readable.
        std::fs::set_permissions(layout.sudoers_dir(), std::fs::Permissions::from_mode(0o555)).unwrap();

        // Empty script: `ScriptedRunner::run` panics on any unscripted
        // call and its `Drop` panics if the script is left unexhausted —
        // together they prove `timer::stop` is never invoked when the
        // unlink fails, not merely that this test forgot to assert it.
        let runner = ScriptedRunner::new(vec![]);
        let result = disable_inner(&layout, &runner, &binaries, subj_pkexec(REAL_UID, AuditEvent::Disable));

        // Restore write permission before any assertion can fail this
        // test early and skip cleanup, leaving a stuck temp directory.
        std::fs::set_permissions(layout.sudoers_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();

        let err = result.unwrap_err();
        assert!(matches!(err, HelperError::Fs(_)), "expected a genuine Fs error, got {err:?}");
        assert!(layout.rule_path(REAL_UID).exists(), "the rule file must survive an unlink failure");
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- resolve_username (pure fallback chain) --------------------------

    #[test]
    fn resolve_username_prefers_a_successful_passwd_lookup() {
        let header = RuleHeader { user: "header-user".to_string(), expires: Expiry::Never };
        assert_eq!(resolve_username(Ok("passwd-user".to_string()), Some(&header)), "passwd-user");
    }

    #[test]
    fn resolve_username_falls_back_to_the_header_user_when_getpwuid_fails() {
        let header = RuleHeader { user: "header-user".to_string(), expires: Expiry::Never };
        assert_eq!(resolve_username(Err(HelperError::Internal("no such uid".to_string())), Some(&header)), "header-user");
    }

    #[test]
    fn resolve_username_falls_back_to_empty_string_when_both_are_unavailable() {
        assert_eq!(resolve_username(Err(HelperError::Internal("no such uid".to_string())), None), "");
    }

    // --- status -------------------------------------------------------

    #[test]
    fn status_live_wrapper_rejects_context_when_pkexec_uid_is_absent() {
        let (root, layout) = fresh_layout("status_live_ctx");
        let err = status(&layout).unwrap_err();
        assert_eq!(err.exit_code(), 10);
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- task 2.3: `AuditEvent::Status` has existed since M1 and was
    // emitted from nowhere; a successful `status` call now produces
    // exactly one `AuditRecord` with `event: Status`, `outcome: Ok`
    // (helper-observability "Journald Audit Records", modified to bind
    // by "every outcome-producing subcommand"). The live `status(&Layout)`
    // wrapper resolves its uid from `PKEXEC_UID` — `#![forbid(unsafe_code)]`
    // makes `std::env::set_var` unavailable to mutate that from a test
    // (see `uid.rs`'s module doc comment), so this exercises the same
    // audit-emitting step `status`'s success path calls, deterministically,
    // the way `resolve_username`/`resolve_expiry_audited` are unit-tested
    // independently of their live wrapper elsewhere in this file.
    #[test]
    fn a_successful_status_call_audits_exactly_one_record_with_event_status_and_outcome_ok() {
        let (root, layout) = fresh_layout("status_audit_active");
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 1_789_000_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let subject = subj_pkexec(REAL_UID, AuditEvent::Status);
        let status = status_inner(&layout, subject, 1_789_000_500);

        let text = capture_audit(|| {
            audit_status(&status, subject);
        });

        assert_eq!(text.matches("nopass audit event").count(), 1, "exactly one AuditRecord must be emitted: {text}");
        assert!(text.contains("EVENT=\"status\""), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"ok\""), "audit text: {text}");
        assert!(text.contains(&format!("UID={REAL_UID}")), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_inner_reports_inactive_for_a_missing_rule() {
        let (root, layout) = fresh_layout("status_missing");
        // An implausibly large uid: `getpwuid` deterministically fails,
        // exercising the `unwrap_or_default()` ("") branch too.
        let status = status_inner(&layout, subj_pkexec(4_294_967_294, AuditEvent::Status), 1_000_000);
        assert!(!status.active);
        assert_eq!(status.expires, None);
        assert_eq!(status.user, "");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_inner_reports_active_with_exact_fields_for_an_existing_valid_rule() {
        let (root, layout) = fresh_layout("status_active");
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 1_789_000_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let status = status_inner(&layout, subj_pkexec(REAL_UID, AuditEvent::Status), 1_789_000_500);
        assert!(status.active);
        assert_eq!(status.user, "jorge");
        assert_eq!(status.expires, Some(Expiry::At { epoch: 1_789_000_000 }));
        assert_eq!(status.rule_path, layout.rule_path(REAL_UID).display().to_string());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_inner_treats_a_foreign_unparseable_file_as_inactive() {
        let (root, layout) = fresh_layout("status_foreign");
        std::fs::write(layout.rule_path(REAL_UID), "not a nopass rule at all\n").unwrap();
        let status = status_inner(&layout, subj_pkexec(REAL_UID, AuditEvent::Status), 1_000_000);
        assert!(!status.active);
        assert_eq!(status.expires, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_inner_never_takes_the_lock_or_creates_any_directory() {
        let (root, layout) = fresh_layout("status_no_lock");
        let _ = status_inner(&layout, subj_pkexec(REAL_UID, AuditEvent::Status), 1_000_000);
        assert!(!layout.lock_path().exists(), "status must never take the mutation lock");
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- expire: context wiring -----------------------------------------

    #[test]
    fn expire_live_wrapper_rejects_context_when_the_real_uid_is_not_root() {
        // The real test-process uid is never 0 in this suite's target
        // environment (a real Linux dev/CI machine, non-root) — proves
        // `expire`'s call site genuinely reads `nix::unistd::getuid()`.
        //
        // Phase 10 correction (discovered running `cargo test --workspace`
        // for real inside the root-lane container, tests/containers/
        // Containerfile.debian): that is literally the command design.md
        // §8 documents for the root lane, and it runs the ENTIRE workspace
        // suite as real root — including this test, whose premise is
        // exactly the opposite. This is the same root-bypasses-the-fixture
        // situation `disable_inner_removal_failure_propagates_and_never_
        // stops_the_timer` (and its siblings) already guard against
        // elsewhere in this file; this one test was missing that guard.
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let (root, layout) = fresh_layout("expire_live_ctx");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]);
        let err = expire(&layout, &runner, &binaries, Some(1000), false).unwrap_err();
        assert_eq!(err.exit_code(), 10);
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- expire_uid_inner (task 7.5) -------------------------------------

    #[test]
    fn expire_audits_itself_as_system_root_because_that_is_the_only_way_it_runs() {
        // `expire` requires real uid 0 AND no PKEXEC_UID (uid.rs), so
        // there is exactly one context it can ever have. This pins the
        // property against being set back to `Pkexec` — which it briefly
        // was, as a uniform interim value while the field was introduced.
        // A record claiming a pkexec context for an invocation that never
        // had one is a lie about the single question this field was added
        // to answer: whether the caller proved anything to polkit, or
        // simply already had root.
        //
        // Phase 7 correction: before this phase, `expire_uid_inner`'s
        // audit records hardcoded the literal `context: UidSource::
        // SystemRoot`, and this test scanned the function body's source
        // text for that literal directly. Phase 7 (task 7.5) routes
        // `expire` through the same `Subject` seam every other
        // `SystemRoot`-context subcommand uses: `context` is now
        // `subject.source()`, so no `UidSource::Pkexec`/`UidSource::
        // SystemRoot` literal remains inside `expire_uid_inner` at all —
        // the OLD scan would now find ZERO matches for either needle and
        // pass vacuously, proving nothing. The property this test exists
        // to pin — expire's audit records always claim SystemRoot, never
        // Pkexec — is proven instead at the seam itself: [`expire`]'s
        // production wrapper must build its `Subject` via
        // `Subject::root_target` (the only constructor that can ever
        // produce `UidSource::SystemRoot`), never `Subject::pkexec`.
        let src = include_str!("ops.rs");
        let wrapper_start = src.find("pub fn expire(").expect("expire must exist");
        let wrapper_end = src.find("fn expire_uid_inner").expect("expire_uid_inner must exist");
        let expire_wrapper = &src[wrapper_start..wrapper_end];

        assert!(
            expire_wrapper.contains("Subject::root_target"),
            "expire's production wrapper must build its Subject via Subject::root_target, \
             the only constructor that can ever produce UidSource::SystemRoot"
        );
        assert!(
            !expire_wrapper.contains("Subject::pkexec"),
            "expire must never build a Pkexec-sourced Subject — doing so would let it \
             claim a context it never had"
        );

        // Runtime half: a real `expire_uid_inner` call through a
        // `SystemRoot`-sourced Subject genuinely emits `CONTEXT=
        // "SystemRoot"` and never `CONTEXT="Pkexec"` — the seam's static
        // guarantee, proven once at runtime too.
        let (root, layout) = fresh_layout("expire_audits_system_root_runtime");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 500_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        let text = capture_audit(|| {
            expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000).unwrap();
        });
        assert!(text.contains("CONTEXT=\"SystemRoot\""), "audit text: {text}");
        assert!(!text.contains("CONTEXT=\"Pkexec\""), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_uid_inner_absent_rule_is_a_no_op_exit_0() {
        let (root, layout) = fresh_layout("expire_uid_absent");
        let binaries = fake_binaries(&root);
        let runner = ScriptedRunner::new(vec![]);
        expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_uid_inner_foreign_file_is_never_deleted() {
        let (root, layout) = fresh_layout("expire_uid_foreign");
        let binaries = fake_binaries(&root);
        std::fs::write(layout.rule_path(REAL_UID), "foo ALL=(ALL) ALL\n").unwrap();
        let runner = ScriptedRunner::new(vec![]);
        expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000).unwrap();
        assert!(layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_uid_inner_future_at_is_skipped_not_expired_and_the_file_stays_intact() {
        // The "Stale revocation" threat-matrix row: a stale timer fires
        // after a newer `enable` moved the epoch further into the future.
        let (root, layout) = fresh_layout("expire_uid_future");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 2_000_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![]);
        expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000).unwrap();
        assert!(layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_uid_inner_never_and_reboot_headers_are_never_deleted() {
        for expiry in [Expiry::Never, Expiry::Reboot] {
            let (root, layout) = fresh_layout("expire_uid_never_reboot");
            let binaries = fake_binaries(&root);
            let content = render_rule(REAL_UID, "jorge", expiry);
            std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
            let runner = ScriptedRunner::new(vec![]);
            expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000).unwrap();
            assert!(layout.rule_path(REAL_UID).exists());
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn expire_uid_inner_deletes_a_genuinely_past_epoch_rule_and_stops_the_timer() {
        let (root, layout) = fresh_layout("expire_uid_past");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 500_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000).unwrap();
        assert!(!layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // helper-observability §Journald Audit Records, "A rejected operation
    // is still journaled" — expire's own removal-failure rejection path.
    #[test]
    fn expire_uid_inner_audits_a_rejection_when_the_rule_file_cannot_be_removed() {
        if nix::unistd::geteuid().is_root() {
            // root bypasses the DAC permission check this test relies on
            // to simulate an unlink failure — same convention as
            // `disable_inner_removal_failure_propagates_and_never_stops_the_timer`.
            return;
        }
        let (root, layout) = fresh_layout("expire_audit_removal_fails");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 500_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        std::fs::set_permissions(layout.sudoers_dir(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let runner = ScriptedRunner::new(vec![]);

        let text = capture_audit(|| {
            let result = expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000);
            std::fs::set_permissions(layout.sudoers_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
            let err = result.unwrap_err();
            assert_eq!(err.exit_code(), 16);
        });

        assert!(text.contains("EVENT=\"expire\""), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"rejected\""), "audit text: {text}");
        assert!(text.contains("REASON=\"fs_error\""), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_uid_inner_writes_the_state_file_with_active_false_after_deleting_an_expired_rule() {
        let (root, layout) = fresh_layout("expire_uid_writes_state");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 500_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);

        expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000).unwrap();

        let raw = std::fs::read_to_string(layout.state_path(REAL_UID)).expect("state file must exist");
        let status: HelperStatus = serde_json::from_str(raw.trim_end()).unwrap();
        assert!(!status.active);
        assert_eq!(status.expires, None);
        assert_eq!(status.user, "jorge");
        assert_eq!(status.updated_at, 1_000_000, "updated_at must be the injected `now`, not a live clock read");
        let _ = std::fs::remove_dir_all(&root);
    }

    // verify-report W1: see `enable_inner_audits_a_successful_grant_with_
    // the_ok_outcome` above for the full rationale.
    #[test]
    fn expire_uid_inner_audits_a_successful_deletion_with_the_ok_outcome() {
        let (root, layout) = fresh_layout("expire_audit_ok");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::At { epoch: 500_000 });
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0)))]);
        let text = capture_audit(|| {
            expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000).unwrap();
        });
        assert!(text.contains("EVENT=\"expire\""), "audit text: {text}");
        assert!(text.contains(&format!("UID={REAL_UID}")), "audit text: {text}");
        assert!(text.contains("OUTCOME=\"ok\""), "audit text: {text}");
        assert!(text.contains("EXIT=0"), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    // design.md §4.3's "skipped_not_expired" branch has its own audit
    // point but is NOT one of the "statefile active:false" arrows — only
    // an actual deletion writes the state file.
    #[test]
    fn expire_uid_inner_skipped_not_expired_does_not_write_a_state_file() {
        let (root, layout) = fresh_layout("expire_uid_skip_no_state");
        let binaries = fake_binaries(&root);
        let content = render_rule(REAL_UID, "jorge", Expiry::Never);
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();
        let runner = ScriptedRunner::new(vec![]);

        expire_uid_inner(&layout, &runner, &binaries, subj_root(REAL_UID, AuditEvent::Expire), 1_000_000).unwrap();

        assert!(layout.rule_path(REAL_UID).exists());
        assert!(!layout.state_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- expire_boot_inner (task 7.6) -------------------------------------

    #[test]
    fn expire_boot_inner_removes_reboot_and_past_at_rules_but_leaves_never_and_future_at() {
        let (root, layout) = fresh_layout("expire_boot_sweep");
        let reboot_uid = 2001;
        let never_uid = 2002;
        let past_uid = 2003;
        let future_uid = 2004;
        std::fs::write(layout.rule_path(reboot_uid), render_rule(reboot_uid, "a", Expiry::Reboot)).unwrap();
        std::fs::write(layout.rule_path(never_uid), render_rule(never_uid, "b", Expiry::Never)).unwrap();
        std::fs::write(
            layout.rule_path(past_uid),
            render_rule(past_uid, "c", Expiry::At { epoch: 500_000 }),
        )
        .unwrap();
        std::fs::write(
            layout.rule_path(future_uid),
            render_rule(future_uid, "d", Expiry::At { epoch: 2_000_000 }),
        )
        .unwrap();

        expire_boot_inner(&layout, None, 1_000_000).unwrap();

        assert!(!layout.rule_path(reboot_uid).exists());
        assert!(layout.rule_path(never_uid).exists());
        assert!(!layout.rule_path(past_uid).exists());
        assert!(layout.rule_path(future_uid).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_boot_inner_restricts_the_sweep_to_only_uid_when_given() {
        let (root, layout) = fresh_layout("expire_boot_restrict");
        let reboot_uid = 4001;
        let past_uid = 4002;
        std::fs::write(layout.rule_path(reboot_uid), render_rule(reboot_uid, "a", Expiry::Reboot)).unwrap();
        std::fs::write(
            layout.rule_path(past_uid),
            render_rule(past_uid, "b", Expiry::At { epoch: 500_000 }),
        )
        .unwrap();

        expire_boot_inner(&layout, Some(reboot_uid), 1_000_000).unwrap();

        assert!(!layout.rule_path(reboot_uid).exists(), "the restricted uid must still be swept");
        assert!(layout.rule_path(past_uid).exists(), "an unrestricted uid must be left alone");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_boot_inner_a_per_file_failure_does_not_abort_the_sweep() {
        if nix::unistd::geteuid().is_root() {
            // root bypasses the DAC permission check this test relies on
            // to simulate an unreadable rule file — see the module-level
            // convention already established by `checks.rs`.
            return;
        }
        let (root, layout) = fresh_layout("expire_boot_partial_failure");
        let ok_uid = 3001;
        let broken_uid = 3002;
        std::fs::write(layout.rule_path(ok_uid), render_rule(ok_uid, "a", Expiry::At { epoch: 500_000 })).unwrap();
        let broken_path = layout.rule_path(broken_uid);
        std::fs::write(&broken_path, render_rule(broken_uid, "b", Expiry::At { epoch: 500_000 })).unwrap();
        std::fs::set_permissions(&broken_path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let err = expire_boot_inner(&layout, None, 1_000_000).unwrap_err();

        assert_eq!(err.exit_code(), 16);
        assert!(!layout.rule_path(ok_uid).exists(), "a failure on one file must not abort the rest of the sweep");
        assert!(broken_path.exists(), "the unreadable file itself is left untouched, not force-removed");

        let _ = std::fs::set_permissions(&broken_path, std::fs::Permissions::from_mode(0o644));
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- expire_boot_inner: orphan state-file cleanup (design.md §4.4,
    // "Stale /run/nopass/*.state files without a rule are removed") -----

    #[test]
    fn expire_boot_inner_removes_an_orphan_state_file_with_no_rule() {
        let (root, layout) = fresh_layout("expire_boot_orphan_removed");
        let orphan_uid = 5001;
        let status = HelperStatus::inactive(orphan_uid, "ghost".to_string(), "gone".to_string(), 1_000_000);
        statefile::write(&layout, &status).unwrap();
        assert!(layout.state_path(orphan_uid).exists(), "precondition: orphan state file exists, no rule");

        expire_boot_inner(&layout, None, 1_000_000).unwrap();

        assert!(!layout.state_path(orphan_uid).exists(), "an orphan state file with no rule must be removed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_boot_inner_leaves_a_state_file_whose_rule_still_exists() {
        let (root, layout) = fresh_layout("expire_boot_orphan_survives");
        let live_uid = 5002;
        std::fs::write(layout.rule_path(live_uid), render_rule(live_uid, "a", Expiry::Never)).unwrap();
        let status = HelperStatus::active_from(
            live_uid,
            &RuleHeader { user: "a".to_string(), expires: Expiry::Never },
            layout.rule_path(live_uid).display().to_string(),
            1_000_000,
        );
        statefile::write(&layout, &status).unwrap();

        expire_boot_inner(&layout, None, 1_000_000).unwrap();

        assert!(layout.state_path(live_uid).exists(), "a state file whose rule still exists must survive");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_boot_inner_skips_a_directory_named_like_a_state_file_during_orphan_cleanup() {
        let (root, layout) = fresh_layout("expire_boot_orphan_dir_skip");
        let run_dir = layout.state_path(5003).parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::create_dir(run_dir.join("5003.state")).unwrap();

        // Must not error or panic attempting to unlink a directory.
        expire_boot_inner(&layout, None, 1_000_000).unwrap();

        assert!(run_dir.join("5003.state").is_dir(), "a directory named like a state file must be left untouched");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expire_boot_inner_orphan_cleanup_still_runs_and_removes_a_genuine_orphan_after_a_rule_sweep_failure() {
        if nix::unistd::geteuid().is_root() {
            // root bypasses the DAC permission check this test relies on
            // to simulate a rule-sweep failure — same convention as
            // `expire_boot_inner_a_per_file_failure_does_not_abort_the_sweep`.
            return;
        }
        let (root, layout) = fresh_layout("expire_boot_orphan_after_rule_failure");
        let broken_rule_uid = 5004;
        let orphan_uid = 5005;
        let broken_path = layout.rule_path(broken_rule_uid);
        std::fs::write(&broken_path, render_rule(broken_rule_uid, "a", Expiry::At { epoch: 500_000 })).unwrap();
        std::fs::set_permissions(&broken_path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let orphan_status = HelperStatus::inactive(orphan_uid, "b".to_string(), "gone".to_string(), 1_000_000);
        statefile::write(&layout, &orphan_status).unwrap();

        let result = expire_boot_inner(&layout, None, 1_000_000);

        std::fs::set_permissions(&broken_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(result.is_err(), "the rule-sweep failure must still be reported");
        assert!(broken_path.exists(), "the unreadable rule file itself is left untouched");
        assert!(
            !layout.state_path(orphan_uid).exists(),
            "orphan state-file cleanup must still run and succeed even though an earlier rule-sweep entry failed"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- grant / revoke / inspect (m3a-headless-grant Phase 7) -----------

    // task 7.6 / 7.13: `grant`'s happy path reaches `enable_inner` with a
    // SystemRoot-sourced Subject, using the EXACT SAME `ScriptedRunner`
    // script `enable_inner`'s own happy-path test uses — `ScriptedRunner`
    // panics on any unscripted call and on drop if the script is left
    // unexhausted, so this also proves `grant` spawns no NEW subprocess
    // call site beyond what `enable_inner` already spawns (task 7.13:
    // "grant adds no new subprocess call site").
    #[test]
    fn grant_reaches_enable_inner_with_a_system_root_sourced_subject_and_spawns_no_extra_subprocess() {
        let range = wide_range();
        let (root, layout) = fresh_layout("grant_happy_path");
        let binaries = fake_binaries(&root);
        let subject = subj_root(REAL_UID, AuditEvent::Grant);
        let raw_user = "jorge";
        let runner = ScriptedRunner::new(vec![
            (sudo_probe_spec(&binaries, raw_user), Ok(ok(0))),
            (visudo_spec(&binaries, &layout, REAL_UID), Ok(ok(0))),
            (systemctl_stop_spec(&binaries, REAL_UID), Ok(ok(0))),
        ]);
        admit_root_target(subject, &range).expect("uid 1 must be admitted by a wide range");
        enable_inner(&layout, &runner, &binaries, subject, raw_user, &range, Expiry::Never)
            .expect("grant's happy path must reach enable_inner and succeed");
        assert!(layout.rule_path(REAL_UID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // task 7.7: `admit_root_target` rejects uid 0 / below-min / above-max
    // / no-passwd-entry the same way for `grant`, `revoke`, AND `inspect`
    // alike, each exit 11 with nothing written (privilege-admission "A
    // SystemRoot-context target still fails UID Range Admission the same
    // way"). Exercised at the shared full-wrapper level (`ops::grant`/
    // `ops::revoke` cannot run end-to-end without real root, so this
    // drives the wrapper-shared `admit_root_target` + a real `Layout`
    // directly, proving no rule file is ever created).
    #[test]
    fn admit_root_target_rejects_every_admission_cause_for_grant_revoke_and_inspect_alike_with_no_write() {
        let (root, layout) = fresh_layout("admit_root_target_no_write");
        let range = UidRange { min: 1000, max: 60_000 };
        let bad_uids = [0u32, 500, 99_999];
        for event in [AuditEvent::Grant, AuditEvent::Revoke, AuditEvent::Inspect] {
            for &uid in &bad_uids {
                let subject = subj_root(uid, event);
                let err = admit_root_target(subject, &range).unwrap_err();
                assert_eq!(err.exit_code(), 11, "event {event:?}, uid {uid}");
                assert!(!layout.rule_path(uid).exists(), "admit_root_target must never write, event {event:?}");
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // task 7.8: a root-invoked grant targets the uid named by --uid,
    // never the invoking shell's own real uid (privilege-admission "A
    // root-invoked grant targets a uid other than the caller's own").
    // `grant`'s live wrapper reads `nix::unistd::getuid()` only to prove
    // AUTHORITY (via `uid::resolve`, real uid must be 0) — this test
    // pins that the TARGET uid handed to `Subject::root_target` is the
    // explicit `uid_flag`, structurally, since this crate's test process
    // is never real root and cannot exercise the live wrapper end-to-end.
    #[test]
    fn grant_targets_the_explicit_uid_flag_never_the_invoking_shells_own_real_uid() {
        let src = include_str!("ops.rs");
        let body_start = src.find("pub fn grant(").expect("grant must exist");
        let body_end = src.find("pub fn revoke(").expect("revoke must exist");
        let grant_wrapper = &src[body_start..body_end];
        assert!(
            grant_wrapper.contains("Subject::root_target(ctx, uid_flag, AuditEvent::Grant)"),
            "grant's target uid must come from the explicit --uid flag: {grant_wrapper}"
        );
        assert!(
            !grant_wrapper.contains("Subject::root_target(ctx, real_uid"),
            "grant must never target the invoking shell's own real uid"
        );
    }

    // task 7.9: `enable`'s declared flag surface carries no `--uid`, and
    // its resolved target is always the caller's own `PKEXEC_UID`
    // (privilege-admission "enable stays self-targeted with no uid
    // argument on its surface") — pins the `Subject::pkexec` type-level
    // guarantee (3.2) at the CLI+ops boundary: `enable`'s wrapper must
    // build its Subject exclusively via `Subject::pkexec`, which has no
    // uid parameter at all.
    #[test]
    fn enable_stays_self_targeted_with_no_uid_argument_on_its_surface() {
        // `Cmd::Enable`'s only fields are `until`/`until_reboot` — an
        // exhaustive struct-literal constructor fails to compile if a
        // `uid` field is ever added without updating this line too.
        let _ = Cmd::Enable { until: None, until_reboot: false };

        let src = include_str!("ops.rs");
        let body_start = src.find("pub fn enable(").expect("enable must exist");
        let body_end = src.find("fn resolve_expiry(").expect("resolve_expiry must exist");
        let enable_wrapper = &src[body_start..body_end];
        assert!(
            enable_wrapper.contains("Subject::pkexec"),
            "enable's production wrapper must build its Subject via Subject::pkexec"
        );
        assert!(
            !enable_wrapper.contains("Subject::root_target"),
            "enable must never accept a caller-supplied target uid"
        );
    }

    // task 7.10: `revoke` calls `admit_root_target` BEFORE `disable_inner`
    // — so a uid-0 (or otherwise inadmissible) target is rejected exit 11
    // before `disable_inner`'s own (deliberately admission-free) removal
    // logic ever runs. `disable_inner` itself stays exactly as permissive
    // as it always was (design.md §3, "the one real tension").
    #[test]
    fn revoke_calls_admit_root_target_before_disable_inner_so_an_inadmissible_target_is_rejected_first() {
        let src = include_str!("ops.rs");
        let body_start = src.find("pub fn revoke(").expect("revoke must exist");
        let body_end = src.find("pub fn inspect(").expect("inspect must exist");
        let revoke_wrapper = &src[body_start..body_end];
        let admit_pos = revoke_wrapper.find("admit_root_target").expect("revoke must call admit_root_target");
        let inner_pos = revoke_wrapper.find("disable_inner(").expect("revoke must call disable_inner");
        assert!(
            admit_pos < inner_pos,
            "revoke must call admit_root_target BEFORE disable_inner, so an inadmissible target is \
             rejected exit 11 before disable_inner's own admission-free removal logic ever runs"
        );
    }

    #[test]
    fn disable_inner_itself_still_takes_no_admission_even_though_revoke_is_now_bounded() {
        // design.md §3's "one real tension", pinned at runtime: the bound
        // lives at revoke's wrapper level (admit_root_target), never
        // inside the shared transaction `disable` also uses — proven by
        // handing `disable_inner` a subject whose uid `admit_root_target`
        // would reject (uid 0) and confirming it still runs the removal
        // unconditionally.
        let (root, layout) = fresh_layout("revoke_disable_inner_still_unbounded");
        let binaries = fake_binaries(&root);
        let content = render_rule(0, "root", Expiry::Never);
        std::fs::write(layout.rule_path(0), content).unwrap();
        let runner = ScriptedRunner::new(vec![(systemctl_stop_spec(&binaries, 0), Ok(ok(0)))]);
        let subject = subj_root(0, AuditEvent::Revoke);
        disable_inner(&layout, &runner, &binaries, subject).unwrap();
        assert!(!layout.rule_path(0).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // task 7.11: `inspect` shares `status_inner` exactly with `status`,
    // producing a byte-identical `HelperStatus` JSON shape, and audits
    // `AuditEvent::Inspect` — never `AuditEvent::Status` (design.md §5,
    // "identical JSON, one implementation").
    #[test]
    fn inspect_shares_status_inner_with_status_but_audits_inspect_not_status() {
        let (root, layout) = fresh_layout("inspect_shares_status_inner");
        let content = render_rule(REAL_UID, "jorge", Expiry::Never);
        std::fs::write(layout.rule_path(REAL_UID), content).unwrap();

        let inspect_subject = subj_root(REAL_UID, AuditEvent::Inspect);
        let status_subject = subj_pkexec(REAL_UID, AuditEvent::Status);
        let inspect_status = status_inner(&layout, inspect_subject, 1_000_000);
        let status_status = status_inner(&layout, status_subject, 1_000_000);
        assert_eq!(
            inspect_status.to_json_line(),
            status_status.to_json_line(),
            "inspect must produce a byte-identical HelperStatus shape to status"
        );

        let text = capture_audit(|| {
            audit_status(&inspect_status, inspect_subject);
        });
        assert!(text.contains("EVENT=\"inspect\""), "audit text: {text}");
        assert!(text.contains("CONTEXT=\"SystemRoot\""), "audit text: {text}");
        assert!(!text.contains("EVENT=\"status\""), "audit text: {text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn inspect_on_a_uid_with_no_rule_reports_active_false_exactly_like_status() {
        let (root, layout) = fresh_layout("inspect_missing_rule");
        let subject = subj_root(4_294_967_294, AuditEvent::Inspect);
        let status = status_inner(&layout, subject, 1_000_000);
        assert!(!status.active, "absence is a fact, not a failure — inspect must report active:false");
        let _ = std::fs::remove_dir_all(&root);
    }

    // task 7.12: `grant`'s double `admit_uid` call (once in
    // `admit_root_target`, once again inside `enable_inner`'s unchanged
    // step 5) is pure and can only ever agree with itself — never
    // diverge (threat matrix "Arbitrary-uid targeting"; design.md §3).
    #[test]
    fn admit_uid_is_pure_so_grants_double_call_can_only_ever_agree_with_itself() {
        let range = UidRange { min: 1000, max: 60_000 };
        for uid in [0u32, 500, 1000, 60_000, 60_001, 4_294_967_294] {
            assert_eq!(
                checks::admit_uid(uid, &range),
                checks::admit_uid(uid, &range),
                "admit_uid must be pure: the same (uid, range) pair must always agree with itself, \
                 which is the property grant's deliberate double call depends on"
            );
        }
    }
}

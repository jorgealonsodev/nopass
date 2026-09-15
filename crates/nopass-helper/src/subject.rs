//! The `Subject` seam (design.md §2; privilege-admission §UID Resolution
//! by Invocation Context).
//!
//! Today the subcommand name implies where a transaction's target uid
//! came from. `Subject` replaces that implication with a type: the only
//! two ways to build one each fix `source` from the `InvocationContext`
//! that was actually resolved, so a future subcommand cannot mislabel
//! its own authority in the audit trail (`journal.rs`'s `context` field
//! reads `subject.source()`, never `subject.event()`).

use crate::error::HelperError;
use crate::journal::AuditEvent;
use crate::uid::InvocationContext;

/// How this transaction's target uid was obtained. Derived from the
/// resolved `InvocationContext`, NEVER from the subcommand name — the
/// name stopped implying the answer the moment `grant` and `enable`
/// could both produce a rule for the same uid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UidSource {
    Pkexec,
    SystemRoot,
}

impl UidSource {
    /// The literal `NOPASS_CONTEXT` value domain (design.md §4): the
    /// `InvocationContext` variant names themselves, so
    /// `journalctl NOPASS_CONTEXT=SystemRoot` matches the spec
    /// byte-for-byte. Deliberately breaks `OUTCOME`'s snake_case
    /// convention — an auditor greps what the requirement says, not what
    /// a neighbouring field's casing suggests.
    pub fn as_str(self) -> &'static str {
        match self {
            UidSource::Pkexec => "Pkexec",
            UidSource::SystemRoot => "SystemRoot",
        }
    }
}

/// The identity a transaction acts on and the authority it acts under.
/// Fields are private: [`Subject::pkexec`] and [`Subject::root_target`]
/// are the only ways to build one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subject {
    uid: u32,
    source: UidSource,
    event: AuditEvent,
}

impl Subject {
    /// Self-targeted. NOTE THE ABSENT UID PARAMETER: the uid can only be
    /// unpacked from `InvocationContext::Pkexec`, so a caller physically
    /// cannot supply one. This is what keeps `enable`/`disable`/`status`
    /// permanently self-targeted, at the type level rather than by test.
    ///
    /// Returns `Err(HelperError::Context(_))` (exit 10) for
    /// `InvocationContext::SystemRoot` rather than panicking: a future
    /// mispairing introduced by a refactor fails closed with no write,
    /// instead of a code in no table the tray or an operator can read.
    pub fn pkexec(ctx: InvocationContext, event: AuditEvent) -> Result<Self, HelperError> {
        match ctx {
            InvocationContext::Pkexec(uid) => Ok(Self { uid, source: UidSource::Pkexec, event }),
            InvocationContext::SystemRoot => {
                Err(HelperError::Context("pkexec-sourced Subject requires a Pkexec invocation context"))
            }
        }
    }

    /// Root-supplied explicit target. THE ONLY FUNCTION IN THE CRATE
    /// that places a caller-supplied uid into a `Subject`. Refuses any
    /// context other than `SystemRoot` — returns
    /// `Err(HelperError::Context(_))` (exit 10), never panics.
    pub fn root_target(ctx: InvocationContext, uid: u32, event: AuditEvent) -> Result<Self, HelperError> {
        match ctx {
            InvocationContext::SystemRoot => Ok(Self { uid, source: UidSource::SystemRoot, event }),
            InvocationContext::Pkexec(_) => {
                Err(HelperError::Context("SystemRoot-sourced Subject requires a SystemRoot invocation context"))
            }
        }
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    pub fn source(&self) -> UidSource {
        self.source
    }

    pub fn event(&self) -> AuditEvent {
        self.event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 3.2 — `Subject::pkexec` takes no `uid: u32` parameter; this table
    // test documents rather than proves that (compile-level — matching
    // M2's declined `trybuild` precedent, `archive/2026-09-15-m2-tray/
    // design.md:716`). What it DOES prove at runtime: `pkexec` unpacks
    // `uid` only from `InvocationContext::Pkexec`, and rejects
    // `SystemRoot` with a `Context` error (exit 10), never a panic.
    #[test]
    fn pkexec_unpacks_uid_only_from_the_pkexec_context() {
        let subject = Subject::pkexec(InvocationContext::Pkexec(1000), AuditEvent::Enable)
            .expect("a Pkexec context must build a Subject");
        assert_eq!(subject.uid(), 1000);
        assert_eq!(subject.source(), UidSource::Pkexec);
        assert_eq!(subject.event(), AuditEvent::Enable);
    }

    #[test]
    fn pkexec_rejects_system_root_context_with_a_context_error_not_a_panic() {
        let err = Subject::pkexec(InvocationContext::SystemRoot, AuditEvent::Enable)
            .expect_err("SystemRoot must not build a Pkexec-sourced Subject");
        assert_eq!(err.exit_code(), 10);
    }

    // 3.3 — `Subject::root_target` accepts only `SystemRoot`. Rejecting
    // `Pkexec(_)` also covers "refuses a non-root real uid": the only
    // way `uid::resolve` ever produces a `Pkexec(_)` context is a
    // pkexec-mediated invocation, which is never the process's own real
    // uid 0 — `root_target` has no other input through which a real uid
    // could reach it.
    #[test]
    fn root_target_accepts_system_root_and_carries_the_explicit_uid() {
        let subject = Subject::root_target(InvocationContext::SystemRoot, 1000, AuditEvent::Grant)
            .expect("SystemRoot must build a Subject for any explicit uid");
        assert_eq!(subject.uid(), 1000);
        assert_eq!(subject.source(), UidSource::SystemRoot);
        assert_eq!(subject.event(), AuditEvent::Grant);
    }

    #[test]
    fn root_target_rejects_pkexec_context_with_a_context_error_not_a_panic() {
        let err = Subject::root_target(InvocationContext::Pkexec(1000), 2000, AuditEvent::Grant)
            .expect_err("Pkexec must not build a SystemRoot-sourced Subject");
        assert_eq!(err.exit_code(), 10);
    }

    // 3.4 — accessors round-trip exactly what each constructor was given.
    #[test]
    fn accessors_round_trip_exactly_what_each_constructor_was_given() {
        let via_pkexec = Subject::pkexec(InvocationContext::Pkexec(42), AuditEvent::Status)
            .expect("Pkexec context must build a Subject");
        assert_eq!(via_pkexec.uid(), 42);
        assert_eq!(via_pkexec.source(), UidSource::Pkexec);
        assert_eq!(via_pkexec.event(), AuditEvent::Status);

        let via_root = Subject::root_target(InvocationContext::SystemRoot, 777, AuditEvent::Revoke)
            .expect("SystemRoot context must build a Subject");
        assert_eq!(via_root.uid(), 777);
        assert_eq!(via_root.source(), UidSource::SystemRoot);
        assert_eq!(via_root.event(), AuditEvent::Revoke);
    }

    // 3.4 — structural assertion: nothing but the two constructors
    // builds a `Subject`. Field privacy is what the COMPILER enforces,
    // and it is the real guarantee: no code outside this module can name
    // `uid`, `source` or `event`, so no code outside this module can
    // build one. What this test adds is a bound on how many places
    // INSIDE the module do.
    //
    // It counts struct literals, not signatures. An earlier version of
    // this test counted occurrences of the exact return type
    // `-> Result<Self, HelperError> {`, and the orchestrator evaded it
    // in one edit: a third constructor spelling its return type
    // `-> Result<Subject, HelperError>` — same type, different bytes —
    // took a bare uid with no context check at all and the test still
    // passed. That is the bypass this whole change exists to prevent,
    // and the guard against it had a spelling-shaped hole.
    //
    // A struct literal has no such freedom. Field privacy means every
    // constructor, however its signature is written, must write
    // `Subject {` inside this module. Counting those counts the thing
    // that matters.
    #[test]
    fn nothing_but_the_two_constructors_builds_a_subject() {
        let src = include_str!("subject.rs");
        // Everything below `mod tests` is test code; a test may build a
        // `Subject` freely and must not count against production.
        let production = &src[..src.find("mod tests").unwrap_or(src.len())];

        // Both spellings of the literal count. Inside `impl Subject`,
        // `Self { … }` and `Subject { … }` are the same construction and
        // a third constructor may use either. The needles are built by
        // concatenation so this file's own source never contains them
        // contiguously, or the assertion would count itself.
        let via_self = format!("{}{}", "Self", " {");
        let via_name = format!("{}{}", "Subject", " {");
        // `pub struct Subject {` and `impl Subject {` are not
        // constructions; subtract them rather than matching a narrower
        // needle, so a renamed impl block cannot slip past.
        let decl = format!("{}{}", "pub struct Subject", " {");
        let impl_header = format!("{}{}", "impl Subject", " {");

        let constructions = production.matches(via_self.as_str()).count()
            + production.matches(via_name.as_str()).count()
            - production.matches(decl.as_str()).count()
            - production.matches(impl_header.as_str()).count();

        assert_eq!(
            constructions, 2,
            "exactly two places may construct a Subject: Subject::pkexec, which has no uid \
             parameter to be given one, and Subject::root_target, which refuses any context \
             but SystemRoot. A third is the widening this change exists to bound."
        );
    }
}

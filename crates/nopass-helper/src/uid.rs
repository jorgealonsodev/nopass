//! Invocation-context uid resolution (privilege-admission §UID Resolution
//! by Invocation Context; design.md §2, §4.1 steps 3; threat matrix
//! "Privileged invocation context").
//!
//! `enable`/`disable`/`status` resolve their target uid exclusively from
//! `PKEXEC_UID` and carry no uid argument of their own; `expire`,
//! `grant`, `revoke`, and `inspect` each require the process's real uid
//! to be 0 AND `PKEXEC_UID` unset or empty, and each takes its own
//! target uid from its own `--uid`/`--boot` flag instead — never from
//! `PKEXEC_UID` (m3a-headless-grant design.md §2). Both `PKEXEC_UID` and
//! the real uid are
//! taken as explicit parameters rather than read from `std::env`/`nix`
//! directly inside `resolve`, so every rejection case can be asserted
//! deterministically under `cargo test` without mutating this process's
//! own environment — which `#![forbid(unsafe_code)]` makes impossible
//! anyway, since `std::env::set_var` requires `unsafe` as of the 2024
//! edition. Production call sites (wired in Phase 7's `ops.rs`) pass
//! `std::env::var("PKEXEC_UID").ok()` and `nix::unistd::getuid().as_raw()`.
//!
//! `resolve` performs no filesystem or process I/O of its own, so every
//! rejection case is zero-mutation by construction — there is no `Layout`
//! or `CommandRunner` in scope for it to act through.
// Phase 7's ops.rs is the production caller; until it lands, `resolve` is
// exercised only by the tests below. No `#[allow(dead_code)]` is needed
// despite that: `InvocationContext` and `resolve` are `pub` inside this
// crate's `pub mod uid` (declared in `lib.rs`), which makes them public
// library API exempt from the `dead_code` lint.

use crate::cli::Cmd;
use crate::error::HelperError;

/// The resolved invocation context: which uid the helper is acting on,
/// and under what authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationContext {
    /// `enable`/`disable`/`status`, resolved from a valid, non-zero
    /// `PKEXEC_UID`.
    Pkexec(u32),
    /// `expire`, `grant`, `revoke`, or `inspect`, running as the
    /// process's own real uid 0 with no `PKEXEC_UID` set. Each still
    /// carries its own target uid separately, via its own `--uid`/
    /// `--boot` flag — this variant names only the authority, not the
    /// target.
    SystemRoot,
}

/// Resolves the invocation context for `cmd`.
///
/// - `Enable`/`Disable`/`Status`: `pkexec_uid` must be present, non-empty,
///   parseable as a `u32`, and non-zero. Anything else is
///   `HelperError::Context` (exit 10).
/// - `Expire`/`Grant`/`Revoke`/`Inspect`: `real_uid` must be exactly `0`
///   and `pkexec_uid` must be absent or empty. `PKEXEC_UID` being set at
///   all (even to a valid value) on any of these four is rejected — none
///   of them is ever a pkexec-context invocation. Anything else is
///   `HelperError::Context` (exit 10).
///
/// An empty `PKEXEC_UID` is treated identically to an absent one,
/// matching the empty-environment convention documented for
/// `nopass-cleanup.service` (design.md §7).
pub fn resolve(cmd: &Cmd, pkexec_uid: Option<&str>, real_uid: u32) -> Result<InvocationContext, HelperError> {
    let pkexec_present = pkexec_uid.map(|v| !v.is_empty()).unwrap_or(false);

    match cmd {
        Cmd::Enable { .. } | Cmd::Disable | Cmd::Status => {
            let raw = pkexec_uid
                .filter(|v| !v.is_empty())
                .ok_or(HelperError::Context("PKEXEC_UID is missing or empty"))?;
            // `u32::from_str` accepts a leading `+` and leading zeros
            // (e.g. `+1000`, `01000`) — this is not octal, Rust's decimal
            // parser has no octal-prefix convention. Both forms resolve
            // to the same uid as the canonical decimal form. This is
            // deliberate and spec-compliant (privilege-admission §UID
            // Resolution by Invocation Context requires only present,
            // non-empty, parseable as u32, and non-zero) — do not "fix"
            // this into a rejection; see
            // `pkexec_uid_with_leading_zero_is_accepted_per_u32_from_str`
            // and `pkexec_uid_with_explicit_plus_sign_is_accepted_per_u32_from_str`
            // below.
            let uid: u32 =
                raw.parse().map_err(|_| HelperError::Context("PKEXEC_UID is not a valid non-negative integer"))?;
            if uid == 0 {
                return Err(HelperError::Context("PKEXEC_UID must not be 0"));
            }
            Ok(InvocationContext::Pkexec(uid))
        }
        // `Expire`, `Grant`, `Revoke`, and `Inspect` share one arm
        // because they share exactly one authority requirement: real uid
        // 0 AND `PKEXEC_UID` unset/empty. That is the entire similarity
        // — each still takes its OWN target uid from its OWN
        // `--uid`/`--boot` flag; `resolve` never reads a target uid out
        // of `Cmd` here, only the invocation context (see
        // `grant_resolves_system_root_regardless_of_the_uid_value_
        // carried_by_cmd` below, which proves the point by resolving two
        // different `--uid` values identically).
        //
        // `Enable`/`Disable`/`Status`, by contrast, have no uid argument
        // on their CLI surface at all (`cli.rs`) and take their target
        // exclusively from `PKEXEC_UID` — the arm above this one. That
        // asymmetry is the security property this module exists to
        // enforce: a `SystemRoot`-context subcommand may name ANY
        // admitted uid as its target (bounds checked later, in
        // `ops.rs`'s `admit_root_target`), while a `Pkexec`-context
        // subcommand is permanently self-targeted and has no flag to ask
        // for anyone else. Do not add a uid argument to
        // `Enable`/`Disable`/`Status`, and do not remove `--uid` from
        // this arm's four subcommands — collapsing either family into
        // the other erases this boundary.
        //
        // This match has no wildcard arm (pinned by
        // `resolve_match_places_every_systemroot_cmd_variant_explicitly_
        // no_wildcard_absorption` below): a ninth `Cmd` variant must
        // fail to compile here, not silently join whichever arm `_`
        // would have caught.
        Cmd::Expire { .. } | Cmd::Grant { .. } | Cmd::Revoke { .. } | Cmd::Inspect { .. } => {
            if pkexec_present {
                return Err(HelperError::Context("this subcommand must not run under a pkexec context"));
            }
            if real_uid != 0 {
                return Err(HelperError::Context("this subcommand requires the real uid to be 0"));
            }
            Ok(InvocationContext::SystemRoot)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enable() -> Cmd {
        Cmd::Enable { until: None, until_reboot: false }
    }

    fn disable() -> Cmd {
        Cmd::Disable
    }

    fn status() -> Cmd {
        Cmd::Status
    }

    fn expire_uid() -> Cmd {
        Cmd::Expire { uid: Some(1000), boot: false }
    }

    fn grant_uid() -> Cmd {
        Cmd::Grant { uid: 1000, until: None, until_reboot: false }
    }

    fn revoke_uid() -> Cmd {
        Cmd::Revoke { uid: 1000 }
    }

    fn inspect_uid() -> Cmd {
        Cmd::Inspect { uid: 1000 }
    }

    fn assert_context_exit_10(result: Result<InvocationContext, HelperError>) {
        match result {
            Err(err) => assert_eq!(err.exit_code(), 10, "expected exit 10, got a different HelperError: {err}"),
            Ok(ctx) => panic!("expected a Context rejection (exit 10), got Ok({ctx:?})"),
        }
    }

    #[test]
    fn enable_resolves_uid_from_pkexec_uid() {
        assert_eq!(resolve(&enable(), Some("1000"), 0).unwrap(), InvocationContext::Pkexec(1000));
    }

    #[test]
    fn disable_and_status_also_resolve_from_pkexec_uid() {
        assert_eq!(resolve(&disable(), Some("1000"), 0).unwrap(), InvocationContext::Pkexec(1000));
        assert_eq!(resolve(&status(), Some("1000"), 0).unwrap(), InvocationContext::Pkexec(1000));
    }

    // Threat matrix "Privileged invocation context" — one RED test per
    // listed adversarial case, each asserting exit 10. `resolve` has no
    // `Layout`/`CommandRunner` capability at all, so zero filesystem
    // mutation holds by construction for every case below, not merely by
    // assertion.

    #[test]
    fn pkexec_uid_absent_on_enable_is_rejected() {
        assert_context_exit_10(resolve(&enable(), None, 0));
    }

    #[test]
    fn pkexec_uid_non_numeric_on_enable_is_rejected() {
        assert_context_exit_10(resolve(&enable(), Some("abc"), 0));
    }

    #[test]
    fn pkexec_uid_empty_on_enable_is_rejected() {
        assert_context_exit_10(resolve(&enable(), Some(""), 0));
    }

    #[test]
    fn pkexec_uid_zero_on_enable_is_rejected() {
        assert_context_exit_10(resolve(&enable(), Some("0"), 0));
    }

    #[test]
    fn pkexec_uid_negative_on_enable_is_rejected() {
        // `u32` has no sign, so a negative literal fails to parse — this
        // lands in the same "not a valid non-negative integer" bucket as
        // the non-numeric case, and still exits 10.
        assert_context_exit_10(resolve(&enable(), Some("-1"), 0));
    }

    #[test]
    fn pkexec_uid_absurdly_large_on_enable_is_rejected() {
        // Overflows `u32::MAX` — `str::parse::<u32>` fails the same way
        // as a non-numeric value.
        assert_context_exit_10(resolve(&enable(), Some("999999999999999999999"), 0));
    }

    #[test]
    fn expire_invoked_with_pkexec_uid_set_is_rejected() {
        assert_context_exit_10(resolve(&expire_uid(), Some("1000"), 0));
    }

    #[test]
    fn pkexec_context_subcommands_ignore_real_uid_and_still_require_pkexec_uid() {
        // Pins the structural property that `real_uid` is irrelevant to
        // Enable/Disable/Status admission — the `Enable | Disable |
        // Status` match arm in `resolve` never reads `real_uid` at all.
        // The same PKEXEC_UID-missing rejection (exit 10) must hold
        // across every `real_uid` tried below, including 0 (root) and
        // non-root uids: if a future change made that arm consult
        // `real_uid` (e.g. to let root bypass the PKEXEC_UID
        // requirement), the `real_uid == 0` case here would start
        // returning `Ok` and this test would fail.
        for real_uid in [0, 1, 1000, 65_534] {
            assert_context_exit_10(resolve(&enable(), None, real_uid));
            assert_context_exit_10(resolve(&disable(), None, real_uid));
            assert_context_exit_10(resolve(&status(), None, real_uid));
        }
    }

    #[test]
    fn pkexec_uid_with_leading_zero_is_accepted_per_u32_from_str() {
        // See the deliberate-acceptance comment on the `raw.parse()` call
        // in `resolve` above — `01000` is not octal here, it parses as
        // decimal 1000.
        assert_eq!(resolve(&enable(), Some("01000"), 0).unwrap(), InvocationContext::Pkexec(1000));
    }

    #[test]
    fn pkexec_uid_with_explicit_plus_sign_is_accepted_per_u32_from_str() {
        // See the deliberate-acceptance comment on the `raw.parse()` call
        // in `resolve` above.
        assert_eq!(resolve(&enable(), Some("+1000"), 0).unwrap(), InvocationContext::Pkexec(1000));
    }

    #[test]
    fn expire_invoked_non_root_without_pkexec_uid_is_rejected() {
        assert_context_exit_10(resolve(&expire_uid(), None, 1000));
    }

    #[test]
    fn expire_resolves_as_system_root_when_real_uid_is_0_and_pkexec_uid_is_unset() {
        assert_eq!(resolve(&expire_uid(), None, 0).unwrap(), InvocationContext::SystemRoot);
    }

    #[test]
    fn expire_resolves_as_system_root_when_pkexec_uid_is_present_but_empty() {
        assert_eq!(resolve(&expire_uid(), Some(""), 0).unwrap(), InvocationContext::SystemRoot);
    }

    #[test]
    fn expire_boot_also_requires_real_root_and_no_pkexec_uid() {
        let boot = Cmd::Expire { uid: None, boot: true };
        assert_eq!(resolve(&boot, None, 0).unwrap(), InvocationContext::SystemRoot);
        assert_context_exit_10(resolve(&boot, Some("1000"), 0));
        assert_context_exit_10(resolve(&boot, None, 1000));
    }

    // --- Phase 6 (m3a-headless-grant): `grant`/`revoke`/`inspect` join
    // `expire` in the SystemRoot arm. Four scenarios per subcommand,
    // mirroring `expire`'s own coverage above exactly (privilege-admission
    // "UID Resolution by Invocation Context", "SystemRoot Context Can
    // Target Any Admitted UID") -------------------------------------------

    #[test]
    fn grant_resolves_as_system_root_when_real_uid_is_0_and_pkexec_uid_is_unset() {
        assert_eq!(resolve(&grant_uid(), None, 0).unwrap(), InvocationContext::SystemRoot);
    }

    #[test]
    fn grant_invoked_with_pkexec_uid_set_is_rejected() {
        assert_context_exit_10(resolve(&grant_uid(), Some("1000"), 0));
    }

    #[test]
    fn grant_resolves_as_system_root_when_pkexec_uid_is_present_but_empty() {
        assert_eq!(resolve(&grant_uid(), Some(""), 0).unwrap(), InvocationContext::SystemRoot);
    }

    #[test]
    fn grant_invoked_non_root_without_pkexec_uid_is_rejected() {
        assert_context_exit_10(resolve(&grant_uid(), None, 1000));
    }

    #[test]
    fn grant_resolves_system_root_regardless_of_the_uid_value_carried_by_cmd() {
        // `resolve` decides only the invocation CONTEXT; it never reads
        // `Cmd::Grant`'s own `uid` field — two different `--uid` values
        // must resolve identically, proving the context (not the target)
        // is what this function computes. The target uid itself is read
        // from `--uid` downstream, in `ops.rs`'s `Subject::root_target`
        // (Phase 7), never from `PKEXEC_UID` and never from `resolve`'s
        // return value (privilege-admission "Grant resolves the
        // SystemRoot context and the explicit target uid").
        let low = Cmd::Grant { uid: 1, until: None, until_reboot: false };
        let high = Cmd::Grant { uid: 999_999, until: None, until_reboot: false };
        assert_eq!(resolve(&low, None, 0).unwrap(), InvocationContext::SystemRoot);
        assert_eq!(resolve(&high, None, 0).unwrap(), InvocationContext::SystemRoot);
    }

    #[test]
    fn revoke_resolves_as_system_root_when_real_uid_is_0_and_pkexec_uid_is_unset() {
        assert_eq!(resolve(&revoke_uid(), None, 0).unwrap(), InvocationContext::SystemRoot);
    }

    #[test]
    fn revoke_invoked_with_pkexec_uid_set_is_rejected() {
        assert_context_exit_10(resolve(&revoke_uid(), Some("1000"), 0));
    }

    #[test]
    fn revoke_resolves_as_system_root_when_pkexec_uid_is_present_but_empty() {
        assert_eq!(resolve(&revoke_uid(), Some(""), 0).unwrap(), InvocationContext::SystemRoot);
    }

    #[test]
    fn revoke_invoked_non_root_without_pkexec_uid_is_rejected() {
        assert_context_exit_10(resolve(&revoke_uid(), None, 1000));
    }

    #[test]
    fn inspect_resolves_as_system_root_when_real_uid_is_0_and_pkexec_uid_is_unset() {
        assert_eq!(resolve(&inspect_uid(), None, 0).unwrap(), InvocationContext::SystemRoot);
    }

    #[test]
    fn inspect_invoked_with_pkexec_uid_set_is_rejected() {
        assert_context_exit_10(resolve(&inspect_uid(), Some("1000"), 0));
    }

    #[test]
    fn inspect_resolves_as_system_root_when_pkexec_uid_is_present_but_empty() {
        assert_eq!(resolve(&inspect_uid(), Some(""), 0).unwrap(), InvocationContext::SystemRoot);
    }

    #[test]
    fn inspect_invoked_non_root_without_pkexec_uid_is_rejected() {
        assert_context_exit_10(resolve(&inspect_uid(), None, 1000));
    }

    // The structural guard: a ninth `Cmd` variant must fail to be added
    // silently. Modeled on `subject.rs`'s
    // `nothing_but_the_two_constructors_builds_a_subject` — pin something
    // the compiler enforces (an exhaustive match has no room for a
    // catch-all pattern to hide behind), not a type signature's spelling.
    // The earlier version of this house-style guard (Phase 3's
    // `subject.rs`) counted one exact return-type spelling and a
    // differently-spelled equivalent constructor walked straight past it
    // — this guard counts syntax the language itself cannot respell:
    // there is exactly one way to write a wildcard match arm (`_ =>`),
    // and exactly one contiguous pattern text names the four SystemRoot
    // variants today.
    #[test]
    fn resolve_match_places_every_systemroot_cmd_variant_explicitly_no_wildcard_absorption() {
        let src = include_str!("uid.rs");
        // Everything below `mod tests` is test code; only production
        // code is in scope for this guard.
        let production = &src[..src.find("mod tests").unwrap_or(src.len())];

        // No bare `_` match arm anywhere in `resolve`'s `match cmd { .. }`.
        // A wildcard would let an eighth `Cmd` variant compile silently
        // into whatever arm `_` happens to catch, instead of failing to
        // compile — which is the entire point of an exhaustive,
        // wildcard-free match (the same discipline `journal.rs`'s
        // `audit_event_for` already applies). The needle is built by
        // concatenation so this file's own source, including this test,
        // never contains it contiguously and cannot accidentally match
        // itself.
        let wildcard_arm = format!("{}{}", "_", " =>");
        assert!(
            !production.contains(wildcard_arm.as_str()),
            "resolve's match over Cmd must have no wildcard arm — a new Cmd variant must fail \
             to compile here, not silently resolve to whatever arm '_' happens to catch"
        );

        // Pin the SystemRoot-producing arm's pattern text exactly: four
        // named variants, no more, no fewer, each immediately followed
        // by `{ .. } |` or `{ .. } =>`. A variant folded into this arm —
        // whether inserted in the middle or appended at either end —
        // breaks this exact contiguous string, so the change shows up as
        // a literal diff to this test rather than disappearing into an
        // already-multi-variant OR-pattern unnoticed.
        let system_root_arm =
            "Cmd::Expire { .. } | Cmd::Grant { .. } | Cmd::Revoke { .. } | Cmd::Inspect { .. } =>";
        assert_eq!(
            production.matches(system_root_arm).count(),
            1,
            "the SystemRoot-producing arm must name exactly Expire, Grant, Revoke, and Inspect — \
             a fifth variant absorbed here must show up as a literal diff to this pinned \
             string, not disappear into the arm silently"
        );
    }
}

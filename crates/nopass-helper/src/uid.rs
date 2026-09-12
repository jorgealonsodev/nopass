//! Invocation-context uid resolution (privilege-admission §UID Resolution
//! by Invocation Context; design.md §2, §4.1 steps 3; threat matrix
//! "Privileged invocation context").
//!
//! `enable`/`disable`/`status` resolve their target uid exclusively from
//! `PKEXEC_UID`; `expire` requires the process's real uid to be 0 AND
//! `PKEXEC_UID` unset or empty. Both `PKEXEC_UID` and the real uid are
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
#![allow(dead_code)] // Phase 7's ops.rs is the production caller; until it
// lands, `resolve` is exercised only by the tests below.

use crate::cli::Cmd;
use crate::error::HelperError;

/// The resolved invocation context: which uid the helper is acting on,
/// and under what authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationContext {
    /// `enable`/`disable`/`status`, resolved from a valid, non-zero
    /// `PKEXEC_UID`.
    Pkexec(u32),
    /// `expire`, running as the process's own real uid 0 with no
    /// `PKEXEC_UID` set.
    SystemRoot,
}

/// Resolves the invocation context for `cmd`.
///
/// - `Enable`/`Disable`/`Status`: `pkexec_uid` must be present, non-empty,
///   parseable as a `u32`, and non-zero. Anything else is
///   `HelperError::Context` (exit 10).
/// - `Expire`: `real_uid` must be exactly `0` and `pkexec_uid` must be
///   absent or empty. `PKEXEC_UID` being set at all (even to a valid
///   value) on `expire` is rejected — an `expire` invocation is never a
///   pkexec-context invocation. Anything else is `HelperError::Context`
///   (exit 10).
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
        Cmd::Expire { .. } => {
            if pkexec_present {
                return Err(HelperError::Context("expire must not run under a pkexec context"));
            }
            if real_uid != 0 {
                return Err(HelperError::Context("expire requires the real uid to be 0"));
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
}

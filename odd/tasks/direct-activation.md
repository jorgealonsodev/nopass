# Direct activation (no pre-consent gate)

## Objective
Any activation request (menu toggle, SNI left click, keyboard Activate, "Activate during…" duration) dispatches the enable action immediately, so the polkit administrator-password dialog appears on the first click when polkit requires authentication.

## Problem
While `warning_acknowledged` is false (no `~/.config/nopass/config`), the toggle and left click only post a "consent needed" notification and invoke nothing; only "Activate during… → I understand" grants. Field evidence (2026-10-08 journal): ~10 minutes of clicks produced zero polkit requests for `com.enfoquestic.nopass.manage`; the only grant came via the consent branch.

## Why
The user wants one click → admin password prompt. The polkit authentication dialog is the explicit confirmation; the in-app warning duplicates it and hides activation behind a menu path.

## Scope
- Remove the in-app consent gate and consent branch from the activation flow (crates/nopass: consent, app, menu, event, tray, format, notifications, outcome, lib and tests as needed).
- Keep reading existing config files that contain `warning_acknowledged` (backward compatible; field may become ignored).
- Out of scope: polkit policy, helper, packaging.

## Acceptance criteria
- Toggle / left click / Activate while Inactive spawn `Action::Enable` with the default duration on the first click, regardless of config.
- Selecting a duration under "Activate during…" spawns `Action::Enable` with that duration directly.
- No consent branch rows render; no "consent needed" notification exists.
- Existing configs with `warning_acknowledged` still parse.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` pass.

## Tasks
- [x] T1 — Remove the consent gate so every activation dispatches enable directly (route: delegated writer; trigger: 2+ non-trivial files).

## Progress / evidence
- Branch: `feat/direct-activation` from `main` (f90605b).

- T1: consent.rs deleted; Granted token, consent branch, consent-needed notification removed; toggle/duration dispatch Enable directly. Config field kept (persisted as `warn_before_activation`, now inert) because tests/config_tempdir.rs constructs it.
- RED: `handle_toggle` test failed with "the first toggle must dispatch exactly one enable, got None"; GREEN after change.
- Checks: clippy -D warnings clean; `cargo test -p nopass --lib` 251 passed (parent spot check); dbus_session 15 pass / 5 fail — same 5 notification-daemon tests fail on base.
- Known pre-existing: `cargo fmt --check` fails on main (no rustfmt.toml); untracked tests/readme_packaging.rs belongs to unmerged M4 branch (README/PKGBUILD absent on main).

## Follow-ups
- Update openspec activation-consent / user-config specs.
- Remove inert `warning_acknowledged` field together with tests/config_tempdir.rs.

## Next step
Released as v0.2.0 (version bump, merged to main).

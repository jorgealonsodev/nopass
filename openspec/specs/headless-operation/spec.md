# Headless Operation Specification

## Purpose

Defines NoPass as a root-invocable operator interface usable with no desktop
session at all — `grant`, `revoke`, and `inspect` functioning identically
with no `DISPLAY`, no `DBUS_SESSION_BUS_ADDRESS`, and no tray process
running — and the operator documentation obligation that keeps the
supported path from being displaced by the `PKEXEC_UID=` forgery
workaround. The subcommand flag surface and `--uid` mechanics live in
`helper-cli`; the `SystemRoot` context resolution and the uid-range bound
live in `privilege-admission`. This capability defines the session-independence
property itself and the operator-facing documentation of it. (proposal
`m3a-headless-grant`)

## Requirements

### Requirement: No Desktop Session Required

`grant`, `revoke`, and `inspect` MUST complete their transaction with no
`DISPLAY` environment variable, no `DBUS_SESSION_BUS_ADDRESS` environment
variable, and no tray process (`nopass`) running on the host. None of the
three MAY depend on a D-Bus session, an X11/Wayland connection, or the tray
process for any part of their behavior.

#### Scenario: Grant succeeds with no session environment present
- GIVEN a shell with `DISPLAY` and `DBUS_SESSION_BUS_ADDRESS` both unset, and
  no `nopass` tray process running
- WHEN `grant --uid 1000 --until-reboot` runs as real uid 0 with
  `PKEXEC_UID` unset
- THEN the transaction completes and exits 0, identically to a run with a
  full desktop session present
- Testable via: root-only container test (uid 0 is the scenario's own
  precondition; the absent session is reproduced with `env_clear`, not by
  finding a machine that happens to lack one)

#### Scenario: Install, grant, and revoke complete end to end with no session
- GIVEN a freshly installed helper on a host with no desktop session and no
  tray process
- WHEN an operator runs `grant --uid <uid>`, observes it with
  `inspect --uid <uid>`, then runs `revoke --uid <uid>`
- THEN every step exits 0, the sudoers rule and state file reflect each
  transition, and no step required a bus, a display, or the tray
- Testable via: root-only container test. NOT lane B: that lane exists to give
  the tray a session bus, which is the one thing this scenario asserts is
  unnecessary, so running it there would prove the opposite of the claim

### Requirement: Operator Documentation Names Only the Supported Headless Commands

`docs/` MUST document `grant`, `revoke`, and `inspect` as the supported
headless procedure, MUST state the desktop assumption the rest of the PRD
makes for `enable`/`disable`/`status` (tray presence, an interactive polkit
agent, `allow_inactive=no`), and MUST NOT present
`sudo PKEXEC_UID=<uid> nopass-helper <cmd>` as a supported or recommended
invocation.

#### Scenario: Documentation lists the three headless subcommands as the supported path
- GIVEN `docs/` after this change
- WHEN the headless operator section is read
- THEN `grant`, `revoke`, and `inspect` are documented with their `--uid`
  requirement, and no example forges `PKEXEC_UID`
- Testable via: `cargo test` (a structural assertion over `docs/`, in the shape
  of the existing guard that scans crate sources for a literal sudoers path)

#### Scenario: Documentation states the desktop assumption made elsewhere
- GIVEN `docs/` after this change
- WHEN the headless section is read
- THEN it names that `enable`/`disable`/`status` assume a tray, an
  interactive polkit agent, and `allow_inactive=no`, and that none of these
  apply to the headless path
- Testable via: `cargo test` (structural assertion over `docs/`)

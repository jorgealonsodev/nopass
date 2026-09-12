# Tray Privileged Invocation Specification

## Purpose

Defines how the tray drives `nopass-helper` through `pkexec`, and the total mapping of every
exit code the invocation can produce onto a distinct user-facing outcome. The tray never
mutates privileged state itself. (PRD RF-02, RF-04 boundary, RF-07; helper-cli's exit-code
contract at `openspec/specs/helper-cli/spec.md`)

## Requirements

### Requirement: pkexec Invocation for Enable and Disable

The tray MUST invoke `enable`/`disable` only via
`pkexec /usr/libexec/nopass-helper <enable|disable> [--until <epoch>|--until-reboot]`, using
an absolute path and no shell wrapping, and MUST NOT invoke `expire` (root/timer-only).

#### Scenario: Enable invocation uses the exact documented argv
- GIVEN the tray toggles from inactive with the 1-hour default duration
- WHEN it invokes the helper
- THEN the captured argv is exactly `pkexec /usr/libexec/nopass-helper enable --until <epoch>`
  with no shell interpolation
- Testable via: `cargo test` (fake spawn port captures argv, no real pkexec)

### Requirement: The Tray Never Accesses /etc/sudoers.d

The tray MUST NOT stat, read, write, or watch any path under `/etc/sudoers.d/`. All knowledge
of rule state comes only from `/run/nopass/<uid>.state`, the `sudo -kn true` probe, and helper
exit codes.

#### Scenario: The tray's I/O surface excludes /etc/sudoers.d entirely
- GIVEN the tray's full set of file-system and command ports
  (`/run/nopass/` read+watch, `pkexec`, `sudo -kn true`)
- WHEN that port surface is enumerated
- THEN no port accepts or resolves any path under `/etc/sudoers.d/`
- Testable via: `cargo test` (structural assertion over the tray's port trait definitions)

### Requirement: Total Exit-Code-to-Outcome Mapping

Every code the invocation can produce MUST map to exactly one distinct user-facing outcome.
No two distinct codes MAY collapse into the same generic message.

| Source | Code | Outcome |
|---|---|---|
| helper | 0 | success |
| helper | 1 | internal error |
| helper | 2 | rejected invocation (should not occur from a well-formed tray call) |
| helper | 10 | invocation-context violation |
| helper | 11 | uid rejected |
| helper | 12 | not a sudoer |
| helper | 13 | invalid duration |
| helper | 14 | visudo rejected the rule |
| helper | 15 | lock busy, retry |
| helper | 16 | filesystem/atomic-write failure |
| helper | 17 | automatic-expiry timer could not be scheduled; the grant was rolled back |
| pkexec | 126 | authorization dialog dismissed |
| pkexec | 127 | not authorized / no polkit agent |
| — | spawn failure | `pkexec` itself could not be launched |

#### Scenario: Each documented code produces its own message
- GIVEN each code 0, 1, 2, 10–17, 126, 127, and a spawn failure in turn
- WHEN the tray maps the invocation result to an outcome
- THEN each produces the outcome listed above, and no two distinct codes share the same
  rendered message
- Testable via: `cargo test` (table-driven over a fake spawn port returning each code)

#### Scenario: Exit 17 is distinguished from a generic failure
- GIVEN the helper exits 17 (timer scheduling failed, the sudoers rule was rolled back)
- WHEN the tray renders the outcome
- THEN the message states that automatic expiry could not be scheduled and the grant was
  rolled back — not a generic "operation failed" string, and not the same message as exit 16
- Testable via: `cargo test`

#### Scenario: pkexec 126 and 127 are distinguished from each other and from helper codes
- GIVEN pkexec exits 126 in one case and 127 in another
- WHEN the tray renders each outcome
- THEN 126 states the dialog was dismissed and 127 states authorization failed or no agent is
  registered, and neither matches any helper exit-code message
- Testable via: `cargo test`

### Requirement: pkexec Invocation Never Blocks the Reactor

A pending `pkexec` call MUST NOT prevent the tray from handling other D-Bus requests (menu
open, quit) while it waits.

#### Scenario: Menu remains responsive during a pending authorization
- GIVEN a `pkexec` invocation has not yet returned
- WHEN the user opens the menu or selects Quit
- THEN the tray responds without waiting for the pending invocation to complete
- Testable via: dbus-run-session (fake spawn port that blocks until explicitly released)

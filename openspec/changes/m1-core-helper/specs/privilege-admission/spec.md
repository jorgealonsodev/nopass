# Privilege Admission Specification

## Purpose

Defines who the helper is allowed to act on and how it establishes that identity: invocation-context UID resolution, UID-range admission, the existing-sudoer probe, and the single polkit action gating all privileged entry. (PRD RF-04, NFR security rows, §7.3)

## Requirements

### Requirement: UID Resolution by Invocation Context

`enable`, `disable`, and `status` MUST resolve the target UID exclusively from the `PKEXEC_UID` environment variable; no username, uid, or path argument exists on those subcommands. `expire` MUST run only when the process's real uid is 0 AND `PKEXEC_UID` is unset; any other combination MUST exit 10 with no system change.

#### Scenario: Enable resolves uid from PKEXEC_UID
- GIVEN `PKEXEC_UID=1000` is set
- WHEN `enable` runs
- THEN the target uid resolved is 1000
- Testable via: `cargo test`

#### Scenario: PKEXEC_UID missing on a pkexec-context subcommand
- GIVEN `PKEXEC_UID` is unset
- WHEN `enable`, `disable`, or `status` runs
- THEN the helper exits 10 and makes no write
- Testable via: `cargo test`

#### Scenario: PKEXEC_UID present but unparseable
- GIVEN `PKEXEC_UID=abc`
- WHEN `enable` runs
- THEN the helper exits 10 and makes no write
- Testable via: `cargo test`

#### Scenario: Expire invoked with PKEXEC_UID set
- GIVEN the process runs as uid 0 but `PKEXEC_UID` is also set
- WHEN `expire --uid 1000` runs
- THEN the helper exits 10 and makes no system change
- Testable via: `cargo test`

#### Scenario: Expire invoked as non-root without PKEXEC_UID
- GIVEN the process's real uid is not 0 and `PKEXEC_UID` is unset
- WHEN `expire --uid 1000` runs
- THEN the helper exits 10 and makes no system change
- Testable via: root-only container test (requires a real non-root invocation)

### Requirement: UID Range Admission

The system MUST admit a target uid only when it exists via `getpwuid` and `UID_MIN <= uid <= UID_MAX`, read from `/etc/login.defs` with defaults 1000/60000 and a parsed `UID_MIN` clamped to a floor of 1000 when the file is missing, unparseable, or lower than 1000. uid 0 MUST always be rejected regardless of `/etc/login.defs` content.

#### Scenario: Default-range uid is admitted
- GIVEN uid 1000 and default `login.defs` values
- WHEN admission runs
- THEN the uid is admitted
- Testable via: `cargo test`

#### Scenario: uid 0 is always rejected
- GIVEN `/etc/login.defs` sets `UID_MIN 0`
- WHEN admission runs for uid 0
- THEN the helper exits 11
- Testable via: `cargo test`

#### Scenario: uid 65534 (`nobody`) is rejected
- GIVEN default `UID_MAX 60000`
- WHEN admission runs for uid 65534
- THEN the helper exits 11 because 65534 exceeds `UID_MAX`
- Testable via: `cargo test`

#### Scenario: Malformed or missing login.defs falls back to the clamped floor
- GIVEN `/etc/login.defs` is missing or `UID_MIN` is non-numeric or below 1000
- WHEN admission runs
- THEN `UID_MIN` is treated as 1000, not a crash and not a lower value
- Testable via: `cargo test`

#### Scenario: uid not present in getpwuid
- GIVEN a uid with no corresponding password-database entry
- WHEN admission runs
- THEN the helper exits 11
- Testable via: `cargo test` (stubbed user lookup)

### Requirement: Existing-Sudoer Probe

"Already a sudoer" is defined as "may run an arbitrary command as root," determined authoritatively by `LANG=C /usr/bin/sudo -n -l -U <user> /bin/sh` exiting 0. Group membership in `sudo`/`wheel`/`admin` MAY be used only as a fast pre-check; it MUST NOT itself grant admission, and sudo's denial text MUST NOT be parsed.

#### Scenario: Probe exits 0 grants admission
- GIVEN the target user's probe exits 0
- WHEN admission runs
- THEN the user is treated as an existing sudoer
- Testable via: `cargo test` (exact argv asserted via `CommandRunner`)

#### Scenario: Probe exits non-zero rejects even a group-flagged user
- GIVEN the target user belongs to `sudo` but the command probe exits non-zero
- WHEN `enable` runs
- THEN the helper exits 12 and no rule file is created
- Testable via: `cargo test`

#### Scenario: Probe invocation discipline
- GIVEN the probe is executed
- WHEN the argv is inspected
- THEN it uses the absolute path `/usr/bin/sudo`, `LANG=C`, and no shell or `PATH` lookup
- Testable via: `cargo test`

### Requirement: Single Polkit Action

The system MUST define exactly one polkit action, `com.enfoquestic.nopass.manage`, with `allow_any=no`, `allow_inactive=no`, `allow_active=auth_admin_keep`, annotated with `org.freedesktop.policykit.exec.path` set to the fixed helper path. No subcommand, including `status`, uses a separate or unprivileged polkit action.

#### Scenario: Installed policy declares the single action with required defaults
- GIVEN `data/com.enfoquestic.nopass.policy`
- WHEN the file is parsed
- THEN exactly one `<action id="com.enfoquestic.nopass.manage">` exists with the specified `allow_*` defaults and `exec.path` annotation
- Testable via: `cargo test` (XML fixture parse of the static data file)

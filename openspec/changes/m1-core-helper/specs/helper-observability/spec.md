# Helper Observability Specification

## Purpose

Defines the shared `HelperStatus` JSON contract, the world-readable state file that lets an unprivileged tray process observe helper state despite `/etc/sudoers.d/` being `0750`, and the journald audit trail. (PRD RF-07, RF-11)

## Requirements

### Requirement: HelperStatus JSON Contract

The system MUST expose one `HelperStatus` struct, serialized identically via `serde_json` for the `status` subcommand's stdout and for `/run/nopass/<uid>.state`, containing at minimum `schema`, `uid`, `user`, `active`, `expires` (internally tagged: `{"kind":"never"}`, `{"kind":"reboot"}`, `{"kind":"at","epoch":<u64>}`, or `null` when inactive), `rule_path`, and `updated_at`.

#### Scenario: Active temporary grant serializes with an `at` expiry
- GIVEN an active grant expiring at epoch 1789000000
- WHEN `HelperStatus` is serialized
- THEN `expires` is `{"kind":"at","epoch":1789000000}` and `active` is `true`
- Testable via: `cargo test`

#### Scenario: Inactive uid serializes with a null expiry
- GIVEN a uid with no active grant
- WHEN `HelperStatus` is serialized
- THEN `active` is `false` and `expires` is `null`
- Testable via: `cargo test`

#### Scenario: stdout and state-file content share the same shape
- GIVEN the same underlying status
- WHEN it is serialized for `status` stdout and for the state file
- THEN both use the identical serializer and produce equivalent JSON
- Testable via: `cargo test`

### Requirement: State File Placement and Permissions

`/run/nopass/<uid>.state` MUST be written as mode `0644`, inside `/run/nopass/` (mode `0755`), and MUST be updated on every `enable`, `disable`, `expire`, and boot-time cleanup sweep. If `/run/nopass/` does not yet exist when the helper runs, the helper MUST create it (mode `0755`) itself before writing the state file rather than skip the write.

#### Scenario: Enable writes the state file with correct mode
- GIVEN a successful `enable`
- WHEN it completes
- THEN `/run/nopass/<uid>.state` exists with mode `0644` and reflects the new active grant
- Testable via: root-only container test

#### Scenario: `/run/nopass/` missing at helper start
- GIVEN `/run/nopass/` does not exist (tmpfiles.d not yet applied)
- WHEN `enable` runs
- THEN the helper creates the directory at `0755` and still writes the state file; the sudoers rule outcome is unaffected
- Testable via: root-only container test

### Requirement: Journald Audit Records

Every `enable`, `disable`, and `expire` outcome MUST be logged to journald with `SYSLOG_IDENTIFIER=nopass-helper`, including uid, outcome, and resulting expiry, such that `journalctl -t nopass-helper` shows the record.

#### Scenario: Successful enable is journaled
- GIVEN a successful `enable`
- WHEN it completes
- THEN one journald record with `SYSLOG_IDENTIFIER=nopass-helper`, the uid, and the resulting expiry is emitted
- Testable via: root-only container test (journald present); `cargo test` asserts the tracing-journald layer is configured with `with_syslog_identifier("nopass-helper")`

#### Scenario: A rejected operation is still journaled
- GIVEN `enable` is rejected with exit 12 (not a sudoer)
- WHEN the rejection occurs
- THEN a journald record distinguishes the rejection outcome from a success record
- Testable via: root-only container test

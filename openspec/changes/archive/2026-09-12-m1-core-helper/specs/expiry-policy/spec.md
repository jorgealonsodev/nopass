# Expiry Policy Specification

## Purpose

Defines the `Expiry` model, duration validation, transient timer management, and boot-time cleanup that bound the exposure window of every temporary grant, including robustness across suspend/resume and reboot. (PRD RF-06, §7.5, NFR robustness rows)

## Requirements

### Requirement: Expiry Model and Header Encoding

The system MUST represent expiry as exactly one of `Never`, `Reboot`, or `At(<UTC epoch seconds>)`, encoded in the `nopass-expires` header as exactly `never`, `reboot`, or an unsigned epoch integer. No other value is valid.

#### Scenario: Each expiry form round-trips
- GIVEN each of `Never`, `Reboot`, `At(1789000000)`
- WHEN rendered then parsed back
- THEN the parsed value equals the original
- Testable via: `cargo test`

#### Scenario: Unrecognized header value is rejected
- GIVEN a header line `nopass-expires: soon`
- WHEN it is parsed
- THEN parsing returns a typed `ParseError`, not a silent default
- Testable via: `cargo test`

### Requirement: Temporary Duration Validation

`enable --until <epoch>` MUST be accepted only when the resulting duration from now falls within `[60, 28800]` seconds inclusive; values outside this range MUST be rejected with exit 13 and no system change. `enable` with no duration flag MUST default to `Never` (permanent mode).

#### Scenario: In-range duration is accepted
- GIVEN `--until` is 3600 seconds from now
- WHEN `enable` runs
- THEN the duration is accepted and `Expiry::At` is set
- Testable via: `cargo test`

#### Scenario: Duration below the minimum is rejected
- GIVEN `--until` is 30 seconds from now
- WHEN `enable` runs
- THEN the helper exits 13 and writes nothing
- Testable via: `cargo test`

#### Scenario: Duration above the maximum is rejected
- GIVEN `--until` is 30000 seconds from now
- WHEN `enable` runs
- THEN the helper exits 13 and writes nothing
- Testable via: `cargo test`

#### Scenario: `--until` in the past is rejected
- GIVEN `--until` is earlier than the current time
- WHEN `enable` runs
- THEN the helper exits 13
- Testable via: `cargo test`

#### Scenario: No duration flag defaults to permanent
- GIVEN `enable` is invoked with neither `--until` nor `--until-reboot`
- WHEN it runs
- THEN `Expiry::Never` is applied and no timer is created
- Testable via: `cargo test`

### Requirement: Transient Timer Replacement

Before creating a new expiry timer for a uid, the system MUST stop any existing `nopass-expire-<uid>.timer` (tolerating a "not loaded" error), then invoke `systemd-run --unit=nopass-expire-<uid> --on-calendar=<UTC epoch as RFC-3339 with explicit Z> /usr/libexec/nopass-helper expire --uid <uid>`. No timer is created for `Never` or `Reboot` expiry. If timer scheduling fails, the just-written rule file MUST be rolled back (unlinked) and the helper MUST exit 17.

#### Scenario: First temporary enable creates the timer
- GIVEN no existing timer for uid 1000
- WHEN `enable --until <epoch>` runs
- THEN `systemctl stop nopass-expire-1000.timer` runs first (tolerating "not loaded"), then `systemd-run` runs with the exact UTC `Z`-suffixed calendar argument
- Testable via: `cargo test` (`CommandRunner` argv assertion)

#### Scenario: Re-enabling replaces an existing timer
- GIVEN a timer for uid 1000 already exists
- WHEN `enable --until <epoch>` runs again
- THEN the old timer is stopped before the new one is created
- Testable via: `cargo test`

#### Scenario: Timer scheduling failure rolls back the rule
- GIVEN the rule file was just written successfully
- WHEN `systemd-run` fails
- THEN the rule file is unlinked, the helper exits 17, and no permanent grant is left behind
- Testable via: `cargo test` (rollback call sequence) and root-only container test (real unlink)

#### Scenario: Permanent and until-reboot enables schedule no timer but still clear a stale one
- GIVEN `enable` with no flags or with `--until-reboot`
- WHEN it runs
- THEN `systemd-run` is never invoked, so no timer is scheduled
- AND `systemctl stop <unit>.timer` IS invoked first, unconditionally and tolerant of any exit status, so a stale timer left by a previous `At` activation cannot outlive the new permanent or until-reboot grant (design.md 4.1 step 14)
- Testable via: `cargo test`

### Requirement: Expiry Re-validation Before Deletion

`expire --uid <n>` MUST re-read the rule file's `nopass-expires` header and delete the rule only if it is an epoch already in the past; it MUST NOT delete a rule whose header is `never`, `reboot`, or a still-future epoch.

#### Scenario: Expire deletes a genuinely past-epoch rule
- GIVEN the header epoch is in the past
- WHEN `expire --uid 1000` runs
- THEN the rule is deleted, the state file is updated, and the action is journaled
- Testable via: root-only container test

#### Scenario: Stale timer fires after a newer enable
- GIVEN uid 1000 was re-enabled with a later epoch after an earlier timer was scheduled, and the stale timer still fires
- WHEN `expire --uid 1000` runs
- THEN the header shows the newer, still-future epoch, and the rule is left intact
- Testable via: `cargo test` (header stub) and root-only container test

#### Scenario: Expire on a uid with no rule file
- GIVEN no rule file exists for uid 1000
- WHEN `expire --uid 1000` runs
- THEN the helper treats it as already-expired and exits 0
- Testable via: `cargo test`

### Requirement: Boot-Time Cleanup Sweep

`expire --boot` MUST additionally treat `reboot`-marked rules as expired and, when invoked without `--uid`, MUST sweep every `90-nopass-*` rule file, deleting each whose header indicates `reboot` or a past epoch, and leaving `never` or future-epoch rules untouched. At least one of `--uid` or `--boot` MUST be present on `expire`.

#### Scenario: Boot sweep removes only stale and reboot-marked rules
- GIVEN two rule files marked `reboot`/past-epoch and one marked `never`
- WHEN `expire --boot` runs without `--uid`
- THEN only the two stale rules are removed; the permanent rule remains
- Testable via: root-only container test

#### Scenario: Expire with neither flag is rejected
- GIVEN `expire` is invoked with neither `--uid` nor `--boot`
- WHEN it is parsed
- THEN the helper exits 2 and makes no system change
- Testable via: `cargo test`

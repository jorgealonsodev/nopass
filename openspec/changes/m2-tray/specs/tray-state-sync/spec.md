# Tray State Sync Specification

## Purpose

Defines the two-source state model — the `/run/nopass/<uid>.state` file plus a live
`sudo -kn true` probe — the `Unknown` state and its "reconcile, never assume inactive" rule,
inotify wiring, the four reconciliation triggers, precedence between the two sources, and the
idle-cost budget. This is the tray-side enforcement of the M1 contract at
`openspec/specs/helper-observability/spec.md`. (PRD RF-07; NFR idle CPU ≈ 0 %)

## Requirements

### Requirement: Absence or Schema Mismatch Produce Unknown, Never Inactive

The state reader's merged state type MUST have exactly three inhabitants —
`Active { expiry }`, `Inactive`, `Unknown` — and MUST have no code path that produces
`Inactive` from a missing `/run/nopass/<uid>.state` file or from a `schema` field other than
`1`. Both conditions MUST produce `Unknown`. A reader whose return type can express
"inactive because the file was absent" violates this requirement regardless of what its
callers do with the value.

#### Scenario: Missing state file never reads as inactive
- GIVEN `/run/nopass/<uid>.state` does not exist and the sudoers rule for that uid is in fact
  active
- WHEN the tray reads state
- THEN the file-derived value is `Unknown`, never `Inactive`, and the merged state resolves to
  `Active` only once the live probe confirms it
- Testable via: `cargo test` (temp directory, no real state file)

#### Scenario: Schema mismatch is treated identically to absence
- GIVEN a state file with `"schema": 2`
- WHEN the tray parses it
- THEN the parsed value is `Unknown`, exactly as if the file were absent
- Testable via: `cargo test`

### Requirement: Live Probe Takes Precedence Over the Cached File

When the file-derived state and the `sudo -kn true` probe result disagree, the probe result
MUST win, and the tray MUST update its displayed state to match the probe.

#### Scenario: Probe overrides a stale "active" file
- GIVEN the state file reports `active: true` but `sudo -kn true` fails
- WHEN reconciliation runs
- THEN the merged state becomes `Inactive` and the icon updates
- Testable via: `cargo test` (fake `CommandRunner`-shaped probe port, no real sudo)

#### Scenario: Probe resolves an Unknown state
- GIVEN the file-derived state is `Unknown` (absent or schema mismatch) and `sudo -kn true`
  succeeds
- WHEN reconciliation runs
- THEN the merged state becomes `Active`
- Testable via: `cargo test`

### Requirement: Reconciliation Runs at Four Defined Triggers

The tray MUST run the `sudo -kn true` probe at startup, immediately after a completed
enable/disable action, on a 60-second timer, and on a menu open WHOSE CACHED READING IS STALE —
and at no other periodic interval. A menu open with a fresh cache MUST NOT spawn a probe: each
one costs a process, and the reading it would produce is the one already held.

#### Scenario: Each trigger invokes the probe exactly once
- GIVEN the tray in a running state
- WHEN startup, a completed action, a menu open, and a 60 s tick each occur once
- THEN the probe port records exactly four invocations, one per trigger
- Testable via: `cargo test` (fake probe port with an invocation counter)

### Requirement: Inotify Watch With Missing-Directory Fallback

The tray MUST watch `/run/nopass/` via inotify and react to a write within 1 second. If
`/run/nopass/` does not exist when the tray starts or the watch is lost, the tray MUST fall
back to reconciliation-only operation, show a warning, and retry establishing the watch on
each 60 s tick.

#### Scenario: A state-file write is observed within budget
- GIVEN an established inotify watch on `/run/nopass/`
- WHEN the state file is rewritten
- THEN the tray reacts within 1 s
- Testable via: `cargo test` (real inotify against a temporary directory, no bus needed)

#### Scenario: Missing run directory falls back to reconciliation only
- GIVEN `/run/nopass/` does not exist at startup
- WHEN the tray starts
- THEN it shows a warning, relies solely on the 60 s reconciliation probe, and retries the
  watch on each tick
- Testable via: `cargo test`

### Requirement: No Periodic Wakeup Beyond the 60-Second Reconciliation Tick

The tray process MUST register no periodic timer other than the 60 s reconciliation tick.
Inotify events and D-Bus calls are edge-triggered and MUST NOT be implemented via polling.

#### Scenario: Only one periodic wakeup source exists
- GIVEN the tray is running idle
- WHEN its reactor's registered timers are enumerated
- THEN exactly one periodic timer exists, firing every 60 s
- Testable via: `cargo test` (instrumented reactor) for the structural claim; real desktop
  session for the measured idle RSS/CPU over a continuous one-hour run

# Tray Presence Specification

## Purpose

Defines StatusNotifierItem registration, the three visual states and their icon names,
tooltip content and refresh points, symbolic variants, the startup visibility budget, and
behaviour when no SNI host or no session bus is present. (PRD RF-01, RF-10; NFR: icon
visible < 1 s, idle CPU ≈ 0 %)

## Requirements

### Requirement: StatusNotifierItem Registration and Startup Visibility

The tray MUST register as a StatusNotifierItem on the session bus at startup, and the icon
MUST become visible to a present SNI host within 1 second of process start.

#### Scenario: Icon registers and becomes visible within budget
- GIVEN a session bus with a StatusNotifierWatcher owner present
- WHEN the tray starts
- THEN SNI registration completes and the item is visible within 1 s
- Testable via: real desktop session (human-visible rendering); registration timing is
  additionally provable via dbus-run-session against a fake watcher

### Requirement: Three Visual States With Distinct Icon Names

The tray MUST render exactly three visual states. Each state carries two icon names, a plain
and a `-symbolic` variant, so six names ship in total and the count of names is not the count
of states. The tray MUST switch icon on every state transition:

| State | Icon name | Symbolic |
|---|---|---|
| Inactive | `nopass-locked` | `nopass-locked-symbolic` |
| Active | `nopass-unlocked` | `nopass-unlocked-symbolic` |
| Active-Temporary | `nopass-unlocked-timed` | `nopass-unlocked-timed-symbolic` |

#### Scenario: Icon name follows the merged state exactly
- GIVEN the merged state is `Active { expiry: At(epoch) }`
- WHEN the tray sets its icon
- THEN it selects `nopass-unlocked-timed`, or its symbolic variant when the host requests one
- Testable via: dbus-run-session (icon name is a property read by a fake StatusNotifierWatcher)

### Requirement: Tooltip Recomputed Only At Existing Wake Points

The tooltip MUST show `<user> — <state> (<remaining>)` for an active-temporary grant, at
minute granularity, and MUST be recomputed only at an inotify event, a completed action, a
menu open, or the 60 s reconciliation tick. The tray MUST NOT run a dedicated countdown
timer to refresh the tooltip (D2).

#### Scenario: Tooltip renders remaining time at minute granularity
- GIVEN an active-temporary grant expiring in 42 minutes
- WHEN the tooltip is read
- THEN the remaining-time element renders exactly `42 min`, and `less than a minute` below 60 s
- AND the string is English. M2 ships one language; localisation is M3, and a Spanish literal here
  would make the spec disagree with every shipped build until then
- Testable via: `cargo test` (pure formatting function, no bus)

#### Scenario: No timer exists solely to refresh the tooltip
- GIVEN the tray is idle between wake points
- WHEN its reactor's wakeup sources are enumerated
- THEN the only periodic wakeup is the 60 s reconciliation tick; no separate countdown timer
  is registered
- Testable via: `cargo test` (instrumented reactor asserts registered wake sources) for the
  structural claim; real desktop session for the measured idle CPU

### Requirement: Degraded Start When No SNI Host Is Present

If `NameHasOwner("org.kde.StatusNotifierWatcher")` is false at startup, the tray MUST still
claim its D-Bus name, MUST emit one notification explaining that no tray host was found and
how to enable one, and MUST subscribe to `NameOwnerChanged` to complete SNI registration the
moment a host appears. It MUST NOT refuse to start over this condition alone.

#### Scenario: Host absent at startup, tray still runs
- GIVEN no StatusNotifierWatcher owner on the session bus
- WHEN the tray starts
- THEN it claims its D-Bus name, emits the degraded notification, and does not exit
- Testable via: dbus-run-session (fake bus, no watcher registered)

#### Scenario: Host appears later and the item registers without a restart
- GIVEN the tray started degraded per the scenario above
- WHEN `NameOwnerChanged` reports a new StatusNotifierWatcher owner
- THEN the tray completes SNI registration in the same process
- Testable via: dbus-run-session

### Requirement: Hard Refusal With No User-Visible Channel

The tray MUST print to stderr and exit non-zero, with a code distinct from RF-10's exit 0, when
there is no session bus reachable at all, or when neither an SNI host nor a notification service
is reachable. These are the only refusals decided by the PREFLIGHT. They are not the only
non-zero exits: `tray-single-instance` requires exit 5 for a name-claim failure that is not
"already taken", which happens before the preflight runs. The earlier wording said "only when"
and contradicted that requirement.

#### Scenario: No session bus at all
- GIVEN no session bus is reachable (e.g. `DBUS_SESSION_BUS_ADDRESS` unset and unresolvable)
- WHEN the tray starts
- THEN it prints to stderr and exits non-zero
- Testable via: lane B, `scripts/run-lane-b.sh`. NOT `cargo test`: removing the bus environment
  does not deny a bus, because zbus derives `/run/user/<uid>/bus` from `getuid()` and reaches the
  host's real session. Denying one needs an address that resolves to nothing, which is a
  controlled-environment concern

#### Scenario: Neither host nor notification service reachable
- GIVEN a session bus with no StatusNotifierWatcher owner and no `org.freedesktop.Notifications`
  owner
- WHEN the tray starts
- THEN it prints to stderr and exits non-zero — distinct from the SNI-host-only degraded case,
  which does not exit
- Testable via: dbus-run-session (fake bus with neither service registered)

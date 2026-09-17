# Tray Menu Specification

## Purpose

Defines the full RF-03 context-menu item tree, the duration submenus, the default-duration
marker, "Current rule" as insensitive reference items, the autostart checkbox, and keyboard
reachability of every item. This is the M3 replacement of M2's three-item minimal menu. (PRD
RF-03, RF-09; NFR accessibility row)

## Requirements

### Requirement: Complete RF-03 Item Tree

The right-click menu MUST render, top to bottom: the state-dependent toggle item, an "Activate
during…" submenu, a "Default duration" submenu, a "Current rule" submenu, a "Start with session"
checkbox, "About", and "Quit". Every item and submenu entry MUST be reachable via the standard
DBusMenu/SNI keyboard-navigation properties the host exposes — no item may depend on a mouse-only
gesture to reach or activate it.

#### Scenario: The full item tree is present in the exported menu
- GIVEN the tray builds its menu for any merged state other than `Unknown`
- WHEN the exported menu tree is inspected
- THEN it contains, in order, the toggle item, "Activate during…", "Default duration", "Current
  rule", "Start with session", "About", and "Quit"
- Testable via: `cargo test` (pure menu-tree builder, no bus)

#### Scenario: Every item exposes standard keyboard-navigation properties
- GIVEN the exported menu tree
- WHEN each item's DBusMenu properties are inspected
- THEN none disables keyboard focus or requires a pointer-only activation method
- Testable via: real desktop session, keyboard-only navigation through every item and submenu

### Requirement: Duration Submenus Render All Six Options In Fixed Order

"Activate during…" and "Default duration" MUST each render exactly the six durations — 15 min,
1 h, 4 h, 8 h, until reboot, permanent — in that fixed order, and MUST render no other duration.

#### Scenario: Both submenus render the same six items in the same order
- GIVEN the tray builds "Activate during…" and "Default duration"
- WHEN their items are enumerated
- THEN both list exactly 15 min, 1 h, 4 h, 8 h, until reboot, permanent, in that order
- Testable via: `cargo test`

### Requirement: Default-Duration Marker Reflects Configured State

The "Default duration" submenu MUST mark exactly the one entry matching `user-config`'s current
default duration. Selecting a different entry MUST update the marker and persist the new default
via `user-config`.

#### Scenario: The marker follows the configured default
- GIVEN the configured default duration is 4 h
- WHEN "Default duration" is built
- THEN exactly the "4 h" entry is marked, and no other entry is marked
- Testable via: `cargo test`

#### Scenario: Selecting a new default moves the marker and persists it
- GIVEN the configured default is 1 h
- WHEN the user selects "8 h" from "Default duration"
- THEN the next menu build marks "8 h" instead of "1 h", and the persisted config's default
  duration is 8 h
- Testable via: `cargo test` (temp `XDG_CONFIG_HOME`)

### Requirement: Current Rule Renders As Insensitive Reference Items

"Current rule" MUST render as a submenu of insensitive (non-activatable) items sourced from
`status`: rule path, granted user, expiry, and remaining time when active; a single insensitive
"No active rule" item when inactive. It MUST NOT be rendered as a desktop notification: a
notification body is transient, coalesced or dropped by several daemons, and cannot be re-read
once dismissed, whereas this is structured reference data the user opens deliberately.

#### Scenario: An active grant's submenu lists path, user, expiry, and remaining time
- GIVEN an active temporary grant
- WHEN "Current rule" is built
- THEN it contains insensitive items for the rule path, the granted user, the expiry, and the
  remaining time, and none of them are activatable
- Testable via: `cargo test`

#### Scenario: An inactive state renders a single insensitive placeholder
- GIVEN no active grant
- WHEN "Current rule" is built
- THEN it contains exactly one insensitive "No active rule" item
- Testable via: `cargo test`

### Requirement: Start-With-Session Checkbox Toggles the Autostart Entry

Selecting "Start with session" MUST invoke `autostart-entry`'s create/remove behavior; its
checked state at each menu build MUST come from `autostart-entry`'s on-disk read, never from a
value cached across menu builds.

#### Scenario: Checking the box creates the autostart entry
- GIVEN "Start with session" is unchecked and the autostart file does not exist
- WHEN the user checks it
- THEN `autostart-entry`'s create path runs and the next menu build shows it checked
- Testable via: `cargo test` (temp `XDG_CONFIG_HOME`)

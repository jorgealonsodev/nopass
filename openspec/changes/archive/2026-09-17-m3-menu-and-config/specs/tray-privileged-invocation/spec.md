# Delta for Tray Privileged Invocation

## MODIFIED Requirements

### Requirement: pkexec Invocation for Enable and Disable

The tray MUST invoke `enable`/`disable` only via
`pkexec /usr/libexec/nopass-helper <enable|disable> [--until <epoch>|--until-reboot]`, using
an absolute path and no shell wrapping, and MUST NOT invoke `expire` (root/timer-only). The
duration argv MUST be driven by the selected duration: 15 min / 1 h / 4 h / 8 h render
`--until <epoch>`, until-reboot renders `--until-reboot`, and permanent renders NEITHER flag.
The click/menu default duration MUST come from `user-config`'s configured default, never a
hardcoded value.
(Previously: pinned a hardcoded 1-hour default with a single scenario asserting `--until
<epoch>` only; no permanent or until-reboot argv scenario existed.)

#### Scenario: Enable invocation uses the exact documented argv for a timed duration
- GIVEN the tray toggles from inactive with a configured default duration of 1 hour
- WHEN it invokes the helper
- THEN the captured argv is exactly `pkexec /usr/libexec/nopass-helper enable --until <epoch>`
  with no shell interpolation
- Testable via: `cargo test` (fake spawn port captures argv, no real pkexec)

#### Scenario: Permanent duration omits both duration flags
- GIVEN the user selects "permanent" from either the click default or the "Activate during…"
  submenu
- WHEN the tray invokes the helper
- THEN the captured argv is exactly `pkexec /usr/libexec/nopass-helper enable` with neither
  `--until` nor `--until-reboot` present
- Testable via: `cargo test`

#### Scenario: Until-reboot duration uses the exact flag
- GIVEN the user selects "until reboot"
- WHEN the tray invokes the helper
- THEN the captured argv includes `--until-reboot` and no `--until <epoch>`
- Testable via: `cargo test`

#### Scenario: All six durations render their documented argv with no collision
- GIVEN each of the six durations (15 min, 1 h, 4 h, 8 h, until reboot, permanent) in turn
- WHEN the tray invokes the helper for each
- THEN each produces its documented argv shape, and no two distinct durations collapse to the
  same argv
- Testable via: `cargo test` (table-driven)

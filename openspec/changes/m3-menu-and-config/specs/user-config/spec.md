# User Config Specification

## Purpose

Defines `~/.config/nopass/config.toml`'s location, schema, defaults, tolerant parsing, and
atomic write — the persisted default duration and the "don't warn again" consent flag consumed
by `tray-menu` and `activation-consent`. (PRD RF-02, RF-03, RF-06)

## Requirements

### Requirement: Config Path Honors XDG_CONFIG_HOME

The system MUST resolve the config path as `$XDG_CONFIG_HOME/nopass/config.toml` when
`XDG_CONFIG_HOME` is set and non-empty, and `~/.config/nopass/config.toml` otherwise.

| `XDG_CONFIG_HOME` | Resolved path |
|---|---|
| unset or empty | `~/.config/nopass/config.toml` |
| `/custom/path` | `/custom/path/nopass/config.toml` |

#### Scenario: Default path is used when XDG_CONFIG_HOME is unset
- GIVEN `XDG_CONFIG_HOME` is unset
- WHEN the config path is resolved
- THEN it resolves to `~/.config/nopass/config.toml`
- Testable via: `cargo test`

#### Scenario: XDG_CONFIG_HOME overrides the default path
- GIVEN `XDG_CONFIG_HOME=/custom/path`
- WHEN the config path is resolved
- THEN it resolves to `/custom/path/nopass/config.toml`
- Testable via: `cargo test`

### Requirement: Schema and Defaults

The config MUST have exactly two fields: `default_duration` (one of `15m`, `1h`, `4h`, `8h`,
`reboot`, `permanent`; default `1h`) and `warn_before_activation` (boolean; default `true`). A
config file that does not exist MUST resolve to these defaults without being created.

#### Scenario: A missing file resolves to schema defaults
- GIVEN no config file exists at the resolved path
- WHEN the config is loaded
- THEN `default_duration` is `1h` and `warn_before_activation` is `true`
- Testable via: `cargo test` (temp directory, no file)

### Requirement: Tolerant Parsing Never Blocks Startup

Malformed TOML or an unrecognized field/value MUST degrade to the schema defaults plus a
logged/observable warning; it MUST NOT cause the tray to refuse to start.

#### Scenario: Malformed TOML degrades to defaults with a warning
- GIVEN the config file contains invalid TOML
- WHEN the tray loads config at startup
- THEN loading returns the schema defaults, an observable warning is produced, and startup
  proceeds
- Testable via: `cargo test`

#### Scenario: An unrecognized default_duration value degrades to the default
- GIVEN `default_duration = "3h"` (not one of the six documented values)
- WHEN the config is loaded
- THEN `default_duration` resolves to `1h` and a warning is produced
- Testable via: `cargo test`

### Requirement: Read At Startup, Written On Menu Selection

The config MUST be read exactly once at startup and written whenever the user selects a new
default duration or confirms "don't warn again"; no other event MUST write it.

#### Scenario: Selecting a new default duration writes the file
- GIVEN the tray is running with `default_duration = 1h`
- WHEN the user selects "8 h" from the "Default duration" submenu
- THEN the config file is rewritten with `default_duration = 8h`
- Testable via: `cargo test` (temp `XDG_CONFIG_HOME`)

### Requirement: Atomic Write

Every config write MUST use a temp-file-then-rename sequence in the same directory as the target
path; a write that fails after the temp file is created MUST leave the previous config file (or
its absence) untouched.

#### Scenario: A successful write replaces the file atomically
- GIVEN an existing config file
- WHEN a new value is written
- THEN a temp file is created in the same directory, written, and renamed over the target path
- Testable via: `cargo test`

#### Scenario: A failed write leaves the prior config intact
- GIVEN an existing config file with `default_duration = 1h`
- WHEN a write attempt fails after creating the temp file (e.g. the rename fails)
- THEN the original file still reads `default_duration = 1h`, and the temp file is not left as
  the effective config
- Testable via: `cargo test` (stubbed filesystem port)

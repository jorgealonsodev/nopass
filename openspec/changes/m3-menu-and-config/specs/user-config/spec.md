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

A config file MUST degrade in one of two ways, and the two are not the same:

- A file that cannot be read, is not valid UTF-8, or does not parse as TOML syntax at all
  (a faulted reading) MUST degrade to the schema defaults plus a logged/observable warning.
- A file that IS syntactically valid TOML but carries an unrecognized value for a known field
  (e.g. a `default_duration` outside the six documented values) or an unknown key MUST degrade
  that one field to its schema default, on its own, without discarding any other valid field
  from the same document, and MUST NOT raise a fault or a warning: a syntactically valid
  document with one bad field is ordinary per-field tolerance, not an error condition worth
  interrupting the user for on every menu open.

Neither case MUST cause the tray to refuse to start.

#### Scenario: Malformed TOML degrades to defaults with a warning
- GIVEN the config file contains invalid TOML (or is not valid UTF-8, or cannot be read)
- WHEN the tray loads config
- THEN loading returns the schema defaults, an observable warning is produced, and startup
  proceeds
- Testable via: `cargo test`

#### Scenario: An unrecognized default_duration value falls back silently
- GIVEN `default_duration = "3h"` (not one of the six documented values), in an otherwise
  syntactically valid document that also sets `warn_before_activation`
- WHEN the config is loaded
- THEN `default_duration` resolves to `1h`, the sibling `warn_before_activation` value from the
  same document is preserved, and no fault or warning is raised
- Testable via: `cargo test`

### Requirement: Read At Startup And At Every Menu Open, Written On Menu Selection

The config MUST be read at startup and re-read from disk at every menu open, so a user who
edits `config.toml` while the tray is running does not need to restart it; it MUST be written
whenever the user selects a new default duration or confirms "don't warn again", and no other
event MUST write it.

#### Scenario: Selecting a new default duration writes the file
- GIVEN the tray is running with `default_duration = 1h`
- WHEN the user selects "8 h" from the "Default duration" submenu
- THEN the config file is rewritten with `default_duration = 8h`
- Testable via: `cargo test` (temp `XDG_CONFIG_HOME`)

#### Scenario: Editing the file while the tray is running is picked up on the next menu open
- GIVEN the tray is running with `default_duration = 1h`, read at startup
- WHEN the user hand-edits `config.toml` to `default_duration = 8h` and then opens the menu
- THEN the menu reflects `default_duration = 8h`, without restarting the tray
- Testable via: `cargo test` (`app.rs::menu_opened_re_reads_config_from_disk`)

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

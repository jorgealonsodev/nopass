# Autostart Entry Specification

## Purpose

Defines creation/removal of `~/.config/autostart/nopass.desktop` from the `data/nopass.desktop`
template, and the rule that the menu checkbox reflects on-disk truth rather than cached intent.
Packaging ships no autostart entry by default; the user opts in from the menu. (PRD RF-09)

## Requirements

### Requirement: Autostart Path Honors XDG_CONFIG_HOME

The system MUST resolve the autostart entry path as
`$XDG_CONFIG_HOME/autostart/nopass.desktop` when `XDG_CONFIG_HOME` is set and non-empty, and
`~/.config/autostart/nopass.desktop` otherwise, per the XDG autostart specification.

#### Scenario: Default path is used when XDG_CONFIG_HOME is unset
- GIVEN `XDG_CONFIG_HOME` is unset
- WHEN the autostart path is resolved
- THEN it resolves to `~/.config/autostart/nopass.desktop`
- Testable via: `cargo test`

### Requirement: Create Writes the Template Verbatim

The system MUST create `~/.config/autostart/` (or its `XDG_CONFIG_HOME` equivalent) if absent,
then write `data/nopass.desktop`'s content unmodified to `nopass.desktop` inside it.

#### Scenario: Creating the entry from a missing autostart directory
- GIVEN `~/.config/autostart/` does not exist
- WHEN the user checks "Start with session"
- THEN the directory is created, and `nopass.desktop` is written with content identical to
  `data/nopass.desktop`
- Testable via: `cargo test` (temp `XDG_CONFIG_HOME`)

### Requirement: Remove Deletes Only the NoPass Entry

Unchecking "Start with session" MUST delete `nopass.desktop` at the resolved path; it MUST NOT
touch any other file under `autostart/`. Removal MUST succeed even if the file was already
absent, treating absence as already-disabled.

#### Scenario: Unchecking removes the entry without touching siblings
- GIVEN `autostart/nopass.desktop` and `autostart/other-app.desktop` both exist
- WHEN the user unchecks "Start with session"
- THEN `nopass.desktop` is deleted and `other-app.desktop` is untouched
- Testable via: `cargo test`

#### Scenario: Removing an already-absent entry does not error
- GIVEN `autostart/nopass.desktop` does not exist
- WHEN removal runs
- THEN it completes without error, treating the state as already-disabled
- Testable via: `cargo test`

### Requirement: Checkbox State Reflects On-Disk Truth

The menu's "Start with session" checked state MUST be computed by checking for the entry's
existence at each menu build; it MUST NOT be derived from a value cached from a prior toggle.

#### Scenario: An externally deleted entry is reflected on the next menu build
- GIVEN the entry was created via the checkbox, then deleted from a terminal
- WHEN the menu is rebuilt
- THEN the checkbox renders unchecked
- Testable via: `cargo test`

### Requirement: Packaging Ships No Autostart Entry By Default

The installed package MUST NOT place a pre-existing `nopass.desktop` under any user's or system
`autostart/` directory; the entry is created only by explicit user action from the menu.

#### Scenario: A fresh install has no autostart entry
- GIVEN a fresh package install with no prior menu interaction
- WHEN `~/.config/autostart/nopass.desktop` is checked
- THEN it does not exist
- Testable via: `cargo test` (asserts the packaging manifest installs no file under
  `autostart/`); real logout/login on a freshly installed package for Lane C

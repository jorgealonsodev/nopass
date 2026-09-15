# Activation Consent Specification

## Purpose

Defines RF-02's first-activation warning as a two-step confirmation inside the menu, its
"don't warn again" persistence, and the invariant that no privileged grant can ever precede it —
regardless of which caller requested activation. This is the highest-risk requirement in M3: a
requirement enumerating menu items is satisfiable while a bypass remains open, so the invariant
is written at the single point where a grant is dispatched, not per menu item. (PRD RF-02;
proposal D1)

## Requirements

### Requirement: No Grant Dispatch Without Recorded Consent

The system MUST NOT dispatch an enable action while activation consent is unrecorded, regardless
of which caller requested activation: the menu's toggle item, an SNI left-click or keyboard
activation, or a received single-instance activation nudge. This check MUST be enforced at the
one point in the system where an enable dispatch is constructed and sent to the helper, not
duplicated separately per caller — a per-caller check is satisfiable while one caller's path is
missed.

#### Scenario: A menu-triggered activation with no recorded consent dispatches nothing
- GIVEN activation consent is unrecorded (no config file, or config with
  `warn_before_activation: true`)
- WHEN the user selects the toggle item from the menu while inactive
- THEN zero enable invocations reach the helper
- Testable via: `cargo test` (fake spawn port asserts zero invocations)

#### Scenario: A non-menu activation path with no recorded consent dispatches nothing
- GIVEN activation consent is unrecorded
- WHEN an SNI left-click/keyboard activation is received while inactive (the same event a menu
  toggle raises, reached through a different caller)
- THEN zero enable invocations reach the helper, identically to the menu-triggered case
- Testable via: `cargo test` (fake spawn port asserts zero invocations)

#### Scenario: A received single-instance activation nudge never itself dispatches an enable
- GIVEN activation consent is unrecorded
- WHEN the running instance receives an `Activate` nudge from a second launch
- THEN the nudge only re-asserts SNI registration and notifies status; it never independently
  constructs or dispatches an enable action
- Testable via: dbus-run-session

### Requirement: First Activation Branches the Menu Instead of Granting

When activation consent is unrecorded and an activation is requested, the system MUST present a
two-step confirmation reachable from the menu: a branch stating the risk in plain terms (any
process running as the user can become root without a password), an "I understand, activate"
item, a "don't warn again" item, and a cancel path. The grant MUST happen only on the explicit
"I understand, activate" confirmation, never on the first request itself.

#### Scenario: Confirming the branch grants exactly once
- GIVEN the two-step confirmation branch is presented after an unconsented activation request
- WHEN the user selects "I understand, activate"
- THEN exactly one enable invocation reaches the helper
- Testable via: `cargo test`

#### Scenario: Cancelling the branch grants nothing
- GIVEN the two-step confirmation branch is presented
- WHEN the user selects cancel or dismisses the branch
- THEN zero enable invocations reach the helper and no consent state changes
- Testable via: `cargo test`

### Requirement: Don't-Warn-Again Persists Consent

Selecting "don't warn again" MUST persist a recorded-consent flag via `user-config` before or as
part of granting; a later activation with that flag set MUST proceed directly to dispatch without
re-presenting the branch.

#### Scenario: A later activation skips the branch once consent is recorded
- GIVEN a prior activation recorded "don't warn again"
- WHEN the user activates again while inactive
- THEN the enable invocation dispatches directly, with no confirmation branch presented
- Testable via: `cargo test` (temp `XDG_CONFIG_HOME`)

### Requirement: A Failed Consent Write Re-Warns Rather Than Silently Granting

If persisting "don't warn again" fails, the system MUST treat consent as still unrecorded for
every subsequent activation; it MUST NOT grant based on an unpersisted in-memory flag.

#### Scenario: A write failure leaves the next activation still gated
- GIVEN the config write for "don't warn again" fails (e.g. read-only filesystem)
- WHEN the user activates again later
- THEN the confirmation branch is presented again, and no enable invocation dispatches without a
  fresh explicit confirm
- Testable via: `cargo test` (fake config port returning a write error)

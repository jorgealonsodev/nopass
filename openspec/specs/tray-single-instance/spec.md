# Tray Single Instance Specification

## Purpose

Defines D-Bus name ownership of `com.enfoquestic.nopass`, the `NameTaken` exit-0 path, and
the activation nudge sent to an already-running instance. (PRD RF-10; proposal decision D3)

## Requirements

### Requirement: D-Bus Name Claim and NameTaken Exit

The tray MUST request the well-known name `com.enfoquestic.nopass` on the session bus at
startup. If `request_name` fails with `Error::NameTaken`, the process MUST exit 0 and MUST
NOT create a second StatusNotifierItem.

#### Scenario: First instance claims the name
- GIVEN no existing owner of `com.enfoquestic.nopass`
- WHEN the tray starts
- THEN it becomes the name's owner
- Testable via: dbus-run-session

#### Scenario: Second instance exits 0 without a second icon
- GIVEN an instance already owns `com.enfoquestic.nopass`
- WHEN a second instance starts
- THEN `request_name` fails with `NameTaken`, the second process exits 0, and no second SNI
  item is registered
- Testable via: dbus-run-session (two connections on the same private bus)

### Requirement: Activation Nudge on a Second Launch

On `NameTaken`, the second instance MUST call `org.freedesktop.Application.Activate` on the
existing owner with a bounded timeout, then exit 0 regardless of whether that call succeeded,
failed, or timed out. The first instance's `Activate` handler MUST re-assert its SNI
registration and emit one status notification.

#### Scenario: Second instance nudges the first before exiting
- GIVEN a first instance already running
- WHEN a second instance is launched and receives `NameTaken`
- THEN it calls `Activate` on the first instance within the bounded timeout and exits 0
- Testable via: dbus-run-session

#### Scenario: A failed or timed-out nudge still exits 0
- GIVEN the `Activate` call to the first instance fails or times out
- WHEN the second instance handles that outcome
- THEN it still exits 0 and never hangs waiting for the call
- Testable via: dbus-run-session (fake owner that never replies to `Activate`)

#### Scenario: The first instance reacts to a received nudge
- GIVEN the first instance is running
- WHEN it receives an `Activate` call
- THEN it re-asserts its SNI registration and emits one notification stating current status
- Testable via: dbus-run-session

### Requirement: Non-NameTaken request_name Errors Are a Real Fault

A `request_name` failure that is not `Error::NameTaken` MUST be treated as a real startup
fault: the tray MUST print to stderr and exit non-zero, distinct from RF-10's clean exit 0.

#### Scenario: An unrelated bus error is not treated as a second instance
- GIVEN `request_name` fails with an error other than `NameTaken` (e.g. a bus connection
  failure)
- WHEN the tray handles the result
- THEN it prints to stderr and exits non-zero, never the RF-10 exit-0 path
- Testable via: `cargo test` (fake connection port returning a non-`NameTaken` error)

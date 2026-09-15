# Tray Notifications Specification

## Purpose

Defines desktop notifications for success, failure, and expiry via
`org.freedesktop.Notifications`, their content, and degraded behaviour when no notification
service is on the bus. (PRD RF-08)

## Requirements

### Requirement: Success, Failure, and Expiry Notifications

The tray MUST emit one desktop notification for a completed enable/disable action, one for
each distinct failure outcome (per `tray-privileged-invocation`'s exit-code mapping), and one
when the tray detects an externally-triggered expiry (state transitions to inactive without a
tray-initiated action).

#### Scenario: Successful enable produces a confirmation notification
- GIVEN a successful enable completes
- WHEN the tray reacts
- THEN it emits a notification confirming passwordless sudo is now active
- Testable via: dbus-run-session (fake `org.freedesktop.Notifications`, asserts the payload)

#### Scenario: Detected expiry produces its own notification
- GIVEN the merged state transitions from active-temporary to inactive without a pending
  tray-initiated action
- WHEN the tray observes the transition
- THEN it emits a notification stating the grant expired
- Testable via: dbus-run-session

### Requirement: Notification Body Is Distinct Per Outcome

A notification's body MUST match the specific outcome from the exit-code mapping
(`tray-privileged-invocation`). No two distinct outcomes MAY produce an identical notification
body.

#### Scenario: A non-sudoer rejection and a visudo rejection read differently
- GIVEN one failure carries helper exit 12 (not a sudoer) and another carries exit 14 (visudo
  rejected)
- WHEN the tray emits each notification
- THEN the two bodies are different and each names its specific cause
- Testable via: dbus-run-session (captures both payloads and asserts inequality)

### Requirement: Degraded Mode When No Notification Service Is Present

If `org.freedesktop.Notifications` has no owner on the session bus, the tray MUST continue
operating using only the icon, the tooltip, and stderr; it MUST NOT retry the notification
call in a loop, and RF-08 is documented as unmet in that environment.

#### Scenario: No notification service, tray still updates icon and tooltip
- GIVEN no owner for `org.freedesktop.Notifications`
- WHEN an action completes
- THEN the icon and tooltip still reflect the new state and a message is written to stderr,
  with no notification call retried
- Testable via: dbus-run-session (fake bus with the notifications name unregistered)

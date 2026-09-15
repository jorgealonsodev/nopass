# Delta for Privilege Admission

## MODIFIED Requirements

### Requirement: Single Polkit Action

The system MUST define exactly one polkit action, `com.enfoquestic.nopass.manage`, with
`allow_any=no`, `allow_inactive=no`, `allow_active=auth_admin_keep`, annotated with
`org.freedesktop.policykit.exec.path` set to the fixed helper path. No subcommand, including
`status`, uses a separate or unprivileged polkit action. The obligation is not only that our own
substring assertions match the file's text — it is that the real polkit engine accepts and
enumerates the installed policy. Silent XML rejection makes every privileged action fail with a
message indistinguishable from user error, so `data/com.enfoquestic.nopass.policy` MUST be
validated against a real polkit authority, and `probe_polkit_readiness`'s `EnumerateActions`
result MUST be consumed to decide readiness, never discarded. A gate performing this validation
MUST FAIL, not skip, when no real polkit authority is reachable.
(Previously: validated only by our own substring assertions over the file's text; the
`EnumerateActions` result `probe_polkit_readiness` reads was not acted upon.)

#### Scenario: Installed policy declares the single action with required defaults
- GIVEN `data/com.enfoquestic.nopass.policy`
- WHEN the file is parsed
- THEN exactly one `<action id="com.enfoquestic.nopass.manage">` exists with the specified
  `allow_*` defaults and `exec.path` annotation
- Testable via: `cargo test` (XML fixture parse of the static data file)

#### Scenario: The real polkit engine enumerates the installed action
- GIVEN `data/com.enfoquestic.nopass.policy` installed where a real polkit authority can read it
- WHEN `EnumerateActions` is queried against that authority
- THEN `com.enfoquestic.nopass.manage` appears in the result with the documented defaults
- Testable via: `scripts/run-lane-b.sh` (real polkit authority, no privileged escalation)

#### Scenario: A malformed policy file fails enumeration
- GIVEN a deliberately malformed variant of the policy file (invalid XML or a missing required
  element)
- WHEN it is submitted to the real polkit engine
- THEN enumeration fails or omits the action, distinguishing the defect from the valid file
- Testable via: `scripts/run-lane-b.sh`

#### Scenario: probe_polkit_readiness consumes the enumeration result
- GIVEN `EnumerateActions` returns a result that does not include
  `com.enfoquestic.nopass.manage`
- WHEN `probe_polkit_readiness` runs
- THEN readiness reflects that absence rather than assuming readiness regardless of the result
- Testable via: `cargo test` (fake polkit client returning a controlled enumeration)

#### Scenario: Absence of a polkit authority fails the gate, not skips it
- GIVEN no real polkit authority is reachable in the running environment
- WHEN this gate runs
- THEN the gate reports failure, not a silent skip or a false pass
- Testable via: `scripts/run-lane-b.sh`

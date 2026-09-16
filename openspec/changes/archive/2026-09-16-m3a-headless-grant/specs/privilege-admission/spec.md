# Delta for Privilege Admission

## MODIFIED Requirements

### Requirement: UID Resolution by Invocation Context

`enable`, `disable`, and `status` MUST resolve the target UID exclusively
from the `PKEXEC_UID` environment variable; no username, uid, or path
argument exists on those subcommands. `expire`, `grant`, `revoke`, and
`inspect` MUST run only when the process's real uid is 0 AND `PKEXEC_UID`
is unset — the `SystemRoot` invocation context — and MUST take their target
uid from an explicit `--uid` flag rather than from `PKEXEC_UID`. Any other
combination MUST exit 10 with no system change.
(Previously: `expire` was the sole `SystemRoot` consumer.)

#### Scenario: Enable resolves uid from PKEXEC_UID
- GIVEN `PKEXEC_UID=1000` is set
- WHEN `enable` runs
- THEN the target uid resolved is 1000
- Testable via: `cargo test`

#### Scenario: PKEXEC_UID missing on a pkexec-context subcommand
- GIVEN `PKEXEC_UID` is unset
- WHEN `enable`, `disable`, or `status` runs
- THEN the helper exits 10 and makes no write
- Testable via: `cargo test`

#### Scenario: PKEXEC_UID present but unparseable
- GIVEN `PKEXEC_UID=abc`
- WHEN `enable` runs
- THEN the helper exits 10 and makes no write
- Testable via: `cargo test`

#### Scenario: Expire invoked with PKEXEC_UID set
- GIVEN the process runs as uid 0 but `PKEXEC_UID` is also set
- WHEN `expire --uid 1000` runs
- THEN the helper exits 10 and makes no system change
- Testable via: `cargo test`

#### Scenario: Expire invoked as non-root without PKEXEC_UID
- GIVEN the process's real uid is not 0 and `PKEXEC_UID` is unset
- WHEN `expire --uid 1000` runs
- THEN the helper exits 10 and makes no system change
- Testable via: root-only container test (requires a real non-root invocation)

#### Scenario: Grant, revoke, and inspect are each rejected when PKEXEC_UID is set
- GIVEN the process runs as uid 0 but `PKEXEC_UID` is also set
- WHEN `grant --uid 1000`, `revoke --uid 1000`, or `inspect --uid 1000`
  runs, in turn
- THEN each exits 10 and makes no system change
- Testable via: `cargo test`
#### Scenario: Grant, revoke, and inspect are each rejected for a non-root caller with no PKEXEC_UID
- GIVEN the process's real uid is not 0 and `PKEXEC_UID` is unset
- WHEN `grant --uid 1000`, `revoke --uid 1000`, or `inspect --uid 1000`
  runs, in turn
- THEN each exits 10 and makes no system change
- Testable via: `cargo test` (the suite already runs as a non-root process with no
  `PKEXEC_UID`, which is exactly this scenario's precondition)

#### Scenario: Grant resolves the SystemRoot context and the explicit target uid
- GIVEN the process's real uid is 0 and `PKEXEC_UID` is unset
- WHEN `grant --uid 1000` runs
- THEN the resolved context is `SystemRoot` and the resolved target uid is
  1000, taken from `--uid`, not from any environment variable
- Testable via: `cargo test`
## ADDED Requirements

### Requirement: SystemRoot Context Can Target Any Admitted UID

A `SystemRoot`-context `grant`, `revoke`, or `inspect` MAY target any uid
passed via `--uid`, including a uid different from the invoking process's
own identity — a genuine widening over the `Pkexec` context, where
`enable`, `disable`, and `status` remain permanently self-targeted and
carry no uid argument at all. Every `SystemRoot`-context target uid MUST
still pass the same UID Range Admission requirement that already governs
every other admission path (uid 0 rejected, configured range enforced, the
user must exist): this widens WHO may be named as a target, not WHAT bounds
a target uid.

#### Scenario: A root-invoked grant targets a uid other than the caller's own
- GIVEN a process running as real uid 0
- WHEN `grant --uid 1000` runs
- THEN the resolved target uid is 1000, regardless of any identity the
  invoking shell itself has
- Testable via: `cargo test`
#### Scenario: enable stays self-targeted with no uid argument on its surface
- GIVEN the `enable` subcommand's declared flags
- WHEN its flag surface is inspected
- THEN no `--uid` flag exists on `enable`, and its resolved target is
  always the caller's own `PKEXEC_UID`
- Testable via: `cargo test`
#### Scenario: A SystemRoot-context target still fails UID Range Admission the same way
- GIVEN a SystemRoot-context `grant`, `revoke`, or `inspect` targeting uid
  0, an out-of-range uid, or a non-existent uid
- WHEN admission runs
- THEN the request exits 11 exactly as any other admission-path caller's
  target would, and nothing is written
- Testable via: `cargo test`
# Delta for Helper CLI

## MODIFIED Requirements

### Requirement: External Command Invocation Discipline

The helper MUST invoke every external program (`visudo`, `sudo`, `systemctl`, `systemd-run`) via
the `CommandRunner` abstraction, using only absolute paths resolved from an ordered candidate
list, never via `PATH` lookup or a shell, and passing no environment variable beyond what each
command explicitly requires (e.g. `LANG=C` for the sudo probe). For `systemd-run`, the
obligation is not only that our own assertion matches the argv we build — it is that the real
`systemd-run` target accepts it. The seven non-calendar `systemd-run` property tokens
(`--unit`, `--description`, `--on-calendar`, `--timer-property=AccuracySec=1s`,
`--timer-property=Persistent=false`, `--timer-property=WakeSystem=false`,
`--timer-property=RemainAfterElapse=false`, `--property=Type=oneshot`) MUST be validated against
real systemd unit-file grammar, not only pinned by field equality against a fake
`CommandRunner`. A gate performing this validation MUST FAIL, not skip, when `systemd-analyze`
is unavailable in the running environment.
(Previously: discipline was verified only by argv field-equality against `ScriptedRunner`, which
proves the shape of a call but never that systemd accepts it — the exact defect shape that let
`--on-calendar` carry rejected RFC-3339 syntax for a whole milestone.)

#### Scenario: Each external call uses its exact absolute-path argv
- GIVEN a fake `CommandRunner`
- WHEN any of `visudo`, `sudo`, `systemctl`, `systemd-run` is invoked
- THEN the captured argv uses the absolute path and no shell wrapping
- Testable via: `cargo test`

#### Scenario: No candidate binary path exists on the host
- GIVEN none of the ordered absolute candidates for a required binary exist
- WHEN the helper attempts to resolve it
- THEN the helper exits 1 (internal error) and never falls back to a `PATH` lookup
- Testable via: `cargo test` (stubbed candidate resolution)

#### Scenario: The synthesized systemd-run unit is accepted by real systemd
- GIVEN the same `[Timer]`/`[Service]` property values `timer.rs` feeds to argv, synthesized
  into a transient-unit form
- WHEN `systemd-analyze verify` runs over that synthesized unit
- THEN verification passes with no rejected property or syntax error
- Testable via: `cargo test --workspace` (Lane A: real `systemd-analyze`, no bus and no
  privileged action are needed to ask it)

#### Scenario: A corrupted property token fails the gate
- GIVEN one `systemd-run` property token is deliberately malformed (e.g. an invalid
  `--on-calendar` value)
- WHEN the same real-tool validation runs
- THEN validation fails, distinguishing the defect from a passing run
- Testable via: `cargo test --workspace` (Lane A)

#### Scenario: Absence of systemd-analyze fails the gate, not skips it
- GIVEN `systemd-analyze` is not present in the running environment
- WHEN this gate runs
- THEN the gate reports failure, not a silent skip or a false pass
- Testable via: `cargo test --workspace` (Lane A; binary hidden from `PATH` in a controlled
  sub-environment)

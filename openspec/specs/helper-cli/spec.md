# Helper CLI Specification

## Purpose

Defines the fixed subcommand/flag surface of `nopass-helper`, its typed exit-code contract, and the discipline for invoking external commands through `CommandRunner`, so the parser rejects anything unforeseen before any privileged handler runs. (PRD §7.2, NFR security rows)

## Requirements

### Requirement: Fixed Subcommand and Flag Surface

The helper MUST expose exactly seven subcommands — `enable`, `disable`,
`status`, `expire`, `grant`, `revoke`, `inspect` — with only their documented
flags, and MUST reject any unrecognized subcommand or flag at parse time
before any handler executes. `grant`, `revoke`, and `inspect` are flat
siblings of the existing four, not verbs of a single `admin`-style dispatch
command: the closed surface (no `allow_external_subcommands`,
`allow_hyphen_values`, or `trailing_var_arg`) is what makes exit 2 happen at
clap's parse step, before any privileged handler runs. An `admin --action
<verb>` form would move the verb out of that validated surface into a
hand-written dispatch layer, so it is rejected as a design.
(Previously: exactly four subcommands — `enable`, `disable`, `status`, `expire`.)

#### Scenario: A documented invocation parses successfully
- GIVEN `enable --until <epoch>`
- WHEN the CLI is parsed
- THEN parsing succeeds and control passes to the `enable` handler
- Testable via: `cargo test`

#### Scenario: Unknown subcommand is rejected
- GIVEN the argument `foo`
- WHEN the CLI is parsed
- THEN the process exits 2, no handler runs, and no system state changes
- Testable via: `cargo test`

#### Scenario: Unknown flag on a known subcommand is rejected
- GIVEN `enable --bogus`
- WHEN the CLI is parsed
- THEN the process exits 2 and no handler runs
- Testable via: `cargo test`

#### Scenario: A documented grant invocation parses successfully
- GIVEN `grant --uid 1000 --until <epoch>`
- WHEN the CLI is parsed
- THEN parsing succeeds and control passes to the `grant` handler
- Testable via: `cargo test`
#### Scenario: An `admin --action` dispatch form is rejected
- GIVEN the argument `admin --action grant --uid 1000`
- WHEN the CLI is parsed
- THEN the process exits 2 because `admin` is not a declared subcommand, and
  no handler runs
- Testable via: `cargo test`
### Requirement: Mutually Exclusive Duration Flags

`enable` and `grant` MUST each treat `--until <epoch>` and `--until-reboot`
as mutually exclusive; supplying both to either subcommand MUST be rejected
at parse time with exit 2.
(Previously: only `enable` carried this constraint.)

#### Scenario: `--until-reboot` alone parses to Reboot
- GIVEN `enable --until-reboot`
- WHEN parsed
- THEN the resolved expiry is `Reboot`
- Testable via: `cargo test`

#### Scenario: Both duration flags together are rejected
- GIVEN `enable --until 100 --until-reboot`
- WHEN parsed
- THEN the process exits 2 before any handler runs
- Testable via: `cargo test`

#### Scenario: `grant --until-reboot` alone parses to Reboot
- GIVEN `grant --uid 1000 --until-reboot`
- WHEN parsed
- THEN the resolved expiry is `Reboot`
- Testable via: `cargo test`
#### Scenario: Both duration flags together on `grant` are rejected
- GIVEN `grant --uid 1000 --until 100 --until-reboot`
- WHEN parsed
- THEN the process exits 2 before any handler runs
- Testable via: `cargo test`
### Requirement: Typed Exit Code Mapping

Every failure condition MUST map deterministically to exactly one of the documented exit codes (0 success, 1 internal error, 2 CLI parse error, 10 invocation-context violation, 11 uid rejected, 12 not-a-sudoer, 13 duration/until invalid, 14 visudo rejection, 15 lock busy, 16 filesystem/atomic-write failure, 17 timer scheduling failure with rollback). No two distinct failure causes share a code with a different meaning.

#### Scenario: Successful enable exits 0
- GIVEN a valid, admitted enable request
- WHEN it completes
- THEN the process exits 0
- Testable via: root-only container test

#### Scenario: Each rejection path exits its documented code
- GIVEN the causes for codes 10 through 17 in turn (context violation, uid rejected, not-a-sudoer, bad duration, visudo rejection, lock busy, fs failure, timer failure)
- WHEN each is triggered
- THEN the process exits the exact documented code and, for 14/16/17, leaves no partial system change
- Testable via: `cargo test` (table-driven, `CommandRunner`-observable causes); codes 14, 16, 17 additionally covered by root-only container tests

### Requirement: External Command Invocation Discipline

The helper MUST invoke every external program (`visudo`, `sudo`, `systemctl`, `systemd-run`) via the `CommandRunner` abstraction, using only absolute paths resolved from an ordered candidate list, never via `PATH` lookup or a shell, and passing no environment variable beyond what each command explicitly requires (e.g. `LANG=C` for the sudo probe).

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
### Requirement: Required Target UID on Headless Subcommands

`grant`, `revoke`, and `inspect` MUST each require an explicit `--uid <uid>`
flag; clap MUST reject any invocation of these three that omits `--uid` at
parse time with exit 2, before any handler executes. No environment
variable substitutes for it on these subcommands.

#### Scenario: `grant` without `--uid` is rejected
- GIVEN the argument `grant`
- WHEN the CLI is parsed
- THEN the process exits 2 and no handler runs
- Testable via: `cargo test`
#### Scenario: `revoke` and `inspect` each require `--uid` the same way
- GIVEN `revoke` and `inspect`, each invoked with no `--uid`
- WHEN the CLI is parsed
- THEN each exits 2 and no handler runs
- Testable via: `cargo test`
#### Scenario: `revoke --uid` and `inspect --uid` parse with only that flag
- GIVEN `revoke --uid 1000` and `inspect --uid 1000`
- WHEN parsed
- THEN each parses successfully with no other flags required
- Testable via: `cargo test`

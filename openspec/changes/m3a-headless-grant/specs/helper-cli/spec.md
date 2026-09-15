# Delta for Helper CLI

## MODIFIED Requirements

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
## ADDED Requirements

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
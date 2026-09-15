# Delta for Helper Observability

## MODIFIED Requirements

### Requirement: Journald Audit Records

Every outcome-producing helper subcommand invocation — including every
subcommand added to the fixed CLI surface after this requirement was
written — MUST be logged to journald with `SYSLOG_IDENTIFIER=nopass-helper`,
including uid, outcome, and resulting expiry, such that
`journalctl -t nopass-helper` shows the record. This requirement binds by
"every outcome," not by an enumerated subcommand list, precisely so a
future subcommand cannot ship unaudited by omission. For a
`SystemRoot`-context invocation (`expire`, `grant`, `revoke`, `inspect`),
the record MUST additionally capture the invocation context as
`SystemRoot` and the explicit target uid, distinguishably from a
`Pkexec`-context record for the same uid, so an auditor can tell a
root-invoked grant from a pkexec-invoked `enable`.
(Previously: enumerated `enable`, `disable`, and `expire` by name.)

#### Scenario: Successful enable is journaled
- GIVEN a successful `enable`
- WHEN it completes
- THEN one journald record with `SYSLOG_IDENTIFIER=nopass-helper`, the uid,
  and the resulting expiry is emitted
- Testable via: root-only container test (journald present); `cargo test`
  asserts the tracing-journald layer is configured with
  `with_syslog_identifier("nopass-helper")`

#### Scenario: A rejected operation is still journaled
- GIVEN `enable` is rejected with exit 12 (not a sudoer)
- WHEN the rejection occurs
- THEN a journald record distinguishes the rejection outcome from a
  success record
- Testable via: root-only container test

#### Scenario: A root-invoked grant is journaled with the SystemRoot context and its explicit target
- GIVEN a successful `grant --uid 1000` run as real uid 0 with no
  `PKEXEC_UID`
- WHEN it completes
- THEN the journald record names its invocation context as `SystemRoot`,
  distinct from a `Pkexec` record, and names 1000 as the explicit target
  uid
- Testable via: root-only container test (a real journald is needed to read the
  record back; asserting on our own writer proves only what we wrote)

#### Scenario: Revoke and inspect are journaled the same way
- GIVEN successful `revoke --uid 1000` and `inspect --uid 1000` runs
- WHEN each completes
- THEN each produces its own `SYSLOG_IDENTIFIER=nopass-helper` record
  naming its own outcome, its `SystemRoot` context, and target uid 1000
- Testable via: root-only container test (same reason as above)

#### Scenario: An unaudited-by-omission subcommand fails this requirement, not escapes it
- GIVEN the fixed subcommand surface enumerated by `helper-cli`'s "Fixed
  Subcommand and Flag Surface"
- WHEN a subcommand is added to that surface without a matching audit call
  site
- THEN it violates this requirement, because the requirement is defined
  over "every outcome-producing subcommand," not over a name list a new
  addition could silently fall outside of
- Testable via: `cargo test`
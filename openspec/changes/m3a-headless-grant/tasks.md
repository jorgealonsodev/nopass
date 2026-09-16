# Tasks: M3a — Headless operation (root-context grant)

Source: `proposal.md`, the four delta specs under `specs/{helper-cli,privilege-admission,
helper-observability,headless-operation}`, `design.md` §0–§10. Eleven phases. TDD: every behaviour
task names its RED assertion and lane before its GREEN implementation step. Two lanes are new;
one existing lane is finally wired to actually run: **A** = `cargo test --workspace`; **R** =
`bash scripts/run-lane-root.sh` (new — Debian+Fedora, `NOPASS_ROOT_TESTS=1`, closes the "15 tests
report ok in a lane that never executes" gap); **R-J** = `bash scripts/run-lane-journal.sh` (new —
the journald read-back lane, G1).

> Size note: this artifact deliberately exceeds the generic 530-word budget, following the M2/M3
> house-style precedent in this repository (per-task RED/GREEN, exact `file:line`, negative
> controls). The privilege boundary is the artifact here, not a summary of it.

**Facts this plan is built against, not re-litigated:**
- **G1 is PROVEN** (design §0): a standalone `systemd-journald` runs on plain `debian:12-slim`
  under **Docker**, no PID 1 systemd, no `--privileged`, no cgroup mount. Both halves verified,
  including the custom `NOPASS_CONTEXT` field with a negative control. **No verification-slice
  task and no fallback ladder are planned.** Non-fatal `Failed to join audit multicast group`
  stderr is explicitly a non-failure.
- **`podman` is absent on this machine; `docker` 29.8.0 is present.** `run-lane-root.sh` and
  `run-lane-journal.sh` are written against the runtime-detection pattern M3's
  `run-lane-polkit.sh` already uses — never hardcode either runtime.
- **`toolgate::require` already exists** (`crates/nopass-core/src/toolgate.rs`, M3 phase 1). No
  new copy is created here; this change does not need it directly since neither new lane calls an
  optional real tool the way M3's Rank 1/2 gates do, but the pattern is noted for consistency.
- **The lane-wiring gap is real and literal**: `NOPASS_ROOT_TESTS` gates
  `crates/nopass-helper/tests/root_system.rs`; no script under `scripts/` names it today. Phase 1
  closes this before any other work, because `lane_wiring.rs` is infrastructure `m3-menu-and-
  config`'s own Phase 10 depends on, not a docs afterthought.

## Review Workload Forecast

| # | Phase | Impl | Tests | Total |
|---|---|---|---|---|
| 1 | Lane infrastructure: `lane_wiring.rs`, `run-lane-root.sh` | 60 | 70 | 130 |
| 2 | Audit schema, part 1: `AuditEvent` growth, `Status` wired | 30 | 60 | 90 |
| 3 | `Subject` seam (`subject.rs`) | 60 | 90 | 150 |
| 4 | Audit schema, part 2: `AuditRecord.context` field | 30 | 70 | 100 |
| 5 | CLI surface: `Grant`/`Revoke`/`Inspect` | 30 | 60 | 90 |
| 6 | `uid.rs`: SystemRoot arm covers four subcommands | 15 | 40 | 55 |
| 7 | `ops.rs` reuse seam: wrappers, `admit_root_target`, `*_inner` retyped | 180 | 220 | 400 |
| 8 | Journald lane: `Containerfile.journald`, `run-lane-journal.sh`, `root_journal.rs` | 70 | 90 | 160 |
| 9 | `root_system.rs`: the two headless transaction scenarios | 10 | 70 | 80 |
| 10 | `docs/headless.md` + `docs_headless.rs` structural guard | 60 | 60 | 120 |
| 11 | Wiring: `config.yaml`, `tests/containers/README.md` | 20 | 0 | 20 |
| | **Total** | **565** | **830** | **1,395** |

Every individual phase stays at or under the 400-line review budget (largest: Phase 7, the
transaction-retyping phase, at exactly 400 — split no further because `Subject` threading through
`enable_inner`/`disable_inner`/`status_inner`/`expire_uid_inner` plus the three wrappers is one
seam and a mid-phase revert boundary would leave the crate non-compiling). The **total** (1,395)
exceeds the 400-line per-PR budget, matching M2/M3's own convention: `High` reflects scale before
chaining mitigates it.

```text
Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High
```

`Decision needed before apply: No` follows from the cached `auto-chain` delivery strategy.
`stacked-to-main` follows M2/M3's precedent: per `proposal.md`'s Rollback Plan, reverting any
slice commit restores exact M1/M2 behaviour with no migration — no on-disk format, state-file
schema, rule-file format, or timer unit name changes anywhere in this change.

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | Lane infra (Phase 1) | PR 1 | `cargo test -p nopass-helper lane_wiring::` | N/A | delete `lane_wiring.rs`, `scripts/run-lane-root.sh`; revert README |
| 2 | Audit events (Phase 2) | PR 2 | `cargo test -p nopass-helper journal::` | N/A | revert `AuditEvent` additions and `Status` wiring in `journal.rs`/`ops.rs` |
| 3 | Subject seam (Phase 3) | PR 3 | `cargo test -p nopass-helper subject::` | N/A | delete `subject.rs`; revert `lib.rs`'s `pub mod subject;` |
| 4 | Audit context field (Phase 4) | PR 4 | `cargo test -p nopass-helper journal:: ops::` | N/A | revert `AuditRecord.context` and all 8 call sites |
| 5 | CLI surface (Phase 5) | PR 5 | `cargo test -p nopass-helper cli::` | N/A | revert `Grant`/`Revoke`/`Inspect` variants in `cli.rs` |
| 6 | uid.rs arm (Phase 6) | PR 6 | `cargo test -p nopass-helper uid::` | N/A | revert the four-subcommand `SystemRoot` match arm |
| 7 | ops.rs seam (Phase 7) | PR 7 | `cargo test -p nopass-helper ops::` | N/A — `ScriptedRunner`/`TempDir` only | revert wrappers, `admit_root_target`, `Subject`-typed `*_inner` signatures |
| 8 | Journald lane (Phase 8) | PR 8 | `bash scripts/run-lane-journal.sh` | Docker, `Containerfile.journald` (G1 proven) | delete `Containerfile.journald`, `run-lane-journal.sh`, `root_journal.rs` |
| 9 | root_system transaction tests (Phase 9) | PR 9 | `bash scripts/run-lane-root.sh` | Docker, `Containerfile.debian`/`.fedora` | revert the two new assertions in `root_system.rs` |
| 10 | Docs (Phase 10) | PR 10 | `cargo test -p nopass-helper docs_headless::` | N/A | delete `docs/headless.md`, `docs_headless.rs` |
| 11 | Gate wiring (Phase 11) | PR 11 | N/A — config/docs only | N/A | revert `config.yaml`'s `gate_commands` addition, README diffs |

---

## Phase 1: Lane Infrastructure — Close the Execution Gap First

*(Design §8 "The execution gap, named and closed"; landed first because `m3-menu-and-config`
Phase 10's `run-lane-polkit.sh` depends on `lane_wiring.rs` existing, not as cleanup)*

- [x] 1.1 RED (Lane A): create `crates/nopass-helper/tests/lane_wiring.rs` —
      `every_gated_test_file_is_named_by_a_runner_script`: scans `crates/*/tests/*.rs` for
      `NOPASS_[A-Z_]+_TESTS` names, asserts each appears in a script under `scripts/`, and each
      such script is named in `tests/containers/README.md`'s checklist. **This test MUST fail
      today** — `NOPASS_ROOT_TESTS` (in `crates/nopass-helper/tests/root_system.rs`, read-only)
      has no runner script — reproducing the exact gap the proposal documents.
- [x] 1.2 Create `scripts/run-lane-root.sh` — mirrors `scripts/run-lane-b.sh` (read-only,
      reference only): detects the available container runtime (`docker` first on this machine,
      falls back to `podman`), builds `tests/containers/Containerfile.debian` and `.fedora`
      (read-only), runs `<runtime> run --rm -e NOPASS_ROOT_TESTS=1 <img> cargo test --workspace`
      for each, `set -euo pipefail`, exits non-zero when neither runtime is found.
- [x] 1.3 GREEN: `cargo test -p nopass-helper --test lane_wiring` passes; 1.1's scan now finds
      `NOPASS_ROOT_TESTS` named in `scripts/run-lane-root.sh`.
- [x] 1.4 Negative control: RED (Lane A) `lane_wiring.rs` —
      `a_gated_test_file_naming_no_runner_script_fails_the_scan`: a fixture-style assertion
      (temp scan input, not a real crate file) proves the scan itself rejects an unwired
      `NOPASS_*_TESTS` name rather than passing vacuously. GREEN: none — this test's pass/fail
      *is* the guard; it must never pass by construction alone.
- [x] 1.5 Modify `tests/containers/README.md` — add the root-lane script invocation to the Quick
      path (replacing the bare `podman build`/`podman run` pair with `bash
      scripts/run-lane-root.sh`) and add its checklist line, satisfying 1.1's second assertion.

---

## Phase 2: Audit Schema, Part 1 — `AuditEvent` Growth and `Status` Finally Wired

*(Design §4; Spec: helper-observability "Journald Audit Records")* — depends on Phase 1 only for
lane discipline, not code.

- [x] 2.1 Modify `crates/nopass-helper/src/journal.rs` — `AuditEvent` gains `Grant`, `Revoke`,
      `Inspect` (`as_str()` → `grant`/`revoke`/`inspect`); correct the stale doc comment at
      `journal.rs:66-67` (read-only reference for the current text) that scopes `journal::audit`
      to `enable`/`disable`/`expire` only.
- [x] 2.2 RED (Lane A) `journal.rs`: each new `AuditEvent` variant's `as_str()` matches the
      literal token the spec names (`grant`, `revoke`, `inspect`). GREEN: satisfied by 2.1.
- [x] 2.3 **Behaviour change, flagged explicitly — not silently resolved**: `AuditEvent::Status`
      has existed since M1 and is emitted from nowhere (`journal.rs:63-74` doc comment,
      read-only reference). This phase wires it: `ops::status` (`ops.rs:415`, read-only
      reference for current signature) gains a `journal::audit` call with
      `AuditEvent::Status`. **This changes existing, already-shipped `status` behaviour** —
      every `status` invocation becomes audited for the first time. RED (Lane A) `ops.rs`: a
      successful `status` call now produces exactly one `AuditRecord` with `event: Status`,
      `outcome: Ok`. GREEN: add the `journal::audit` call to `ops::status`.
- [x] 2.4 Create `fn audit_event_for(cmd: &Cmd) -> AuditEvent` in `journal.rs` — one exhaustive
      match arm per `Cmd` variant, **no wildcard arm**, so an eighth subcommand added later
      without an arm fails to compile (threat matrix "Unaudited new subcommand").
- [x] 2.5 RED (Lane A) `journal.rs`: `audit_event_for` is total (every `Cmd` variant maps) and
      injective (no two variants map to the same `AuditEvent`) over the current seven-variant
      surface. GREEN: satisfied by 2.4; test documents the compile-time guarantee is also
      runtime-checked.

---

## Phase 3: The `Subject` Seam

*(Design §2; must land before any new subcommand per the non-negotiable ordering)* — depends on
Phase 2 (`AuditEvent`).

- [x] 3.1 Create `crates/nopass-helper/src/subject.rs` — `pub enum UidSource { Pkexec,
      SystemRoot }`; `pub struct Subject { uid: u32, source: UidSource, event: AuditEvent }`
      with private fields.
- [x] 3.2 RED (Lane A) `subject.rs`: `Subject::pkexec(ctx, event)` — **note the absent uid
      parameter, asserted structurally**: a table test confirms `pkexec`'s signature has no
      `uid: u32` argument (compile-level; the test documents rather than proves this, matching
      M2's declined `trybuild` precedent). It unpacks `uid` only from
      `InvocationContext::Pkexec(uid)` and returns `Err(HelperError::Context(...))` (exit 10) for
      `InvocationContext::SystemRoot`, never `unreachable!()`. GREEN: implement `Subject::pkexec`.
- [x] 3.3 RED (Lane A) `subject.rs`: `Subject::root_target(ctx, uid, event)` — accepts only
      `InvocationContext::SystemRoot`, rejects `Pkexec(_)` with `HelperError::Context` (exit 10,
      not a panic). GREEN: implement `Subject::root_target`.
- [x] 3.4 RED (Lane A) `subject.rs`: `Subject::uid()`/`source()`/`event()` accessors round-trip
      exactly what each constructor was given; no third constructor exists (grep-level structural
      assertion — the type has exactly two ways to be built). GREEN: implement accessors.
- [x] 3.5 Modify `crates/nopass-helper/src/lib.rs` — add `pub mod subject;`.

---

## Phase 4: Audit Schema, Part 2 — `AuditRecord.context`, Derived From `Subject` Only

*(Design §4; Spec: helper-observability, privilege-admission "Authority confusion" threat row)* —
depends on Phase 3 (`UidSource`).

- [x] 4.1 Modify `crates/nopass-helper/src/journal.rs` — `AuditRecord` gains `pub context:
      UidSource` (eighth field); `journal::audit` emits `CONTEXT` after the existing `NOPASS_*`
      prefix, value domain the literal strings `Pkexec`/`SystemRoot`.
- [x] 4.2 RED (Lane A) `journal.rs`: a `Pkexec`-context record and a `SystemRoot`-context record
      for the **same uid** are distinguishable on `CONTEXT` alone, asserted via the
      `tracing_subscriber` capture technique `journal.rs`'s own tests already use (threat matrix
      "Authority confusion in the audit trail"). GREEN: satisfied by 4.1.
- [x] 4.3 **The non-negotiable test**: RED (Lane A) `journal.rs` or `ops.rs` — `context` is read
      exclusively from `subject.source()`, never inferred from `record.event`/the subcommand
      name: construct an `AuditRecord` whose `event` is `Grant` but whose `context` is
      `Pkexec` (a deliberately mismatched pairing built directly, bypassing any wrapper) and
      assert the emitted `CONTEXT` is `Pkexec` — proving the field has no event-based fallback
      or override anywhere in `audit()`. GREEN: none — `audit()`'s straight-through field read
      already satisfies this; the test pins that no future "helpful" derivation is added.
- [x] 4.4 Modify `crates/nopass-helper/src/ops.rs` (8 call sites: `enable_inner`, `disable_inner`
      via `audit_rejection`, `status_inner`'s new 2.3 call, and `journal.rs`'s own test fixtures)
      — every `AuditRecord { .. }` literal adds `context: subject.source()` (or the equivalent
      once each site is retyped in Phase 7; sites not yet retyped pass `UidSource::Pkexec`
      explicitly as an interim literal, corrected in Phase 7). GREEN: `cargo build --workspace`
      compiles with the new required field satisfied everywhere.

---

## Phase 5: CLI Surface — `Grant`, `Revoke`, `Inspect`

*(Design §1; Spec: helper-cli "Fixed Subcommand and Flag Surface", "Required Target UID",
"Mutually Exclusive Duration Flags")* — depends on Phase 1 only for lane discipline.

- [x] 5.1 Modify `crates/nopass-helper/src/cli.rs` — add three flat `Cmd` variants: `Grant {
      uid: u32 (required), until: Option<u64>, until_reboot: bool }` with its own `ArgGroup`
      named `"grant_when"` (distinct from `Enable`'s `"when"` — group names are global);
      `Revoke { uid: u32 (required) }`; `Inspect { uid: u32 (required) }`. No `admin` dispatch
      verb.
- [x] 5.2 RED (Lane A) `cli.rs`: `grant --uid 1000 --until <epoch>` parses to the expected
      variant (helper-cli "A documented grant invocation parses successfully"). GREEN: satisfied
      by 5.1.
- [x] 5.3 RED (Lane A) `cli.rs`: `admin --action grant --uid 1000` exits 2 because `admin` is not
      a declared subcommand (helper-cli "An admin --action dispatch form is rejected"). GREEN:
      none — the closed surface already rejects it; test pins the design decision.
- [x] 5.4 RED (Lane A) `cli.rs`: `grant` with no `--uid` exits 2; `revoke`/`inspect` each with no
      `--uid` exit 2 (helper-cli "Required Target UID on Headless Subcommands", both scenarios).
      GREEN: satisfied by `required = true` in 5.1.
- [x] 5.5 RED (Lane A) `cli.rs`: `revoke --uid 1000` and `inspect --uid 1000` each parse with only
      that flag; `grant --uid 1000 --until-reboot` parses to `Reboot`; `grant --uid 1000 --until
      100 --until-reboot` exits 2 (helper-cli "grant --until-reboot alone", "Both duration flags
      together on grant"). GREEN: satisfied by 5.1's `ArgGroup`.

---

## Phase 6: `uid.rs` — SystemRoot Arm Covers Four Subcommands

*(Design §2; Spec: privilege-admission "UID Resolution by Invocation Context")* — depends on
Phase 5 (`Cmd` variants must exist to match on).

- [x] 6.1 Modify `crates/nopass-helper/src/uid.rs` — the `Cmd::Expire { .. }` arm at `uid.rs:80`
      (read-only reference for the current line) becomes `Cmd::Expire { .. } | Cmd::Grant { .. }
      | Cmd::Revoke { .. } | Cmd::Inspect { .. }`; its two `&'static str` error messages
      generalize from `"expire"` to `"this subcommand"`.
- [x] 6.2 RED (Lane A) `uid.rs`: `grant --uid 1000`, `revoke --uid 1000`, `inspect --uid 1000`
      each run with `PKEXEC_UID` set ⇒ exit 10, no write (privilege-admission "Grant, revoke, and
      inspect are each rejected when PKEXEC_UID is set"). GREEN: satisfied by 6.1.
- [x] 6.3 RED (Lane A) `uid.rs`: the same three, run as non-root with `PKEXEC_UID` unset ⇒ exit
      10 (privilege-admission "...rejected for a non-root caller with no PKEXEC_UID" — the
      existing unprivileged test process already satisfies this precondition). GREEN: satisfied
      by 6.1.
- [x] 6.4 RED (Lane A) `uid.rs`: `grant --uid 1000` resolved as real uid 0 with `PKEXEC_UID`
      unset ⇒ `InvocationContext::SystemRoot`, with the target uid taken from `--uid`, not from
      any environment variable (privilege-admission "Grant resolves the SystemRoot context and
      the explicit target uid"). GREEN: satisfied by 6.1.
- [x] 6.5 RED (Lane A): existing `uid.rs` tests `expire_resolves_as_system_root_when_...` and
      `expire_invoked_non_root_without_pkexec_uid_is_rejected` (`uid.rs:210`, `uid.rs:214`,
      read-only references) pass unchanged — the generalized match arm produces byte-identical
      behaviour for `Cmd::Expire`. GREEN: none — regression check only.

---

## Phase 7: The `ops.rs` Reuse Seam — Wrappers, `admit_root_target`, `*_inner` Retyped

*(Design §2–§3, §6–§7; Spec: privilege-admission "SystemRoot Context Can Target Any Admitted
UID"; largest phase, kept as one unit because `Subject` threading through every `*_inner` is one
seam)* — depends on Phases 3, 4, 6.

- [x] 7.1 RED (Lane A) `checks.rs` (read-only — `admit_uid` itself is unchanged): a table test
      documents `admit_root_target`'s expected causes before it exists — uid 0, below `min`,
      above `max`, no passwd entry — each mapped to exit 11 with a distinct `audit_reason`. This
      test targets the not-yet-created `ops::admit_root_target` and fails to compile/RED until
      7.2 lands.
- [x] 7.2 Create `fn admit_root_target(subject: &Subject, range: &UidRange) -> Result<(),
      HelperError>` in `ops.rs` — the one shared wrapper-level admission function for `grant`,
      `revoke`, and `inspect`; audits its own rejection with the correct event and `SystemRoot`
      context via `journal::audit`. GREEN: satisfies 7.1.
- [x] 7.3 Modify `crates/nopass-helper/src/ops.rs` — `enable_inner` (`ops.rs:190`), `disable_inner`
      (`ops.rs:345`), `status_inner` (`ops.rs:432`), `expire_uid_inner` (`ops.rs:478`, all
      read-only references for current signatures) each change their `uid: u32` parameter to
      `subject: Subject`; every internal `uid` use becomes `subject.uid()`; downstream calls
      (`fileops`, `timer`, `statefile`, `checks::admit_uid`, `layout.rule_path`) keep taking a
      bare `u32` unchanged.
- [x] 7.4 RED (Lane A) `ops.rs`: the existing `enable`/`disable`/`status` test suite (the whole
      block from `ops.rs:650` onward, read-only reference for current line) passes unchanged
      after retyping — each production wrapper now builds a `Subject::pkexec(ctx, event)` before
      calling its `*_inner`. GREEN: update `ops::enable`/`disable`/`status`/`expire` to construct
      a `Subject` and pass it through.
- [x] 7.5 Modify `ops.rs` — route `ops::expire` through `Subject::root_target(ctx, uid,
      AuditEvent::Expire)` (the seam's only other `SystemRoot` consumer), so the seam is the
      single path rather than a second one.
- [x] 7.6 RED (Lane A) `ops.rs`: `pub fn grant(...)` — builds `Cmd::Grant`, resolves context via
      `uid::resolve` (exit 10 path), `Subject::root_target(ctx, uid_flag, AuditEvent::Grant)`,
      `resolve_expiry_audited` (exit 13), `admit_root_target` (exit 11), then
      `enable_inner(subject, ...)`. Assert the happy path reaches `enable_inner` with a
      `SystemRoot`-sourced `Subject`. GREEN: implement `grant`.
- [x] 7.7 RED (Lane A) `ops.rs`: `admit_root_target` rejects uid 0 / below-min / above-max /
      no-passwd-entry for `grant`, `revoke`, and `inspect` alike, each exit 11 with no write
      (privilege-admission "A SystemRoot-context target still fails UID Range Admission the same
      way"). GREEN: satisfied by 7.2.
- [x] 7.8 RED (Lane A) `ops.rs`: `grant --uid 1000` targets uid 1000 regardless of any identity
      the invoking shell has (privilege-admission "A root-invoked grant targets a uid other than
      the caller's own"). GREEN: satisfied by 7.6.
- [x] 7.9 RED (Lane A) `ops.rs`: `enable`'s declared flag surface carries no `--uid`, and its
      resolved target is always the caller's own `PKEXEC_UID` (privilege-admission "enable stays
      self-targeted with no uid argument on its surface"). GREEN: none — pins the
      `Subject::pkexec` type-level guarantee from 3.2 at the CLI+ops boundary.
- [x] 7.10 RED (Lane A) `ops.rs`: `pub fn revoke(...)` calls `admit_root_target` before
      `disable_inner(subject, ...)`; `revoke --uid 0` exits 11 and writes nothing, even though
      `disable_inner` itself still runs no admission (design §3's "one real tension" — pins that
      `disable`'s pkexec path is untouched while `revoke`'s SystemRoot path is bounded). GREEN:
      implement `revoke`.
- [x] 7.11 RED (Lane A) `ops.rs`: `pub fn inspect(...)` calls `admit_root_target` then
      `status_inner(layout, subject.uid(), now)`, plus an `AuditEvent::Inspect` audit record; a
      uid with no rule prints `"active":false` and exits 0, byte-identical shape to `status`.
      GREEN: implement `inspect`.
- [x] 7.12 RED (Lane A) `ops.rs`: `grant`'s double `admit_uid` call (once in `admit_root_target`,
      once again inside `enable_inner`'s unchanged step 5) is pure and can only ever agree with
      the first — a table test confirms both calls reject the same uid identically, never
      diverging (threat matrix "Arbitrary-uid targeting"). GREEN: none — pins the deliberate
      double-check design decision from §3.
- [x] 7.13 RED (Lane A): threat matrix "Subprocess argv" completion — `grant`'s `--uid` reaches
      no new subprocess call site; existing `enable` argv assertions (`checks::is_sudoer`,
      `checks.rs:150`, read-only reference) already cover the only subprocess `enable_inner`
      spawns, and `grant` adds none. GREEN: none — structural confirmation, not new code.
- [x] 7.14 Modify `crates/nopass-helper/src/main.rs` — `dispatch` (`main.rs:61`, read-only
      reference for current signature) gains `Cmd::Grant { uid, until, until_reboot } =>
      ops::grant(...)`, `Cmd::Revoke { uid } => ops::revoke(...)`, `Cmd::Inspect { uid } =>
      ops::inspect(...)`.
- [x] 7.15 RED (Lane A) `main.rs`: extend `dispatch_routes_every_subcommand_and_rejects_missing_
      pkexec_context` (`main.rs:80`, read-only reference) to include `Cmd::Grant`, `Cmd::Revoke`,
      `Cmd::Inspect` in its loop — each resolves to exit 10 under the unprivileged test process,
      proving `dispatch` reaches the real `ops::*` entry points. GREEN: satisfied by 7.14.

---

## Phase 8: The Journald Lane — G1 Realized, With Its Negative Control

*(Design §0 G1 resolved, §8; Spec: helper-observability (root-only scenarios))* — depends on
Phase 4 (`context` field) and Phase 7 (wrappers emit it).

- [x] 8.1 Create `tests/containers/Containerfile.journald` — plain `debian:12-slim`,
      `apt-get install -y --no-install-recommends systemd python3`, `mkdir -p
      /run/systemd/journal /var/log/journal`, entrypoint starts
      `/usr/lib/systemd/systemd-journald &`, `sleep 2`, then runs `cargo test --workspace` with
      `NOPASS_JOURNAL_TESTS=1`. **Distinct from `Containerfile.debian`/`.fedora`, which MUST
      keep having no systemd** (G2 — their absence is load-bearing for the real exit-17 rollback
      test).
- [x] 8.2 Create `scripts/run-lane-journal.sh` — same runtime-detection pattern as
      `run-lane-root.sh` (docker first, podman fallback), builds and runs
      `Containerfile.journald` with `NOPASS_JOURNAL_TESTS=1`; the script MUST NOT treat journald's
      non-fatal `Failed to join audit multicast group` stderr line as failure — assert on exit
      code only, never on stderr content.
- [x] 8.3 Create `crates/nopass-helper/tests/root_journal.rs`, gated
      `NOPASS_JOURNAL_TESTS=1` — `successful_enable_is_journaled`: run a real `enable`
      transaction, `journalctl -t nopass-helper` reads back `SYSLOG_IDENTIFIER=nopass-helper`,
      uid, and resulting expiry (helper-observability "Successful enable is journaled").
- [x] 8.4 RED (Lane R-J) `root_journal.rs`: `root_invoked_grant_is_journaled_with_system_root_
      context_and_explicit_target`: run `grant --uid 1000`, then
      `journalctl NOPASS_CONTEXT=SystemRoot` matches it and returns `NOPASS_UID=1000` in `-o
      json` (helper-observability "A root-invoked grant is journaled with the SystemRoot context
      and its explicit target"). GREEN: satisfied by Phase 7's wiring; this test proves it against
      a real journal, not our own writer.
- [x] 8.5 **Negative control** — RED (Lane R-J) `root_journal.rs`:
      `a_context_value_never_written_matches_nothing`: `journalctl NOPASS_CONTEXT=Pkexec` after
      only `SystemRoot`-context invocations have run returns zero matches — without this, 8.4
      could pass on a query that matches everything. GREEN: none — this test's pass/fail *is* the
      gate.
- [x] 8.6 RED (Lane R-J) `root_journal.rs`: `revoke --uid 1000` and `inspect --uid 1000` each
      produce their own record naming their own outcome, `SystemRoot` context, and target uid
      1000 (helper-observability "Revoke and inspect are journaled the same way"). GREEN:
      satisfied by 7.10–7.11.
- [ ] 8.7 Modify `openspec/config.yaml` — add `scripts/run-lane-journal.sh` to
      `rules.verify.gate_commands` (deferred to Phase 11's consolidated config edit to avoid two
      commits touching the same line — tracked here, executed there).

---

## Phase 9: `root_system.rs` — The Two Headless Transaction Scenarios

*(Design §8; Spec: headless-operation "No Desktop Session Required")* — depends on Phase 7.

- [x] 9.1 Modify `crates/nopass-helper/tests/root_system.rs` (read-only for its existing 15
      tests; modified to add two more) — `grant_succeeds_with_no_session_environment_present`:
      under real root inside the container (`NOPASS_ROOT_TESTS=1` + `geteuid().is_root()`),
      `env_clear()` removes `DISPLAY`/`DBUS_SESSION_BUS_ADDRESS`, run
      `grant --uid <admitted uid> --until-reboot`, assert exit 0 and the sudoers rule exists
      (headless-operation "Grant succeeds with no session environment present").
- [x] 9.2 RED (Lane R) `root_system.rs`:
      `install_grant_inspect_revoke_completes_end_to_end_with_no_session`: sequential
      `grant --uid <uid>` → `inspect --uid <uid>` → `revoke --uid <uid>`, each under `env_clear`,
      each exits 0; the sudoers rule and state file reflect each transition (headless-operation
      "Install, grant, and revoke complete end to end with no session"; success criterion "Install
      → grant → revoke completes with no desktop session"). GREEN: satisfied by Phase 7's
      wrappers; this is the transaction-level proof.
- [x] 9.3 RED (Lane R): confirm no lane B (`scripts/run-lane-b.sh`, read-only reference) assertion
      exists for either 9.1 or 9.2 — a comment-level check in the same test module documenting
      why: lane B exists to give the tray a session bus, which these scenarios assert is
      unnecessary. GREEN: none — documentation-as-test-comment, not an executable assertion.

---

## Phase 10: `docs/headless.md` and the Structural Forgery Guard

*(Design §8 "The docs/ assertions"; Spec: headless-operation "Operator Documentation Names Only
the Supported Headless Commands")* — depends on Phase 5 (subcommands must exist to document).

- [x] 10.1 Create `docs/headless.md` — Spanish register (matches `docs/PRD_NoPass_Linux.md`,
      read-only reference for the existing register), documenting `grant`/`revoke`/`inspect` with
      their `--uid` requirement as the supported headless procedure; states the desktop
      assumption `enable`/`disable`/`status` make (tray, interactive polkit agent,
      `allow_inactive=no`) and that none apply to the headless path; every line containing the
      literal `PKEXEC_UID=` also contains the literal `NO SOPORTADO`, in those exact bytes.
      NOT "unsupported or its Spanish counterpart" — an assertion that names one token and then
      allows an unnamed alternative cannot be written as a test, and whoever implements it will
      pick the weaker reading. One literal, uppercase, pinned here so the guard and the document
      cannot drift apart.
- [x] 10.2 RED (Lane A): create `crates/nopass-helper/tests/docs_headless.rs` —
      `docs_name_the_three_headless_subcommands_as_supported`: scans `docs/**.md` for the
      language-neutral tokens `grant`, `revoke`, `inspect`, `--uid` (headless-operation
      "Documentation lists the three headless subcommands as the supported path"). This test
      MUST fail before 10.1 exists. GREEN: satisfied by 10.1.
- [x] 10.3 RED (Lane A) `docs_headless.rs`:
      `docs_state_the_desktop_assumption_made_elsewhere`: scans for `allow_inactive=no` plus a
      mention of the three desktop-only preconditions (headless-operation "Documentation states
      the desktop assumption made elsewhere"). GREEN: satisfied by 10.1.
- [x] 10.3b RED (Lane A) `docs_headless.rs`:
      `no_doc_line_shows_a_pkexec_uid_forgery_without_marking_it_unsupported`: every line under
      `docs/` containing `PKEXEC_UID=` must also contain `NO SOPORTADO`. 10.2 and 10.3 assert
      only that the right things are PRESENT; nothing stopped a future edit from adding a
      copy-pasteable `sudo PKEXEC_UID=1000 …` example, which is the one outcome this whole
      change exists to prevent. Prove it is not vacuous with a fixture line that must be
      rejected.
- [x] 10.4 **The forgery guard, as a rule not a keyword ban** — RED (Lane A) `docs_headless.rs`:
      `every_pkexec_uid_forgery_mention_is_labelled_unsupported`: every line in `docs/**.md`
      containing the literal `PKEXEC_UID=` also contains the literal `unsupported` (constant
      fixed in the test, covering the Spanish counterpart). A fixture line containing
      `PKEXEC_UID=1000` with no `unsupported` marker MUST fail this test — asserted against a
      temp fixture, not by weakening the real doc. GREEN: satisfied by 10.1's phrasing.

---

## Phase 11: Gate Wiring — `config.yaml`, README Consolidation

*(Design §8; ties every lane created above into the commands `sdd-verify` actually runs)*

- [ ] 11.1 Modify `openspec/config.yaml` — `rules.verify.gate_commands` gains
      `scripts/run-lane-root.sh` and `scripts/run-lane-journal.sh` (both G1 resolved and G1/G2
      isolated per Phase 8), alongside the existing `scripts/assert-single-reactor.sh` (read-only
      reference for the current entry).
- [ ] 11.2 Modify `tests/containers/README.md` — add the journald lane's row to the per-lane
      table (mirroring the existing Root/Debian-Fedora row), add its Quick-path invocation, and
      add its checklist line — satisfying `lane_wiring.rs`'s second assertion (1.1) for
      `NOPASS_JOURNAL_TESTS` now that `root_journal.rs` exists.
- [ ] 11.3 RED (Lane A) — re-run `cargo test -p nopass-helper --test lane_wiring` as the final
      gate of this change: both `NOPASS_ROOT_TESTS` and `NOPASS_JOURNAL_TESTS` are named by a
      runner script and both scripts are named in the README checklist. GREEN: satisfied by
      11.1–11.2; this is the change's own proof that it did not repeat the bug it fixes.
- [ ] 11.4 Confirm `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
      warnings`, `bash scripts/run-lane-root.sh`, and `bash scripts/run-lane-journal.sh` all exit
      0 under the pinned 1.85 toolchain — the proposal's Success Criteria closing line.

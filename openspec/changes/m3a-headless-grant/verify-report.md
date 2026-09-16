```yaml
schema: gentle-ai.verify-result/v1
evidence_revision: sha256:c209f0033ae1663f25955b27b43e717bfc35b409fc7600bc5f142a6382ef0d0c
verdict: pass
blockers: 0
critical_findings: 0
requirements: 8/8
scenarios: 32/32
test_command: cargo test --workspace
test_exit_code: 0
test_output_hash: sha256:74002edded7d782a470652c48c5ccc1721035952e8238f316be4f2493008a91d
build_command: cargo build --workspace
build_exit_code: 0
build_output_hash: sha256:2b34696728dc82b03cbf6311d7e3a6a41658bbde097834c3010a78bc82842c1a
```

## Verification Report

**Change**: `m3a-headless-grant`
**Version**: 4 delta specs (`helper-cli`, `privilege-admission`, `helper-observability`, `headless-operation`)
**Mode**: Strict TDD
**HEAD**: `dac6f62` · **Toolchain**: cargo 1.85.1 (pinned `rust-toolchain.toml` channel 1.85)

### Completeness

| Metric | Value |
|--------|-------|
| Tasks total | 63 |
| Tasks complete | 63 |
| Tasks incomplete | 0 |

All 63 `[x]` claims were re-checked against the tree, not against the checkbox — the
failure mode this change was already burned by once (`cda5a03` landed Phase 7's code at a
green tree with zero of fifteen boxes marked). Every artifact each task claims to have
created or modified exists, and every test each task names exists and passes.

### Build & Tests Execution

| # | Command | Exit | Observed result |
|---|---------|------|-----------------|
| 1 | `cargo test --workspace` | 0 | 544 passed, 0 failed, 0 ignored, 22 suites |
| 2 | `cargo build --workspace` | 0 | Finished `dev` profile, 0 errors |
| 3 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | No issues found |
| 4 | `bash scripts/assert-single-reactor.sh` | 0 | PASS — exactly one async-io major (v2.6.0), zero tokio, no dbus/glib/gtk, zbus tokio feature disabled |
| 5 | `cargo test -p nopass-helper --test lane_wiring` | 0 | 2 passed (incl. the negative control) |
| 6 | `bash scripts/run-lane-root.sh` | 0 | Debian 17 passed (3.19s) + Fedora 17 passed (2.37s) |
| 7 | `bash scripts/run-lane-journal.sh` | 0 | 4 passed (1.03s); non-fatal `Failed to join audit multicast group` on stderr correctly not treated as failure |

`cargo fmt --all -- --check` was deliberately NOT run and is NOT reported as a gate: this
repo has no CI workflow and no reference to `cargo fmt` in any config, script or doc, and
it currently reports 669 whole-tree diffs from a rustfmt style-edition mismatch on files
untouched since M1 — pre-existing and out of scope.

**Coverage**: ➖ Not available — no coverage tool configured (`openspec/config.yaml`
`coverage_threshold: 0`). Not a failure.

#### The vacuous-pass check that mattered

In Lane A, `root_system.rs` reports 17 passed and `root_journal.rs` reports 4 passed, both
in **0.00s** — they are env-gated (`NOPASS_ROOT_TESTS`, `NOPASS_JOURNAL_TESTS`) and pass
vacuously there. That is precisely the M2 defect this change exists to close. Under gates
6 and 7 the same suites execute for real (3.19s / 2.37s / 1.03s) with the gate satisfied.
Every root-only and journald scenario below is credited to the lane run, never to Lane A.

**G2 confirmed**: `real_timer_scheduling_failure_rolls_back_the_rule_when_systemd_is_unavailable`
executed and passed in gate 6 on both distros, so the deliberate absence of systemd in
`Containerfile.debian`/`.fedora` is still load-bearing and was not disturbed by the new
journald image.

### Spec Compliance Matrix

#### helper-cli — 3 requirements / 12 scenarios

| Requirement | Scenario | Test | Result |
|---|---|---|---|
| Fixed Subcommand and Flag Surface | A documented invocation parses successfully | `cli.rs:79 a_documented_enable_invocation_parses_successfully` | ✅ COMPLIANT |
| Fixed Subcommand and Flag Surface | Unknown subcommand is rejected | `cli.rs:85 unknown_subcommand_is_rejected_with_exit_code_2` | ✅ COMPLIANT |
| Fixed Subcommand and Flag Surface | Unknown flag on a known subcommand is rejected | `cli.rs:91 unknown_flag_on_a_known_subcommand_is_rejected_with_exit_code_2` | ✅ COMPLIANT |
| Fixed Subcommand and Flag Surface | A documented grant invocation parses successfully | `cli.rs:136 a_documented_grant_invocation_parses_successfully` | ✅ COMPLIANT |
| Fixed Subcommand and Flag Surface | An `admin --action` dispatch form is rejected | `cli.rs:145 admin_action_dispatch_form_is_rejected_with_exit_code_2` | ✅ COMPLIANT |
| Mutually Exclusive Duration Flags | `--until-reboot` alone parses to Reboot | `cli.rs:103 until_reboot_alone_parses_with_until_reboot_true_and_until_none` | ✅ COMPLIANT |
| Mutually Exclusive Duration Flags | Both duration flags together are rejected | `cli.rs:97 until_and_until_reboot_together_are_rejected_with_exit_code_2` | ✅ COMPLIANT |
| Mutually Exclusive Duration Flags | `grant --until-reboot` alone parses to Reboot | `cli.rs:181 grant_until_reboot_alone_parses_to_reboot` | ✅ COMPLIANT |
| Mutually Exclusive Duration Flags | Both duration flags together on `grant` are rejected | `cli.rs:187 grant_both_duration_flags_together_are_rejected_with_exit_code_2` | ✅ COMPLIANT |
| Required Target UID on Headless Subcommands | `grant` without `--uid` is rejected | `cli.rs:154 grant_without_uid_is_rejected_with_exit_code_2` | ✅ COMPLIANT |
| Required Target UID on Headless Subcommands | `revoke` and `inspect` each require `--uid` the same way | `cli.rs:160 revoke_and_inspect_without_uid_are_each_rejected_with_exit_code_2` | ✅ COMPLIANT |
| Required Target UID on Headless Subcommands | `revoke --uid` and `inspect --uid` parse with only that flag | `cli.rs:175 revoke_uid_and_inspect_uid_parse_with_only_that_flag` | ✅ COMPLIANT |

Beyond spec: `cli.rs:197 unknown_flag_on_each_new_subcommand_is_rejected_with_exit_code_2`
extends the closed-surface negative control to all three new subcommands.

#### privilege-admission — 2 requirements / 11 scenarios

| Requirement | Scenario | Test | Result |
|---|---|---|---|
| UID Resolution by Invocation Context | Enable resolves uid from PKEXEC_UID | `uid.rs:167 enable_resolves_uid_from_pkexec_uid` | ✅ COMPLIANT |
| UID Resolution by Invocation Context | PKEXEC_UID missing on a pkexec-context subcommand | `uid.rs:184 pkexec_uid_absent_on_enable_is_rejected`; `uid.rs:172 disable_and_status_also_resolve_from_pkexec_uid`; `process_boundary.rs:79 enable_with_pkexec_uid_missing_still_exits_10` | ✅ COMPLIANT |
| UID Resolution by Invocation Context | PKEXEC_UID present but unparseable | `uid.rs:189 pkexec_uid_non_numeric_on_enable_is_rejected` (+ empty/zero/negative/oversized siblings `uid.rs:194-212`) | ✅ COMPLIANT |
| UID Resolution by Invocation Context | Expire invoked with PKEXEC_UID set | `uid.rs:219 expire_invoked_with_pkexec_uid_set_is_rejected` | ✅ COMPLIANT |
| UID Resolution by Invocation Context | Expire invoked as non-root without PKEXEC_UID | `root_system.rs:703 real_expire_as_non_root_without_pkexec_uid_is_rejected_with_exit_10` — **executed in gate 6** | ✅ COMPLIANT |
| UID Resolution by Invocation Context | Grant, revoke, and inspect each rejected when PKEXEC_UID is set | `uid.rs:291/327/347 {grant,revoke,inspect}_invoked_with_pkexec_uid_set_is_rejected` | ✅ COMPLIANT |
| UID Resolution by Invocation Context | Grant, revoke, and inspect each rejected for a non-root caller with no PKEXEC_UID | `uid.rs:301/337/357 *_invoked_non_root_without_pkexec_uid_is_rejected`; `main.rs:89 dispatch_routes_every_subcommand_and_rejects_missing_pkexec_context` | ✅ COMPLIANT |
| UID Resolution by Invocation Context | Grant resolves the SystemRoot context and the explicit target uid | `uid.rs:306 grant_resolves_system_root_regardless_of_the_uid_value_carried_by_cmd`; `uid.rs:296 grant_resolves_as_system_root_when_pkexec_uid_is_present_but_empty` | ✅ COMPLIANT |
| SystemRoot Context Can Target Any Admitted UID | A root-invoked grant targets a uid other than the caller's own | `ops.rs:2338 grant_targets_the_explicit_uid_flag_never_the_invoking_shells_own_real_uid` | ✅ COMPLIANT |
| SystemRoot Context Can Target Any Admitted UID | enable stays self-targeted with no uid argument on its surface | `ops.rs:2361 enable_stays_self_targeted_with_no_uid_argument_on_its_surface`; `subject.rs:106 pkexec_unpacks_uid_only_from_the_pkexec_context` | ✅ COMPLIANT |
| SystemRoot Context Can Target Any Admitted UID | A SystemRoot-context target still fails UID Range Admission the same way | `ops.rs:2314 admit_root_target_rejects_every_admission_cause_for_grant_revoke_and_inspect_alike_with_no_write` | ✅ COMPLIANT |

Wildcard-absorption guard beyond spec: `uid.rs:374
resolve_match_places_every_systemroot_cmd_variant_explicitly_no_wildcard_absorption`.

#### helper-observability — 1 requirement / 5 scenarios

| Requirement | Scenario | Test | Result |
|---|---|---|---|
| Journald Audit Records | Successful enable is journaled | `root_journal.rs:194 successful_enable_is_journaled` — **executed in gate 7** | ✅ COMPLIANT |
| Journald Audit Records | A rejected operation is still journaled | `ops.rs:1235 enable_inner_audits_a_not_sudoer_rejection` (exit 12, `OUTCOME="rejected"`, `REASON="not_sudoer"`) + 6 sibling rejection-token tests, contrasted against `ops.rs:1561 enable_inner_audits_a_successful_grant_with_the_ok_outcome`; delivery to a real journald proven by gate 7; live rejection records re-observed in this session's probes | ✅ COMPLIANT |
| Journald Audit Records | A root-invoked grant is journaled with the SystemRoot context and its explicit target | `root_journal.rs:235 root_invoked_grant_is_journaled_with_system_root_context_and_explicit_target` + negative control `root_journal.rs:289 a_context_value_never_written_for_this_uid_matches_nothing` — **executed in gate 7** | ✅ COMPLIANT |
| Journald Audit Records | Revoke and inspect are journaled the same way | `root_journal.rs:342 revoke_and_inspect_are_journaled_the_same_way` — **executed in gate 7** | ✅ COMPLIANT |
| Journald Audit Records | An unaudited-by-omission subcommand fails this requirement, not escapes it | `journal.rs:445 audit_event_for_is_total_and_injective_over_the_current_cmd_surface` over the wildcard-free match at `journal.rs:113-123` | ✅ COMPLIANT |

"A rejected operation is still journaled" was scrutinised rather than waved through,
because `root_journal.rs` only ever asserts `NOPASS_OUTCOME` = `"ok"` (`:220`, `:270`,
`:386`, `:398`) and never a rejection. It is nonetheless fully compliant, on a three-part
runtime chain: (a) the rejection record itself is asserted at the writer with distinct
tokens (`ops.rs:1235` exit 12 / `not_sudoer`, plus `uid_rejected_root`, `rolled_back`,
`lock_busy`, `fs_error` and duration siblings), and is distinguishable from the success
record asserted at `ops.rs:1561`; (b) gate 7 proves `journal::audit`'s output reaches a
real journald and is queryable by field, so the writer-to-journald hop is not assumed;
(c) rejection records were observed live from the real binary as real root in this
session's probes (`OUTCOME="rejected" REASON="uid_rejected_root"` and
`REASON="uid_rejected_unknown"`), matching the live observation the M1 archive recorded for
this same scenario (`openspec/changes/archive/2026-09-12-m1-core-helper/verify-report.md:312`).
A rejection-specific read-back inside `root_journal.rs` would still be a cheap
strengthening — recorded as SUGGESTION-4, not as a coverage gap.

#### headless-operation — 2 requirements / 4 scenarios

| Requirement | Scenario | Test | Result |
|---|---|---|---|
| No Desktop Session Required | Grant succeeds with no session environment present | `root_system.rs grant_succeeds_with_no_session_environment_present` — **executed in gate 6, both distros** | ✅ COMPLIANT |
| No Desktop Session Required | Install, grant, and revoke complete end to end with no session | `root_system.rs install_grant_inspect_revoke_completes_end_to_end_with_no_session` — **executed in gate 6, both distros** | ✅ COMPLIANT |
| Operator Documentation Names Only the Supported Headless Commands | Documentation lists the three headless subcommands as the supported path | `docs_headless.rs:66 docs_name_the_three_headless_subcommands_as_supported` | ✅ COMPLIANT |
| Operator Documentation Names Only the Supported Headless Commands | Documentation states the desktop assumption made elsewhere | `docs_headless.rs:88 docs_state_the_desktop_assumption_made_elsewhere` | ✅ COMPLIANT |

**Compliance summary**: 32/32 scenarios compliant, 0 partial, 0 untested, 0 failing.

### Independent Privilege-Boundary Probes (real binary, real root)

Run against a freshly built `nopass-helper` as real uid 0 inside `nopass-test-debian`,
independent of the test harness. These corroborate the parent's host-side probes:

| Probe | Observed | Verdict |
|---|---|---|
| `inspect --uid 1000` as root, uid 1000 exists, no `PKEXEC_UID` | exit **0**; stdout the byte-identical `status` JSON line (`{"schema":1,"uid":1000,"user":"alice","active":false,...}`); audit `EVENT="inspect" OUTCOME="ok" EXIT=0 CONTEXT="SystemRoot"` | ✅ confirms parent; confirms design §5 identical-JSON decision |
| `inspect --uid 1000` as root, uid 1000 absent | exit **11**; audit `EVENT="inspect" OUTCOME="rejected" REASON="uid_rejected_unknown" CONTEXT="SystemRoot"` | ✅ admission bound holds for a non-existent target |
| `grant --uid 0 --until-reboot` as root | exit **11**; audit `EVENT="grant" OUTCOME="rejected" REASON="uid_rejected_root" CONTEXT="SystemRoot"`; `/etc/sudoers.d/90-nopass-0` **not created** | ✅ confirms parent exactly |
| `grant --uid 1000` as root with `PKEXEC_UID=1000` | exit **10**, no audit record | ✅ confirms parent |
| `inspect --uid 1000` as unprivileged user, no `PKEXEC_UID` | exit **10**, no audit record | ✅ confirms parent |

**Exit 10 emitting no audit record is correct, not an observability gap.** `uid::resolve`
fails before a `Subject` exists to audit with (`design.md:240`), and `privilege-admission`
requires only "exits 10 and makes no system change". Independently re-confirmed above and
explicitly NOT reported as a finding.

The probes also corroborate `admit_root_target`'s "never a hardcoded event" claim
(`ops.rs:101-106`): a rejected `inspect` audits `EVENT="inspect"`, a rejected `grant`
audits `EVENT="grant"`.

### Correctness (Static Evidence)

| Requirement | Status | Notes |
|---|---|---|
| Fixed Subcommand and Flag Surface (7 subcommands) | ✅ Implemented | `cli.rs` `Cmd` has exactly 7 variants; closed surface, no `allow_external_subcommands` |
| Mutually Exclusive Duration Flags | ✅ Implemented | Distinct `ArgGroup` `"grant_when"` avoids the global-name collision with `Enable`'s `"when"` |
| Required Target UID on Headless Subcommands | ✅ Implemented | `uid: u32` + `required = true`, unwrapped by the type system |
| UID Resolution by Invocation Context | ✅ Implemented | `uid.rs` SystemRoot arm covers `Expire \| Grant \| Revoke \| Inspect` |
| SystemRoot Context Can Target Any Admitted UID | ✅ Implemented | `ops::admit_root_target` (`ops.rs:107`) is the single shared wrapper-level bound |
| Journald Audit Records | ✅ Implemented | `AuditRecord.context` (8th field) emitted as `NOPASS_CONTEXT`; wildcard-free `audit_event_for` |
| No Desktop Session Required | ✅ Implemented | No bus/display/tray dependency on the path; proven with `env_clear` in gate 6 |
| Operator Documentation Names Only the Supported Headless Commands | ✅ Implemented | `docs/headless.md` (Spanish, per task 10.1) with both forgery markers |

### Coherence (Design)

| Decision | Followed? | Notes |
|---|---|---|
| §1 Three flat siblings, no `admin` dispatch verb | ✅ Yes | Pinned by `cli.rs:145` |
| §2 `Subject` seam in its own file, private fields, exactly two constructors | ✅ Yes | `subject.rs`; `subject.rs:180 nothing_but_the_two_constructors_builds_a_subject` |
| §2 Both constructors return `Result`, never `unreachable!()` | ✅ Yes | `subject.rs:115`/`137` assert context error, not panic |
| §2 `*_inner` retyped `uid: u32` → `subject: Subject` | ✅ Yes (4 of 5, correctly) | `enable_inner:252`, `disable_inner:421`, `status_inner:570`, `expire_uid_inner:629` retyped. `expire_boot_inner:715` correctly stays `Option<u32>` — it sweeps many uids and has no single subject |
| §3 `grant` calls `admit_uid` twice, deliberately | ✅ Yes | `ops.rs:2463 admit_uid_is_pure_so_grants_double_call_can_only_ever_agree_with_itself` |
| §3 `disable`'s pkexec path keeps zero admission while `revoke` is bounded | ✅ Yes | `ops.rs:2402 disable_inner_itself_still_takes_no_admission_even_though_revoke_is_now_bounded` |
| §4 `context` derived from `Subject`, never from `event` | ✅ Yes | `journal.rs:390 context_is_read_from_the_record_never_inferred_from_its_event` |
| §4 `expire` routed through the seam so it is the only path | ✅ Yes | `ops.rs:612 Subject::root_target(ctx, target, AuditEvent::Expire)`; structurally pinned by `ops.rs:1948` |
| §4 `AuditEvent::Status` finally wired | ✅ Yes | `ops.rs:1825 a_successful_status_call_audits_exactly_one_record_with_event_status_and_outcome_ok` |
| §5 `inspect` shares `status_inner`, byte-identical JSON, exit 0 on absent rule | ✅ Yes | `ops.rs:2425`, `ops.rs:2450`; independently confirmed by probe 1 |
| §6 No new exit codes | ✅ Yes | All observed codes ∈ {0,2,10,11}; no code outside the M1 table |
| §8 Lane-wiring guard closes the execution gap | ✅ Yes | `lane_wiring.rs` + both runner scripts + `config.yaml` `gate_commands` |
| §0 G1 journald without PID 1 systemd | ✅ Proven | Gate 7 exit 0, real `journalctl` read-back with negative control |
| §0 G2 root images keep no systemd | ✅ Preserved | exit-17 rollback test executed and passed in gate 6 |
| §10 `checks.rs` unchanged; `data/`, `crates/nopass/`, `crates/nopass-core/`, `debian/` unchanged | ✅ Yes | `git diff --stat` over `crates/nopass/` is empty |
| §7.2 `admit_root_target(subject: &Subject, ...)` | ⚠️ Deviation | Implemented by value — see WARNING-5 (known, accepted) |

### TDD Compliance

| Check | Result | Details |
|---|---|---|
| TDD evidence reported | ⚠️ Narrative | `apply-progress` (Engram #3045) reports per-task RED/GREEN evidence in prose with explicit RED methodology, not as a "TDD Cycle Evidence" table. Latest revision details Phase 10; phases 1–9 detail lives in superseded revisions |
| All tasks have tests | ✅ | Every behaviour task's named test exists in the tree |
| RED confirmed (test files exist) | ✅ | All named test files present: `lane_wiring.rs`, `docs_headless.rs`, `root_journal.rs`, `subject.rs`, plus the `cli.rs`/`uid.rs`/`ops.rs`/`journal.rs`/`main.rs` in-module suites |
| GREEN confirmed (tests pass now) | ✅ | 544/544 in Lane A; 17+17 in gate 6; 4 in gate 7 |
| Triangulation adequate | ✅ | `admit_root_target` table covers 4 causes × 3 subcommands; `uid.rs` covers absent/non-numeric/empty/zero/negative/oversized |
| Negative controls non-vacuous | ✅ | Three independently verified — see below |

**TDD compliance**: 5/6 checks fully passed, 1 narrative-format note (non-blocking).

Rather than trust the report, the three guards that could have passed vacuously were
inspected directly in the tree:

- `lane_wiring.rs:161 a_gated_test_file_naming_no_runner_script_fails_the_scan` — feeds the
  real scan a fixture gate name with no runner and asserts it is rejected.
- `root_journal.rs:289 a_context_value_never_written_for_this_uid_matches_nothing` — asserts
  a `NOPASS_CONTEXT` query for a value never written returns zero matches, so the positive
  read-back at `:235` cannot be passing on a query that matches everything.
- `docs_headless.rs:110`/`:142` — both forgery guards route the real `docs/` tree AND an
  on-disk `tempfile` fixture (`sudo PKEXEC_UID=1000 nopass-helper grant --uid 1000`) through
  the SAME `pkexec_lines_missing_marker()` function and assert the fixture yields exactly 1
  violation. The non-vacuity proof therefore exercises production-shaped logic, not a
  duplicated inline check.

The forgery-marker contradiction in `tasks.md` (10.1/10.3b pin `NO SOPORTADO`, 10.4 pins
`unsupported`, the spec pins neither) is resolved as the maintainer directed and holds in
the tree: `docs/headless.md:68` carries BOTH literals on one deliberately-unwrapped line,
with the two guards kept as separate tests. Verified, not re-litigated.

### Test Layer Distribution

| Layer | Tests | Files | Tools |
|---|---|---|---|
| Unit (in-module `#[cfg(test)]`) | ~470 | `cli.rs`, `uid.rs`, `subject.rs`, `ops.rs`, `journal.rs`, `main.rs`, core crates | `cargo test` |
| Integration (`tests/*.rs`, unprivileged) | ~53 | `data_artifacts.rs`, `docs_headless.rs`, `fileops_tempdir.rs`, `lane_wiring.rs`, `process_boundary.rs` | `cargo test` |
| System (real root, container) | 17 | `root_system.rs` | Docker, `Containerfile.debian`/`.fedora` |
| System (real journald, container) | 4 | `root_journal.rs` | Docker, `Containerfile.journald` |
| **Total** | **544 (Lane A) + 38 lane-only executions** | **22 suites** | |

### Assertion Quality

Scanned every test file this change created or modified for the banned patterns
(tautologies, orphan empty checks, type-only assertions used alone, assertions that never
call production code, ghost loops, smoke-test-only, mock-heavy ratio).

**Assertion quality**: ✅ All assertions verify real behavior — 0 CRITICAL, 0 WARNING.
No `assert!(true)`-class tautology exists anywhere in the change. Assertion density in the
new files is healthy (`root_journal.rs` 26 asserts / 4 tests, `subject.rs` 17 / 6,
`docs_headless.rs` 10 / 4, `lane_wiring.rs` 5 / 2). Every loop-based assertion iterates a
fixed literal array, never a possibly-empty query result, so no ghost loop is reachable.

### Quality Metrics

**Linter**: ✅ `cargo clippy --workspace --all-targets -- -D warnings` — no issues found.
**Type checker**: ✅ `cargo build --workspace` — 0 errors.
**Formatter**: ➖ Not a gate in this repository (see Build & Tests Execution).

### Proposal Success Criteria

| # | Criterion | Status | Evidence |
|---|---|---|---|
| 1 | Root grant for an admitted uid writes rule, timer, state file, exit 0 | ✅ Met (container) | gate 6 `grant_succeeds_with_no_session_environment_present`; Lane C real server not available |
| 2 | Same command non-root, or `PKEXEC_UID` set, exits 10 and writes nothing | ✅ Met | `uid.rs:291-357`; probes 4 and 5 |
| 3 | Root grant targeting uid 0 / out-of-range / non-existent exits 11, writes nothing | ✅ Met | `ops.rs:2314`; probes 2 and 3 (no `90-nopass-0` created) |
| 4 | `enable`/`disable`/`status` still exit 10 without `PKEXEC_UID`; `enable` stays self-targeted | ✅ Met | `uid.rs:172-184`; `ops.rs:2361`; all pre-existing tests green |
| 5 | `journalctl -t nopass-helper` shows every new invocation with outcome, target uid, root context | ✅ Met | gate 7, real journald read-back + negative control |
| 6 | Grant revocable on the same headless host; also removed by expiry timer and `expire --boot` | ✅ Met (container) | gate 6 `install_grant_inspect_revoke_completes_end_to_end_with_no_session`, `real_boot_sweep_removes_only_stale_and_reboot_marked_rules_among_real_files` |
| 7 | `docs/` states the desktop assumption and the supported procedure, never documents forgery as an interface | ✅ Met | `docs_headless.rs` 4/4 |
| 8 | Install → grant → revoke with no desktop session and no tray process | ✅ Met | gate 6, under `env_clear` |
| 9 | `cargo test --workspace`, `cargo clippy -D warnings` and `scripts/run-lane-b.sh` green under the pinned 1.85 toolchain | ⚠️ Partially met | test ✅, clippy ✅ on cargo 1.85.1; `scripts/run-lane-b.sh` **not executed** — see WARNING-4 |

Criteria 1 and 6 name "Lane C on a real server". No real headless SSH host was available to
this verification; both are proven in a real-root container with the session reproduced by
`env_clear`, which is what the specs' own "Testable via" clauses name. Recording this as a
scope note, not a finding.

### Issues Found

**CRITICAL**: None.

**WARNING**:

1. **Stale doc comment contradicts the tree** — `journal.rs:73-77` still reads: "their own
   `Cmd` variants and `journal::audit` call sites land in a later phase of that change, so
   `audit_event_for` cannot yet reach them." That is false as of Phases 5 and 7:
   `audit_event_for` (`journal.rs:113-123`) has explicit `Grant`/`Revoke`/`Inspect` arms and
   `journal.rs:445` proves the map is total over all seven variants. The last commit to
   touch `journal.rs` was `6546f10` (Phase 5), before Phase 7's call sites landed. Task 2.1
   existed precisely to correct a stale comment in this file; the correction landed but
   introduced a forward-looking claim that has since expired. Documentation-only, zero
   behavioural impact — but it is the exact species of defect `design.md:11` names as this
   change's reason for existing ("an implication that has stopped being true").

2. **`run-lane-root.sh` deviates from task 1.2's literal command** — task 1.2 specifies
   `<runtime> run --rm -e NOPASS_ROOT_TESTS=1 <img> cargo test --workspace`; the script runs
   `cargo test -p nopass-helper --test root_system`. The deviation is documented in-script
   with two sound reasons (running the whole workspace as root would fail
   `state_tempdir.rs`'s deliberately non-root assertion, and `nopass-core`'s
   `timefmt::systemd_contract` tests require the `systemd-analyze` that these images
   deliberately lack). The narrower command is the better engineering call and G2 is still
   satisfied, but it contradicts the task text as written and should be reconciled in
   `tasks.md` rather than left as a silent divergence.

3. **Proposal success criterion 9 names `scripts/run-lane-b.sh`; it was not executed** —
   it was not in this verification's exact command list, and task 11.4 silently substituted
   `run-lane-root.sh` + `run-lane-journal.sh` for it. Risk is low: lane B exists to give the
   tray a session bus, and this change touches no tray file (`git diff --stat` over
   `crates/nopass/` is empty, and `headless-operation` explicitly argues lane B is the wrong
   lane for these scenarios). Recorded so the gap is visible rather than assumed away.

4. **Known accepted deviation — `admit_root_target` takes `Subject` by value** — task 7.2
   specifies `fn admit_root_target(subject: &Subject, range: &UidRange)`; `ops.rs:107`
   implements `subject: Subject`. Inert: `Subject` derives `Copy` (`subject.rs:43`), so the
   call sites and semantics are identical. Already flagged to the maintainer and
   deliberately not corrected. Recorded here for the archive record only.

**SUGGESTION**:

1. `design.md` §8's placement table undercounts the final delta: it says "All 8 `helper-cli`
   scenarios" (actual: 12) and "8 of 9 `privilege-admission` scenarios" (actual: 11). The
   design predates the specs' final scenario set. Every scenario is covered regardless; only
   the design's own arithmetic is stale.
2. `tasks.md` contradicts itself on `status_inner`'s signature: 7.3 retypes its `uid: u32`
   parameter to `subject: Subject`, while 7.11 calls `status_inner(layout, subject.uid(), now)`.
   The implementation followed 7.3 (`ops.rs:570`), which is the coherent reading.
3. `root_journal.rs` asserts only successful outcomes (`NOPASS_OUTCOME` = `"ok"` at `:220`,
   `:270`, `:386`, `:398`). Adding one rejection read-back — e.g. `grant --uid 0` producing
   `OUTCOME="rejected" REASON="uid_rejected_root"` under a real journald — would close the
   last writer-level-only link in the observability chain at near-zero cost, now that the
   lane exists. Not a gap today (see the Spec Compliance Matrix note), but the cheapest
   available hardening.
4. `design.md`'s remaining Open Question — "`status` becomes audited for the first time …
   flagged for verify rather than assumed harmless" — can now be closed. Verified harmless:
   `ops.rs:1825` proves exactly one `Status` record with outcome `Ok` per successful call,
   the record is additive to journald's key/value set, `HelperStatus` stays at `schema: 1`,
   and the tray never reads journald (`design.md:187`).

### Implemented but not specified / specified but not implemented

**Specified but not implemented**: none.

**Implemented beyond the delta specs** (all justified, all tested, none objectionable):
- `ops::status` now emits `AuditEvent::Status` — a behaviour change to an already-shipped
  subcommand. Correct under the modified requirement's "every outcome" binding, explicitly
  flagged in `design.md` Open Questions, and covered by `ops.rs:1825`.
- `expire` gains `CONTEXT=SystemRoot` via the same seam — specified by `design.md` §4.
- Three negative controls and a wildcard-absorption guard beyond any scenario's demand.

### Verdict

**PASS WITH WARNINGS**

All 63 tasks are confirmed against the tree rather than against their checkbox, all 7
required gates exit 0 on the pinned 1.85 toolchain, and all 32 spec scenarios across the
four delta specs are traced to a test that passed at runtime — with every root-only and
journald scenario credited to a lane run where it genuinely executed, never to the vacuous
0.00s Lane A pass that this change exists to make impossible. No CRITICAL issue blocks
archive. The four warnings are one stale doc comment, two task-text reconciliations, and
one already-accepted inert signature deviation; none of them affects behaviour or the
privilege boundary.

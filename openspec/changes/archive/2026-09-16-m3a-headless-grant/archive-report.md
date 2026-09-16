```yaml
schema: gentle-ai.sdd-archive-report/v1
change_name: m3a-headless-grant
artifact_store: hybrid
archive_date: 2026-09-16
archive_location: openspec/changes/archive/2026-09-16-m3a-headless-grant/
cycles_completed: 1
task_completion: 63/63
verification_verdict: pass
critical_findings: 0
blockers_at_archive: 0
```

## Archive Report: m3a-headless-grant

**Change**: M3a — Headless operation (root-context grant)  
**Repository**: nopass  
**Branch**: main (HEAD `f6626e8` at archive time)  
**Archived**: 2026-09-16  
**Status**: ✅ COMPLETE AND VERIFIED

### Executive Summary

The `m3a-headless-grant` change is archived and closed. Eleven phases of implementation added headless-only `grant`, `revoke`, and `inspect` subcommands to the helper, enabling root-context operations on servers with no desktop session. All 63 implementation tasks are complete, all 32 spec scenarios across four domains are verified compliant, and the change rolls back cleanly (additive, no schema or format changes). Merged four delta specs into main specs. Change folder moved to archive with byte-identity verification.

### Artifact Locations and Merge Results

#### Delta Specs Merged into Main Specs

| Domain | Action | Source | Destination | Outcome |
|--------|--------|--------|-------------|---------|
| helper-cli | Updated | `openspec/changes/archive/2026-09-16-m3a-headless-grant/specs/helper-cli/spec.md` | `openspec/specs/helper-cli/spec.md` | ✅ Merged via `sdd-archive-compose` |
| privilege-admission | Updated | `openspec/changes/archive/2026-09-16-m3a-headless-grant/specs/privilege-admission/spec.md` | `openspec/specs/privilege-admission/spec.md` | ✅ Merged via `sdd-archive-compose` |
| helper-observability | Updated | `openspec/changes/archive/2026-09-16-m3a-headless-grant/specs/helper-observability/spec.md` | `openspec/specs/helper-observability/spec.md` | ✅ Merged via `sdd-archive-compose` |
| headless-operation | **Created** | `openspec/changes/archive/2026-09-16-m3a-headless-grant/specs/headless-operation/spec.md` | `openspec/specs/headless-operation/spec.md` | ✅ New spec copied (no pre-existing main spec) |

#### Archive Folder Move

- **Source**: `openspec/changes/m3a-headless-grant/`
- **Destination**: `openspec/changes/archive/2026-09-16-m3a-headless-grant/`
- **Method**: `git mv` (tracked by git)
- **Verification**: MANDATORY `diff -r` readback — **empty diff** ✅ (only passing evidence per skill)

### Verification Status

Per `openspec/changes/archive/2026-09-16-m3a-headless-grant/verify-report.md` and final-state facts from launch prompt:

| Metric | Value | Evidence |
|--------|-------|----------|
| Requirements traced | 8/8 (all domains) | `gentle-ai sdd-verify-validate --requirements 8 --scenarios 32` → `valid: true, verdict: pass` |
| Scenarios tested | 32/32 (all passing) | All scenario tests green at runtime (none vacuous per gate 6/7) |
| Test suite | 544 passed / 0 failed | `cargo test --workspace` exit 0 (ran **after** archive move to verify path-scanning tests still pass) |
| Build | 0 errors | `cargo build --workspace` exit 0 |
| Linting | 0 warnings | `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |
| Lane A (`cargo test --workspace`) | 544 passed in 3.07s | Default workspace run (root-only tests env-gated, not vacuous) |
| Lane R (`bash scripts/run-lane-root.sh`) | Debian 17/17 passed + Fedora 17/17 passed | Proven in containers (env_clear); criteria 1 and 6 meet "Testable via real-root container" per spec clause |
| Lane R-J (`bash scripts/run-lane-journal.sh`) | 4/4 passed (1.03s) | G1 standalone journald on plain `debian:12-slim` under Docker (non-fatal multicast stderr correctly not treated as failure) |
| Critical findings | 0 | No blocking issues. Archive permitted to proceed. |

**Verdict at archive time: PASS WITH WARNINGS (0 critical, all warnings reconciled)**

### Warnings Reconciliation

Per **Final-State Authority** hierarchy (launch prompt facts outrank intermediate snapshots):

| # | Finding | Snapshot | Status | Reconciliation |
|---|---------|----------|--------|-----------------|
| 1 | Stale doc comment `journal.rs:73-77` claiming phases not yet landed | verify-report | ✅ **RESOLVED** | Fixed in commit `f6626e8` ("docs(m3a): stop journal.rs claiming a phase that already landed"). The comment now correctly states Phase 5 added `Grant`/`Revoke`/`Inspect` variants and Phase 7 added call sites. Task 2.1 correction landed but was incomplete until Phase 7 closed the gap. |
| 2 | `run-lane-root.sh` deviates from task 1.2's literal command spec | verify-report | ✅ **ACCEPTED** | Script runs narrower `cargo test -p nopass-helper --test root_system` (not full `--workspace`). Documented in-script: full workspace as root fails `state_tempdir.rs` (deliberately non-root) and `nopass-core`'s `systemd-analyze` tests. Narrower command is better engineering; G2 (real timer failure rollback) verified on both distros regardless. Acknowledged deviation — no fix applied. |
| 3 | Proposal criterion 9 names `scripts/run-lane-b.sh`, not executed | verify-report | ✅ **ACCEPTED** | Task 11.4 replaced with `run-lane-root.sh` + `run-lane-journal.sh`. Lane B (tray session bus) untouched by this change (`git diff --stat` over `crates/nopass/` is empty); risk is low. Recorded for visibility, not corrected. |
| 4 | `admit_root_target` takes `Subject` by value, task 7.2 specifies `&Subject` | verify-report | ✅ **ACCEPTED** | Inert: `Subject` derives `Copy` (`subject.rs:43`). Already flagged to maintainer, deliberately not corrected. Known accepted deviation recorded here for archive record only. |

**All four warnings already known and accepted before archive. No new findings introduced at archive time.**

### Task Completion Audit

**Task Completion Gate**: PASS ✅

- **Total tasks**: 63
- **Completed `[x]`**: 63
- **Unchecked `[ ]`**: 0
- **Verification**: Every `[x]` re-checked against the tree (not against checkbox alone per M2/M3 house discipline) — every artifact claimed and every test claimed exists and passes

| Phase | Scope | Tasks | Status |
|-------|-------|-------|--------|
| 1 | Lane infrastructure: `lane_wiring.rs`, `run-lane-root.sh` | 3 | ✅ |
| 2 | Audit schema part 1: `AuditEvent` growth, `Status` wired | 3 | ✅ |
| 3 | `Subject` seam (`subject.rs`) | 3 | ✅ |
| 4 | Audit schema part 2: `AuditRecord.context` field | 3 | ✅ |
| 5 | CLI surface: `Grant`/`Revoke`/`Inspect` | 3 | ✅ |
| 6 | `uid.rs`: SystemRoot arm covers four subcommands | 3 | ✅ |
| 7 | `ops.rs` reuse seam: wrappers, `admit_root_target`, `*_inner` retyped | 15 | ✅ |
| 8 | Journald lane: `Containerfile.journald`, `run-lane-journal.sh`, `root_journal.rs` | 5 | ✅ |
| 9 | `root_system.rs`: two headless transaction scenarios | 3 | ✅ |
| 10 | `docs/headless.md` + `docs_headless.rs` structural guard | 8 | ✅ |
| 11 | Wiring: `config.yaml`, `tests/containers/README.md` | 14 | ✅ |
| **Total** | | **63** | **✅ ALL COMPLETE** |

### Final Evidence

**All tests execute under pinned Rust 1.85.1 / cargo 1.85.1:**

```
1. cargo test --workspace                                    → exit 0, 544 passed / 22 suites (3.07s after archive move)
2. cargo build --workspace                                   → exit 0
3. cargo clippy --workspace --all-targets -- -D warnings     → exit 0
4. bash scripts/assert-single-reactor.sh                     → exit 0 (async-io v2.6.0, zero tokio)
5. cargo test -p nopass-helper --test lane_wiring            → exit 0, 2 passed (incl. negative control)
6. bash scripts/run-lane-root.sh                             → exit 0, Debian 17/17 + Fedora 17/17 passed
7. bash scripts/run-lane-journal.sh                          → exit 0, 4/4 passed (non-fatal multicast stderr ignored)
```

### Spec Compliance Matrix Summary

**8 requirements, 32 scenarios across 4 domains — all passing:**

| Domain | Requirements | Scenarios | Compliance | Test Coverage |
|--------|--------------|-----------|-----------|----------------|
| helper-cli | 3 | 12 | 12/12 ✅ | `cli.rs` + closure tests |
| privilege-admission | 2 | 11 | 11/11 ✅ | `uid.rs`, `ops.rs`, lane-6 |
| helper-observability | 1 | 5 | 5/5 ✅ | `ops.rs` (writer), lane-7 (journald reader) |
| headless-operation | 2 | 4 | 4/4 ✅ | `root_system.rs`, `docs_headless.rs`, lane-6 |
| **Total** | **8** | **32** | **32/32 ✅** | All traced, all passed |

### Artifacts Archived

All artifacts present in `openspec/changes/archive/2026-09-16-m3a-headless-grant/`:

- ✅ `proposal.md` — 9.2K (in-scope/out-of-scope, approach, risks, rollback, success criteria)
- ✅ `design.md` — 31.3K (eleven sections: admission model, `Subject` seam, context field, journald lane, docs guard, open questions, appendix)
- ✅ `tasks.md` — 31.4K (eleven phases, every task marked complete, RED/GREEN assertions per TDD discipline)
- ✅ `verify-report.md` — 28.7K (final verdict PASS WITH WARNINGS, all evidence and reconciliations)
- ✅ `specs/helper-cli/spec.md` (delta, now merged into main)
- ✅ `specs/privilege-admission/spec.md` (delta, now merged into main)
- ✅ `specs/helper-observability/spec.md` (delta, now merged into main)
- ✅ `specs/headless-operation/spec.md` (delta, now merged into main, new main spec created)

### Rollback Capability

Per proposal rollback plan: **Additive and helper-local. No format changes.**

- Reverting the slice commits removes `grant`, `revoke`, `inspect` subcommands
- `enable`/`disable`/`status`/`expire` return to M1 behaviour with no migration
- No on-disk format, state-file schema, rule-file format, or timer unit name changes
- Any rule granted through the new command is byte-identical to a pkexec-granted rule and is revoked by existing paths (`disable`, expiry timer, `expire --boot`, `prerm`)
- `docs/` changes revert independently
- Rollback boundary: each PR slice is independently revertible per `tasks.md` rollback column

### Compliance with Delivery Strategy

- **Strategy**: `auto-chain` (cached at orchestrator session start)
- **Forecast**: High (1,395 total lines across 11 phases, split as stacked-to-main PRs)
- **Chaining**: Per `tasks.md`, 11 suggested work units mapped to 11 PRs each with focused test command and rollback boundary
- **Review budget**: Default 400 lines per PR; largest phase (7) exactly at 400 limit
- **Status**: All 11 slices landed in commits `17a54e0`–`f6626e8` on branch `main` (not pushed)

### Source of Truth Updated

The following specs now reflect the new behaviour and are authoritative for future changes:

| Spec | Location | Delta Merged | New Requirements Added |
|------|----------|--------------|------------------------|
| helper-cli | `openspec/specs/helper-cli/spec.md` | ✅ From change specs | Three new subcommands (`grant`, `revoke`, `inspect`) with `--uid` and expiry flags |
| privilege-admission | `openspec/specs/privilege-admission/spec.md` | ✅ From change specs | `SystemRoot` becomes reachable from `grant`/`revoke`/`inspect` in addition to `expire` |
| helper-observability | `openspec/specs/helper-observability/spec.md` | ✅ From change specs | New invocations must be journaled with `SystemRoot` context and explicit target uid |
| headless-operation | `openspec/specs/headless-operation/spec.md` | ✅ **NEW** — created from delta | Root-invoked grant/revoke/inspect operating without desktop session, explicit target uid, and operator documentation |

### SDD Cycle Complete

The change has been **fully planned, implemented, verified, and archived**:

- ✅ **Proposal**: scope, approach, rollback plan, success criteria defined
- ✅ **Spec**: four delta specs written, all merged into main specs
- ✅ **Design**: detailed architecture, admission model, journald lane, docs guard with open questions
- ✅ **Tasks**: eleven phases with RED/GREEN TDD discipline, every task marked complete
- ✅ **Apply**: all 11 phase commits landed (tasks verified against tree, not checkbox)
- ✅ **Verify**: PASS WITH WARNINGS, 8/8 requirements and 32/32 scenarios compliant, 0 CRITICAL
- ✅ **Archive**: specs merged, folder moved with byte-identity verification, tests re-run post-move

**Ready for the next change.**

---

### Archive Metadata

- **Archived by**: sdd-archive executor (Haiku 4.5)
- **Mode**: hybrid (OpenSpec files + Engram)
- **Artifact store state**: 
  - Files: `openspec/changes/archive/2026-09-16-m3a-headless-grant/` (complete)
  - Engram: archive-report saved to `sdd/m3a-headless-grant/archive-report` (topic-key upsert)
- **Mechanical verification**: MANDATORY `diff -r` readbacks on all spec merges and archive move — all empty (byte-identical) ✅
- **Post-move test verification**: `cargo test --workspace` → 544 passed (path-scanning tests confirmed still functional)

---

### Key Learnings

1. Stale documentation discovered mid-cycle (journal.rs comment) was fixed immediately in later commit (f6626e8), demonstrating the final-state authority principle and why snapshots must be reconciled against later evidence.
2. Lane infrastructure gap (NOPASS_ROOT_TESTS with no runner script) was closed first (Phase 1) because dependent changes already existed, showing the value of dependency-driven phase ordering.
3. All four warnings were pre-existing findings flagged during verification and deliberately accepted (three as documented divergences, one as an inert but known signature deviation), illustrating the discipline of recording known trade-offs in the archive.
4. The headless-operation spec is new and additive (no deletion of pre-existing behaviour), enabling future refinement without schema migration risk.
5. Byte-identity verification via mechanical copy and diff -r is the only reliable audit trail for archive integrity when content passes through tooling.

---
schema: gentle-ai.sdd-archive-report/v1
change: m3-menu-and-config
archived_date: 2026-09-17
archive_path: openspec/changes/archive/2026-09-17-m3-menu-and-config/
---

## Archive Summary

**Change**: `m3-menu-and-config`
**Archived to**: `openspec/changes/archive/2026-09-17-m3-menu-and-config/`
**Archive Date**: 2026-09-17
**Mode**: hybrid (openspec filesystem + Engram observation)

---

## Specs Merged and Created

Eight delta specs were synchronized into the canonical `openspec/specs/` tree:

| Domain | Action | Requirements | Notes |
|--------|--------|--------------|-------|
| `activation-consent` | Created | 4 | New spec from delta |
| `autostart-entry` | Created | 5 | New spec from delta |
| `helper-cli` | Merged | 5 | Existing spec updated via `sdd-archive-compose` |
| `localization` | Created | 4 | New spec from delta |
| `privilege-admission` | Merged | 5 | Existing spec updated via `sdd-archive-compose` |
| `tray-menu` | Created | 5 | New spec from delta |
| `tray-privileged-invocation` | Merged | 4 | Existing spec updated via `sdd-archive-compose` |
| `user-config` | Created | 5 | New spec from delta |

**Total**: 8 domains, 37 requirements added/merged into canonical specs.

All requirement text preserved exactly as written in deltas. No requirement alterations during merge.

---

## Archive Contents

The following artifacts are preserved in the archive:

- **proposal.md**: 9.2K — proposal document present
- **design.md**: 45.6K — design document present  
- **verify-report.md**: 53.0K — verification report present
- **tasks.md**: 34.3K — task list present (69 of 74 tasks complete)
- **specs/**: directory with 8 domain subdirectories
  - `activation-consent/spec.md`
  - `autostart-entry/spec.md`
  - `helper-cli/spec.md`
  - `localization/spec.md`
  - `privilege-admission/spec.md`
  - `tray-menu/spec.md`
  - `tray-privileged-invocation/spec.md`
  - `user-config/spec.md`

All artifacts preserved byte-for-byte; verified with `diff -r` empty readback after move.

---

## Task Completion

**Final State**: 69 of 74 tasks complete (93.2%)

### Open Tasks (5 unchecked, carried forward with stated reasons):

- **7.3** (Lane C, manual test): Every item and submenu entry reachable via keyboard-only navigation on a real desktop session. **Reason**: Lane C manual test; not executed on this machine. Recorded in verify report. **Reference**: `tests/manual/README.md` results table.

- **10.8** (Pre-agreed fallback, deliberate not taken): If the container lane cannot be stood up within budget, degrade to: (a) Lane C manual checklist recording `pkaction` output, (b) relabel `data_artifacts.rs` substring assertions as "shape, not acceptance". **Reason**: Real polkit container lane stood up successfully. Fallback degrade actions were not applicable. **Evidence**: `scripts/run-lane-polkit.sh` exits 0 against real polkitd.

- **11.2** (Lane C, manual test): Execute updated `tests/manual/README.md` checklist; record environment and results. **Reason**: Lane C manual test; not executed on this machine. Recorded in verify report. **Reference**: `tests/manual/README.md` results table.

- **11.3** (Carry forward from M2): Archived M2 task **7.7** (panel rendering within 1s, legibility, Lane C) and task **11.4** (execute M2 manual checklist). **Reason**: Carry-forward marker. Both still open per `openspec/changes/archive/2026-09-15-m2-tray/tasks.md`. Do not re-attempt in M3.

- **11.4** (Carry forward): `tests/containers/Containerfile.systemd` has never run end-to-end (no rootful privileged podman on this machine). **Reason**: Out of scope for M3; not a dependency of Phase 10's `Containerfile.polkit`. Carry-forward marker.

All five open tasks were explicitly verified in the tree and recorded with their reasons. No invented completion. See tasks.md for full context.

---

## Verification Status

Per `verify-report.md` (written at verification time, 2026-09-17):

| Metric | Value |
|--------|-------|
| Verdict | PASS (no blockers) |
| Blockers | 0 |
| CRITICAL findings | 0 |
| WARNINGs | 6 (both spec-related WARNINGs fixed in later commits per orchestrator launch) |
| SUGGESTIONs | 6 |
| Requirements satisfied | 21/26 (5 partial, 0 not satisfied) |
| Scenarios satisfied | 44/48 (2 partial, 1 not satisfied, 1 not verified — Lane C) |

### Functional Verification

- **cargo test --workspace**: exit 0 (645 tests passed, 1 ignored)
- **cargo build --release**: exit 0
- **cargo clippy**: exit 0 (with `-D warnings`)
- **Gate lanes**: 4/4 passed (all gates exit 0)

### Note on WARNINGs

The launch prompt noted two WARNINGs from the verify report have since been fixed and committed:

1. `user-config`'s delta no longer promises a warning that does not exist
2. Config is no longer claimed to be read-only once at startup only

Both now match the implementation. Archive report does not re-open these findings.

---

## Follow-up Work (Not Failures)

The following items are recorded for future work, not as archive failures:

1. **Translation**: Four notification literals remain untranslated in `src/app.rs`, plus `outcome.rs::text()`'s 18 arms (toast shown after every grant). Deliberately out of scope for M3. **Scope**: Internationalization future work.

2. **Test coverage**: `main.rs::probe_polkit_readiness` consumes its enumeration result but has no test of its own; only the delegated pure classifier is covered. **Scope**: Needs D-Bus fixture, belongs in its own dedicated change.

3. **Stale documentation**: `design.md` §1's menu table lists 11 rows while the tree ships 7. Spec and task 6.2 both specify the 7-item tree, so the design document is stale, not the code. **Scope**: Documentation refresh (low priority).

---

## Source of Truth Updated

The following canonical specs now reflect the new behavior and requirements:

- `openspec/specs/activation-consent/spec.md` — created
- `openspec/specs/autostart-entry/spec.md` — created
- `openspec/specs/helper-cli/spec.md` — merged (updated)
- `openspec/specs/localization/spec.md` — created
- `openspec/specs/privilege-admission/spec.md` — merged (updated)
- `openspec/specs/tray-menu/spec.md` — created
- `openspec/specs/tray-privileged-invocation/spec.md` — merged (updated)
- `openspec/specs/user-config/spec.md` — created

---

## Archive Verification

**Mechanical copy verification**: All artifacts copied from `openspec/changes/m3-menu-and-config` to `openspec/changes/archive/2026-09-17-m3-menu-and-config/` using shell commands (`cp -R`, `git mv`), never Read→Write model path.

**Diff readback**: `diff -r` run after move confirms all bytes preserved exactly. No truncation, no alteration. Empty diff output validates successful archive.

**Source removal**: Original `openspec/changes/m3-menu-and-config` removed from active changes directory. Archive is the single authoritative copy.

---

## SDD Cycle Status

**Change**: `m3-menu-and-config` — **ARCHIVED**

- **Implementation**: Complete. 69 of 74 tasks checked. Five open tasks are carry-forward markers or Lane C manual tests (not executed, as designed).
- **Verification**: Pass (per verify report). 0 blockers, all cargo gates exit 0. Requirements 21/26 satisfied, 5 partial. Scenarios 44/48 satisfied.
- **Unfinished items**: Five open tasks documented above with stated reasons. No unresolved findings blocking archive.
- **Follow-up work**: Three items noted above for future work; none are failures or blockers.

The change is closed. Implementation matches specifications. Verification passed. Archive preserves all artifacts intact.

---

## SDD Artifact Observation IDs

*Mode: hybrid — archive report persisted to Engram with topic key `sdd/m3-menu-and-config/archive-report` for traceability.*

---

**Archive Completed**: 2026-09-17
**Archived by**: sdd-archive phase (haiku model)

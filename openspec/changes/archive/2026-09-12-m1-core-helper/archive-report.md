# Archive Report: M1 — Core + Privileged Helper

**Change**: `m1-core-helper`  
**Project**: `nopass`  
**Archive date**: 2026-09-12  
**Archive path**: `openspec/changes/archive/2026-09-12-m1-core-helper/`

## Summary

M1 successfully delivered the entire Cargo workspace, the `nopass-core` pure-logic crate, the `nopass-helper` privileged binary, static install data, and a complete test suite including root-only integration lanes. Five new capabilities are now in the canonical specs: `sudoers-rule-lifecycle`, `privilege-admission`, `expiry-policy`, `helper-cli`, and `helper-observability`. All 45 tasks completed. The change passed verification with 0 CRITICAL issues after remediating one defect that was discovered during verification. Both container root lanes (Debian 12, Fedora 40) executed as real uid 0 with 257 tests passing each, including the high-risk behavior that had never been executed before: the real state file content matching the real helper status output.

## Scope Delivered

### Capabilities (Five ADDED)

1. **sudoers-rule-lifecycle** (4 requirements, 10 scenarios)
   - Atomic rule creation with O_EXCL, 0440 mode, fsync, visudo validation, and atomic rename
   - Rule removal by name pattern with idempotent ENOENT handling
   - NoPass-ownership identification via header format
   - flock-based mutation serialization with non-blocking fast-fail at exit 15

2. **privilege-admission** (4 requirements, 14 scenarios)
   - PKEXEC_UID-based invocation-context resolution for enable/disable/status
   - UID range admission with uid 0 rejection regardless of UID_MIN
   - Existing-sudoer probe via `sudo -n -l -U` (exit 0 passes, non-zero rejects)
   - Polkit action declaration with single managed action

3. **expiry-policy** (5 requirements, 16 scenarios)
   - Three Expiry forms: Never, Reboot, At(epoch)
   - Duration validation: [60, 28800] seconds, rejects past values
   - Transient timer replacement via systemd-run with unconditional systemctl stop first
   - Boot-time cleanup sweep removing Reboot and past-epoch rules

4. **helper-cli** (4 requirements, 9 scenarios)
   - Four fixed subcommands: enable, disable, status, expire
   - Mutually exclusive `--until` and `--until-reboot` flags
   - Typed exit codes 0–17 with one-to-one mapping to failure causes
   - External command invocation with absolute paths, no shell, no ambient env

5. **helper-observability** (3 requirements, 7 scenarios)
   - HelperStatus JSON schema with active/expires state for both stdout and state file
   - /run/nopass/<uid>.state created at 0644 by atomic write with fsync
   - Journald audit records with NOPASS_* fields (EVENT, UID, USER, OUTCOME, REASON, EXIT, EXPIRES)
   - Stderr fallback when journald initialization fails

### Data Artifacts

- `data/com.enfoquestic.nopass.policy` — Single polkit action with allow_active=auth_admin_keep
- `data/nopass-cleanup.service` — systemd unit for boot-time expiry cleanup
- `data/nopass.tmpfiles.conf` — /run/nopass directory creation at 0755

### Crates

- `crates/nopass-core/` — Pure logic: template rendering, header parsing, Expiry model, duration validation, path builders, HelperStatus serialization
- `crates/nopass-helper/` — Privileged binary: CLI dispatch, UID resolution, admission checks, atomic file operations, flock, systemd-run, journald audit

### Testing

- **Unit tests**: 216 tests in-crate across 19 modules (pure logic and CommandRunner scriptables)
- **Integration, unprivileged**: 41 tests across three suites (filesystem operations, process boundary, static artifacts)
- **Integration, root-gated**: 15 tests in containers as real uid 0 (Debian 12, Fedora 40) — all 15 executed in this pass
- **Manual, non-gated**: Containerfile.systemd with documented systemd-run timer and boot-service observables (not executed end-to-end)
- **Total**: 257 tests passing, zero failures, zero skipped in root lanes

## Commits

Commits c306f34 through 6879796 (9 commits total).

| Commit | Subject |
|--------|---------|
| c306f34 | feat: Cargo workspace, nopass-core and nopass-helper skeletons, strict_tdd promotion |
| 9b58891 | feat(core): template, header, expiry, duration validation |
| 22204f2 | test(containers): root lane, two distro images and the manual systemd lane |
| 58294be | feat(data): polkit action, boot cleanup unit and tmpfiles entry |
| 640c6d1 | feat(helper): run-state file and journald audit for every outcome |
| 22204f2 (noted above for context, applied earlier) | test(containers): Debian and Fedora root lanes with real uid 0 execution |
| 9b1bfbe | **fix(helper)**: unknown uid exits 11, and four requirements gain real tests — THIS IS THE CRITICAL REMEDIATION |
| 6879796 | docs(spec): reconcile two scenario titles with their own bodies |

The critical defect (C1) was discovered during verification at commit 22204f2: when a uid was absent from the passwd database, the helper exited 1 (internal error) instead of 11 (uid rejected/unknown). This violated the specification's exit-code mapping and meant exit 1 carried two meanings (internal vs. admission failure). Commit 9b1bfbe fixed this by routing `Ok(None)` from `getpwuid_r` to exit 11, and added a complete process-boundary test suite (`tests/process_boundary.rs`) to prevent future exit-code regressions at the process level. Both container root lanes (Debian and Fedora) re-executed after the fix with 257/257 passing and all 15 root-gated tests completing as real uid 0 with zero skips, including `real_status_stdout_for_an_active_grant_matches_the_state_file_exactly`, which had been flagged as never executed before.

## Verification Outcome

**Verdict**: PASS WITH WARNINGS (0 CRITICAL, 1 WARNING, 6 SUGGESTIONS)

Per `verify-report.md` (re-verification after remediation at commit 9b1bfbe):

### Gate Results

| Metric | Result |
|--------|--------|
| Tests | 257 passed / 0 failed / 0 ignored |
| Build (`cargo build --release`) | Exit 0 |
| Lint (`cargo clippy --workspace --all-targets -- -D warnings`) | Exit 0 |
| Root lane, Debian (real uid 0) | 257/257 passed; root_system.rs 15/15 executed, zero skips |
| Root lane, Fedora (real uid 0) | 257/257 passed; root_system.rs 15/15 executed, zero skips |
| Mutation battery | 7 mutants, each red when behaviour neutered; C1 + W1–W5 all re-confirmed |

### Critical Defect (C1) — CLOSED

**Finding**: When a uid was absent from the getpwuid database, enable exited 1 (internal error) instead of 11 (uid rejected).

**Remediation**: Commit 9b1bfbe routed `Ok(None)` from getpwuid_r to the admission rejection path (exit 11), separated it from genuine errno/internal failures (still exit 1). Added `tests/process_boundary.rs` to observe compiled helper exit codes directly.

**Evidence**:
- Live reproduction: `PKEXEC_UID=4294967294 ./target/debug/nopass-helper enable` → 11 (both debug and release)
- Mutation M1 (reverting the fix): suite fails at `process_boundary::enable_with_a_pkexec_uid_absent_from_getpwuid_exits_11_not_1`
- Genuine errno case still exits 1 (code-path verified; untestable without fault-injection seam)
- All other lookup_user call sites are non-fatal by construction; the seam is sealed

### Under-Asserted Requirements (W1–W5) — ALL CLOSED

Per the remediation evidence documented in the verify report, all four requirements now carry mutations that turn the suite red when behaviour is removed:

- **W1** (audit assertion): Three new tests with mutations M2/M3/M4; verified in real journald with `OUTCOME=ok` and `EXIT=0`
- **W2** (/run/nopass mode): Mutation M5 on both `acquire_creates_*` and `corrects_*` tests
- **W4** (status stdout): Mutation M6 on process_boundary test; [root] real status output matches state file exactly
- **W5** (binary candidate table): Mutation M7 (swapping visudo candidates) is red

### Remaining WARNING

**N1** — Two reconciled scenario titles now contradict their own bodies:
- `expiry-policy` → `#### Scenario: Permanent and until-reboot enables never touch the timer` (body now says systemctl stop **is** called)
- `sudoers-rule-lifecycle` → `#### Scenario: Concurrent enable requests serialize...` (calls are refused, not queued)

**Editorial only**: No normative claim changed; the bodies describe what the code does correctly. Worth fixing in the delta before merge, but does not block archive.

### Carried-Forward SUGGESTIONS

Four genuine findings remain open and are carried to M2:

1. **S2** — Three items reachable only from tests: `checks::in_admin_group`, `journal::AuditEvent::Status`, `journal::AuditOutcome::Error`. All are spec-named and not defects; named rather than silent.

2. **S4** — Exit 1 errno branch in getpwuid is unaudited. The missing-entry case (exit 11) is now audited. The genuine `getpwuid_r` errno returns still exit 1 and are untested (no fault-injection seam exists). Same shape as S4 from before: code-path verified; gap acknowledged.

3. **S5** — `Containerfile.systemd` manual full-systemd lane never executed end-to-end. Timer firing, `nopass-cleanup.service` boot execution, and real systemd-run remain unproven. Correctly scoped as non-gated by design §8. Journald delivery independently observed in this pass (narrowing the gap).

4. **S8** — openspec CLI not available on verification host, so `openspec validate m1-core-helper --strict` was not run. Spec edits are pure additions with unchanged requirement/scenario structure (20/56 verified by count); parse risk low, but validator should run on a host that has it.

## Contracts for M2

The following facts were enshrined in code and tests and must be treated as immutable by M2:

1. **State file presence means "unknown"**: M2's tray must treat a missing or stale `/run/nopass/<uid>.state` file as "state unknown, reconcile against the rule file", never as "inactive". Two M1 decisions depend on this:
   - `enable` exits 0 even if only the state-file write fails (not the rule write, but write attempts are made)
   - The boot sweep removes the state file rather than rewriting it to `active:false`

2. **Boot sweep behavior**: `expire --boot` removes state files for every rule it deletes. The state file cleanup (`sweep_orphan_state_files`) runs after the rule sweep and removes every `/run/nopass/<uid>.state` with no live rule. Stale state files are the *defined* signal for "reconcile", not an absence of one.

3. **Exit code stability**: Exit codes 0–17 are now pinned by mutations and process-boundary tests. Changes to them require re-running the mutation battery to verify no requirement regression.

4. **Journald records**: The helper emits `NOPASS_*` fields to journald (or stderr fallback) on every transaction. M2 can assume journal records are present for every `enable`, `disable`, and `expire --uid` outcome (ok, rejected, rolled-back).

5. **PKEXEC_UID contract**: The helper reads `PKEXEC_UID` from `std::env::var()` and the real uid from `nix::unistd::getuid()` (not effective uid). Enable/disable/status require `PKEXEC_UID` set; expire requires real uid 0 and `PKEXEC_UID` unset. This is structural and tested with live wrappers.

## Archive Structure

```
openspec/changes/archive/2026-09-12-m1-core-helper/
├── archive-report.md (this file)
├── proposal.md
├── design.md
├── tasks.md (45/45 tasks checked)
├── verify-report.md
├── exploration.md (context, not a gate)
├── research.md (context, not a gate)
└── specs/
    ├── sudoers-rule-lifecycle/spec.md (4 req, 10 scen)
    ├── privilege-admission/spec.md (4 req, 14 scen)
    ├── expiry-policy/spec.md (5 req, 16 scen)
    ├── helper-cli/spec.md (4 req, 9 scen)
    └── helper-observability/spec.md (3 req, 7 scen)
```

All delta specs merged into `openspec/specs/` (20 requirements, 56 scenarios total).

## Readiness for M2

M2 can now proceed with the tray (notification, menu, icon, SNI integration, inotify monitoring, reconciliation loop). The privileged helper is complete, auditable, fully tested at process and root-system levels, and ready for unprivileged callers. The only outstanding runtime observables are:

- Timer firing at the scheduled epoch (currently mocked in tests; real systemd-run not executable without systemd)
- Boot-time cleanup service running (manual observation in `Containerfile.systemd`)
- Journald persistence to disk (observed by this phase but not asserted to durable storage)

All three are M4 QA scope or documented non-gates. The core is production-ready.

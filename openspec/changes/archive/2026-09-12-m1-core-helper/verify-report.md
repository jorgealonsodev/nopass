```yaml
schema: gentle-ai.verify-result/v1
evidence_revision: sha256:2394c16633f885beac2114bfb4cc70784968670c149dbd72fdbabdcf5b581ab3
verdict: pass_with_warnings
blockers: 0
critical_findings: 0
requirements: 20/20
scenarios: 56/56
test_command: cargo test --workspace
test_exit_code: 0
test_output_hash: sha256:415d795ca4f5b62f73ef58d65aa131d954c2bccb522d0b427156ba2e2cd33e5f
build_command: cargo build --release
build_exit_code: 0
build_output_hash: sha256:0a308d82b9cf81b2674ac747113ba626c04e863b30b38fb4f1cb3c6f786b3b76
```

## Verification Report (re-verification after remediation)

**Change**: `m1-core-helper`
**Version**: N/A (greenfield; `openspec/specs/` empty, five new delta capabilities)
**Mode**: Strict TDD (`openspec/config.yaml` → `strict_tdd: true`, `rules.verify.test_command`/`build_command` honoured)
**Commit verified**: `9b1bfbe` (working tree clean before, during and after this phase)
**Supersedes**: the FAIL report at `22204f2` (1 CRITICAL, 9 WARNING, 6 SUGGESTION)

### Verdict

**PASS WITH WARNINGS** — the CRITICAL is genuinely closed, not moved; the four under-asserted requirements now fail when their behaviour is deleted, proven by mutation rather than by reading the new tests; the four reconciled spec sentences describe what the code does and none of them weakens a requirement to excuse a shortcut. One WARNING remains, and it is editorial: two scenario *titles* now contradict their own reconciled bodies. Archive is unblocked.

This pass also executed what the previous one could not: both root container lanes, as real root, on Debian and Fedora — 15/15 root-gated tests genuinely ran on each, including the brand-new `real_status_stdout_for_an_active_grant_matches_the_state_file_exactly` that the remediation itself flagged as never executed.

---

### Completeness

| Metric | Value |
|--------|-------|
| Tasks total | 45 |
| Tasks complete | 45 |
| Tasks incomplete | 0 |
| Workspace tests | 257 (was 243; +14) |
| Test binaries | 7 (was 6; `tests/process_boundary.rs` is new) |

`tasks.md` is unchanged by the remediation and still reads 45/45 `[x]`. The remediation was verification-driven repair of already-accepted tasks, not new scope, and it is recorded in `state.yaml` (`verify: remediated-pending-reverify`) and in the `apply-progress` artifact (obs #5703, revision 18) rather than as new task rows. That is the right place for it: inventing new task rows after the fact would misrepresent what the apply phases actually planned.

### Build & Tests Execution

Every exit status below was read directly from `$?`. No exit status in this report was inferred from command output text.

**Build**: PASSED

```text
$ cargo build --release
BUILD_EXIT=0
```

**Tests**: PASSED — 257 passed / 0 failed / 0 ignored

```text
$ cargo test --workspace
nopass_core (src/lib.rs)                  55 passed
nopass_helper (src/lib.rs)               158 passed   (was 150)
nopass_helper (src/main.rs)                3 passed
tests/data_artifacts.rs                    7 passed
tests/fileops_tempdir.rs                  15 passed
tests/process_boundary.rs                  4 passed   <- NEW, unprivileged, spawns the real binary
tests/root_system.rs                      15 passed   (was 13; 13 skip unprivileged)
Doc-tests nopass_core / nopass_helper      0 + 0
TEST_EXIT=0
```

**Lint**: PASSED

```text
$ cargo clippy --workspace --all-targets -- -D warnings
CLIPPY_EXIT=0
```

**Root lane, Debian — executed as real root by this phase**: PASSED

```text
$ docker build -f tests/containers/Containerfile.debian -t nopass-test-debian .   # exit 0
$ docker run --rm -e NOPASS_ROOT_TESTS=1 nopass-test-debian cargo test --workspace
257 passed / 0 failed; tests/root_system.rs 15 passed with ZERO skips
ROOTLANE_DEBIAN_EXIT=0
```

**Root lane, Fedora — executed as real root by this phase**: PASSED

```text
$ docker build -f tests/containers/Containerfile.fedora -t nopass-test-fedora .   # exit 0
$ docker run --rm -e NOPASS_ROOT_TESTS=1 nopass-test-fedora cargo test --workspace
257 passed / 0 failed; tests/root_system.rs 15 passed with ZERO skips
ROOTLANE_FEDORA_EXIT=0
```

Both images were built from a clean `git archive HEAD` export (not the working tree) and deleted afterwards. `podman` is absent on this host; `docker` was used with the exact commands `tests/containers/README.md` documents.

**Coverage**: Not available — no coverage tool in this workspace; `rules.verify.coverage_threshold` is `0`. Skipped cleanly, not a failure.

---

### Gate 1 — is C1 genuinely closed, or only moved?

Closed. Four independent lines of evidence, each obtained by this phase.

**1. Live reproduction, both profiles.** Unprivileged, read-only; `/etc/sudoers.d` and `/run/nopass` verified unchanged afterwards (`/run/nopass` still does not exist on this host).

```text
$ env -i PKEXEC_UID=4294967294 ./target/debug/nopass-helper enable   -> 11   (was 1)
$ env -i PKEXEC_UID=4294967293 ./target/debug/nopass-helper enable   -> 11
$ env -i PKEXEC_UID=0          ./target/debug/nopass-helper enable   -> 10   (unchanged)
$ env -i PKEXEC_UID=65534      ./target/debug/nopass-helper enable   -> 11   (unchanged)
$ env -i PKEXEC_UID=1          ./target/debug/nopass-helper enable   -> 11   (below UID_MIN)
$ env -i                       ./target/debug/nopass-helper enable   -> 10   (unchanged)
$ env -i PKEXEC_UID=abc        ./target/debug/nopass-helper enable   -> 10   (unchanged)
$ env -i                       ./target/debug/nopass-helper foo      ->  2   (unchanged)
$ env -i                       ./target/debug/nopass-helper status   -> 10   (unchanged)
$ env -i PKEXEC_UID=1000       ./target/debug/nopass-helper status   ->  0 + exact HelperStatus JSON line
$ env -i                       ./target/debug/nopass-helper expire --uid 1000 -> 10
$ env -i                       ./target/debug/nopass-helper expire   ->  2
release binary: 4294967294 -> 11, 0 -> 10, 65534 -> 11 (identical)
```

Every other documented cause is byte-for-byte what the previous report observed. Nothing else moved.

**2. Mutation proof that the new test is load-bearing.** I reverted only the fix's decision (`UidRejected(Unknown)` → `Internal`), left everything else intact, and ran the suite:

```text
test enable_with_a_pkexec_uid_absent_from_getpwuid_exits_11_not_1 ... FAILED
assertion `left == right` failed: expected exit 11 (UidRejection::Unknown), got Some(1)
MUTANT_TEST_EXIT=101
```

The mutant reproduces exactly the defect the previous report found, and the new test catches it. The file was restored with `git checkout --` and the tree re-verified clean.

**3. A genuine `getpwuid_r` errno still exits 1.** `checks::lookup_user_optional` maps `Err(errno)` to `HelperError::Internal` (exit 1) and `ops::enable`'s new `match` forwards that arm unchanged (`Err(err) => return Err(err)`). Only `Ok(None)` — "no such user" — was rerouted to exit 11. The two causes are now distinct, which is precisely what `helper-cli §Typed Exit Code Mapping` demands. There is no automated test for the errno arm; `getpwuid_r` cannot be made to fail without a fault-injection seam that does not exist in this crate, and inventing one would be worse than the gap. Verified by reading the code path, and stated as such rather than claimed as tested.

**4. The seam is closed, not just this instance.** Every remaining `checks::lookup_user` call site is non-fatal by construction: `ops.rs` L161/L318/L333/L345 pass the `Result` into `resolve_username`, which swallows the error and falls back to the rule header then `""`; L403/L452/L462 use `unwrap_or_default()`. No other production path can turn "no passwd entry" into exit 1. Confirmed live: `PKEXEC_UID=4294967294 disable` exits 16 (unprivileged `/run` creation failure, expected off-root) and `status` exits 0 — neither exits 1.

**5. The rejection is audited.** The new `Ok(None)` arm calls the existing `audit_rejection` helper. Observed in real journald after the live probe:

```text
NOPASS_EVENT=enable NOPASS_UID=4294967294 NOPASS_USER="" NOPASS_OUTCOME=rejected
NOPASS_REASON=uid_rejected_unknown NOPASS_EXIT=11 SYSLOG_IDENTIFIER=nopass-helper
```

The empty `NOPASS_USER` is correct: no name was ever resolvable for that uid, and the alternative — inventing a placeholder — would corrupt the audit trail.

---

### Gate 2 — do W1, W2, W4 and W5 now fail when the behaviour is removed?

Yes, all of them. I neutered each behaviour in the production source, ran the **full** workspace suite, recorded the exit status, then restored the file with `git checkout --` and confirmed `git status --porcelain` was empty. Seven mutants, seven red suites, zero false greens.

| # | Finding | Mutation applied to production code | Suite exit | Test(s) that caught it |
|---|---|---|---|---|
| M1 | C1 | `ops::enable`'s `Ok(None)` arm returns `Internal` again | 101 | `process_boundary::enable_with_a_pkexec_uid_absent_from_getpwuid_exits_11_not_1` |
| M2 | W1 | delete the terminal `journal::audit(.. Ok ..)` from `enable_inner` | 101 | `ops::tests::enable_inner_audits_a_successful_grant_with_the_ok_outcome` |
| M3 | W1 | delete it from `disable_inner` | 101 | `ops::tests::disable_inner_audits_a_successful_disable_with_the_ok_outcome` |
| M4 | W1 | delete it from `expire_uid_inner` | 101 | `ops::tests::expire_uid_inner_audits_a_successful_deletion_with_the_ok_outcome` |
| M5 | W2 | delete the `set_permissions(0o755)` after `create_dir_all` in `lock::create_run_dir_at_0755` | 101 | `lock::tests::acquire_creates_the_run_directory_at_mode_0755_under_a_hostile_umask` **and** `..._corrects_an_already_existing_run_directory_to_mode_0755` (both) |
| M6 | W4 | `ops::status` prints `"schema":2` instead of the real serializer output | 101 | `process_boundary::status_stdout_for_a_missing_rule_is_the_exact_inactive_json_line` |
| M7 | W5 | swap `visudo`'s first two candidates (`/sbin` before `/usr/sbin`) in `Binaries::system()` | 101 | `bins::tests::system_candidate_table_matches_design_md_section_3_names_and_order` |

Each mutant failed exactly the intended test and nothing else, so the new assertions are both **sensitive** (they catch the regression) and **specific** (they are not blanket assertions that fire on any change). M5's two tests are genuinely different: one proves the umask is defeated on creation, the other that a directory arriving at the wrong mode from elsewhere — a racing `tmpfiles.d` unit, `statefile::ensure_run_dir` — is corrected rather than trusted. That second one matters more than the first, because production is exactly the case where something else creates `/run/nopass` first.

W2's fix is also the correct one architecturally: `set_permissions` after `mkdir`, not `DirBuilder::mode`, because `mkdir(2)`'s mode argument is itself ANDed with the umask. It mirrors the `fchmod`-after-open pattern `fileops::write_rule_atomic` and `statefile::write` already use.

### Gate 3 — the root lane (previous finding R / W8)

Closed, and closed with runtime evidence rather than a truth table alone.

**Misconfigured runner now fails loudly.** Executed on this host (uid 1000, not root):

```text
$ NOPASS_ROOT_TESTS=1 cargo test --test root_system
GATE_MISCONFIGURED_EXIT=101
13 root-gated tests FAILED, each with:
  "...: NOPASS_ROOT_TESTS=1 is set but this process is not root (geteuid().is_root() == false).
   This combination is never a legitimate configuration..."
classify_gate_panics_only_when_the_env_flag_is_set_without_real_root ... ok
gate_requires_both_conditions_true_before_admitting ... ok
```

This is the decisive check, and it is stronger than a mutation test would have been: the previous report's worry was "a CI job that sets the variable in a non-root container reports 13/13 passing with 12 tests having executed nothing". That exact configuration now exits 101 with 13 loud failures naming the unmet condition and the remedy. The `classify_gate` extraction is also the right call — mutating `NOPASS_ROOT_TESTS` inside a parallel test binary would race every other test reading it, and edition 2024 makes `set_var` `unsafe`, which this crate forbids at its root.

**Developer default unchanged.** `env -u NOPASS_ROOT_TESTS cargo test --test root_system` → exit 0, 15 passed, quiet skips with the `SKIPPED <test>: ...` stderr line preserved.

**And the lane was actually run.** Both container lanes executed as real root with zero skips (see Build & Tests above). This closes the remediation's own self-declared Risk 1 — that `real_status_stdout_for_an_active_grant_matches_the_state_file_exactly` had never genuinely executed anywhere. It has now, on two distros.

### Gate 4 — are the four reconciled spec sentences honest?

Short answer: yes, all four. None of them weakens a requirement to match an implementation shortcut; three of them make the spec *more* demanding than it was, and the fourth is a documented design decision with a stated cost. Judged one at a time, against the code and against `design.md`.

**1. `sudoers-rule-lifecycle` §Atomic Rule Creation — "Concurrent enable requests serialize instead of corrupting the file"** (was: "the second call blocks on the flock until the first completes")

Honest. The new text states the `flock` is `LockExclusiveNonblock`, the second caller fails fast at exit 15 and mutates nothing, and gives the rationale: a privileged helper invoked from a desktop click must never hang holding a polkit authorization open.

This does drop a liveness property — the second caller no longer eventually proceeds — and that is a real, user-visible cost: a fast double-click produces an error instead of a queued success. The spec now says so plainly instead of promising something the binary does not do. Three things make this a reconciliation rather than an excuse: the safety property the requirement actually exists to protect (no interleaved writes, no corrupt `/etc/sudoers.d`) is fully preserved and proven by `concurrent_enable_calls_serialize_second_caller_gets_lock_busy_and_state_stays_consistent`, which genuinely executed as root on both distros in this pass; the same capability's own `§Mutation Serialization` scenario *already* said "exits 15 (lock busy)", so the old sentence contradicted its own document, not just the code; and `design.md`'s Architecture Decision "`flock` on `/run/nopass/lock`, not an `O_EXCL` lock file" made this call before any code was written. A spec edited to excuse a defect would have quietly dropped the failure mode; this one names the exit code, the cost, and the reason.

**2. `expiry-policy` §Transient Timer Replacement — "Permanent and until-reboot enables never touch the timer"** (was: "neither `systemctl` nor `systemd-run` is invoked")

Honest, and *stronger* than what it replaced. The new text says `systemd-run` is never invoked and `systemctl stop <unit>.timer` **is** invoked first, unconditionally and tolerant of any exit status, so a stale timer from a previous `At` activation cannot outlive the new grant. The old sentence would have permitted a genuine bug: grant `--until 1h`, then re-grant permanently, and the hour-old timer would still fire and revoke the permanent grant. `design.md` §4.1 step 14 mandates the unconditional stop for exactly that reason. The clause is not decorative — `ScriptedRunner` panics on an unexhausted script, so `enable_inner_audits_a_successful_grant_with_the_ok_outcome` (which queues `systemctl stop` for `Expiry::Never`) fails if the stop disappears.

**3. `helper-observability` §State File Placement and Permissions — boot sweep removes rather than updates**

Honest, and grounded in a reader contract that predates it. Verified in code: `expire_boot_inner` sweeps the rules, then `sweep_orphan_state_files` removes every `/run/nopass/<uid>.state` with no live rule — a superset of "the state file of each rule it deleted", so the new sentence is if anything an understatement. Verified in design: §4.4 says "Stale `/run/nopass/*.state` files without a rule are removed", and `design.md` L563 independently specifies the consumer side — "the M2 tray MUST treat a missing/stale state file as 'unknown' and reconcile with `sudo -kn true` rather than as 'inactive'". So a missing file is the *defined* signal, not an absence of one, and the original MUST ("updated on every ... boot-time cleanup sweep") was asking for a `active:false` rewrite that nothing reads and that would have to be garbage-collected later anyway. The edit narrows `expire` to `expire --uid` in the update clause, which is exactly the split between the two code paths.

**4. `privilege-admission` §UID Range Admission — "uid 0 is always rejected"**

Honest, and purely additive. Exit 11 for uid 0 at admission is retained verbatim and even strengthened ("regardless of what `UID_MIN` permits"); the new `AND` records that the polkit path never reaches it because `PKEXEC_UID=0` is refused earlier at exit 10, that both refusals are absolute, and that exit 11 stays reachable from `expire` and library callers. No requirement was relaxed — I re-verified live that `PKEXEC_UID=0 enable` still exits 10, and `checks.rs` still rejects uid 0 before consulting the range (`uid_0_is_always_rejected_even_with_uid_min_0`, `admit_uid_agrees_with_uid_range_admits_across_the_boundary_table`). This is documentation of defence in depth, which is the opposite of an excuse.

**What I would still change**, and it is the one remaining WARNING: two of these scenarios now have *titles* that contradict their own bodies (below).

---

### Spec Compliance Matrix

Legend: COMPLIANT = the code satisfies the requirement and a test that passed at runtime pins it. Root-gated evidence is marked `[root]` and did genuinely execute in this pass on both Debian and Fedora.

#### `sudoers-rule-lifecycle` — 4 requirements, 10 scenarios

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| Rule File Naming and Content Format | Template renders the fixed format | `template.rs > renders_fixed_format_for_at_expiry` (four-line golden) | COMPLIANT |
| | Username `.` preserved, disallowed stripped | 10 `sanitize_username` tests incl. `..._strips_line_terminators`, `render_rule_cannot_gain_a_line_from_the_username` | COMPLIANT |
| **Verdict** | | Allowlist filter, truncate-after-filter, `"unknown"` fallback pinned | **SATISFIED** |
| Atomic Rule Creation | Successful atomic write | `fileops_tempdir.rs > write_rule_atomic_creates_the_final_file_...`, `mode_0440_holds_even_under_umask_0o077`; `[root] real_enable_writes_a_root_owned_mode_0440_rule_and_a_mode_0644_state_file` | COMPLIANT |
| | `visudo -cf` rejects a corrupted candidate | `visudo_reject_unlinks_tmp_...`, `visudo_reject_leaves_a_preexisting_legitimate_rule_...`; `[root] real_visudo_cf_accepts_valid_content_and_rejects_malformed_content` | COMPLIANT |
| | Rename fails after successful validation | `ops.rs > enable_inner_rename_failure_over_a_symlink_yields_exit_16`; `[root] real_rename_failure_over_a_symlink_rolls_back_and_leaves_no_partial_rule` — asserts exit 16, tmp unlinked, the object at the final path untouched **and its target byte-identical** | COMPLIANT (reclassified, see D-S3) |
| | Concurrent enable requests serialize | `lock.rs > second_nonblocking_acquire_on_an_already_held_lock_is_lock_busy`, `ops.rs > disable_inner_lock_busy_yields_exit_15`; `[root] concurrent_enable_calls_serialize_second_caller_gets_lock_busy_and_state_stays_consistent`. Spec text now matches the shipped non-blocking semantics | COMPLIANT (was PARTIAL — W3) |
| **Verdict** | | Atomicity, `O_EXCL`, mode, rollback, serialization all proven | **SATISFIED** |
| Rule Removal | Disable removes an existing rule | `ops.rs > disable_inner_unlink_precedes_timer_stop_...`, `..._leaves_a_foreign_non_nopass_file_untouched`; `[root] real_disable_removes_an_existing_rule_and_marks_the_state_file_inactive` | COMPLIANT |
| | Rule externally deleted before disable | `disable_inner_is_idempotent_when_the_rule_was_already_externally_deleted`, `remove_rule_is_an_idempotent_no_op_...` | COMPLIANT |
| **Verdict** | | Name pattern **and** header ownership both enforced | **SATISFIED** |
| Mutation Serialization | Sequential acquire and release | `lock.rs > sequential_acquire_and_release_...`, `dropping_the_guard_releases_the_kernel_level_flock_...` | COMPLIANT |
| | Lock already held rejects the caller | `lock.rs` L71 `EWOULDBLOCK → LockBusy`; `[root]` lane proves it across real processes | COMPLIANT |
| **Verdict** | | Every mutating transaction acquires before any write; `status` deliberately does not | **SATISFIED** |

#### `privilege-admission` — 4 requirements, 14 scenarios

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| UID Resolution by Invocation Context | Enable resolves uid from PKEXEC_UID | `uid.rs > enable_resolves_uid_from_pkexec_uid`, `disable_and_status_also_resolve_from_pkexec_uid` | COMPLIANT |
| | PKEXEC_UID missing | `uid.rs` L59-61 + `ops.rs > {enable,disable,status}_live_wrapper_rejects_context_...`; `process_boundary > enable_with_pkexec_uid_missing_still_exits_10`; live: 10 | COMPLIANT |
| | PKEXEC_UID present but unparseable | `pkexec_uid_non_numeric_...`, `..._negative_...`, `..._absurdly_large_...`; live: `abc` → 10 | COMPLIANT |
| | Expire invoked with PKEXEC_UID set | `expire_invoked_with_pkexec_uid_set_is_rejected`, `expire_resolves_as_system_root_when_pkexec_uid_is_present_but_empty` | COMPLIANT |
| | Expire invoked as non-root without PKEXEC_UID | `expire_live_wrapper_rejects_context_when_the_real_uid_is_not_root`; `[root] real_expire_as_non_root_without_pkexec_uid_is_rejected_with_exit_10` (demotes a child via `Command::uid()`) | COMPLIANT |
| **Verdict** | | Zero-mutation by construction; real uid, not effective | **SATISFIED** |
| UID Range Admission | Default-range uid is admitted | `default_range_uid_is_admitted_when_it_exists_in_the_passwd_database`; `[root] real_getpwuid_resolves_a_real_user_and_admits_the_uid_into_range` | COMPLIANT |
| | uid 0 is always rejected (exit 11) | `uid_0_is_always_rejected_even_with_uid_min_0`, `admit_uid_agrees_with_uid_range_admits_across_the_boundary_table`; spec now records that polkit refuses it earlier at 10 (live-verified) | COMPLIANT (was PARTIAL — W9) |
| | uid 65534 (`nobody`) rejected | `uid_65534_is_rejected_for_exceeding_the_default_uid_max`; `process_boundary > enable_with_pkexec_uid_65534_still_exits_11_via_the_range_check`; live: 11 | COMPLIANT |
| | Malformed/missing login.defs clamps to floor | `missing_login_defs_clamps_...`, `unparseable_login_defs_clamps_...`, `uid_min_0_clamps_to_the_floor` | COMPLIANT |
| | uid not present in `getpwuid` (exit 11) | `checks.rs > lookup_user_optional_returns_none_for_a_uid_absent_from_getpwuid`, `..._still_maps_an_absent_uid_to_an_internal_error_for_its_existing_callers`; `process_boundary > enable_with_a_pkexec_uid_absent_from_getpwuid_exits_11_not_1`; live: 11; mutation M1 red | COMPLIANT (was FAILING — C1) |
| **Verdict** | | The exit-11 cause is reachable in production and audited with `uid_rejected_unknown` | **SATISFIED** |
| Existing-Sudoer Probe | Probe exits 0 grants admission | `is_sudoer_builds_the_exact_pinned_argv_with_no_shell_and_only_lang_c_env` (full `CommandSpec` equality) | COMPLIANT |
| | Probe non-zero rejects a group-flagged user | `probe_nonzero_rejects_even_when_the_user_is_group_flagged`, `enable_inner_sudoer_probe_rejection_maps_to_exit_12_with_nothing_created`; `[root] real_getgrouplist_advisory_precheck_runs_against_a_real_user` | COMPLIANT |
| | Probe invocation discipline | absolute path via `Binaries`, `LANG`/`LC_ALL` only; denial text never parsed; `enable_inner_sanitizes_the_raw_username_before_the_sudo_probe` | COMPLIANT |
| **Verdict** | | Probe is the sole authority; group check advisory | **SATISFIED** |
| Single Polkit Action | Installed policy declares the single action | `data_artifacts.rs > policy_declares_exactly_one_action`, `..._action_id_is_fixed`, `..._has_required_allow_defaults`, `..._exec_path_is_default_helper_path` | COMPLIANT |
| **Verdict** | | | **SATISFIED** |

#### `expiry-policy` — 5 requirements, 16 scenarios

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| Expiry Model and Header Encoding | Each expiry form round-trips | `expiry.rs > each_expiry_form_round_trips_through_render_and_parse`, `header.rs > parses_never_and_reboot_expires` | COMPLIANT |
| | Unrecognized header value rejected | `unrecognized_expires_value_is_rejected_with_typed_error` (closed grammar, typed error not a default) | COMPLIANT |
| **Verdict** | | `expiry_serializes_as_internally_tagged_json` pins `{"kind":"at","epoch":N}` | **SATISFIED** |
| Temporary Duration Validation | In-range / below min / above max / past / no flag | `validate_until_accepts_the_minimum_and_maximum_boundary` (60 and 28800), `..._one_second_below/above...`, `..._a_value_in_the_past`, `..._equal_to_now`; `resolve_expiry_rejects_until_{below,above,in_the_past}` → exit 13; `resolve_expiry_defaults_to_never_with_no_flags` | COMPLIANT (all 5) |
| **Verdict** | | Validated at step 4, before the lock and any filesystem contact | **SATISFIED** |
| Transient Timer Replacement | First temporary enable creates the timer | `schedule_builds_the_exact_pinned_systemd_run_argv_in_order` (12 tokens, UTC `Z`), `stop_runs_the_exact_pinned_systemctl_stop_argv_and_tolerates_non_zero` | COMPLIANT |
| | Re-enabling replaces an existing timer | `enable_inner_at_expiry_schedules_the_timer_with_the_exact_pinned_argv` — `ScriptedRunner` queue order is the ordering assertion | COMPLIANT |
| | Timer scheduling failure rolls back the rule | `enable_inner_systemd_run_failure_rolls_back_the_rule_and_exits_17`, `..._audits_a_rolled_back_timer_failure...`; `[root] real_timer_scheduling_failure_rolls_back_the_rule_when_systemd_is_unavailable` | COMPLIANT |
| | Permanent and until-reboot: no `systemd-run`, but `systemctl stop` IS called | `enable_inner_never_and_reboot_never_call_systemd_run` plus every `Expiry::Never` script that queues `systemctl stop` and would panic on non-exhaustion. Spec text now matches design §4.1 step 14 | COMPLIANT (was PARTIAL — W6) |
| **Verdict** | | Argv pinned verbatim; rollback proven | **SATISFIED** |
| Expiry Re-validation Before Deletion | Expire deletes a genuinely past-epoch rule, updates state, **is journaled** | `expire_uid_inner_deletes_a_genuinely_past_epoch_rule_and_stops_the_timer`, `..._writes_the_state_file_with_active_false...`, and now `expire_uid_inner_audits_a_successful_deletion_with_the_ok_outcome` (mutation M4 red); `[root] real_expire_uid_deletes_a_past_epoch_rule_and_leaves_a_future_epoch_rule_intact` | COMPLIANT (was PARTIAL — W1) |
| | Stale timer fires after a newer enable | `expire_uid_inner_future_at_is_skipped_not_expired_...`, `..._never_and_reboot_headers_are_never_deleted` | COMPLIANT |
| | Expire on a uid with no rule file | `expire_uid_inner_absent_rule_is_a_no_op_exit_0` | COMPLIANT |
| **Verdict** | | Lock precedes the header re-read; foreign files never deleted | **SATISFIED** |
| Boot-Time Cleanup Sweep | Boot sweep removes only stale and reboot-marked rules | `expire_boot_inner_removes_reboot_and_past_at_rules_but_leaves_never_and_future_at` (4-way fixture), `..._a_per_file_failure_does_not_abort_the_sweep`, `..._restricts_the_sweep_to_only_uid_when_given`; `[root] real_boot_sweep_removes_only_stale_and_reboot_marked_rules_among_real_files` | COMPLIANT |
| | Expire with neither flag is rejected | `cli.rs > expire_with_neither_uid_nor_boot_is_rejected_with_exit_code_2`; live: 2 | COMPLIANT |
| **Verdict** | | `expire_boot_inner` takes no `CommandRunner`/`Binaries`, so "no systemd call in boot mode" holds structurally | **SATISFIED** |

#### `helper-cli` — 4 requirements, 9 scenarios

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| Fixed Subcommand and Flag Surface | Documented invocation / unknown subcommand / unknown flag | `a_documented_enable_invocation_parses_successfully`, `unknown_subcommand_is_rejected_with_exit_code_2`, `unknown_flag_on_a_known_subcommand_...`; live: `foo` → 2 | COMPLIANT (all 3) |
| **Verdict** | | Exactly four subcommands, no external/hyphen/trailing-var-arg escapes | **SATISFIED** |
| Mutually Exclusive Duration Flags | `--until-reboot` alone / both together | `until_reboot_alone_parses_...`, `until_and_until_reboot_together_are_rejected_with_exit_code_2` | COMPLIANT (both) |
| **Verdict** | | | **SATISFIED** |
| Typed Exit Code Mapping | Successful enable exits 0 | `[root] real_enable_writes_a_root_owned_mode_0440_rule_and_a_mode_0644_state_file` asserts `Some(0)` from the real binary — genuinely executed on Debian and Fedora in this pass | COMPLIANT |
| | Each rejection path exits its documented code | `error.rs` 13 tests incl. `every_failure_cause_maps_to_a_distinct_code`; 4 process-level tests in `process_boundary.rs`; 12 live probes above. **Exit 1 no longer means two different things** | COMPLIANT (was FAILING — C1) |
| **Verdict** | | 0, 2, 10, 11, 13, 14, 15, 16, 17 each observed at the process boundary or in a passing unit test | **SATISFIED** |
| External Command Invocation Discipline | Each external call uses its exact absolute-path argv | `system_runner_refuses_bare_program_name_without_spawning`, `..._refuses_relative_dot_slash_program_path...`, `..._does_not_shell_interpret_arguments`, `..._clears_ambient_env_and_keeps_pkexec_uid_out_of_the_child`, `..._treats_signal_death_as_failure...`, plus `system_candidate_table_matches_design_md_section_3_names_and_order` now pinning the production table (mutation M7 red) | COMPLIANT (was PARTIAL — W5) |
| | No candidate binary path exists on the host | `empty_candidate_list_yields_binary_missing_mapped_to_exit_1`, `a_directory_as_the_only_candidate...`, `a_relative_candidate_is_skipped`, `a_dangling_symlink_candidate_is_skipped`, `schedule_yields_binary_missing_exit_1_when_systemd_run_has_no_candidate` | COMPLIANT |
| **Verdict** | | No shell, no `PATH`, no env beyond `LANG`/`LC_ALL`; enforced structurally before spawn | **SATISFIED** |

#### `helper-observability` — 3 requirements, 7 scenarios

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| HelperStatus JSON Contract | Active grant serializes with an `at` expiry | `active_from_serializes_the_exact_golden_json_for_at_expiry` (byte golden, field order) | COMPLIANT |
| | Inactive uid serializes with a null expiry | `inactive_serializes_false_active_and_null_expires` | COMPLIANT |
| | stdout and state-file content share the same shape | `process_boundary > status_stdout_for_a_missing_rule_is_the_exact_inactive_json_line` parses the real process stdout, asserts exactly one line and all seven fields (mutation M6 red); `[root] real_status_stdout_for_an_active_grant_matches_the_state_file_exactly` compares real stdout against the real state file field-for-field — genuinely executed on both distros in this pass | COMPLIANT (was PARTIAL — W4) |
| **Verdict** | | Both halves of the scenario now rest on observed output, not on a shared-serializer argument | **SATISFIED** |
| State File Placement and Permissions | Enable writes the state file with correct mode | `write_atomically_creates_the_state_file_with_mode_0644_...`, `write_holds_mode_0644_under_a_hostile_umask`; `[root]` asserts `0o644` on the real file | COMPLIANT |
| | `/run/nopass/` missing at helper start | `lock.rs > acquire_creates_the_run_directory_at_mode_0755_under_a_hostile_umask` and `..._corrects_an_already_existing_run_directory_to_mode_0755` (mutation M5 red on both); `[root] real_run_nopass_directory_is_created_at_0755_when_missing_before_enable`. The `0755` is now enforced by `set_permissions`, not inherited from the ambient umask | COMPLIANT (was PARTIAL — W2) |
| **Verdict** | | Requirement prose reconciled with the boot sweep's remove-not-rewrite behaviour, grounded in design §4.4 and the tray's documented "missing = unknown, reconcile" contract | **SATISFIED** (W7 closed) |
| Journald Audit Records | Successful enable is journaled | `enable_inner_audits_a_successful_grant_with_the_ok_outcome` (+ disable/expire twins; mutations M2/M3/M4 each red) **and** observed in real journald by this phase: `NOPASS_EVENT=enable NOPASS_OUTCOME=ok NOPASS_EXIT=0 SYSLOG_IDENTIFIER=nopass-helper` with every documented `NOPASS_*` field | COMPLIANT (was PARTIAL — W1) |
| | A rejected operation is still journaled | 7 runtime-captured assertions with distinct tokens (`uid_rejected_root`, `not_sudoer`, `rolled_back`/`timer_failed`, `lock_busy`, `fs_error`, duration); observed live: `uid_rejected_unknown` / `uid_rejected_above_max` delivered to real journald | COMPLIANT |
| **Verdict** | | Field names, `SYSLOG_IDENTIFIER`, `NOPASS_` prefix, stderr fallback pinned; **delivery over a real journald socket is now observed, not assumed** | **SATISFIED** |

**Compliance summary**: 56/56 scenarios COMPLIANT · 0 PARTIAL · 0 FAILING. 20/20 requirements satisfied.

---

### Coherence (Design)

| Decision (design.md) | Followed? | Notes |
|---|---|---|
| §4.1 enable step order 1-18 | Yes | The C1 fix changed only the missing-case *handling* of step 5's `getpwuid` half. No side-effecting step (lock, write, external command) was added, removed or reordered — re-checked line by line against the diff |
| §4.1 rollback table, all 7 rows | Yes | Unchanged by the remediation; re-confirmed |
| §4.2 unlink precedes timer stop | Yes | Unchanged |
| §4.3 lock before header re-read | Yes | Unchanged |
| §4.4 `--boot` makes no systemd call; orphan `*.state` cleanup | Yes | Structural (no runner parameter); `sweep_orphan_state_files` verified to run after the rule sweep |
| §4.5 `status` takes no lock, writes nothing | Yes | Now also verified at the process boundary |
| §3 binary candidate table | Yes | **Now pinned by a test**, not only by the source |
| §6 exact `systemd-run` argv, 4 `--timer-property` | Yes | Unchanged |
| §7 three `data/` artifacts | Yes | Byte-match |
| §9 exit-code table | **Yes** (was the C1 deviation) | the `UidRejected` / `11` / "no getpwuid entry" row now holds in production |
| §9 `REASON` never raw subprocess text | Yes | Closed `&'static str` set |
| §8.1 `/run/nopass` at `0755` | Yes, and now enforced | `lock::create_run_dir_at_0755` is the first code in the transaction to touch that directory and now owns its mode |
| §9 `journal::init` uses `init` | Deviation (accepted) | `try_init` for test safety; recorded |
| §3 `nix::sys::stat::stat` for binary resolution | Deviation (accepted) | `std::fs::metadata`; additionally rejects directories; recorded |
| File Changes table omits `helper/src/lib.rs` | Deviation (accepted) | A Cargo integration test is its own crate; recorded |
| Context failures (exit 10) unaudited | Deviation (accepted) | No uid resolvable at that point; recorded and reasoned in code |

All previously-accepted deviations remain accepted and none was disturbed. The one design deviation that *was* a defect (§9 exit-code table) is gone.

---

### TDD Compliance

| Check | Result | Details |
|---|---|---|
| TDD evidence reported | Yes | `apply-progress` revision 18 documents RED-before-GREEN for every one of the 14 new tests, and names the two it confirmed RED by temporarily reverting production code (`lock.rs` W2, and each `journal::audit` call site) |
| All tasks have tests | Yes | 45/45; every named RED test present and passing |
| RED confirmed | Yes, and independently | I did not take the RED claims on trust: mutations M1-M7 re-created each regression and all seven produced a red suite |
| GREEN confirmed | Yes | 257/257 on the host, 257/257 as real root on Debian, 257/257 as real root on Fedora |
| Triangulation adequate | Yes | `sanitize_username` 13 cases, `admit_uid` × `UidRange::admits` 36 combinations, `validate_until` 6 boundaries, `classify_gate` 4-row truth table, `process_boundary` 3 distinct exit codes from the same wrapper |
| Safety net for modified files | Yes | The remediation touched 4 existing production files and 2 existing test files; the full suite was run before and after each, and `lookup_user`'s ~9 existing callers are explicitly pinned by `lookup_user_still_maps_an_absent_uid_to_an_internal_error_for_its_existing_callers` |

**TDD compliance**: 6/6. The previous pass's one shortfall — an unretrievable consolidated evidence table — is resolved: revision 18 of the artifact carries the remediation's own RED/GREEN evidence inline.

### Test Layer Distribution

| Layer | Tests | Files | Tools |
|---|---|---|---|
| Unit (in-crate `#[cfg(test)]`, pure or `ScriptedRunner`) | 216 | 19 modules | `cargo test` |
| Integration, unprivileged, real filesystem (`Layout::under`) | 22 | `fileops_tempdir.rs`, `data_artifacts.rs` | `cargo test`, `tempfile` |
| Integration, unprivileged, real process (`CARGO_BIN_EXE`) | 4 | `process_boundary.rs` **(new)** | `cargo test` |
| Integration, root-gated, real system | 15 (13 gated) | `root_system.rs` | `cargo test` + Debian/Fedora containers — **all 15 executed in this pass** |
| Manual, non-gated | 0 automated | `Containerfile.systemd` | docker/podman, never executed end to end |
| **Total** | **257** | **7 binaries** | |

The new `process_boundary.rs` layer is the structural fix for C1's root cause, not just for C1: it is the first unprivileged test binary that observes the compiled helper's real process exit status, which is the layer at which every exit-code requirement in `helper-cli` is actually stated.

### Assertion Quality

No banned pattern found: zero tautologies, zero assertions that never call production code, zero ghost loops, zero smoke-only tests, zero mock-heavy files. The 14 new assertions were audited individually and all are behavioural:

- The three audit tests assert `OUTCOME="ok"` and `EXIT=0` in captured audit text produced by a real transaction, not that `audit` was "called".
- The two lock tests assert an observed `st_mode & 0o777`, one of them after something else pre-created the directory at `0700` — the case production actually hits.
- The candidate-table test reads the private `candidates` map so it can pin production data without requiring those paths to exist, and asserts the exact per-name order rather than set membership.
- The four `process_boundary` tests assert `output.status.code()` from a spawned process, and the `status` one additionally parses stdout as `HelperStatus` and asserts every field including `user == ""` for an unresolvable uid.
- `classify_gate_...` asserts all four rows of the truth table, including the one that must *not* skip.

**Assertion quality**: 0 CRITICAL, 0 WARNING. All five weaknesses named in the previous report are closed, each confirmed by a mutation rather than by inspection.

### Quality Metrics

**Linter**: `cargo clippy --workspace --all-targets -- -D warnings` — exit 0, zero warnings, including the new `process_boundary.rs` target.
**Type checker**: N/A for Rust beyond the compiler; `cargo build --release` exit 0.

---

## Issues Found

### CRITICAL

None.

### WARNING

**N1 — Two reconciled scenario titles now contradict their own bodies.** The reconciliation fixed the sentences but left the headings that introduce them, and `sdd-archive` merges these files into `openspec/specs/` where the headings are what a reader scans first.

- `expiry-policy` → `#### Scenario: Permanent and until-reboot enables never touch the timer`. The body now states that `systemctl stop <unit>.timer` **is** invoked. Stopping a timer is touching it. This is a direct self-contradiction inside one scenario, and it is the one I would fix: the body is right, so the title should read something like "Permanent and until-reboot enables schedule no timer but still clear a stale one".
- `sudoers-rule-lifecycle` → `#### Scenario: Concurrent enable requests serialize instead of corrupting the file`. Milder — the calls still do not interleave — but "serialize" now describes a caller that is refused rather than queued, which is not what the word normally promises.

Secondary, same finding: `openspec/config.yaml` `rules.specs` requires Given/When/Then for scenarios, and the sudoers reconciliation adds a bare `- Rationale:` bullet that is neither. The content belongs in the spec; the convention would put it in the requirement prose or an `- AND` clause. `privilege-admission` did this correctly by folding its rationale into an `- AND` bullet.

None of this changes behaviour, none of it changes a normative claim, and it does not block archive. It is worth ten minutes before the merge because it is cheaper to fix in a delta than in a canonical spec.

### SUGGESTION

- **S2 (carried, unchanged)** — Items reachable only from tests: `checks::in_admin_group` (spec-permitted as an advisory pre-check), `journal::AuditEvent::Status` and `journal::AuditOutcome::Error` (declared members of design §9's value domains, never constructed). None is a defect; all three remain named rather than silently tolerated.
- **S4 (carried, narrowed)** — The `Ok(None)` branch of `ops::enable`'s lookup is now audited (observed live). What remains unaudited is the genuine `getpwuid_r` errno branch and the other exit-1 internal failures, where `uid` is already known. Same reasoning that made `resolve_expiry_audited` audit exit 13 would apply, and it is a few lines.
- **S5 (carried, unchanged)** — `Containerfile.systemd` has still never been executed end to end, so PRD §10.10 (a `--until-reboot` grant removed by `nopass-cleanup.service` on the next real boot) and an actual `systemd-run` timer firing remain unproven by anything. Correctly scoped as manual by design §8. Note that this pass *did* independently observe `journalctl -t nopass-helper` delivery on the host, so that third part of the manual lane's purpose is no longer outstanding — only the timer and the boot unit are.
- **S6 (carried, accepted)** — `fileops::remove_rule`'s read-then-unlink TOCTOU, recorded in a code comment and accepted because only root can write `/etc/sudoers.d`.
- **S7 (new, minor)** — Considered and dismissed as a defect, recorded so it is not rediscovered: `ops::disable` and `ops::status` do not run `admit_uid`, so they act on a uid outside `[UID_MIN, UID_MAX]`. This is not a violation — removing a NOPASSWD rule is monotonically safety-increasing, the rule must still carry the NoPass banner to be touched, and `status` writes nothing. Naming it because it is the same shape as C1 ("the wrapper decides something the inner function tests") and a future reader deserves to know it was examined rather than missed.
- **S8 (new, informational)** — `openspec` is not installed on this host, so `openspec validate m1-core-helper --strict` was not run. The spec edits are additive bullets and the requirement/scenario heading counts are unchanged (20 and 56, verified by direct count), so parse risk is low — but the archive phase should run the validator on a host that has it.

---

## Disposition of every previous finding

| ID | Previous finding | Disposition | Basis |
|---|---|---|---|
| **C1** | `getpwuid`-absent uid exits 1, not 11 | **CLOSED** | Live: 11 on debug and release. Mutation M1 restores the defect and turns the suite red. All other `lookup_user` call sites confirmed non-fatal, so the cause cannot resurface elsewhere. A genuine `getpwuid_r` errno still exits 1 (code-path verified; untestable without a fault-injection seam). The rejection is audited with `uid_rejected_unknown`, observed in real journald |
| **W1** | No success-path audit assertion | **CLOSED** | Three new tests, three separate mutations (M2/M3/M4), each red. Also observed `OUTCOME=ok` records in real journald |
| **W2** | `/run/nopass` mode was umask-dependent | **CLOSED** | `set_permissions(0o755)` after `create_dir_all`, in `lock`, which is the first code in the transaction to touch that directory. Mutation M5 turns both new tests red. The second test covers the production case (directory pre-created by something else at the wrong mode) |
| **W3** | Spec said the second caller blocks; code fails fast | **CLOSED** | Delta spec now states `LockExclusiveNonblock`, exit 15, mutates nothing, with the polkit-hang rationale. Matches `design.md`'s Architecture Decision and the capability's own `§Mutation Serialization` scenario, which already said exit 15 |
| **W4** | `status` stdout never asserted | **CLOSED** | `process_boundary` parses the real process stdout (mutation M6 red) for the inactive half; `[root] real_status_stdout_for_an_active_grant_matches_the_state_file_exactly` covers the active half and **actually executed** on Debian and Fedora in this pass |
| **W5** | `Binaries::system()` candidate table unpinned | **CLOSED** | New test pins names and per-name order without requiring the paths to exist; mutation M7 (swapping `visudo`'s first two candidates) is red |
| **W6** | `expiry-policy` "never touch the timer" contradicted by design | **CLOSED** | Spec now states `systemd-run` is never invoked and `systemctl stop` is, unconditionally. This is *stricter* than the sentence it replaced — the old wording would have permitted a stale timer to revoke a later permanent grant. Title wording carried forward as N1 |
| **W7** | "state file updated on every ... boot sweep" unmet | **CLOSED** | Spec now says the sweep removes the state file of each rule it deleted. Verified against `sweep_orphan_state_files` (which is a superset of that claim) and against design §4.4 plus the tray's documented "missing = unknown, reconcile" contract at design.md L563 |
| **W8 / R** | Root lane could pass green while misconfigured | **CLOSED, with runtime proof** | `NOPASS_ROOT_TESTS=1` as non-root now exits 101 with 13 loud failures (executed on this host). Unset stays a quiet skip, exit 0. The decision is a pure `classify_gate` with a 4-row truth table proven unconditionally. Going further: both container lanes were executed as real root, 15/15 with zero skips on each |
| **W9** | "uid 0 rejected → 11" unreachable via pkexec | **CLOSED** | Spec keeps exit 11 at admission verbatim and adds that polkit refuses `PKEXEC_UID=0` earlier at exit 10, both refusals absolute. Purely additive. Live-verified: `PKEXEC_UID=0 enable` → 10 |
| **S1** | No process-level exit-code test | **CLOSED** | `tests/process_boundary.rs`, 4 tests, exactly the twenty-line shape the previous report suggested — and it is what caught C1 |
| **S2** | Items reachable only from tests | **STILL OPEN (accepted)** | Unchanged; not a defect, and all three are named |
| **S3** | Rename-failure "prior rule file untouched" unasserted | **CLOSED — reclassified, my previous assessment was wrong** | On re-reading the test, `[root] real_rename_failure_over_a_symlink_rolls_back_...` asserts both that the object at the final path survives untouched and that its target is byte-identical. More importantly, a rename **onto a plain prior rule file succeeds** — that is the intended replace path — so the scenario's precondition (validated tmp, failing rename, prior file present) is only reachable via the symlink refusal or a genuine filesystem fault that cannot be provoked without an injection seam. The symlink fixture is the only realistic instantiation, and it asserts the clause. I marked this PARTIAL last pass on fixture shape rather than on reachability; that was the wrong call and I am correcting it rather than carrying it forward |
| **S4** | exit-1 `lookup_user` failure unaudited | **PARTIALLY CLOSED** | The missing-entry case is now audited. The errno case and other internal failures still are not — carried forward as a SUGGESTION |
| **S5** | `Containerfile.systemd` never executed | **STILL OPEN (accepted, scoped to M4 QA)** | Narrowed: journald delivery is now independently observed, so only the firing timer and the boot unit remain unproven |
| **S6** | `remove_rule` TOCTOU | **STILL OPEN (accepted)** | Recorded in code; unchanged |

Nothing from the previous report was closed by assertion alone. Every "CLOSED" above rests on a mutation that turned the suite red, a live process exit status, an observed journald record, or a real-root container run.

---

## Gate results observed by this phase

| Gate | Command | Exit | Result |
|---|---|---|---|
| Tests | `cargo test --workspace` | 0 | 257 passed, 0 failed, 0 ignored |
| Release build | `cargo build --release` | 0 | clean |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | clean, zero warnings |
| Root lane, Debian | `docker build -f tests/containers/Containerfile.debian` then `docker run -e NOPASS_ROOT_TESTS=1 ... cargo test --workspace` | 0 / 0 | 257 passed; `root_system.rs` 15/15 executed as real root, zero skips |
| Root lane, Fedora | same with `Containerfile.fedora` | 0 / 0 | 257 passed; `root_system.rs` 15/15 executed as real root, zero skips |
| Root-gate misconfiguration | `NOPASS_ROOT_TESTS=1 cargo test --test root_system` (as uid 1000) | 101 | 13 loud failures naming the unmet condition — the intended new behaviour |
| Root-gate developer default | `env -u NOPASS_ROOT_TESTS cargo test --test root_system` | 0 | 15 passed, quiet skips with stderr reasons preserved |
| Mutation battery | 7 mutants, full suite each | 101 × 7 | Every neutered behaviour produced a red suite; each mutant failed exactly its intended test |
| Live exit-code probes | `nopass-helper` × 12 unprivileged invocations, debug and release | — | All 12 match the documented contract, including the C1 case |
| Journald delivery | `journalctl -t nopass-helper -o json` | 0 | Real records with `SYSLOG_IDENTIFIER=nopass-helper` and every documented `NOPASS_*` field, for both `ok` and `rejected` outcomes |
| Task completion | `tasks.md` | — | 45/45 `[x]`, 0 unchecked |
| Requirement/scenario count | direct count of `### Requirement:` / `#### Scenario:` | — | 20 / 56, unchanged by the reconciliation |
| Working tree | `git status --porcelain` | — | Empty at `9b1bfbe`, before and after the mutation battery |
| Coverage | — | — | Not available; threshold is 0 |

All exit statuses were read directly from `$?`, never inferred from output text. Every mutated file was restored with `git checkout --` and the tree re-verified empty after each. No source file, spec file, commit, or attempt token was created, modified, or settled by this phase. The two container images built for the root lanes were removed afterwards.

## Verdict

**PASS WITH WARNINGS** — 0 CRITICAL, 1 WARNING, 6 SUGGESTION. The CRITICAL is genuinely closed and its seam is closed with it; the four under-asserted requirements now fail when their behaviour is deleted, proven by mutation rather than by reading; the root lane fails loudly when misconfigured and was actually executed as real root on two distros; and the four reconciled spec sentences describe what the code does — three of them demanding *more* than the text they replaced, none of them written to excuse a shortcut. **Archive is unblocked.** The single WARNING is two scenario titles that no longer match their own bodies: worth fixing in the delta before the merge, but it blocks nothing.

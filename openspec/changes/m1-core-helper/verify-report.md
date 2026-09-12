```yaml
schema: gentle-ai.verify-result/v1
evidence_revision: sha256:fbc8564e8c96d507603ca385a75a4f4ebd78df5efb8711b00fd194f87d13623d
verdict: fail
blockers: 1
critical_findings: 1
requirements: 11/20
scenarios: 46/56
test_command: cargo test --workspace
test_exit_code: 0
test_output_hash: sha256:9d6c39b6cd8601318a34c6ed45018d879bf0e72ea3108875b349919171ebb8f8
build_command: cargo build --release
build_exit_code: 0
build_output_hash: sha256:222a02ed477d249a21359df8cae975864a62d848f4739cadc5b00b6190a963c6
```

## Verification Report

**Change**: `m1-core-helper`
**Version**: N/A (greenfield; `openspec/specs/` empty, five new delta capabilities)
**Mode**: Strict TDD (`openspec/config.yaml` → `strict_tdd: true`, `rules.apply.tdd: true`)
**Commit verified**: `22204f2` (working tree clean)

### Verdict

**FAIL** — one CRITICAL spec-conformance defect: a documented exit-code cause is unreachable through the production `enable` path, so `privilege-admission §UID Range Admission` and `helper-cli §Typed Exit Code Mapping` are not satisfied as written. Everything else is either satisfied or partially satisfied; the build is otherwise of unusually high quality and all three gates pass.

---

### Completeness

| Metric | Value |
|--------|-------|
| Tasks total | 45 |
| Tasks complete | 45 |
| Tasks incomplete | 0 |

Every task in `tasks.md` is `[x]`, and each checked task's named test exists and passes. Three tasks carry self-disclosed scope notes (7.3 `resolve_username` initially unwired, 8.1 `statefile::remove` initially unwired, 8.3 audit-wiring scope). All three were subsequently closed by later phases: `resolve_username` is called from `disable_inner`, and `statefile::remove`/`list_state_uids` are called from `ops::sweep_orphan_state_files`. Task 7.1's "three `--timer-property`" miscount is correctly resolved in favour of design.md's four; the implementation and its pinned argv both carry four.

### Build & Tests Execution

Executed by this verification phase, exit statuses read directly from `$?`, never grepped.

**Build**: PASSED

```text
$ cargo build --release
    Finished `release` profile [optimized] target(s)
BUILD_EXIT=0
```

**Tests**: PASSED — 243 passed / 0 failed / 0 ignored

```text
$ cargo test --workspace
nopass_core (src/lib.rs)                  55 passed
nopass_helper (src/lib.rs)               150 passed
nopass_helper (src/main.rs)                3 passed
tests/data_artifacts.rs                    7 passed
tests/fileops_tempdir.rs                  15 passed
tests/root_system.rs                      13 passed   <- 12 of these SKIP unprivileged
Doc-tests nopass_core / nopass_helper      0 + 0
TEST_EXIT=0
```

**Lint**: PASSED

```text
$ cargo clippy --workspace --all-targets -- -D warnings
CLIPPY_EXIT=0
```

**Coverage**: Not available — no coverage tool present in this workspace; `rules.verify.coverage_threshold` is `0`. Skipped cleanly, not a failure.

**Independent live probes** (read-only, unprivileged, no system state touched — verified `/etc/sudoers.d` and `/run/nopass` unchanged afterwards):

```text
$ PKEXEC_UID=4294967294 ./target/debug/nopass-helper enable   -> exit 1    (spec requires 11)
$ PKEXEC_UID=65534      ./target/debug/nopass-helper enable   -> exit 11   (correct)
$ PKEXEC_UID=0          ./target/debug/nopass-helper enable   -> exit 10   (design §9; spec says 11)
$ ./target/debug/nopass-helper foo                            -> exit 2    (correct)
$ env -u PKEXEC_UID ./target/debug/nopass-helper status       -> exit 10   (correct)
$ PKEXEC_UID=1000 ./target/debug/nopass-helper status         -> exit 0, exact HelperStatus JSON line
```

---

### Spec Compliance Matrix

Legend: COMPLIANT = code satisfies the requirement and a test that passed at runtime pins it · PARTIAL = code satisfies it but the test is weaker than the requirement, or the spec text and the shipped behaviour diverge · FAILING = the production path contradicts the requirement.

#### `sudoers-rule-lifecycle` — 4 requirements, 10 scenarios

| Requirement | Scenario | Evidence (code / test) | Result |
|---|---|---|---|
| Rule File Naming and Content Format | Template renders the fixed format | `nopass-core/src/template.rs::render_rule` / `template.rs > renders_fixed_format_for_at_expiry` (exact four-line golden, U+2014 banner) | COMPLIANT |
| | Username `.` preserved, disallowed stripped | `template.rs::sanitize_username` / 10 tests incl. `sanitize_username_preserves_dotted_name_verbatim`, `sanitize_username_strips_line_terminators`, `render_rule_cannot_gain_a_line_from_the_username` (asserts `lines().count() == 4`) | COMPLIANT |
| **Requirement verdict** | | Allowlist filter, truncate-after-filter, `"unknown"` fallback all pinned | **SATISFIED** |
| Atomic Rule Creation | Successful atomic write | `helper/src/fileops.rs::write_rule_atomic` (`create_new` = `O_CREAT\|O_EXCL`, `fchown 0:0`, `fchmod 0440`, `sync_all`, `visudo -cf`, `rename`) / `tests/fileops_tempdir.rs > write_rule_atomic_creates_the_final_file_with_exact_content_via_visudo_accept`, `mode_0440_holds_even_under_umask_0o077`; `tests/root_system.rs > real_enable_writes_a_root_owned_mode_0440_rule_and_a_mode_0644_state_file` (root-gated) | COMPLIANT |
| | `visudo -cf` rejects a corrupted candidate | `fileops.rs` L133-136 / `fileops_tempdir.rs > visudo_reject_unlinks_tmp_and_leaves_the_final_path_untouched`, `visudo_reject_leaves_a_preexisting_legitimate_rule_at_the_final_path_byte_and_mode_identical`; `root_system.rs > real_visudo_cf_accepts_valid_content_and_rejects_malformed_content` (real `/usr/sbin/visudo`) | COMPLIANT |
| | Rename fails after successful validation | `fileops.rs` L147-157 / `ops.rs > enable_inner_rename_failure_over_a_symlink_yields_exit_16`; `root_system.rs > real_rename_failure_over_a_symlink_rolls_back_and_leaves_no_partial_rule`. Both assert exit 16 + tmp unlinked; **neither asserts the scenario's third clause, "any prior rule file is left untouched"** — both fixtures place a symlink, not a legitimate prior rule, at the final path | PARTIAL |
| | Concurrent enable requests serialize | `helper/src/lock.rs` (`FlockArg::LockExclusiveNonblock`) / `root_system.rs > concurrent_enable_calls_serialize_second_caller_gets_lock_busy_and_state_stays_consistent`. **The spec says the second caller "blocks on the flock until the first completes"; the shipped behaviour is non-blocking fail-fast exit 15** (design.md Architecture Decision "flock on /run/nopass/lock"). The safety property is proven; the spec text was never amended | PARTIAL |
| **Requirement verdict** | | Atomicity, `O_EXCL`, mode, rollback all proven; two clauses under-asserted | **PARTIALLY SATISFIED** |
| Rule Removal | Disable removes an existing rule | `fileops.rs::remove_rule` (requires `is_nopass_owned` before unlink) / `ops.rs > disable_inner_unlink_precedes_timer_stop_and_removes_an_existing_rule`, `disable_inner_leaves_a_foreign_non_nopass_file_untouched`; `root_system.rs > real_disable_removes_an_existing_rule_and_marks_the_state_file_inactive` | COMPLIANT |
| | Rule externally deleted before disable | `fileops.rs` L183, L203 (`ENOENT` → `Ok(false)`) / `ops.rs > disable_inner_is_idempotent_when_the_rule_was_already_externally_deleted`, `fileops_tempdir.rs > remove_rule_is_an_idempotent_no_op_when_the_file_was_already_externally_deleted` | COMPLIANT |
| **Requirement verdict** | | Name-pattern **and** header ownership both enforced; idempotent | **SATISFIED** |
| Mutation Serialization | Sequential operations acquire and release | `lock.rs::LockGuard` (RAII) / `lock.rs > sequential_acquire_and_release_succeed_with_no_overlap_required`, `dropping_the_guard_releases_the_kernel_level_flock_for_an_independently_opened_fd` | COMPLIANT |
| | Lock already held rejects the caller | `lock.rs` L71 (`EWOULDBLOCK` → `LockBusy`) / `lock.rs > second_nonblocking_acquire_on_an_already_held_lock_is_lock_busy`, `ops.rs > disable_inner_lock_busy_yields_exit_15` | COMPLIANT |
| **Requirement verdict** | | Every mutating transaction (`enable_inner`, `disable_inner`, `expire_uid_inner`, `expire_boot_inner`) acquires before any write; `status` deliberately does not | **PARTIALLY SATISFIED** (spec wording divergence, see W3) |

#### `privilege-admission` — 4 requirements, 14 scenarios

| Requirement | Scenario | Evidence (code / test) | Result |
|---|---|---|---|
| UID Resolution by Invocation Context | Enable resolves uid from PKEXEC_UID | `helper/src/uid.rs::resolve` / `uid.rs > enable_resolves_uid_from_pkexec_uid`, `disable_and_status_also_resolve_from_pkexec_uid` | COMPLIANT |
| | PKEXEC_UID missing | `uid.rs` L59-61 / `pkexec_uid_absent_on_enable_is_rejected`, `pkexec_uid_empty_on_enable_is_rejected`; live wrappers pinned by `ops.rs > {enable,disable,status}_live_wrapper_rejects_context_when_pkexec_uid_is_absent` (these read the real, unset process environment) | COMPLIANT |
| | PKEXEC_UID present but unparseable | `uid.rs` L73-74 / `pkexec_uid_non_numeric_on_enable_is_rejected`, `pkexec_uid_negative_on_enable_is_rejected`, `pkexec_uid_absurdly_large_on_enable_is_rejected` | COMPLIANT |
| | Expire invoked with PKEXEC_UID set | `uid.rs` L81-83 / `expire_invoked_with_pkexec_uid_set_is_rejected`, `expire_resolves_as_system_root_when_pkexec_uid_is_present_but_empty` | COMPLIANT |
| | Expire invoked as non-root without PKEXEC_UID | `uid.rs` L84-86 / `expire_invoked_non_root_without_pkexec_uid_is_rejected`, `ops.rs > expire_live_wrapper_rejects_context_when_the_real_uid_is_not_root`; `root_system.rs > real_expire_as_non_root_without_pkexec_uid_is_rejected_with_exit_10` genuinely demotes a child via `Command::uid()` | COMPLIANT |
| **Requirement verdict** | | Zero-mutation by construction (`resolve` has no `Layout`/`CommandRunner` in scope); real uid, not effective | **SATISFIED** |
| UID Range Admission | Default-range uid is admitted | `helper/src/checks.rs::admit_uid` / `default_range_uid_is_admitted_when_it_exists_in_the_passwd_database` | COMPLIANT |
| | uid 0 is always rejected (exit 11) | `checks.rs` L71-73 (rejects 0 before consulting the range) / `uid_0_is_always_rejected_even_with_uid_min_0`, `admit_uid_agrees_with_uid_range_admits_across_the_boundary_table`. **Unreachable in production**: `PKEXEC_UID=0` is rejected by `uid::resolve` as exit 10 (design.md §9 documents 10 for "zero PKEXEC_UID"), verified live | PARTIAL |
| | uid 65534 (`nobody`) rejected | `checks.rs` L74-80 delegating to `UidRange::admits` / `uid_65534_is_rejected_for_exceeding_the_default_uid_max`; verified live: exit 11 | COMPLIANT |
| | Malformed/missing login.defs clamps to floor | `nopass-core/src/logindefs.rs::parse` / `missing_login_defs_clamps_to_the_default_floor_and_max`, `unparseable_login_defs_clamps_to_the_default_floor_and_max`, `uid_min_0_clamps_to_the_floor` | COMPLIANT |
| | uid not present in `getpwuid` (exit 11) | `checks.rs` L81-84 → `UidRejection::Unknown` / `uid_absent_from_getpwuid_is_rejected`. **The production `ops::enable` path never reaches this check**: `ops.rs` L90 calls `checks::lookup_user(uid)?` first, which maps a missing entry to `HelperError::Internal` → **exit 1**. Verified live | **FAILING** |
| **Requirement verdict** | | See CRITICAL C1 | **NOT SATISFIED** |
| Existing-Sudoer Probe | Probe exits 0 grants admission | `checks.rs::is_sudoer` / `is_sudoer_builds_the_exact_pinned_argv_with_no_shell_and_only_lang_c_env` (`ScriptedRunner` asserts full `CommandSpec` equality) | COMPLIANT |
| | Probe non-zero rejects a group-flagged user | `checks.rs` L143-146 (`in_admin_group` is never consulted by `is_sudoer`) / `probe_nonzero_rejects_even_when_the_user_is_group_flagged`; `ops.rs > enable_inner_sudoer_probe_rejection_maps_to_exit_12_with_nothing_created` | COMPLIANT |
| | Probe invocation discipline | `checks.rs` L136-141 (absolute path via `Binaries`, `LANG`/`LC_ALL` only, `Vec<String>` argv) / same argv test; denial text never parsed (only `outcome.status` is inspected); `ops.rs > enable_inner_sanitizes_the_raw_username_before_the_sudo_probe` pins that `"a;rm -rf /"` reaches the probe as `"arm-rf"` | COMPLIANT |
| **Requirement verdict** | | Probe is the sole authority; group check is advisory and in fact never called (see S2) | **SATISFIED** |
| Single Polkit Action | Installed policy declares the single action | `data/com.enfoquestic.nopass.policy` (byte-matches design §7) / `tests/data_artifacts.rs > policy_declares_exactly_one_action`, `policy_action_id_is_fixed`, `policy_has_required_allow_defaults`, `policy_exec_path_is_default_helper_path` | COMPLIANT |
| **Requirement verdict** | | `allow_any=no`, `allow_inactive=no`, `allow_active=auth_admin_keep`, `exec.path=/usr/libexec/nopass-helper`, exactly one `<action id=` | **SATISFIED** |

#### `expiry-policy` — 5 requirements, 16 scenarios

| Requirement | Scenario | Evidence (code / test) | Result |
|---|---|---|---|
| Expiry Model and Header Encoding | Each expiry form round-trips | `nopass-core/src/expiry.rs::Expiry`, `header.rs::parse_expires` / `expiry.rs > each_expiry_form_round_trips_through_render_and_parse`, `header.rs > parses_never_and_reboot_expires` | COMPLIANT |
| | Unrecognized header value rejected | `header.rs` L77-94 (closed grammar `never\|reboot\|0\|[1-9][0-9]{0,18}`) / `unrecognized_expires_value_is_rejected_with_typed_error` (asserts `HeaderError::MalformedExpires`, not a default) | COMPLIANT |
| **Requirement verdict** | | Struct variant `At { epoch }` confirmed; `expiry_serializes_as_internally_tagged_json` pins `{"kind":"at","epoch":N}` | **SATISFIED** |
| Temporary Duration Validation | In-range accepted / below min / above max / past / no flag | `expiry.rs::validate_until`, `ops.rs::resolve_expiry` / `validate_until_accepts_the_minimum_and_maximum_boundary` (60 and 28800 exactly), `..._one_second_below_the_minimum`, `..._one_second_above_the_maximum`, `..._a_value_in_the_past`, `..._equal_to_now`; `ops.rs > resolve_expiry_rejects_until_{below,above,in_the_past}...` all assert exit 13, `resolve_expiry_defaults_to_never_with_no_flags` | COMPLIANT (all 5) |
| **Requirement verdict** | | Validation happens at step 4, before the lock and before any filesystem contact | **SATISFIED** |
| Transient Timer Replacement | First temporary enable creates the timer | `helper/src/timer.rs::{stop,schedule}` / `schedule_builds_the_exact_pinned_systemd_run_argv_in_order` (12 argv tokens verbatim, `--on-calendar=2026-09-12T15:00:00Z`), `stop_runs_the_exact_pinned_systemctl_stop_argv_and_tolerates_non_zero` | COMPLIANT |
| | Re-enabling replaces an existing timer | `ops.rs` L209 (`timer::stop` runs unconditionally before L213's `schedule`) / `ops.rs > enable_inner_at_expiry_schedules_the_timer_with_the_exact_pinned_argv` — the `ScriptedRunner` queue order **is** the ordering assertion | COMPLIANT |
| | Timer scheduling failure rolls back the rule | `ops.rs` L213-220 (`rollback_rule` + `fsync_parent`) / `enable_inner_systemd_run_failure_rolls_back_the_rule_and_exits_17` (asserts the rule file is gone), `enable_inner_audits_a_rolled_back_timer_failure_with_the_rolled_back_outcome`; `root_system.rs > real_timer_scheduling_failure_rolls_back_the_rule_when_systemd_is_unavailable` | COMPLIANT |
| | Permanent and until-reboot never touch the timer | `ops.rs` L206-220 / `enable_inner_never_and_reboot_never_call_systemd_run`. **`systemd-run` is genuinely never invoked, but `systemctl stop` IS** — design.md §4.1 step 14 mandates it unconditionally so a stale timer from a prior `At` activation is cleared. The spec scenario says "neither `systemctl` nor `systemd-run` is invoked" | PARTIAL |
| **Requirement verdict** | | Argv pinned verbatim; rollback proven; one spec clause contradicted by design | **SATISFIED** (with W6 divergence) |
| Expiry Re-validation Before Deletion | Expire deletes a genuinely past-epoch rule | `ops.rs::expire_uid_inner` L429-494 — lock acquired **before** the header re-read, closing the stale-revocation race / `expire_uid_inner_deletes_a_genuinely_past_epoch_rule_and_stops_the_timer`, `..._writes_the_state_file_with_active_false_after_deleting_an_expired_rule`; `root_system.rs > real_expire_uid_deletes_a_past_epoch_rule_and_leaves_a_future_epoch_rule_intact`. **The scenario's third clause, "the action is journaled", is not asserted by any test** (see W1) | PARTIAL |
| | Stale timer fires after a newer enable | `ops.rs` L450 (`!header.expires.is_expired(now)`) / `expire_uid_inner_future_at_is_skipped_not_expired_and_the_file_stays_intact`, `expire_uid_inner_never_and_reboot_headers_are_never_deleted` | COMPLIANT |
| | Expire on a uid with no rule file | `ops.rs` L440 / `expire_uid_inner_absent_rule_is_a_no_op_exit_0` | COMPLIANT |
| **Requirement verdict** | | Foreign/unparseable files are never deleted (`expire_uid_inner_foreign_file_is_never_deleted`) | **PARTIALLY SATISFIED** |
| Boot-Time Cleanup Sweep | Boot sweep removes only stale and reboot-marked rules | `ops.rs::expire_boot_inner`/`sweep_one` using `is_expired_at_boot` / `expire_boot_inner_removes_reboot_and_past_at_rules_but_leaves_never_and_future_at` (4-way fixture), `..._a_per_file_failure_does_not_abort_the_sweep`, `..._restricts_the_sweep_to_only_uid_when_given`; `root_system.rs > real_boot_sweep_removes_only_stale_and_reboot_marked_rules_among_real_files` | COMPLIANT |
| | Expire with neither flag is rejected | `cli.rs` `ArgGroup::new("target").required(true)` / `cli.rs > expire_with_neither_uid_nor_boot_is_rejected_with_exit_code_2` | COMPLIANT |
| **Requirement verdict** | | `expire_boot_inner` takes no `CommandRunner`/`Binaries` at all, so "no systemd call in boot mode" holds **structurally**, not by assertion — the strongest form of this guarantee. Orphan `*.state` cleanup (design §4.4) is wired and covered by 4 tests | **SATISFIED** |

#### `helper-cli` — 4 requirements, 9 scenarios

| Requirement | Scenario | Evidence (code / test) | Result |
|---|---|---|---|
| Fixed Subcommand and Flag Surface | Documented invocation parses / unknown subcommand / unknown flag | `helper/src/cli.rs` (no `allow_external_subcommands`, `allow_hyphen_values`, `trailing_var_arg`) / `a_documented_enable_invocation_parses_successfully`, `unknown_subcommand_is_rejected_with_exit_code_2`, `unknown_flag_on_a_known_subcommand_is_rejected_with_exit_code_2`; process-level exit 2 confirmed live | COMPLIANT (all 3) |
| **Requirement verdict** | | Exactly four subcommands | **SATISFIED** |
| Mutually Exclusive Duration Flags | `--until-reboot` alone / both together | `cli.rs` `ArgGroup::new("when").multiple(false)` / `until_reboot_alone_parses_with_until_reboot_true_and_until_none`, `until_and_until_reboot_together_are_rejected_with_exit_code_2` | COMPLIANT (both) |
| **Requirement verdict** | | | **SATISFIED** |
| Typed Exit Code Mapping | Successful enable exits 0 | `main.rs::to_exit_code` / `root_system.rs > real_enable_writes_a_root_owned_mode_0440_rule_and_a_mode_0644_state_file` asserts `Some(0)` from the real binary | COMPLIANT |
| | Each rejection path exits its documented code | `helper/src/error.rs::exit_code` / `error.rs` 13 tests incl. `every_failure_cause_maps_to_a_distinct_code`, `audit_reason_never_leaks_raw_subprocess_stderr_text`. **The mapping table is exhaustive and correct, but one documented cause — "missing from `getpwuid`" (code 11) — surfaces as code 1 through `ops::enable`.** Verified live | **FAILING** |
| **Requirement verdict** | | See CRITICAL C1 | **NOT SATISFIED** |
| External Command Invocation Discipline | Each external call uses its exact absolute-path argv | `runner.rs::SystemRunner` (rejects a non-absolute program **before spawning** — `env_clear()` alone does not defeat glibc's `confstr(_CS_PATH)` fallback), `bins.rs::Binaries::resolve` (absolute **and** `metadata().is_file()`) / `system_runner_refuses_bare_program_name_without_spawning`, `system_runner_refuses_relative_dot_slash_program_path_without_spawning`, `system_runner_does_not_shell_interpret_arguments`, `system_runner_clears_ambient_env_and_keeps_pkexec_uid_out_of_the_child`, `system_runner_treats_signal_death_as_failure_even_under_expect_any`, plus every `ScriptedRunner` argv assertion. **The production `Binaries::system()` candidate table itself is unpinned** — all 12 `bins.rs` tests use `from_candidates` (see W5) | PARTIAL |
| | No candidate binary path exists on the host | `bins.rs` L67-74 / `empty_candidate_list_yields_binary_missing_mapped_to_exit_1`, `a_directory_as_the_only_candidate_yields_binary_missing`, `a_relative_candidate_is_skipped`, `a_dangling_symlink_candidate_is_skipped`; `timer.rs > schedule_yields_binary_missing_exit_1_when_systemd_run_has_no_candidate` | COMPLIANT |
| **Requirement verdict** | | No shell, no `PATH`, no env beyond `LANG`/`LC_ALL` anywhere; enforced structurally in `SystemRunner` | **PARTIALLY SATISFIED** |

#### `helper-observability` — 3 requirements, 7 scenarios

| Requirement | Scenario | Evidence (code / test) | Result |
|---|---|---|---|
| HelperStatus JSON Contract | Active grant serializes with an `at` expiry | `nopass-core/src/state.rs` / `active_from_serializes_the_exact_golden_json_for_at_expiry` (exact byte golden incl. field order) | COMPLIANT |
| | Inactive uid serializes with a null expiry | `state.rs::inactive` / `inactive_serializes_false_active_and_null_expires` (exact byte golden) | COMPLIANT |
| | stdout and state-file content share the same shape | `ops::status` L367 `print!("{}", status.to_json_line())` and `statefile::write` L81 `status.to_json_line()` — the same serializer / `state.rs > active_and_inactive_status_share_the_identical_serializer_shape`. **No test ever captures the `status` subcommand's stdout**; only `status_inner` (which returns a struct and prints nothing) is tested. I confirmed the real stdout manually: it matches the state-file line byte-for-byte (see W4) | PARTIAL |
| **Requirement verdict** | | All seven fields present; `schema: 1` pinned | **PARTIALLY SATISFIED** |
| State File Placement and Permissions | Enable writes the state file with correct mode | `statefile.rs::write` (`O_EXCL` + `fchmod 0644` on the open fd + `fsync` + `rename`) / `write_atomically_creates_the_state_file_with_mode_0644_and_the_exact_json_line`, `write_holds_mode_0644_under_a_hostile_umask`; `root_system.rs` asserts `0o644` on the real file | COMPLIANT |
| | `/run/nopass/` missing at helper start | `statefile.rs::ensure_run_dir` (creates then `set_permissions(0o755)`) / `write_auto_creates_the_run_directory_at_mode_0755_when_missing`; `root_system.rs > real_run_nopass_directory_is_created_at_0755_when_missing_before_enable`. **In the real transaction `LockGuard::acquire` creates the directory first, with a bare `create_dir_all` and no mode**, so its permissions are `0777 & ~umask`; `ensure_run_dir` then short-circuits on an existing directory and never corrects it (see W2) | PARTIAL |
| **Requirement verdict** | | "updated on every enable, disable, expire" holds; "and boot-time cleanup sweep" does not — the sweep *deletes* the state file rather than updating it to `active:false` (W7) | **PARTIALLY SATISFIED** |
| Journald Audit Records | Successful enable is journaled | `journal.rs::{init,audit}` + the terminal `journal::audit(... AuditOutcome::Ok ...)` in `enable_inner` L235, `disable_inner` L331, `expire_uid_inner` L484 / `journal.rs > syslog_identifier_and_field_prefix_are_pinned_per_design_md_section_9`, `audit_emits_every_documented_field_name_and_value`. **No test asserts that any success path actually calls `audit`** — all 8 `capture_audit` sites in `ops.rs` are rejection/rollback paths (see W1) | PARTIAL |
| | A rejected operation is still journaled | `ops.rs::audit_rejection` / 5 runtime-captured assertions with distinct stable tokens: `enable_inner_audits_a_uid_rejection_with_the_rejected_outcome` (`uid_rejected_root`), `enable_inner_audits_a_not_sudoer_rejection` (`not_sudoer`), `enable_inner_audits_a_rolled_back_timer_failure...` (`rolled_back`/`timer_failed`), `disable_inner_audits_a_lock_busy_rejection` (`lock_busy`), `expire_uid_inner_audits_a_rejection_when_the_rule_file_cannot_be_removed` (`fs_error`), plus `duration_rejection_is_audited_with_the_real_uid` and its uid-triangulation twin. `error.rs > uid_rejected_audit_reason_carries_a_distinct_token_per_rejection_variant` covers all four uid causes | COMPLIANT |
| **Requirement verdict** | | Field names, `SYSLOG_IDENTIFIER`, `NOPASS_` prefix and stderr fallback all pinned; delivery over a real journald socket is unproven by any gate (no journald in the Debian/Fedora root images; the systemd lane is documented as not a gate) | **PARTIALLY SATISFIED** |

**Compliance summary**: 46/56 scenarios COMPLIANT · 8 PARTIAL · 2 FAILING. 11/20 requirements fully satisfied, 7 partially satisfied, 2 not satisfied.

---

### Coherence (Design)

| Decision (design.md) | Followed? | Notes |
|---|---|---|
| §4.1 enable step order 1-18 | Yes | Verified line by line against `ops::enable`/`enable_inner`. One benign refinement: `sanitize_username` is hoisted above step 5 so rejection audits never carry a raw value |
| §4.1 rollback table, all 7 rows | Yes | Rows 8-10/12 → exit 16 + `unlink(tmp)`; row 11 → exit 14; row 13 → warn-only; row 15 → `unlink(rule)` + `fsync(dir)` + `rolled_back` audit + exit 17; row 16 → logged, exit stays 0. Row 16 carries the strongest test in the suite (`enable_inner_statefile_write_failure_is_logged_but_never_changes_the_exit_code`, with a documented reverted-mutation RED proof) |
| §4.2 unlink precedes timer stop | Yes | `disable_inner` L312-318, pinned by `disable_inner_removal_failure_propagates_and_never_stops_the_timer` using an empty `ScriptedRunner` (proof by construction) |
| §4.3 lock before header re-read | Yes | `expire_uid_inner` L429 then L438 — the stale-revocation race is genuinely closed |
| §4.4 `--boot` makes no systemd call | Yes | Enforced by signature: `expire_boot_inner` has no `CommandRunner`/`Binaries` parameter |
| §4.4 orphan `*.state` cleanup | Yes | `sweep_orphan_state_files`, runs **after** the rule sweep so a just-deleted rule cannot save its own state file |
| §4.5 `status` takes no lock, writes nothing | Yes | `status_inner_never_takes_the_lock_or_creates_any_directory` |
| §6 exact `systemd-run` argv, 4 `--timer-property` | Yes | Verbatim, including UTC `Z`. `tasks.md` 7.1's "three" is a miscount, correctly resolved in design's favour and disclosed |
| §7 three `data/` artifacts | Yes | All three byte-match the design blocks |
| §9 exit-code table | **No** | The `getpwuid`-absent cause reaches exit 1, not 11 (C1) |
| §9 `REASON` never raw subprocess text | Yes | `audit_reason()` is a closed `&'static str` set; `audit_reason_never_leaks_raw_subprocess_stderr_text` |
| §9 `journal::init` uses `init` | Deviation (accepted) | `try_init` for test safety; configuration otherwise identical. Recorded |
| §3 `nix::sys::stat::stat` for binary resolution | Deviation (accepted) | `std::fs::metadata`; follows symlinks identically and additionally rejects directories. Recorded |
| File Changes table omits `helper/src/lib.rs` | Deviation (accepted) | A Cargo integration test is its own crate and can only reach the package's public library API. Recorded |
| `Layout::under` subpath shape, `HelperStatus: Deserialize` | Unspecified by design | `<root>/sudoers.d` + `<root>/run/nopass`, mirroring production. Harmless |
| Context failures (exit 10) unaudited | Deviation (accepted) | No uid is resolvable at that point; inventing a sentinel would be worse. Recorded and reasoned in code |

All five previously-recorded deviations are genuinely recorded in code or `tasks.md`, and none of them breaks a spec requirement. Confirmed, not re-litigated.

### Self-reported invariants — independently confirmed

| Claim | Confirmed | Where |
|---|---|---|
| `sanitize_username` strips everything outside the allowlist, newlines included | Yes | `template.rs` L22-32 is a positive allowlist filter; `sanitize_username_strips_line_terminators` covers `\n`, `\r\n`, U+0085, U+2028 |
| `ops::enable` sanitizes once and feeds both the probe and the template | Yes | `enable_inner` L174; `render_rule` re-sanitizes idempotently. Pinned by `enable_inner_sanitizes_the_raw_username_before_the_sudo_probe` |
| `SystemRunner` refuses a non-absolute program before spawning | Yes | `runner.rs` L85-87, before `Command::new`. Two tests |
| `Binaries::resolve` accepts only an absolute regular file | Yes | `bins.rs` L71 (`is_absolute()` **and** `metadata().is_file()`); directory, relative and dangling-symlink candidates each have a test |
| `list_rule_uids` requires `file_type().is_file()` before the filename filter | Yes | `fileops.rs` L260-263, before the name check and before any content read; `list_rule_uids_skips_a_directory_named_like_a_rule`, `..._skips_a_symlink_named_like_a_rule` |
| `admit_uid` rejects uid 0 before the range and delegates to `UidRange::admits` | Yes | `checks.rs` L71-80; `admit_uid_agrees_with_uid_range_admits_across_the_boundary_table` drives a 4x9 table |
| `expire --uid` takes the lock before re-reading the header | Yes | `ops.rs` L429 then L438 |
| `expire --boot` cannot call systemd; removes orphan state files | Yes | Structural (no runner parameter) + 4 orphan tests |
| Every enable/disable/expire outcome is journaled, rejections included, with a distinguishing reason token | **Partially** | Rejection paths: yes, 7 runtime-captured assertions. **Success paths: the calls exist but no test asserts them** (W1) |

---

### TDD Compliance

| Check | Result | Details |
|---|---|---|
| TDD Evidence reported | Partial | The `apply-progress` artifact readable at `sdd/m1-core-helper/apply-progress` (obs #5703, revision 17 of 17) contains only the Phase 10 section; phases 1-9 are "preserved by reference" in superseded revisions that the topic key no longer returns. No consolidated "TDD Cycle Evidence" table is retrievable |
| All tasks have tests | Yes | Every one of the 45 tasks naming a RED test has that test present and passing; verified by name |
| RED confirmed (test files exist) | Yes | All 6 test binaries and all named `#[cfg(test)]` modules exist |
| GREEN confirmed (tests pass) | Yes | 243/243 passed on execution by this phase, exit 0 |
| Triangulation adequate | Yes | Notably strong: `sanitize_username` (13 cases), `admit_uid` vs `UidRange::admits` (36 combinations), `validate_until` (6 boundaries incl. 59/60/28800/28801), `format_utc_rfc3339` (4 vectors), uid-rejection reason tokens (4 variants), audit uid triangulation |
| Safety net for modified files | Yes | Task 10.5 discloses two additive, non-weakening corrections to pre-existing Phase 4/7 tests, found by actually running the documented root-lane command |

**TDD compliance**: 5/6 checks pass. The one shortfall is artifact retrievability, not protocol adherence — `tasks.md`'s Phase 6 and Phase 7 blockquotes are themselves mid-build RED/GREEN evidence, and one test (`enable_inner_statefile_write_failure_is_logged_but_never_changes_the_exit_code`) documents an explicit reverted-mutation RED proof in its comment. I am not treating the missing table as a CRITICAL.

### Test Layer Distribution

| Layer | Tests | Files | Tools |
|---|---|---|---|
| Unit (in-crate `#[cfg(test)]`, pure or `ScriptedRunner`) | 208 | 19 modules | `cargo test` |
| Integration, unprivileged (own crate, real filesystem via `Layout::under`) | 22 | `fileops_tempdir.rs`, `data_artifacts.rs` | `cargo test`, `tempfile` |
| Integration, root-gated (real `/etc/sudoers.d`, real `visudo`, real binary subprocess) | 13 (1 executes unprivileged, 12 skip) | `root_system.rs` | `cargo test` + Debian/Fedora containers |
| Manual, non-gated | 0 automated | `Containerfile.systemd` | podman, never executed end to end |
| **Total** | **243** | **6 binaries** | |

### Assertion Quality

No banned pattern found: zero tautologies, zero assertions that never call production code, zero ghost loops, zero smoke-only tests, zero mock-heavy files (`ScriptedRunner` is a contract double whose `Drop` *fails* on an unexhausted script — the opposite of a permissive mock). Empty-collection assertions always have a non-empty companion (`list_state_uids_returns_an_empty_vec_when_the_run_directory_does_not_exist` pairs with `list_state_uids_finds_canonical_state_files_and_skips_non_canonical_and_non_file_entries`). Two duplicate tests were deliberately removed during the build and the removal reasoned in a comment.

The weaknesses below are not trivial assertions; they are **absent** assertions.

| File | Subject | Issue | Severity |
|---|---|---|---|
| `src/ops.rs` | terminal `journal::audit(... Ok ...)` in `enable_inner`/`disable_inner`/`expire_uid_inner` | No `capture_audit` on any success path — deleting all three calls leaves 243/243 green | WARNING |
| `src/ops.rs` | `pub fn status`'s `print!` | Never executed by a test; only `status_inner` is | WARNING |
| `src/bins.rs` | `Binaries::system()` | Candidate lists and their order are never asserted | WARNING |
| `src/main.rs` | `main`/`run` | No test observes the compiled binary's process exit code unprivileged | SUGGESTION |
| `tests/root_system.rs` | 12 gated tests | Report as "passed" while executing zero assertions | WARNING |

**Assertion quality**: 0 CRITICAL, 4 WARNING, 1 SUGGESTION.

### Quality Metrics

**Linter**: `cargo clippy --workspace --all-targets -- -D warnings` — no errors, no warnings (exit 0).
**Type checker**: N/A for Rust beyond the compiler; `cargo build --release` exit 0.

---

## Issues Found

### CRITICAL

**C1 — A documented exit-code cause is unreachable: `getpwuid`-absent uid exits 1, not 11.**

- **Where**: `crates/nopass-helper/src/ops.rs` L90 — `let raw_user = checks::lookup_user(uid)?;`
- **Mechanism**: `checks::lookup_user` maps `Ok(None)` (no passwd entry) to `HelperError::Internal(..)` → exit 1. It runs **before** `enable_inner` reaches `checks::admit_uid`, which is the only place `UidRejection::Unknown` → exit 11 is produced. The exit-11 path for this cause is therefore dead in production.
- **Evidence**: `PKEXEC_UID=4294967294 ./target/debug/nopass-helper enable` → exit 1 (expected 11). Nothing was written.
- **Violates**: `privilege-admission §UID Range Admission` scenario "uid not present in getpwuid → the helper exits 11"; `helper-cli §Typed Exit Code Mapping` ("Every failure condition MUST map deterministically to exactly one of the documented exit codes... No two distinct failure causes share a code with a different meaning") — exit 1 now means both "internal error" and "unknown target uid"; `design.md §9` row `UidRejected | 11 | ... no getpwuid entry`; `proposal.md` exit table row 11.
- **Why no test caught it**: admission is tested only through `enable_inner`, which is handed an already-resolved `raw_user`. The production `enable` wrapper has exactly one test (`enable_live_wrapper_rejects_context_when_pkexec_uid_is_absent`, exit 10), so L88-94 of the wrapper — duration, `lookup_user`, `login.defs` read — is otherwise unexercised.
- **Impact**: fail-closed, no security exposure, nothing written. It is a contract defect: the M2 tray is specified to distinguish 11 (retryable, user-facing "this account cannot be managed") from 1 (internal error, report a bug).
- **Fix shape** (not applied — verification does not edit): in `ops::enable`, resolve the username through a fallback that does not abort, or run `checks::admit_uid` before `lookup_user`, so a missing passwd entry surfaces as `HelperError::UidRejected(UidRejection::Unknown)`. Add a test at the `enable` wrapper level, not only at `enable_inner`.

### WARNING

**W1 — No success-path audit assertion.** All eight `capture_audit` call sites in `ops.rs` cover rejection, rollback or duration failures. Removing the terminal `journal::audit(&AuditRecord { outcome: AuditOutcome::Ok, .. })` from `enable_inner` (L235), `disable_inner` (L331) and `expire_uid_inner` (L484) would leave the whole suite green. This is the single clearest case of "the count doing more work than the assertions": `helper-observability §Journald Audit Records` scenario "Successful enable is journaled" and `expiry-policy`'s "the action is journaled" clause both rest on it. Cheap fix: wrap three existing success tests in the `capture_audit` helper that already exists in the same module.

**W2 — `/run/nopass` directory mode is umask-dependent.** `lock::LockGuard::acquire` (L56-59) creates the directory with a bare `create_dir_all` and no mode, and it runs *before* `statefile::ensure_run_dir`, which short-circuits on an existing directory and never corrects its permissions. The observed `0755` therefore depends on the ambient umask (`0o022`) rather than on code. Under a hostile umask the directory would be `0700`, and an unprivileged tray could not read the `0644` state file inside it — defeating the stated purpose of the requirement. `statefile.rs`'s own doc comment anticipates this race but does not close it. The root-lane assertion `real_run_nopass_directory_is_created_at_0755_when_missing_before_enable` passes for ambient reasons, not behavioural ones.

**W3 — Spec text contradicts shipped lock semantics.** `sudoers-rule-lifecycle §Mutation Serialization` scenario "Concurrent enable requests serialize" states the second caller "blocks on the flock until the first completes". The implementation is `LockExclusiveNonblock` → exit 15, deliberately, per design's Architecture Decision. The build documented this divergence in a test comment but never amended the delta spec. Archiving would merge a scenario into `openspec/specs/` that the code deliberately contradicts.

**W4 — `status` subcommand stdout is never asserted.** Only `status_inner` (which returns a struct) is tested. `ops::status`'s `print!("{}", status.to_json_line())` has no covering test, and the root lane never invokes `status` at all. I verified the real output manually and it is correct; a regression to the printed form would not be caught. This leaves `helper-observability`'s "stdout and state-file content share the same shape" scenario resting on a shared-serializer argument rather than on observed output.

**W5 — The production `Binaries::system()` candidate table is unpinned.** All twelve `bins.rs` tests construct `Binaries::from_candidates` with synthetic paths; the only test touching `system()` is a negative one (`system_binaries_reject_an_unknown_name`). A typo or reordering in the real table — `visudo` order `/usr/sbin` → `/sbin` → `/usr/bin` is distro-load-bearing per design §3 — would fail nothing.

**W6 — `expiry-policy` "Permanent and until-reboot enables never touch the timer" is contradicted by design.** `systemd-run` is genuinely never called (proven structurally by `ScriptedRunner`'s exhaustion check), but `systemctl stop` *is* called unconditionally, as design §4.1 step 14 requires so a stale timer from a prior `At` activation is cleared. The scenario says "neither `systemctl` nor `systemd-run` is invoked". Behaviour is correct; the spec sentence is not.

**W7 — `helper-observability` "State File ... MUST be updated on every ... boot-time cleanup sweep" is not met.** The boot sweep *deletes* a state file whose rule it removed (via `sweep_orphan_state_files`) rather than updating it to `active:false`. This is the design's intent (§4.4) and the tray is specified to treat a missing state file as "unknown, reconcile", but the spec's literal MUST is unsatisfied.

**W8 — The root lane can pass green while misconfigured.** `root_gate` skips (and the test reports "ok") whenever either condition is false — including the case `NOPASS_ROOT_TESTS=1` set but the process not root. A CI job that sets the variable in a non-root container would report 13/13 passing with 12 tests having executed nothing. See the dedicated assessment below.

**W9 — `privilege-admission` "uid 0 is always rejected → exits 11" is unreachable via pkexec.** `PKEXEC_UID=0` is rejected at context resolution as exit 10 (design §9 documents 10 for a zero `PKEXEC_UID`); the exit-11 path for uid 0 exists only in `admit_uid`, reachable from `expire`/library callers. Behaviour is strictly safer than the spec; the two documents disagree.

### SUGGESTION

- **S1** — No test observes the compiled binary's process exit code in the unprivileged lane; `main::run` is untested and every exit-code assertion is library-level. `env!("CARGO_BIN_EXE_nopass-helper")` (already used by `root_system.rs`) makes an unprivileged process-level test of exits 2, 10, 11 and 1 about twenty lines. This is the single cheapest addition that would have caught C1.
- **S2** — Items reachable only from tests (discharging the Phase 7 blockquote's obligation 3, "name explicitly anything still reachable only from tests" — the dead-code lint no longer reports these because `lib.rs` makes them public API): `checks::in_admin_group` (called only from `root_system.rs`; spec-permitted, since group membership "MAY be used only as a fast pre-check"), `journal::AuditEvent::Status` and `journal::AuditOutcome::Error` (declared members of design §9's documented value domains, never constructed in production). None is a defect; all three are now named. Every other module is reachable from `main::dispatch`: `dispatch` → `ops::{enable,disable,status,expire}` → `uid`, `checks`, `lock`, `fileops`, `timer`, `statefile`, `journal`, `bins`, `runner`, `error`, `cli` and all seven `nopass-core` modules.
- **S3** — `sudoers-rule-lifecycle` "Rename fails after successful validation" asserts exit 16 and tmp cleanup but never the clause "any prior rule file is left untouched"; both fixtures plant a symlink rather than a legitimate prior rule. The equivalent clause *is* asserted for the `visudo` path.
- **S4** — The exit-1 `lookup_user` failure in `ops::enable` is not journaled, although `uid` is known at that point — the same reasoning that made `resolve_expiry_audited` audit exit 13 applies here.
- **S5** — `Containerfile.systemd` was authored but never executed end to end, so PRD §10.10 (a `--until-reboot` grant removed by `nopass-cleanup.service` on the next boot) and `journalctl -t nopass-helper` delivery remain unproven by anything. Correctly scoped as manual by design §8; noted so M4 QA inherits it explicitly.
- **S6** — `fileops::remove_rule`'s read-then-unlink TOCTOU is recorded in a code comment and accepted (only root can write `/etc/sudoers.d`). Confirmed as recorded, not re-litigated.

---

## Answers to the three judgement questions

**1. Coverage honesty — is the count doing more work than the assertions?**

Mostly no. 208 of the 243 tests carry real, specific assertions, and several are unusually strong: `ScriptedRunner` asserts full `CommandSpec` equality and panics on an unexhausted script, so "no systemd-run was called" is proven by construction rather than by an easily-deleted assertion; `expire_boot_inner` cannot call systemd because it has no runner parameter; `admit_uid_agrees_with_uid_range_admits_across_the_boundary_table` drives 36 combinations to prevent two comparisons drifting apart. The suite deleted duplicate tests rather than keeping them for the count.

Four requirements would survive a behavioural regression:

- **Success-path journaling (W1)** — delete three `journal::audit` calls: 243/243 still green.
- **`status` stdout (W4)** — change the printed form: nothing fails.
- **`Binaries::system()` table (W5)** — reorder or misspell a candidate: nothing fails.
- **`/run/nopass` directory mode (W2)** — the `0755` assertion is satisfied by the ambient umask, not by the code under test.

And one requirement is not merely under-asserted but actually violated in production while every test passes: **C1**, because admission is tested one layer below where the production path makes its decision.

**2. The root lane — acceptable residual risk, or does it need a harder gate?**

Acceptable for M1, with one change I would make before archive. What is genuinely good: the gate logic is factored into a pure `gate_satisfied(bool, bool)` proven unconditionally on every machine, so the AND-not-OR property is real evidence rather than a claim; each skip prints the exact unmet condition to stderr; the README and module doc say so plainly; the lane was actually executed as real root in both Debian and Fedora images during Phase 10, and doing so surfaced two genuine pre-existing test bugs.

The residual risk that is *not* acceptable is W8: `NOPASS_ROOT_TESTS=1` set without root silently skips. That combination is never a legitimate configuration — it is always a misconfigured runner — and it is exactly the failure mode that would let a CI root lane rot green. Making that one combination `panic!` instead of `return` is a two-line change that converts "silently unproven" into "loudly broken", and it costs nothing on a developer machine (where the variable is unset). Everything else about the lane is honest and I would not require an `#[ignore]` migration or a CI job for M1.

**3. Anything the ten phases never covered that the specs require.**

Each phase verified its own layer, and the seams between layers are where the gaps are:

- The **production `ops::enable` wrapper** (`ops.rs` L88-94) is verified only for its exit-10 branch. That seam holds C1.
- The **process boundary** — `main::run`, `journal::init`'s placement before dispatch, and the real binary's exit status — is verified only under root. No unprivileged test spawns the binary.
- The **`status` transaction end to end** is verified nowhere: not its stdout, not its exit code; the root lane never invokes it.
- **Success-path journaling** at the call sites (each phase verified `journal::audit` in isolation, or its rejection callers, but never a success caller).
- The **`/run/nopass` directory-creation seam between `lock` and `statefile`** — Phase 6 owned the lock, Phase 8 owned the state file, and neither owned the mode the other leaves behind.
- **Four spec sentences** (W3, W6, W7, W9) that the design deliberately overrides and that no phase was tasked with reconciling back into the delta specs, because each phase read design.md as authoritative and moved on. That is the right call mid-build and the wrong state to archive in.

---

## Gate results observed by this phase

| Gate | Command | Exit | Result |
|---|---|---|---|
| Tests | `cargo test --workspace` | 0 | 243 passed, 0 failed, 0 ignored |
| Release build | `cargo build --release` | 0 | clean |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | clean |
| Task completion | `tasks.md` | — | 45/45 `[x]`, 0 unchecked |
| Working tree | `git status --porcelain` | — | clean at `22204f2` |
| Live exit-code probes | `./target/debug/nopass-helper` (6 unprivileged invocations) | — | 5 of 6 match the documented contract; the `getpwuid`-absent case does not (C1) |
| Coverage | — | — | Not available; threshold is 0 |

All exit statuses were read directly from `$?`, never inferred from output text. No source file, commit, or attempt token was touched by this phase.

## Verdict

**FAIL** — 1 CRITICAL, 9 WARNING, 6 SUGGESTION. One spec requirement is violated in production (`C1`), which blocks archive: archiving would merge five delta specs into `openspec/specs/` while the implementation contradicts two of their requirements. The remaining work is small and well-bounded — one behavioural fix, three cheap test additions, one two-line gate hardening, and four spec-sentence reconciliations that record decisions the design already made.

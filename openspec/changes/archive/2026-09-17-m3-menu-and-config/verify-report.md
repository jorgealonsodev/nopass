```yaml
schema: gentle-ai.verify-result/v1
evidence_revision: sha256:9f3a126749f3f49340e881fe47d0e60b4697c547
verdict: pass
blockers: 0
critical_findings: 0
warnings: 6
suggestions: 6
requirements: 21/26 satisfied, 5 partial, 0 not satisfied
scenarios: 44/48 satisfied, 2 partial, 1 not satisfied, 1 not verified (Lane C)
test_command: cargo test --workspace
test_exit_code: 0
test_output_hash: sha256:33ad70f318edbbe535c25a57403bbb69512c9139a7cba3e172997b0024ab71e1
test_command_c_locale: LANG=C LC_ALL=C cargo test --workspace
test_exit_code_c_locale: 0
test_output_hash_c_locale: sha256:33ad70f318edbbe535c25a57403bbb69512c9139a7cba3e172997b0024ab71e1
build_command: cargo build --release
build_exit_code: 0
lint_command: cargo clippy --workspace --all-targets -- -D warnings
lint_exit_code: 0
gates_run: 4
gates_passed: 4
neutering_experiments: 11
neutering_experiments_caught: 10
```

## Verification Report

**Change**: `m3-menu-and-config`
**Version**: 8 delta specs (`tray-menu`, `activation-consent`, `user-config`, `autostart-entry`,
`localization`, `tray-privileged-invocation`, `helper-cli`, `privilege-admission`)
**Mode**: Strict TDD
**HEAD**: `a808c53` (clean worktree, verified before and after every experiment) ·
**Toolchain**: cargo 1.85.1 / rustc 1.85.1 (pinned `rust-toolchain.toml` channel 1.85)
**Host locale**: `es_ES.UTF-8` · **Container runtime**: docker 29.8.0 (`podman` absent)

---

## 1. Completeness

| Metric | Value |
|--------|-------|
| Tasks total | 74 |
| Tasks complete `[x]` | 69 |
| Tasks open `[ ]` | 5 |

The five open boxes were each checked against the tree rather than taken on trust, because
"open" can hide two different things: work not done, and work whose result was invented.

| Task | Kind | Verified state |
|---|---|---|
| 7.3 | Lane C, human, real desktop — full-tree keyboard navigation | Unchecked. `tests/manual/README.md` §12 exists (added by 11.1) and its **Results** table row 12 is empty. No result recorded anywhere in the change. ✅ nothing fabricated |
| 11.2 | Lane C, human — execute the manual checklist | Unchecked. The entire Results table (rows 1–15) and the Environment table are blank. ✅ nothing fabricated |
| 10.8 | Pre-agreed fallback, deliberately NOT taken | Unchecked, with an explicit `**NOT TAKEN**` note. Confirmed correct: `scripts/run-lane-polkit.sh` exits 0 against a real `polkitd` (gate 4 below), so neither degrade action applied. `data_artifacts.rs` was **not** relabelled and no substring assertion is presented as the Rank 2 gate — `polkit_contract.rs`'s module doc says the opposite, in as many words |
| 11.3 | Carry-forward marker — archived M2 tasks 7.7 / 11.4 | Unchecked by design. Both are still `[ ]` in `openspec/changes/archive/2026-09-15-m2-tray/tasks.md` (read-only reference, unmodified in this change) |
| 11.4 | Carry-forward marker — `Containerfile.systemd` never run end to end | Unchecked by design. The file exists and is named by no runner script; `scripts/` contains no `run-lane-systemd.sh`. It is correctly out of Phase 10's dependency chain: `Containerfile.polkit` is a plain rootful image with no systemd-as-PID-1 |

Two claims inside completed tasks were re-checked because they assert an absence, and an
absence is the easiest thing to claim falsely:

- **1.10 / 6.3 / 7.2 — "`GrantDuration::ALL` is the single ordering constant."** True.
  `duration.rs:38` is the only `ALL` array for this type; `menu.rs:250` (`duration_items`) and
  `tray.rs:336` (`RadioGroup::select`) both index into that same array, and
  `duration.rs:174` pins its exact sequence.
- **9.1 — "`UNIT_PROPERTIES` is the only place these five spellings exist."** True.
  `property_args()` (`timer.rs:53`) and `synthesize_unit()` (`timer.rs:76`) both read it;
  `grep` finds no second spelling of `AccuracySec=`/`Persistent=`/`WakeSystem=`/
  `RemainAfterElapse=`/`Type=oneshot` in the tree outside that array and the argv-pinning
  test that quotes it.

---

## 2. Build & Tests Execution

Every command below was run directly and its **own** exit status read — never a pipeline's.

| # | Command | Exit | Observed result |
|---|---------|------|-----------------|
| 1 | `cargo test --workspace` (host locale `es_ES.UTF-8`) | **0** | 645 passed, 0 failed, 1 ignored, 24 suites |
| 2 | `LANG=C LC_ALL=C cargo test --workspace` | **0** | 645 passed, 0 failed, 1 ignored, 24 suites |
| 3 | `cargo build --release` | **0** | Finished `release` profile, 0 warnings, 0 errors |
| 4 | `cargo clippy --workspace --all-targets -- -D warnings` | **0** | No issues found |
| 5 | `scripts/assert-single-reactor.sh` | **0** | PASS — exactly one async-io major (v2.6.0), zero tokio, no dbus/glib/gtk, zbus tokio feature disabled; `cargo +1.85 build --release -p nopass` succeeded |
| 6 | `scripts/run-lane-polkit.sh` | **0** | docker detected, `Containerfile.polkit` built, `dbus-daemon --system` + real `/usr/lib/polkit-1/polkitd` started, **3 passed** (0.02 s) |
| 7 | `scripts/run-lane-journal.sh` | **0** | `Containerfile.journald`, standalone `systemd-journald`, **4 passed** (1.75 s) |
| 8 | `scripts/run-lane-root.sh` | **0** | Debian **17 passed** (4.45 s) + Fedora **17 passed** (4.60 s) |
| 9 | `scripts/run-lane-b.sh` (not a configured gate — see WARNING-5) | **0** | `dbus-run-session` with the isolated config, **20 passed** (4.26 s) |

Gate commands 5–8 are exactly `openspec/config.yaml`'s `verify.gate_commands`, in order.
All four exit 0. Command 9 is not in that list; it was run anyway because M3 puts two of its
own tasks (7.2, 8.6) in that lane.

`cargo fmt` was **not** run and is **not** reported as a gate: this repository has no CI
workflow, no `cargo fmt` reference in any config, script or doc, and `openspec/config.yaml`
`verify` names four gate commands, none of which is `fmt`. Same standing as in the archived
M2 and m3a reports.

**Coverage**: ➖ Not available — no coverage tool configured (`coverage_threshold: 0`). Not a
failure.

### 2.1 The locale check, done properly

Both runs were compared by normalized output, not just by exit code. The set of
`test <name> ... <outcome>` and `test result: ...` lines, with timings stripped and sorted,
hashes identically under both locales:

```
es_ES.UTF-8 : sha256:33ad70f318edbbe535c25a57403bbb69512c9139a7cba3e172997b0024ab71e1
LANG=C      : sha256:33ad70f318edbbe535c25a57403bbb69512c9139a7cba3e172997b0024ab71e1
```

The test-name sets are byte-identical (641 distinct names, `diff` empty). No test appears,
disappears, or changes outcome with the locale. This matters here because two of this
change's own gates shell out to tools whose diagnostics localize —
`systemd_unit_contract.rs:79` and `polkit_contract.rs:75` both force `LC_ALL=C`/`LANG=C` on
the child and assert only on non-localized identifiers, which is why the pair of runs agrees.

### 2.2 The vacuous-pass check

Under gate 1 alone, 44 of the 645 tests report `ok` in 0.00 s without executing anything:

| Suite | Tests | Env gate | Executed by |
|---|---|---|---|
| `dbus_session.rs` | 20 | `NOPASS_DBUS_TESTS` | command 9 (`run-lane-b.sh`) — 20 passed for real |
| `root_system.rs` | 17 | `NOPASS_ROOT_TESTS` | gate 8 — 17 passed, twice |
| `root_journal.rs` | 4 | `NOPASS_JOURNAL_TESTS` | gate 7 — 4 passed |
| `polkit_contract.rs` | 3 | `NOPASS_POLKIT_TESTS` | gate 6 — 3 passed |

Every scenario credited below to a lane is credited to that lane's own run, never to gate 1.
`lane_wiring.rs` (2 tests, incl. its own negative control) proves each of those four gate
names is claimed by a script under `scripts/`, so none of them executes in no lane at all —
the m3a remedy still holds with `NOPASS_POLKIT_TESTS` added. What `lane_wiring.rs` does
**not** check is whether that script is in `verify.gate_commands`; see WARNING-5.

The one `ignored` test is `systemd_unit_contract.rs:180
hidden_gate_precondition_under_stripped_path`, deliberately `#[ignore]`d because it is the
payload the PATH-stripped subprocess in `absence_of_systemd_analyze_fails_the_gate_not_skips_it`
re-spawns. It is not a skipped check; it is a fixture.

---

## 3. Spec Compliance Matrix

26 requirements, 48 scenarios, across 8 delta specs.

### 3.1 `activation-consent` — 4 requirements / 7 scenarios — **all satisfied**

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| No Grant Dispatch Without Recorded Consent | Menu-triggered activation with no recorded consent dispatches nothing | `app.rs:1020 toggle_requested_while_unconsented_dispatches_nothing_and_notifies_once` — drives `handle_toggle` with `ScriptedRunner::new(vec![])` (panics on any call) | ✅ Satisfied |
| No Grant Dispatch Without Recorded Consent | Non-menu activation path dispatches nothing | Same test. `tray.rs:169` (`Inner::activate`, SNI left-click/keyboard) and `tray.rs:218` (fallback toggle item) and `tray.rs:241` (`MenuNodeKind::Toggle`) **all** raise the one `TrayEvent::ToggleRequested`; `app.rs:712` routes it to the single `handle_toggle` | ✅ Satisfied |
| No Grant Dispatch Without Recorded Consent | Single-instance nudge never itself dispatches an enable | `app.rs:1041 activate_requested_the_second_instance_nudge_never_dispatches_an_enable_while_unconsented`; `app.rs:713` is literally `Event::ActivateRequested => self.tray.reassert()`; Lane B `dbus_session.rs:1165 ten_rapid_activations_produce_exactly_one_notification_and_zero_pkexec_spawns` over the real wire | ✅ Satisfied |
| First Activation Branches the Menu Instead of Granting | Confirming the branch grants exactly once | `app.rs:1077 confirming_the_branch_grants_exactly_once`; `consent.rs:231 confirm_without_persist_grants_exactly_once_per_arm` (second call yields `None`) | ✅ Satisfied |
| First Activation Branches the Menu Instead of Granting | Cancelling the branch grants nothing | `app.rs:1096`; `consent.rs:243`; Lane B `dbus_session.rs:534 exported_menu_reflects_the_consent_branch_and_cancel_raises_consent_cancelled_over_the_real_wire` | ✅ Satisfied |
| Don't-Warn-Again Persists Consent | A later activation skips the branch once consent is recorded | `app.rs:1110 confirming_with_persist_writes_the_config_and_a_later_activation_skips_the_branch` — real `TempDir` config, re-read off disk | ✅ Satisfied |
| A Failed Consent Write Re-Warns | A write failure leaves the next activation still gated | `app.rs:1151 a_write_failure_during_persist_re_warns_rather_than_silently_granting_the_next_activation`; `consent.rs:267` | ✅ Satisfied |

**The invariant, proven at the type level rather than asserted.** `Granted` (`consent.rs:26`)
is a tuple struct with a private `()` and no `Default`/`new`/`Clone`/`Copy`/`From`.
`EnableRequest::new` (`outcome.rs:52`) is the only constructor of `Action::Enable`'s payload
and demands a `Granted`. `grep` over the whole workspace finds exactly **three**
`EnableRequest::new` sites in production — `app.rs:459`, `app.rs:497`, `app.rs:526` — and each
is inside the `Some(granted)` arm of a `ConsentState::grant()` match or of
`ConsentState::confirm()`'s return. There is no fourth. The test-only producer
`consent.rs:141 granted_for_test()` is `#[cfg(test)] pub(crate)`: absent from release builds
and unreachable from `tests/` integration crates. `menu.rs:782` and `tray.rs:644` additionally
scan their own production source and fail if the strings `EnableRequest`, `consent::Granted`
or `outcome::Action` ever appear there.

Neutering experiment **F4** (below) confirms the runtime half: forcing `grant()` to always
return `Some` fails 9 tests, 4 of them driving real activation paths through `App`.

### 3.2 `autostart-entry` — 5 requirements / 6 scenarios — **4 satisfied, 1 partial**

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| Autostart Path Honors XDG_CONFIG_HOME | Default path when unset | `autostart.rs:134`, `:123`, `:145` (unset / set / empty) | ✅ Satisfied |
| Create Writes the Template Verbatim | Creating from a missing autostart directory | `autostart.rs:169 enable_creates_a_missing_autostart_directory_and_writes_bytes_byte_equal_to_template`; `atomicfile.rs:68` `create_dir_all`; `TEMPLATE` is `include_str!` of `data/nopass.desktop` and `enable(p)` takes no content parameter at all | ✅ Satisfied |
| Remove Deletes Only the NoPass Entry | Unchecking removes without touching siblings | `autostart.rs:203 disable_deletes_the_entry_and_leaves_a_sibling_desktop_file_untouched` | ✅ Satisfied |
| Remove Deletes Only the NoPass Entry | Removing an already-absent entry does not error | `autostart.rs:221` | ✅ Satisfied |
| Checkbox State Reflects On-Disk Truth | Externally deleted entry reflected on the next build | `autostart.rs:234–269` (4-row table incl. `Hidden=true`, `X-GNOME-Autostart-enabled=false`, `Indeterminate`); `menu.rs:633 start_with_session_checkbox_reflects_autostart_read_disk_truth_and_toggling_updates_the_next_build` builds a *fresh* `MenuModel` from a fresh `autostart::read` | ✅ Satisfied |
| Packaging Ships No Autostart Entry By Default | A fresh install has no autostart entry | Lane A half: `autostart.rs:291 packaging_manifest_installs_no_file_under_any_autostart_directory` scans `crates/nopass/Cargo.toml`, `debian/postinst`, `debian/prerm` for `autostart/`. Lane C half (real logout/login on a freshly installed package) — `tests/manual/README.md` §14, **unexecuted** | ⚠️ Partial |

### 3.3 `helper-cli` (MODIFIED) — 1 requirement / 5 scenarios — **all satisfied**

| Scenario | Evidence | Result |
|---|---|---|
| Each external call uses its exact absolute-path argv | `timer.rs:125 schedule_builds_the_exact_pinned_systemd_run_argv_in_order` + M1/M2 `runner.rs`/`bins.rs` suite, green in gates 1–2 | ✅ Satisfied |
| No candidate binary path exists on the host | `bins.rs` resolution tests, green | ✅ Satisfied |
| The synthesized systemd-run unit is accepted by real systemd | `systemd_unit_contract.rs:89 the_production_unit_properties_are_accepted_by_systemd_analyze` — `synthesize_unit()` reads `UNIT_PROPERTIES`, the same array `property_args()` renders into argv; real `systemd-analyze verify` in Lane A | ✅ Satisfied |
| A corrupted property token fails the gate | `systemd_unit_contract.rs:122 a_corrupted_property_token_is_rejected_by_systemd_analyze` (`AccuracySec=1s` → `AccuracySecc=1s`). Independently reproduced as experiments **F5** and **F6** | ✅ Satisfied |
| Absence of systemd-analyze fails the gate, not skips it | `systemd_unit_contract.rs:194 absence_of_systemd_analyze_fails_the_gate_not_skips_it` re-spawns this exact test binary with `PATH` pointing at an empty dir and asserts the child **fails** with `toolgate::require`'s own panic text | ✅ Satisfied |

**The allowlist is exact, not lenient.** `our_diagnostics` (`systemd_unit_contract.rs:36`)
keeps only lines prefixed with one of our two synthesized files' paths or basenames. Under
experiment F5 I watched it correctly drop this machine's unrelated
`/etc/systemd/system/teamviewerd.service: PIDFile= references a path below legacy directory`
notice while surfacing
`nopass-expire-1000.timer:7: Unknown key name 'Persistant' in section 'Timer'` — a foreign
diagnostic filtered out and our own injected defect let through, in the same run.

### 3.4 `localization` — 4 requirements / 5 scenarios — **all satisfied** (see WARNING-3)

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| Catalogue Behind format.rs, Call Sites Unchanged | Spanish locale changes format.rs output without touching callers | `format.rs:774/:782/:792/:801` (`status_line_in`, `tooltip_in`, `toggle_label_in` under `Lang::Es`); `format.rs:717 every_msg_arm_renders_both_languages_and_they_are_distinct`; no signature outside `format.rs` changed (`menu.rs`/`tray.rs`/`app.rs` call the same public fns) | ✅ Satisfied |
| Locale Resolution Order With English Fallback | `LC_ALL` beats `LC_MESSAGES` and `LANG` | `format.rs:704 from_locale_string_table` — drives `from_env_vars` with explicit triples, including `LC_ALL=en_US` + `LC_MESSAGES=es_ES` ⇒ English, and the empty-but-present `LC_ALL` glibc case | ✅ Satisfied |
| Locale Resolution Order With English Fallback | No locale variable set resolves to English | Same table | ✅ Satisfied |
| The Sudoers Header Is Never Reachable | Header byte-identical under a Spanish locale | `template.rs:170 render_rule_header_is_byte_identical_under_a_spanish_process_locale` — re-execs the test binary as a child with real `LANG`/`LC_ALL`/`LC_MESSAGES=es_ES.UTF-8` and asserts the exact four header bytes lines. A real locale, not a simulated one | ✅ Satisfied |
| polkit's Own Dialog Localization Is Not Reimplemented | Installed policy carries a Spanish description and message | `data_artifacts.rs:125 policy_declares_spanish_description_and_message`; `data/com.enfoquestic.nopass.policy:8,:10` carry the `xml:lang="es"` variants | ✅ Satisfied |

### 3.5 `privilege-admission` (MODIFIED) — 1 requirement / 5 scenarios — **4 satisfied, 1 partial**

| Scenario | Evidence | Result |
|---|---|---|
| Installed policy declares the single action with required defaults | `data_artifacts.rs:29/:38/:52/:67` | ✅ Satisfied |
| The real polkit engine enumerates the installed action | `polkit_contract.rs:85` — **executed in gate 6** against a real `polkitd` on a private system bus. Asserts `pkaction --action-id … --verbose` exits 0, the action-id header, and `implicit active:   auth_admin_keep` | ✅ Satisfied |
| A malformed policy file fails enumeration | `polkit_contract.rs:113` — **executed in gate 6**. Three-part: the malformed id is rejected/omitted on direct query, absent from the unfiltered listing, *and* the valid sibling is still enumerated (so "polkitd died" cannot masquerade as a pass). The fixture (`com.enfoquestic.nopass.malformed.policy`) is a genuine GMarkup failure — an unclosed `<action>` — not a DTD-only violation | ✅ Satisfied |
| probe_polkit_readiness consumes the enumeration result | `preflight.rs:356/:364 classify_enumeration_*` prove the *classifier* never assumes readiness (empty list and missing-id both ⇒ `ActionAbsent`), and `preflight.rs:158 polkit_ladder` consumes it. But `probe_polkit_readiness` itself lives in `main.rs:341` and has **no test at all** — see WARNING-4 | ⚠️ Partial |
| Absence of a polkit authority fails the gate, not skips it | Two independent halves, both checked. Script half: I ran `run-lane-polkit.sh` with a `PATH` containing neither runtime → **exit 1**, `neither docker nor podman found on PATH`. Image half: `Containerfile.polkit`'s entrypoint `kill -0`s `polkitd` and `exit 1`s if it died | ✅ Satisfied |

### 3.6 `tray-menu` — 5 requirements / 8 scenarios — **4 satisfied, 1 partial**

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| Complete RF-03 Item Tree | The full item tree is present in the exported menu | Lane A `menu.rs:451 the_full_item_tree_is_present_in_order_for_any_merged_state_other_than_unknown` (7 nodes, exact kinds and exact labels); Lane B `dbus_session.rs:370` and `:443` decode the **real DBusMenu `GetLayout` reply** and match it item-for-item against `menu_tree`, including `RadioGroup.selected` and submenu nesting | ✅ Satisfied |
| Complete RF-03 Item Tree | Every item exposes standard keyboard-navigation properties | Lane C only, by the spec's own `Testable via`. `tests/manual/README.md` §12 written (task 11.1), **never executed** (task 7.3) | ⬜ Not verified |
| Duration Submenus Render All Six In Fixed Order | Both submenus render the same six items in the same order | `menu.rs:493` — asserts both kind sequences equal `GrantDuration::ALL` *and* both label sequences equal each other *and* the exact six English labels | ✅ Satisfied |
| Default-Duration Marker Reflects Configured State | The marker follows the configured default | `menu.rs:537 default_duration_marks_exactly_the_entry_matching_the_configured_default` — one `Some(true)` **and** exactly five `Some(false)` | ✅ Satisfied |
| Default-Duration Marker Reflects Configured State | Selecting a new default moves the marker and persists it | `menu.rs:559` (disk round-trip, next build re-derived from the persisted value); `app.rs:1264 default_duration_selected_persists_and_the_next_menu_model_marks_it` | ✅ Satisfied |
| Current Rule Renders As Insensitive Reference Items | Active grant lists path, user, expiry, remaining | `menu.rs:584` — 4 children, all `enabled: false`, exact labels including `Expires: <RFC-3339>` rather than a raw epoch | ✅ Satisfied |
| Current Rule Renders As Insensitive Reference Items | Inactive renders a single insensitive placeholder | `menu.rs:608`; plus `menu.rs:618` for the probe-active/file-unreadable third case | ✅ Satisfied |
| Start-With-Session Checkbox Toggles the Autostart Entry | Checking the box creates the autostart entry | `menu.rs:633`; `app.rs:1287 autostart_toggled_enables_a_disabled_entry_and_disables_an_enabled_one`; `tray.rs:278 checkmark_item` | ✅ Satisfied |

### 3.7 `tray-privileged-invocation` (MODIFIED) — 1 requirement / 4 scenarios — **all satisfied**

| Scenario | Evidence | Result |
|---|---|---|
| Enable invocation uses the exact documented argv for a timed duration | `app.rs:1230 a_left_click_toggle_uses_configs_default_duration_never_a_hardcoded_one` — `ScriptedRunner` asserts the exact `CommandSpec` and panics on drop if unused | ✅ Satisfied |
| Permanent duration omits both duration flags | `duration.rs:120 exactly_one_variant_over_all_produces_neither_flag`; covered end-to-end by `app.rs:1182` | ✅ Satisfied |
| Until-reboot duration uses the exact flag | `duration.rs:136`; end-to-end by `app.rs:1182` | ✅ Satisfied |
| All six durations render their documented argv with no collision | `app.rs:1182 all_six_durations_render_their_documented_argv_end_to_end_through_app_with_no_shell` — iterates `GrantDuration::ALL`, drives `App::handle(Event::DurationSelected(d))`, pins the exact `pkexec` spec per duration and asserts no `sh`/`-c`; `duration.rs:107` proves the XOR | ✅ Satisfied |

The hardcoded `now + 3600` the proposal names at `app.rs:328-344` is gone: `handle_toggle`
(`app.rs:459`) reads `self.config.default_duration`, and `handle_duration_selected`
(`app.rs:497`) uses the **selected** duration. Experiment **F9** proves those two are not
interchangeable.

### 3.8 `user-config` — 5 requirements / 8 scenarios — **3 satisfied, 2 partial**

| Requirement | Scenario | Evidence | Result |
|---|---|---|---|
| Config Path Honors XDG_CONFIG_HOME | Default path when unset | `config.rs:223` | ✅ Satisfied |
| Config Path Honors XDG_CONFIG_HOME | XDG_CONFIG_HOME overrides the default | `config.rs:212`, `:234` (empty-string branch) | ✅ Satisfied |
| Schema and Defaults | A missing file resolves to schema defaults | `config.rs:247 resolve_of_absent_yields_schema_defaults_and_no_fault` — and no file is created | ✅ Satisfied |
| Tolerant Parsing Never Blocks Startup | Malformed TOML degrades to defaults **with a warning** | Defaults ✅ (`config_tempdir.rs:16`, `:30`, `:44`). The **warning at startup** ❌: `main.rs:162` is `let (initial_config, _fault) = …` — the fault is discarded. The notification fires only at the first `Trigger::MenuOpened` (`app.rs:598 refresh_config`) | ⚠️ Partial |
| Tolerant Parsing Never Blocks Startup | An unrecognized `default_duration` value degrades to the default **and a warning is produced** | Default ✅. Warning ❌ — and the test pins the opposite: `config_tempdir.rs:222` asserts `fault == None` for exactly this input. See WARNING-1 | ❌ Not satisfied |
| Read At Startup, Written On Menu Selection | Selecting a new default duration writes the file | `config_tempdir.rs:103`; `menu.rs:559`; `app.rs:1264` | ✅ Satisfied (scenario) |
| Atomic Write | A successful write replaces the file atomically | `config_tempdir.rs:115 write_uses_atomicfile_and_leaves_no_tmp_sibling_behind` (also pins mode `0600`, distinguishing it from `autostart`'s `0644`) | ✅ Satisfied |
| Atomic Write | A failed write leaves the prior config intact | `config_tempdir.rs:128 a_write_that_fails_after_the_tmp_file_is_created_leaves_the_prior_config_untouched` (pre-planted symlink at the final path); `atomicfile.rs:108` refuses to rename over a symlink; `config_tempdir.rs:154` proves the `.bak` preservation path | ✅ Satisfied |

The requirement **"Read At Startup, Written On Menu Selection"** is marked ⚠️ Partial at the
requirement level even though its one scenario passes: its normative sentence says the config
"MUST be read exactly once at startup", and the implementation re-reads it at every menu open
by deliberate design (D4, task 8.3). See WARNING-2. The *write* half of the sentence holds —
`config::write` is reached only from `menu::select_default_duration` and
`App::handle_consent_confirmed`.

---

## 4. Neutering experiments

Eleven experiments. Each deletes or inverts exactly one thing, runs the gate that owns it, and
is reverted immediately; `git status --porcelain` was empty before the first and after the
last, and the full suite was re-run green at the end (645 passed, exit 0).

| # | Neutered | Gate run | Caught? | What it proves |
|---|---|---|---|---|
| **F1** | `Msg::ToggleUnavailableActionInFlight`'s `Lang::Es` text set equal to its English text | `cargo test --workspace` | ❌ **NOT caught** — 645 passed, exit 0 | The distinctness guard's `const ALL: [Msg; 40]` does not cover the two arms Phase 8 added to a 42-arm enum. **WARNING-3** |
| **F2** | `Msg::MenuQuit`'s `Lang::Es` set equal to English | `cargo test -p nopass --lib format::` | ✅ Caught by name — `every_msg_arm_renders_both_languages_and_they_are_distinct` fails with `MenuQuit must render distinct text per language` | The guard works — for the 40 arms it looks at. Contrast with F1 |
| **F3** | `unconsented_toggle_body()` switched from `Msg::NotifyConsentNeededBody` to `Msg::NotifyConfigUnreadableBody` | `cargo test -p nopass --lib` | ✅ Caught — `toggle_requested_while_unconsented_dispatches_nothing_and_notifies_once` (`app.rs:1037`) | The one remaining `Msg`-against-`Msg` assertion is **not** vacuous: it pins *which* catalogue entry the call site uses. It still cannot catch a wrong translation. **SUGGESTION-2** |
| **F4** | `ConsentState::grant()` made to return `Some(Granted(()))` unconditionally | `cargo test -p nopass --lib` | ✅ Caught — **9 failures**: `app.rs` 1020/1059/1096/1151 and `consent.rs` 170/243/267/196/288 | Four of the nine drive real activation paths through `App`, not just the state machine |
| **F5** | Production `UNIT_PROPERTIES` entry `Persistent=false` → `Persistant=false` | `cargo test -p nopass-helper --test systemd_unit_contract` | ✅ Caught — real `systemd-analyze` reports `Unknown key name 'Persistant' in section 'Timer'` against **our** file; the unrelated `teamviewerd.service` notice was correctly filtered out in the same run | The Rank 1 gate is judged by the real tool, and its allowlist is exact |
| **F6** | `WakeSystem=false` → `WakeSystem=maybe` **in the production array *and* in `timer.rs`'s own pinned-argv literal**, so the field-equality test was updated in lockstep | `cargo test -p nopass-helper --lib timer::` then `--test systemd_unit_contract` | ✅ Caught — argv test: **6 passed**; real-tool gate: **FAILED** (`Failed to parse boolean value, ignoring: maybe`) | **The decisive Rank 1 result.** A developer who "fixes" the pin to match a bad value keeps the field-equality test green; only the real tool catches it. This is exactly the defect class the change exists to close |
| **F7** | `</action>` removed from `data/com.enfoquestic.nopass.policy` | `cargo test -p nopass-helper --test data_artifacts` | ❌ **Not caught** — 8 passed | Lane A's substring assertions pass on a policy file real polkitd rejects outright. Stated here as the *justification* for the Rank 2 gate, not as a defect |
| **F8** | Same corruption | `scripts/run-lane-polkit.sh` | ✅ Caught — lane **exit 101**, all 3 tests fail, incl. `expected com.enfoquestic.nopass.manage to still be enumerated…` | **The decisive Rank 2 result.** F7+F8 together are the whole argument for Phase 10 |
| **F9** | `handle_duration_selected` builds its `EnableRequest` from `config.default_duration` instead of the selected duration | `cargo test -p nopass --lib` | ✅ Caught — `all_six_durations_render_their_documented_argv_end_to_end_through_app_with_no_shell` | "The user picks 4 hours and the tray grants 1 hour" fails a test |
| **F10** | `radio_group_item`'s `selected` hardcoded to `0` | `--lib`, then `run-lane-b.sh` | ✅ Caught twice — Lane A `the_default_duration_shell_becomes_a_labelled_submenu_wrapping_a_radio_group`; Lane B 3 failures over the real DBusMenu wire | The marker is pinned on both sides of the bus |
| **F11** | `RadioGroup::select` ignores its `index` and always yields `ALL[0]` | `cargo test -p nopass --lib` | ✅ Caught — `radio_group_select_maps_every_index_back_through_grant_duration_all` | The index→duration lookup cannot silently mis-grant |

Plus one non-neutering direct check: `run-lane-polkit.sh` invoked with a `PATH` containing
neither `docker` nor `podman` → **exit 1** with `neither docker nor podman found on PATH`.

**Score: 10 of 11 caught.** The one miss (F1) is WARNING-3.

---

## 5. Test Layer Distribution

| Layer | Tests | Files | Tools |
|---|---|---|---|
| Unit (in-module `#[cfg(test)]`) | 530 | `consent.rs`, `config.rs`, `autostart.rs`, `duration.rs`, `menu.rs`, `format.rs`, `tray.rs`, `app.rs`, `preflight.rs`, `outcome.rs`, `notifications.rs`, `timer.rs`, core crates | `cargo test` |
| Integration, unprivileged (`tests/*.rs`) | 67 | `atomicfile_tempdir.rs`, `config_tempdir.rs`, `state_tempdir.rs`, `watch_inotify.rs`, `icon_assets.rs`, `reactor_responsiveness.rs`, `data_artifacts.rs`, `docs_headless.rs`, `fileops_tempdir.rs`, `lane_wiring.rs`, `process_boundary.rs` | `cargo test` |
| Session bus, headless (Lane B) | 20 | `dbus_session.rs` | `dbus-run-session` + isolated conf |
| Real systemd tool (Lane A, gated by presence not env) | 5 | `systemd_unit_contract.rs` | real `systemd-analyze` |
| Real polkit authority (container) | 3 | `polkit_contract.rs` | docker, `Containerfile.polkit`, `dbus-daemon --system` + `polkitd` |
| Real root (container) | 17 | `root_system.rs` | docker, `Containerfile.debian`/`.fedora` |
| Real journald (container) | 4 | `root_journal.rs` | docker, `Containerfile.journald` |
| Real desktop (Lane C) | 4 checklist items (§12–15) | `tests/manual/README.md` | **unexecuted** |
| **Total** | **645 (Lane A) + 44 lane-only executions** | **24 suites** | |

---

## 6. Assertion Quality

Every test file this change created or modified was scanned for the banned patterns:
tautologies, orphan empty checks, type-only assertions used alone, assertions that never call
production code, ghost loops over possibly-empty collections, smoke-test-only, mock-heavy
ratio.

**Assertion quality: 0 CRITICAL, 0 WARNING, 2 SUGGESTION.**

- **No tautology** of the `assert!(true)` class exists anywhere in the change.
- **No ghost loop.** Every loop-based assertion iterates a fixed literal array
  (`GrantDuration::ALL`, `Msg`'s `ALL`, `UNIT_PROPERTIES`, explicit `[TrayState; 2]` /
  `[UnavailableReason; 3]` tables) or a collection whose length is asserted first
  (`menu.rs:519/:520` assert `children.len() == 6` before mapping; `menu.rs:591` asserts
  `children.len() == 4` before the insensitivity loop).
- **No smoke-test-only.** Every menu and tray test asserts *what* was rendered — exact labels,
  exact kinds, exact `enabled`/`checked` — never merely that a render happened.
- **Assertion density** is healthy: `menu.rs` 64 asserts / 16 tests, `consent.rs` 19 / 13,
  `config_tempdir.rs` 25 / 13, `polkit_contract.rs` 10 / 3, `systemd_unit_contract.rs` 7 / 5.
- Two weaknesses, both graded SUGGESTION rather than WARNING because F3 and F4 show each still
  catches something real: the labelled-but-unexercised caller table at `consent.rs:196`
  (**SUGGESTION-1**) and the `Msg`-against-`Msg` comparison at `app.rs:1037`
  (**SUGGESTION-2**).

I deliberately went looking for the four defect shapes this project has produced before.
Three are absent and one recurred:

| Historic shape | Status in M3 |
|---|---|
| A gated suite executing in no lane (`systemd-analyze` skip; the unwired root lane) | **Absent.** `lane_wiring.rs` enforces it structurally and `NOPASS_POLKIT_TESTS` is wired to `run-lane-polkit.sh`. Residual: the script is not in `verify.gate_commands` for Lane B — WARNING-5 |
| An empty tray menu every Lane B test hid by calling `render_menu` itself | **Found and already closed by this change.** `tray.rs:197` falls back to M2's three-item menu when `nodes` is empty, and `app.rs:1387 menu_opened_calls_render_menu_on_the_tray_port_with_the_current_state` pins the production wiring. The Lane B tests *do* still call `render_menu` themselves (`dbus_session.rs:406/467/509/519/563`) — the fallback is what makes that safe rather than a live regression |
| A distinctness guard iterating a fixed array the new arms were never added to | **Recurred.** 42 enum arms, 40 in the array — WARNING-3, proven by F1 |
| A tautological `Msg`-against-`Msg` assertion | **One instance remains**, at `app.rs:1037`. Non-vacuous (F3) but weaker than pinning the literal — SUGGESTION-2 |

---

## 7. Quality Metrics

**Linter**: ✅ `cargo clippy --workspace --all-targets -- -D warnings` — exit 0, no issues.
**Type checker / build**: ✅ `cargo build --release` — exit 0, 0 warnings.
**Formatter**: ➖ Not a gate in this repository (§2).
**Reactor invariant**: ✅ `assert-single-reactor.sh` — one `async-io` major (v2.6.0), zero
tokio, no dbus/glib/gtk, zbus `tokio` feature off. `toml_edit =0.25.15` added exactly one
transitive package (`toml_writer 1.1.2`) and did not disturb it.

---

## 8. Proposal Success Criteria

| # | Criterion | Status | Evidence |
|---|---|---|---|
| 1 | Every RF-03 item exists, keyboard-reachable, default-duration submenu marks the current value | ⚠️ Partially met | Tree ✅ Lane A `menu.rs:451` + Lane B `dbus_session.rs:370/:443`; marker ✅ `menu.rs:537`, F10. **Keyboard reachability: Lane C, unexecuted (7.3)** |
| 2 | Each of the six durations produces its documented argv, permanent being neither flag | ✅ Met | `duration.rs:107/:120/:128/:136`; `app.rs:1182` end-to-end; F9 |
| 3 | A first activation produces zero helper invocations until explicit confirm; after "don't warn again" a later activation invokes directly | ⚠️ Partially met | Lane A ✅ `app.rs:1020/:1059/:1077/:1110`, F4. **"Observed once on a real desktop in Lane C": unexecuted (11.2)** |
| 4 | `config.toml` round-trips both fields; malformed content yields defaults **and a warning**, never a refusal to start | ⚠️ Partially met | Round-trip ✅, defaults ✅, never-refuses ✅. **Warning ❌ at startup and ❌ for an unrecognized value** — WARNING-1 |
| 5 | Autostart checkbox creates and deletes the entry; the installed package still ships none | ⚠️ Partially met | File behaviour ✅ `autostart.rs:169/:203/:221`, packaging ✅ `autostart.rs:291`. **Lane C logout/login: unexecuted** |
| 6 | Under `LANG=es_ES.UTF-8` menu and notifications are Spanish; the sudoers header is byte-identical | ⚠️ Partially met | Menu ✅, header ✅ `template.rs:170` under a real Spanish child process. **Notifications: partly** — see §9 |
| 7 | A corrupted `systemd-run` token fails Rank 1; a malformed policy fails Rank 2; both fail (not skip) when the tool is missing | ✅ Met | F5, F6 (Rank 1); F7+F8 (Rank 2); `systemd_unit_contract.rs:194` and the `PATH`-stripped `run-lane-polkit.sh` run (both gates fail loudly on absence) |
| 8 | `cargo test --workspace`, `cargo clippy -D warnings`, `assert-single-reactor.sh` green under the pinned 1.85 toolchain | ✅ Met | Commands 1, 2, 4, 5 — all exit 0 on cargo 1.85.1 |

Criteria 1, 3 and 5 each name Lane C explicitly. No real desktop session was available to this
verification, and no result was invented for one. Recorded as scope, not as a finding — this is
what tasks 7.3 and 11.2 exist to carry.

---

## 9. The untranslated-notification count

The task brief says a review found six hardcoded English notifications and that four
M2-inherited ones remain, deliberately. I checked both halves.

**No NEW untranslated user-visible string was added by M3.** Every untranslated literal still
in the tree predates this change, by `git log -L` blame:

| Location | Strings | Introduced by |
|---|---|---|
| `app.rs:220` "No tray host found" + body | 1 notification | `b8c9622` (M2) |
| `app.rs:226` "No notification service found" + body | 1 | `b8c9622` (M2) |
| `app.rs:329` "Could not watch for changes" + body | 1 | `29ebe32` (M2) |
| `app.rs:692` "Lost the change watch" + body | 1 | `a5c93d3` (M2) |
| `notifications.rs:86` `expiry_notification()` | 1 | `f37df89` (M2) |
| `notifications.rs:99` `already_running_notification()` | 1 | `f37df89` (M2) |
| `notifications.rs:77` `" Expires: {countdown}."` suffix | 1 fragment | `f37df89` (M2) |
| `outcome.rs:196` `OutcomeKind::text()` | **18** summary/body pairs | `23e494c` (M2) |

The claim "four remain" is **accurate as written** — four raw literals at `notify.post` call
sites in `app.rs` — and `tests/manual/README.md` §15 names exactly those four as the known
out-of-scope gap. It **understates the untranslated user-visible surface**, which is 24
notification summary/body pairs plus one composed fragment. In practice a Spanish user sees a
Spanish menu, a Spanish tooltip, a Spanish consent branch and a Spanish first-activation
nudge — and then an English "Passwordless sudo enabled" toast after every single grant,
because `OutcomeKind::text()` is the most frequently seen notification in the product and is
entirely English.

This is **not a regression and not a WARNING against M3** — the `localization` requirement
scopes the catalogue to "every user-facing string `format.rs` renders", and `outcome.rs`
renders its own. It is recorded here so the archive carries the real number rather than the
four, and so M4 can scope it deliberately. Filed as **SUGGESTION-6**.

---

## 10. Issues Found

### CRITICAL — none

No requirement is unsatisfied in a way that admits an unconsented grant, a wrong argv, a
silently-rejected policy, or a gate that skips instead of failing. The two invariants this
change was built around — consent before dispatch, and real tools judging the two hardening
gates — both survived direct attack (F4, F5, F6, F7, F8).

### WARNING

**WARNING-1 — `user-config`'s "plus a warning" is not implemented, and a test pins the
contrary.**
The requirement text reads: *"Malformed TOML or an unrecognized field/value MUST degrade to
the schema defaults plus a logged/observable warning."* Two of its three paths do not produce
one:

- *Unrecognized value.* `config::read` (`config.rs:136-142`) resolves a bad
  `default_duration` to `Hour1` **inside a `Parsed` reading**, so `config::resolve` returns
  `(config, None)` and `App::refresh_config` (`app.rs:602`) never notifies.
  `config_tempdir.rs:222` asserts this explicitly: `assert_eq!(fault, None, "a semantically
  bad but syntactically valid document is Parsed, not Faulted")`. The behaviour is
  deliberate (design D4's "per-field tolerance") and the test encodes the deliberate
  behaviour — but the spec scenario says a warning is produced, and task 2.4's own text says
  "degrade to defaults **plus an observable warning**". Task 2.4 is checked `[x]`.
- *At startup.* `main.rs:162` is `let (initial_config, _fault) = …` — the fault is discarded.
  Even for genuinely malformed TOML, the warning fires only at the first `Trigger::MenuOpened`.
  A user whose `config.toml` is broken and who never opens the menu is never told.

Impact is bounded — the safe default is always applied and the tray always starts — but a user
who typos `default_duration = "3h"` silently loses the setting with no feedback at all.
**Archive-relevant**: merging this delta as written installs a canonical `user-config`
requirement the tree does not meet.

**WARNING-2 — `user-config`'s "read exactly once at startup" contradicts design D4, and the
delta spec was never reconciled.**
The requirement says *"The config MUST be read exactly once at startup … no other event MUST
write it."* Design D4 is titled, verbatim, *"Config is read at startup **and** at every menu
open"*, and task 8.3 implements it (`app.rs:710` → `refresh_config`). The implementation is
the better behaviour and is well argued; the spec sentence is simply stale. Same
archive-relevance as WARNING-1: `openspec/specs/user-config` would inherit a MUST the code
intentionally violates. (The *write* half of the sentence does hold — `config::write` has
exactly two callers.)

**WARNING-3 — the distinctness guard has a hole, and it is this project's fourth instance of
that shape.**
`format.rs:718` declares `const ALL: [Msg; 40]`; the `Msg` enum has **42** arms.
`ToggleUnavailableInstallationIncomplete` and `ToggleUnavailableActionInFlight`, both added in
Phase 8, are not in the array. Experiment F1: with one of them untranslated (Spanish text set
equal to English), `cargo test --workspace` reports **645 passed, exit 0**. Experiment F2: the
same neutering of a covered arm fails by name. Both arms happen to be correctly translated
today, so there is no user-visible defect — the guard simply cannot see them, and cannot see
the next arm either. Commit `d078207`'s own trailer declares this: *"two Msg arms added in
phase 8 are still absent from the distinctness array."* It is declared, not fixed.

**WARNING-4 — `probe_polkit_readiness` itself has no test.**
The `privilege-admission` scenario is named after that function. What is tested is
`preflight::classify_enumeration` (`preflight.rs:356`, `:364`) and `preflight::polkit_ladder`
— good, pure, well-triangulated seams. `probe_polkit_readiness` lives at `main.rs:341`, in a
binary target with no `#[cfg(test)]` module, and nothing exercises it. The unexercised line
that matters is `main.rs:375`:
`preflight::classify_enumeration(actions.iter().map(|a| a.0.as_str()))` — a tuple-index
selection out of polkit's `sssssuuua{ss}` reply. If `.0` were wrong, or the proxy signature
drifted, every enumeration would classify as `ActionAbsent`, the toggle would render
permanently `Unavailable(InstallationIncomplete)`, and Lane A would stay green. The container
lane proves the *authority* enumerates the action; it does not exercise our client's decoding
of the reply.

**WARNING-5 — Lane B is not in `verify.gate_commands`, and M3 made it load-bearing.**
`openspec/config.yaml` names four gates; `scripts/run-lane-b.sh` is not among them. 20 of the
645 Lane A tests are `NOPASS_DBUS_TESTS`-gated and report `ok` in 0.00 s without executing —
four of them added by this change (task 7.2's `exported_menu_matches_menu_tree_item_for_item_*`
pair, task 8.6's `exported_menu_reflects_action_missing_polkit_readiness_*`, and the consent
branch over the real wire). This was recorded as WARNING-3 in the m3a verify report and
explicitly accepted there on the grounds that m3a touched no tray file; that justification
does not survive M3, which rewrote `tray.rs::menu()`. I ran the lane manually — **exit 0, 20
passed in 4.26 s**, and F10 confirms it catches a real defect — so the scenarios *are* proven.
The gap is that a future `sdd-verify` run would not prove them.

**WARNING-6 — design §1's menu table is not what ships, and one M2 affordance was dropped.**
`menu_tree_in` (`menu.rs:149`) emits exactly seven nodes. Design §1's table lists eleven rows:
item 1 (the `<user> — <state>` status line, marked *"unchanged from M2"*), items 2/5/9
(separators), and item 10 as an **`About NoPass ▸` SubMenu** with three insensitive children
(version, helper path, config path). None of those is implemented: `about_node`
(`menu.rs:361`) is a childless insensitive `Static` leaf. The `tray-menu` spec and task 6.2
both list the seven-item tree, so **spec and tasks are satisfied** — but the status-line item
now survives only in `tray.rs::fallback_menu`, which means the moment the full M3 tree
renders, the M2 menu's status line disappears. Either the design table or the shipped tree
should be reconciled before this design is archived as the record of what was built.

### SUGGESTION

**SUGGESTION-1 — `consent.rs:196` does not exercise the callers it names.**
`every_activation_raising_caller_dispatches_nothing_while_unacknowledged` (task 3.5) declares
`let callers = ["menu toggle", "SNI left-click", "SNI keyboard Activate", "single-instance
activation nudge"]` and loops over it — but the loop body builds a fresh `unacknowledged()`
and calls `state.grant()`, which is identical for all four rows and identical to
`grant_returns_none_while_unacknowledged` at `:170`. The strings are used only inside a
`panic!` message. The `ScriptedRunner::new(vec![])` is constructed and dropped without ever
being handed to anything, so its "script exhausted" panic can never fire. The test is not
vacuous (F4 fails it) — it is just not the per-caller table its name, its comment and task 3.5
all claim. The real per-caller proof is in `app.rs` (`:1020`, `:1041`, `:1059`), which does
drive `handle_toggle`, `Event::ActivateRequested` and `Event::DurationSelected` against a
runner that panics on any call. Renaming `consent.rs:196` to what it actually asserts, or
rewriting it to call the four `App` entry points, would close the gap between claim and
content.

**SUGGESTION-2 — `app.rs:1037` compares the catalogue against itself.**
`assert_eq!(posts[0].2, format::Msg::NotifyConsentNeededBody.text(format::lang()))` is the
same expression `unconsented_toggle_body()` evaluates. F3 shows it catches an arm swap, so it
earns its place — but it cannot catch a wrong translation, which is what the R2 correction was
about. Pinning the two literals under `Lang::En` and `Lang::Es` explicitly would.

**SUGGESTION-3 — a failed consent write is silent.**
Design D4 says *"a failed `warning_acknowledged = true` write posts an error outcome"*, and the
proposal's risk table says *"Write errors surface as an outcome"*. `handle_consent_confirmed`
(`app.rs:515-534`) posts nothing: the closure's `false` return is consumed by
`ConsentState::confirm` and nothing else happens. The `activation-consent` spec requires only
re-warning, which *is* implemented, so no requirement is unmet — but a user on a read-only
`~/.config` who clicks "don't warn me again" gets no explanation for why it keeps asking.

**SUGGESTION-4 — close the fixed-array class properly.**
`Msg`'s `ALL: [Msg; 40]` (WARNING-3) and `GrantDuration::ALL: [GrantDuration; 6]` share one
weakness: adding an enum variant does not break the array. `duration.rs:174` pins the exact
sequence so a *reorder* fails, but a seventh variant would still slip through silently. An
exhaustive `match` in the test that maps every variant to its array slot closes both without a
new dependency — the compiler becomes the guard, which is exactly design D2's own argument for
choosing a `match` catalogue over an i18n crate.

**SUGGESTION-5 — one silent-skip surface remains in the polkit lane.**
`Containerfile.polkit`'s entrypoint runs `cargo test … --test polkit_contract` without setting
`NOPASS_POLKIT_TESTS`; it comes from `docker run -e` in `run-lane-polkit.sh`. Running
`docker run --rm nopass-test-polkit` by hand therefore exits **0** with all three tests
skipped. The script always sets it and is the only supported entry, so this is not reachable
through any documented path — but baking the variable into the image would remove the last
place in this lane where a green exit can mean "nothing ran".

**SUGGESTION-6 — record the real untranslated count (see §9).**
Four raw literals at `app.rs` `notify.post` sites, as documented — but 24 notification
summary/body pairs in total, dominated by `outcome.rs::text()`'s 18 arms, which is the toast a
user sees after every grant and revoke. All M2-inherited, none new. Worth scoping explicitly in
M4 rather than carrying "four" forward.

### Ranked by severity

| Rank | Finding | Severity | Why this rank |
|---|---|---|---|
| 1 | WARNING-1 — `user-config` "plus a warning" unimplemented; `config_tempdir.rs:222` pins the contrary | WARNING | The only place a delta spec scenario is **not satisfied**. Archive-relevant: it becomes a canonical MUST the tree fails |
| 2 | WARNING-2 — "read exactly once at startup" contradicts design D4 | WARNING | Same archive-relevance; no behavioural risk, the implementation is the better one |
| 3 | WARNING-3 — distinctness guard blind to 2 of 42 `Msg` arms | WARNING | Proven live by F1. No user-visible defect today; it is the guard for the next arm that is missing, and this is the shape's fourth recurrence |
| 4 | WARNING-4 — `probe_polkit_readiness` untested | WARNING | A wrong tuple index would permanently disable the toggle and no automated lane would notice |
| 5 | WARNING-6 — design §1 table ≠ shipped tree; M2's status-line menu item dropped | WARNING | User-visible affordance loss, but spec- and task-compliant; the design is what is stale |
| 6 | WARNING-5 — Lane B not a configured gate | WARNING | Mitigated for *this* verification (run manually, exit 0); the risk is entirely to future runs |
| 7 | SUGGESTION-1 … 6 | SUGGESTION | Test-expressiveness, documentation and scope items; none affects a shipped behaviour |

---

## 11. Verdict

**The change is ready to archive.** Zero CRITICAL findings, zero blockers.

- All 4 configured gate commands exit 0, plus Lane B run separately at exit 0.
- `cargo test --workspace` is green under **both** `es_ES.UTF-8` and `LANG=C LC_ALL=C`, at the
  645/1-ignored baseline, with byte-identical normalized output under both.
- `cargo clippy -D warnings` and `cargo build --release` exit 0.
- The consent invariant holds structurally (three gated `EnableRequest::new` sites, one
  unconstructible token, two source-scanning guards) and under attack (F4).
- Both hardening gates are judged by the real tool and were each independently proven to fail
  on a corrupted real input (F5/F6, F7/F8) — including, for Rank 1, the case where the
  field-equality pin was updated in lockstep with the defect, which is the precise scenario the
  gate exists for.
- The five open tasks are open for the reasons the plan gives, and **no result is recorded for
  any of them**. The Lane C results table in `tests/manual/README.md` is entirely blank.

What the tasks claim but the tree does not fully support, stated plainly:

1. **Task 2.4** claims an unrecognized `default_duration` degrades "to defaults plus an
   observable warning". It degrades to defaults; there is no warning, and
   `config_tempdir.rs:222` asserts there is none. (WARNING-1)
2. **Task 3.5** claims a test "table-driven over every known caller". The table is four unused
   label strings around four identical `grant()` calls; the per-caller proof is elsewhere, in
   `app.rs`. (SUGGESTION-1)
3. **Task 5.3** claims every `Msg` arm's rendered pair is asserted distinct. Forty of
   forty-two are. (WARNING-3)
4. **Task 8.5** claims `probe_polkit_readiness`'s enumeration result "is consumed". It is —
   but the consuming function has no test; only the pure classifier it delegates to does.
   (WARNING-4)

None of these admits an unconsented grant, a wrong argv, an unvalidated policy or a silently
skipped gate. They are recorded so the archive carries the real state, and so WARNING-1 and
WARNING-2 are reconciled in the delta specs before those two texts become canonical
`openspec/specs/user-config` requirements.

---

## 12. Method and limitations

- Every command's own exit status was captured directly (`echo "EXIT=$?"` immediately after
  the command, never after a pipe). Where a summary here says "exit 0", that is the command's
  status, not a grep's.
- Neutering edits were applied one at a time from a byte-exact backup and restored
  immediately; `git status --porcelain` was verified empty before the first experiment and
  after the last, and `cargo test --workspace` was re-run green (645, exit 0) at the end.
- **Lane C was not executed.** No real desktop session, no logout/login cycle, no keyboard
  navigation of a live SNI menu. Tasks 7.3 and 11.2 remain the correct place for that, and
  nothing in this report is credited to a desktop.
- `tests/containers/Containerfile.systemd` was not built or run (task 11.4's carry-forward);
  it needs rootful privileged podman, which this machine does not have.
- Coverage was not measured — no tool is configured, and `coverage_threshold: 0`.
- The polkit lane ran under docker 29.8.0 rootful; `podman` is absent, and
  `run-lane-polkit.sh`'s detection order (docker first) is what made that work.

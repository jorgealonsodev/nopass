# Verify Report: M2 — Tray application (`m2-tray`)

Change: `m2-tray` · Project: `nopass` · Predecessor: `m1-core-helper` (archived)

Verified against `proposal.md`, `design.md`, `tasks.md`, the five delta specs under
`specs/{tray-presence, tray-state-sync, tray-privileged-invocation, tray-notifications,
tray-single-instance}`, and the consumed M1 capabilities `openspec/specs/helper-cli/spec.md`,
`openspec/specs/helper-observability/spec.md` and `openspec/specs/expiry-policy/spec.md`.

Tree verified at commit `ce01fc8`. This is the **second** formal verification pass. The first
(this file's previous revision, at `7bd28c6`) returned *verified with gaps, not ready to archive*;
its nine gaps, four spec contradictions and two lane mislabels are this pass's checklist, and every
one is dispositioned in §6. Since then the change has also absorbed a defect found by hand rather
than by any gate: `systemd-run --on-calendar` had been given RFC-3339 since M1, which systemd
rejects, so no timed grant had ever worked. §7 treats that defect's *shape* as the most important
question this pass can answer.

**Working-tree note, recorded because it affects what "clean" means below.** The tree was clean at
the start of this pass. At 15:33 local, while the neutering experiments were running, three files
changed underneath this verification from outside it — `data/icons/nopass-locked-symbolic.svg`,
`data/icons/nopass-unlocked-symbolic.svg` and `data/icons/nopass-unlocked-timed-symbolic.svg`, each
rewritten to draw the keyhole as an `evenodd` hole rather than a white fill, with a comment saying
a white keyhole is invisible on a dark panel. These are not this verification's edits. Nothing this
pass touched lies outside `crates/**/*.rs`, every such edit was reverted, and `git status` shows no
modified `.rs` file. The three SVGs were left exactly as found. Gate results are reported at both
tree states.

---

## 1. Overall verdict

**Verified. Archive is unblocked on the contract; the only open items are the two Lane C tasks
(7.7, 11.4), which no automated lane can discharge.**

All five gates exit 0. All twenty requirements have implementing code. The requirement clause that
blocked the first pass — the inotify watch retry — is genuinely implemented and genuinely pinned:
deleting the retry call makes a named test fail. All seven previously uncaught seams are now
caught, each by a test that fails when the production line is removed. The test that executed in no
gate now executes in gate 1, in 0.30 s rather than 0.00 s.

Of 34 neutering experiments — the 26 from the first pass, re-run in full, plus 8 new to this pass —
**32 were caught, 1 hangs rather than failing (unchanged from the first pass), and 1 is uncaught**.

| Call | Count |
|---|---|
| Satisfied | 14 |
| Partially satisfied | 6 |
| Not satisfied | 0 |
| **Total requirements** | **20** |

Nothing remains in the "no implementation at all" category. What remains is a flaky gate (H1), one
spec sentence that is still false about shipped behaviour after being reconciled (H2), a mitigation
for the calendar defect that reproduces the mechanism which hid it (H3), and a ranked set of
external-contract assumptions that the calendar defect makes impossible to leave unnamed (§7).
H1 is the only one I would fix before archive rather than after; it is cheap, and a
nondeterministic gate silently degrades every conclusion in this report.

---

## 2. Gate results

Exit status read from the command itself, never grepped from its output. Run sequentially from a
clean tree at `ce01fc8`, before any experiment was applied.

| # | Gate | Exit | Evidence |
|---|---|---|---|
| 1 | `cargo test --workspace` | **0** | 451 test results, 0 failed, 0 ignored |
| 2 | `cargo build --release` | **0** | release profile, LTO, `codegen-units = 1` |
| 3 | `cargo clippy --workspace --all-targets -- -D warnings` | **0** | no warnings |
| 4 | `bash scripts/assert-single-reactor.sh` | **0** | `exactly one async-io major (v2.6.0), zero tokio, no dbus/glib/gtk, zbus tokio feature disabled` |
| 5 | `bash scripts/run-lane-b.sh` | **0** | 17 tests under `dbus-run-session` with the isolated config, 4.23 s |

Re-run at the end of the pass, with the three third-party icon edits present and every experiment
reverted: **all five again exit 0**. `git status --porcelain` then lists only those three SVGs and
this report.

Per-binary distribution under gate 1, with each binary's own reported wall time — the instrument
the first pass used to find G2, applied to every binary rather than one:

| Target | Tests | Time | Executes in a gate? |
|---|---|---|---|
| `nopass` unit (`src/lib.rs`) | 157 | 1.60 s | yes (gate 1) |
| `nopass` `tests/dbus_session.rs` | 17 | 0.00 s | skipped in gate 1, **executed in gate 5** (4.23 s) |
| `nopass` `tests/icon_assets.rs` | 6 | 0.00 s | yes (gate 1) |
| `nopass` `tests/reactor_responsiveness.rs` | 1 | **0.30 s** | yes (gate 1) — G2 fixed |
| `nopass` `tests/state_tempdir.rs` | 3 | 0.30 s | yes (gate 1) |
| `nopass` `tests/watch_inotify.rs` | 6 | 0.41 s | yes (gate 1) |
| `nopass-core` unit | 57 | 0.02 s | yes (gate 1) — but see **H3** for two of them |
| `nopass-helper` unit + bin | 163 | 0.19 s | yes (gate 1) |
| `nopass-helper` `tests/data_artifacts.rs` | 7 | 0.00 s | yes (gate 1) |
| `nopass-helper` `tests/fileops_tempdir.rs` | 15 | 0.03 s | yes (gate 1) |
| `nopass-helper` `tests/process_boundary.rs` | 4 | 0.00 s | yes (gate 1) |
| `nopass-helper` `tests/root_system.rs` | 15 | 0.00 s | **skipped in all five gates** (M1's root container lane) |

**451, not 449.** Both remediation commits state 449. `ce01fc8` added the two
`timefmt::systemd_contract` tests without updating the count inherited from `08b9dfc`. Nothing
turns on it; recorded because a stated count is exactly the kind of thing later work trusts without
re-deriving.

---

## 3. Per-spec requirement audit

### `tray-presence` — 5 requirements, 8 scenarios

| # | Requirement | Call | Code | Evidence |
|---|---|---|---|---|
| P1 | StatusNotifierItem Registration and Startup Visibility | **Partial** | `main.rs:99-112` (step 7 before step 11), `tray.rs::KsniTray::spawn` | `dbus_session.rs::sni_properties_match_the_view_model_for_each_tray_state` proves registration. The **< 1 s budget is asserted in no automated lane**; Lane C item 1, open. Partial live evidence in §8 |
| P2 | Three Visual States With Distinct Icon Names | **Partial** | `format.rs::icon_name_for_style`, `tray.rs::ViewModel::from_state`, `app.rs::render` | Code is correct and now fully pinned: 6 `format::tests` icon cases, `icon_assets.rs` (6), the SNI property test, and `a_probe_driven_state_transition_calls_render_on_the_tray_port` (experiment F18 now caught). **The requirement text is still false** — see **H2** |
| P3 | Tooltip Recomputed Only At Existing Wake Points | **Satisfied** | `format.rs::countdown/tooltip/status_line`, `event.rs::tick` | `countdown_floors_to_whole_minutes_across_the_documented_boundaries` (E3 caught), `only_one_timer_interval_call_exists_in_the_crate`. C1 reconciled and the reconciled text matches `countdown`'s two shipped forms exactly |
| P4 | Degraded Start When No SNI Host Is Present | **Partial** | `preflight::decide` row 2, `app.rs::announce_degraded_mode`, `main.rs::spawn_host_watch` | `no_tray_host_mode_posts_exactly_one_environment_notification_and_does_not_exit` (F4 caught), `host_appearing_later_reasserts_the_tray_without_needing_a_restart` (F12 caught). **Unchanged gap:** `main.rs::spawn_host_watch` — the real `NameOwnerChanged` subscription — is referenced by no test; the late-host scenario is proven only from a synthetic `Event::HostAppeared` |
| P5 | Hard Refusal With No User-Visible Channel | **Satisfied** | `main.rs` exits 3 / 4, `preflight::decide` | `real_binary_with_no_session_bus_address_exits_3_with_a_stderr_message`, `real_binary_with_neither_host_nor_notifications_exits_4`, `real_binary_with_only_the_sni_host_present_keeps_running_degraded`, plus the `preflight::decide` table. C4 reconciled; L1's lane label corrected to lane B, which is where the test actually lives |

### `tray-state-sync` — 5 requirements, 8 scenarios

| # | Requirement | Call | Code | Evidence |
|---|---|---|---|---|
| S1 | Absence or Schema Mismatch Produce Unknown, Never Inactive | **Satisfied** | `state.rs::{FileReading, parse, read}`, `reconcile.rs::merge` rows 7–8 | 9 `state::tests`, 3 `state_tempdir.rs`, `row_7_absent_file_and_no_probe_yields_unknown`, `row_8_every_fault_kind_and_no_probe_yields_unknown`. F9 (two-stage schema ordering) and F10 (ENOENT ⇒ `Absent`) both caught |
| S2 | Live Probe Takes Precedence Over the Cached File | **Satisfied** | `reconcile.rs::merge` rows 1–2 | `probe_password_required_always_yields_inactive_regardless_of_file`, `threat_matrix_probe_overrides_a_state_file_claiming_active`. E9 (trusting an expired file) caught by two tests |
| S3 | Reconciliation Runs at Four Defined Triggers | **Satisfied** | `reconcile.rs::probe_required`, `app.rs::maybe_probe`, `tray.rs::menu_about_to_show` | Every one of the first pass's three gaps is closed: F3 caught by `menu_opened_with_a_stale_cache_forces_a_probe_even_when_the_state_is_not_unknown`, F5 by `a_file_event_stamps_file_observed_at_with_the_observation_time`, F6 by `about_to_show_over_the_real_dbusmenu_wire_raises_menu_opened` driving the real `com.canonical.dbusmenu.AboutToShow`. F13 and E23 still caught. C3 reconciled, and the reconciled sentence describes what `maybe_probe` does |
| S4 | Inotify Watch With Missing-Directory Fallback | **Partial** | `watch.rs::start`, `app.rs::{maybe_retry_watch, handle, run}` | **G1 resolved.** `App` owns `run_dir`, `uid` and `Option<Watch>`; `run` retries once at startup and `Event::Tick` retries thereafter; `main.rs`'s one-shot call and its `mem::forget` are gone. Experiment N1 (deleting the retry call from `Event::Tick`) fails `a_tick_retries_the_watch_until_established_and_never_spawns_a_second_one`. Experiment N2b (a genuinely leaked second watch) fails the same test. **Residual:** the requirement's "or the watch is lost" half is not implemented, and "show a warning" is an `eprintln!` no test observes — see **H4**; the already-held short-circuit itself is unpinned — see **H5** |
| S5 | No Periodic Wakeup Beyond the 60-Second Reconciliation Tick | **Satisfied** | `event.rs::{tick, tick_stream, TICK_INTERVAL_SECS}` | `only_one_timer_interval_call_exists_in_the_crate`, `tick_stream_yields_tick_events_periodically`, `ticks_keep_firing_on_schedule_while_a_live_tray_holds_the_bus`. The measured idle RSS/CPU half is Lane C item 11, open |

### `tray-privileged-invocation` — 4 requirements, 6 scenarios

| # | Requirement | Call | Code | Evidence |
|---|---|---|---|---|
| I1 | pkexec Invocation for Enable and Disable | **Satisfied** | `invoke.rs::pkexec_spec`, `Locale::{from_env, pairs}` | `pkexec_spec_builds_the_exact_documented_enable_argv`, `..._disable_argv`, `pkexec_spec_never_adds_user_or_shell_flags`, `env_pass_through_is_exactly_lang_lc_all_lc_messages_and_nothing_else` (E12 caught by 3 tests), `a_non_absolute_program_is_rejected_before_any_spawn_for_either_ports_spec_shape`. "MUST NOT invoke `expire`" is structural: `Action` has only `Enable`/`Disable`. See §7 rank 10 for what the argv does *not* prove |
| I2 | The Tray Never Accesses /etc/sudoers.d | **Satisfied** | absence of any such path in the port surface | `no_production_code_in_the_crate_references_a_path_under_etc_sudoers_d`. **Caveat unchanged:** a flat, non-recursive `read_dir` over `crates/nopass/src` with everything after the first `#[cfg(test)]` discarded — a proxy for "enumerate the port surface", not the enumeration; a dynamically composed path evades it |
| I3 | Total Exit-Code-to-Outcome Mapping | **Satisfied** | `outcome.rs::{DocumentedCode, classify, escalate, OutcomeKind::text}` | 29 `outcome::tests` including set-uniqueness over all 18 outcomes; E4 (collapsing 127) and E26 (dropping the `TimerUnscheduled` escalation) both caught |
| I4 | pkexec Invocation Never Blocks the Reactor | **Satisfied** | `runner.rs::run_off_reactor`, `app.rs::{spawn_action, spawn_probe}` | **G2 resolved.** `tests/reactor_responsiveness.rs` lost its `NOPASS_DBUS_TESTS` gate and executes under gate 1, taking 0.30 s rather than reporting `ok` in 0.00 s. Experiment N5 (making `run_off_reactor` run on the reactor thread) fails it. L2's lane label corrected to `cargo test` |

### `tray-notifications` — 3 requirements, 4 scenarios

| # | Requirement | Call | Code | Evidence |
|---|---|---|---|---|
| N1 | Success, Failure, and Expiry Notifications | **Satisfied** | `app.rs::{handle_action_finished, reconcile}`, `notifications.rs::{action_notification, expiry_notification}` | **G3 resolved on both halves.** The `pending_escalation.is_none()` guard implements the spec's "without a pending tray-initiated action"; E2 (deleting the whole block) now fails `a_file_event_transitioning_active_to_inactive_with_no_pending_action_announces_expiry`, and `a_file_event_during_a_pending_action_does_not_announce_expiry` reproduces the exact race the first pass described |
| N2 | Notification Body Is Distinct Per Outcome | **Satisfied** | `outcome.rs::text`, `notifications.rs::action_notification` | `every_documented_outcome_renders_a_distinct_summary_and_body_pair`, `not_sudoer_and_visudo_rejected_payloads_are_captured_and_distinct`, `visudo_rejected_and_a_hostile_username_never_leak_untrusted_text_into_the_delivered_payload` |
| N3 | Degraded Mode When No Notification Service Is Present | **Satisfied** | `notifications.rs::FreedesktopNotifier::post` | `no_notification_service_owner_degrades_without_a_retry_loop`, `a_hostile_notification_daemon_returning_garbage_ids_cannot_affect_the_tray` |

### `tray-single-instance` — 3 requirements, 6 scenarios

| # | Requirement | Call | Code | Evidence |
|---|---|---|---|---|
| U1 | D-Bus Name Claim and NameTaken Exit | **Partial** | `instance.rs::{acquire, classify_request_name}`, `main.rs:78-88` | `first_instance_claims_the_name_and_a_second_gets_name_taken`, `ok_becomes_owner`, `name_taken_becomes_already_running`; E1 (dropping `DoNotQueue`) fails 3 Lane B tests. **Unchanged gap (was G7):** "the second process exits 0, and no second SNI item is registered" is never observed at process level — no test launches two real binaries |
| U2 | Activation Nudge on a Second Launch | **Partial** | `instance.rs::{nudge, AppInterface::activate, accept}`, `NUDGE_TIMEOUT`, `NUDGE_RATE_LIMIT` | `second_instance_nudges_the_first_and_the_first_reacts_exactly_once`, `a_nudge_to_an_owner_that_never_replies_still_returns_within_the_bounded_timeout`, `ten_rapid_activations_produce_exactly_one_notification_and_zero_pkexec_spawns` (E7 caught), plus 4 unit tests on `accept`. **Unchanged caveat:** F8 — removing the bound from `nudge` makes the Lane B suite **hang**, not fail. Re-verified this pass with a compiling patch: the run was still alive at 300 s and had to be killed. The defect is detected, but as a CI timeout rather than a named assertion |
| U3 | Non-NameTaken request_name Errors Are a Real Fault | **Satisfied** | `instance.rs::classify_request_name`, `main.rs:80-84` (exit 5) | `any_other_error_is_propagated_never_treated_as_a_second_instance`, `a_failure_error_is_also_propagated_never_treated_as_a_second_instance`. Caveat unchanged: exit code 5 itself is asserted by no test. C4's reconciliation now names this requirement explicitly, so P5 and U3 no longer contradict each other |

---

## 4. All 34 neutering experiments

Each experiment deletes or inverts exactly one production behaviour, runs the gate that owns it,
and is reverted with `git checkout -- <file>` before the next. Every one of the 26 from the first
pass was re-run, not only those tied to a fixed finding.

### 4.1 The seven that were uncaught in the first pass — all caught now

| # | Behaviour removed | File | Gate | Result | Test that failed |
|---|---|---|---|---|---|
| E2 | The expiry-notification block in `reconcile` | `app.rs` | gate 1 | **CAUGHT** | `a_file_event_transitioning_active_to_inactive_with_no_pending_action_announces_expiry` |
| E8 | `is_relevant`'s event-kind filter (`true` for everything) | `watch.rs` | gate 1 | **CAUGHT** | `access_events_are_never_relevant`, `other_and_the_fully_generic_any_kind_are_never_relevant` |
| F3 | The `MenuOpened` cache-staleness clause in `maybe_probe` | `app.rs` | gate 1 | **CAUGHT** | `menu_opened_with_a_stale_cache_forces_a_probe_even_when_the_state_is_not_unknown` |
| F5 | The `file_observed_at = now` stamp on a file event | `app.rs` | gate 1 | **CAUGHT** | `a_file_event_stamps_file_observed_at_with_the_observation_time` |
| F6 | `menu_about_to_show` raising `MenuOpened` | `tray.rs` | gate 5 | **CAUGHT** | `about_to_show_over_the_real_dbusmenu_wire_raises_menu_opened` |
| F7 | `activate` raising `ToggleRequested` (left click) | `tray.rs` | gate 5 | **CAUGHT** | `activate_over_the_real_sni_wire_raises_toggle_requested` |
| F18 | `App::render`'s call to `TrayPort::render` | `app.rs` | gate 1 | **CAUGHT** | `a_probe_driven_state_transition_calls_render_on_the_tray_port` |

The two Lane B additions are the substantive ones: they drive the real
`org.kde.StatusNotifierItem.Activate` and `com.canonical.dbusmenu.AboutToShow` methods against a
live `ksni` item and read the resulting `TrayEvent` off the channel `KsniTray::spawn` returns,
rather than calling the callback directly. That is what makes F6 and F7 evidence rather than a
restatement of the code. F3's new test is also a genuine improvement over the one that failed to
catch it: it forces `current_state` to `Inactive` so `probe_required` alone returns `false`,
isolating the cache-staleness clause from the unrelated `Unknown` rule that was masking it.

### 4.2 The nineteen that were caught in the first pass — all still caught

| # | Behaviour removed | Gate | Result |
|---|---|---|---|
| E1 | `acquire` drops `DoNotQueue` for the plain `request_name` | 5 | CAUGHT (3 failures) |
| E3 | `format::countdown` ceils instead of flooring | 1 | CAUGHT |
| E4 | `classify` collapses exit 127 into one outcome | 1 | CAUGHT (2 failures) |
| E5 | `ViewModel` drops `NeedsAttention` on `Unknown` | 1 | CAUGHT |
| E6 | `icon_name` gives `Unknown` the locked icon | 1 | CAUGHT (3 relevant failures) |
| E7 | `accept` drops the nudge rate limit | 5 | CAUGHT |
| E9 | `coherent_active` trusts an expired file | 1 | CAUGHT (2 failures) |
| E10 | `ActionGate::try_begin` always grants | 1 | CAUGHT (2 failures) |
| E11 | `toggle_label` makes `Unknown` actionable | 1 | CAUGHT (2 failures) |
| E12 | `Locale::pairs` leaks one extra env var | 1 | CAUGHT (3 failures) |
| E23 | `probe_required` drops `ActionCompleted` | 1 | CAUGHT (3 failures) |
| E26 | `escalate` never escalates `TimerUnscheduled` | 1 | CAUGHT (2 failures) |
| F4 | `announce_degraded_mode` announces nothing | 1 | CAUGHT |
| F9 | `state::parse` checks `schema` after deserialising | 1 | CAUGHT |
| F10 | `state::read` maps `ENOENT` to `Faulted(Io)` | 1 | CAUGHT (2 failures) |
| F12 | `HostAppeared` no longer reasserts | 1 | CAUGHT |
| F13 | `probe_required` drops `Unknown` + `MenuOpened` | 1 | CAUGHT |
| F14 | The debounce emits one event per raw signal | 1 | CAUGHT |
| F8 | `nudge` loses its timeout bound | 5 | **HANG** — see below |

**F8 is the one unchanged partial.** With a patch that actually compiles (the first pass's and my
own first attempt both happened to fail the borrow checker, which masks the real behaviour), the
Lane B suite was still running at 300 s and had to be killed, leaving two orphan
`dbus_session` processes I reaped by hand. The defect is detected — the gate never returns 0 — but
as a timeout, which in CI reads as infrastructure trouble rather than as a named assertion.

**F14's catcher moved.** Removing the debounce's inner absorb loop is now caught by
`a_tick_retries_the_watch_until_established_and_never_spawns_a_second_one` — the new G1 test, which
counts `FileChanged` events. `watch_inotify.rs`'s own
`creating_the_target_file_yields_exactly_one_debounced_event` did not appear among the failures,
which is worth a look by whoever owns that file: an "exactly one" assertion that survives the
debounce being deleted is not asserting what its name says.

### 4.3 Eight new experiments

| # | Behaviour removed or inverted | File | Gate | Result |
|---|---|---|---|---|
| N1 | `Event::Tick` no longer calls `maybe_retry_watch` | `app.rs` | 1 | **CAUGHT** — `a_tick_retries_the_watch_until_established_and_never_spawns_a_second_one` |
| N2 | `maybe_retry_watch` drops its `if self.watch.is_some() { return; }` guard | `app.rs` | 1 | **UNCAUGHT** — see **H5** |
| N2b | The same, plus leaking the old `Watch` so a second one is genuinely live | `app.rs` | 1 | **CAUGHT** — same test |
| N3 | `timer::schedule` reverts to `format_utc_rfc3339` | `timer.rs` | `-p nopass-helper` | **CAUGHT** (7 failures, incl. `schedule_builds_the_exact_pinned_systemd_run_argv_in_order`) |
| N3b | `ops.rs`'s second call site reverts to `format_utc_rfc3339` | `ops.rs` | `-p nopass-helper` | **CAUGHT** (5 `ops::tests` failures) |
| N3c | Both call sites revert together | both | `-p nopass-helper` | **CAUGHT** (2 `timer::tests` failures) |
| N4 | `format_systemd_calendar` renders RFC-3339 | `timefmt.rs` | `-p nopass-core` | **CAUGHT** — `calendar_strings_are_accepted_by_systemd_analyze` |
| N5 | `run_off_reactor` runs on the reactor thread | `runner.rs` | 1 | **CAUGHT** — `a_pending_privileged_action_does_not_block_other_reactor_work` |

N4 is the important one: the calendar fix's own test is genuinely mutation-checked *on a machine
that has `systemd-analyze`*. H3 is about what happens on a machine that does not.

### 4.4 Independent confirmation of the calendar defect

Not a neutering experiment; a direct check of the claim the fix rests on, against systemd 255
(`255.4-1ubuntu8.17`) on this machine:

```
$ systemd-analyze calendar "2026-09-15 13:16:35 UTC"   →  exit 0, "Normalized form: 2026-09-15 13:16:35 UTC"
$ systemd-analyze calendar "2026-09-15T13:16:35Z"      →  exit 1, "Failed to parse calendar specification"
```

The defect and the fix are both real, and the corrected `expiry-policy` sentence describes the form
systemd actually accepts.

---

## 5. Gaps, ranked by severity

### H1 — HIGH — gate 1 is nondeterministic, at roughly 8 %

`crates/nopass/src/runner.rs:216`, `system_runner_treats_signal_death_as_a_failure`, fails
intermittently with:

```
expected RunnerError::Signaled, got Err(Spawn { program: "/tmp/user/1000/nopass_tray_test_self_kill_…",
                                                reason: "Text file busy (os error 26)" })
```

Measured: **1 failure in 12 consecutive `cargo test -p nopass --lib` runs.** The mechanism is the
classic fork/exec race, not a bug in the code under test. `write_temp_script` (`runner.rs:159`)
creates an executable file and the test immediately `exec`s it; meanwhile other test threads are
forking to spawn their own children, and a forked child inherits any write file descriptor another
thread still has open, so `execve` returns `ETXTBSY`.

Why this is the highest-severity item in the list, despite fixing no user-visible behaviour: **it
corrupted this verification twice.** It appeared among the failures of experiment E6 and produced a
false `CAUGHT` for the first run of N3b, which I only noticed because the failing test was in the
wrong crate. A flaky gate is precisely the instrument by which a real regression gets re-run away,
and this report's central claim — "32 of 34 neutering experiments were caught" — rests on gate 1
answering the same question the same way twice.

The usual fixes are to write the script to a per-test `TempDir` rather than the shared temp
directory and retry once on `ETXTBSY`, or to drop the temp script entirely and have the child kill
itself via `/bin/sh -c 'kill -9 $$'`, which needs no new executable at all.

### H2 — HIGH — `tray-presence` P2's requirement text is still false after reconciliation

The first pass raised this as C2 and it is reconciled only halfway. The current text reads:

> The tray MUST render exactly three visual states. Each state carries two icon names, a plain and
> a `-symbolic` variant, so six names ship in total and the count of names is not the count of
> states.

The names-versus-states confusion is genuinely fixed, and that was a real confusion. But two
claims that the first pass raised survive, and one of them is now *more* specific and therefore
more wrong than what it replaced:

1. **"exactly three visual states" — the shipped tray renders four.** `TrayState::Unknown` is a
   first-class state throughout the design (D3, D6) and throughout the code: it has its own icon
   (`dialog-question-symbolic`), its own SNI status (`NeedsAttention`, pinned by
   `view_model_for_unknown_requests_attention_and_disables_the_toggle`), and its own menu rendering
   (`toggle_label(Unknown) == None` ⇒ an insensitive "Checking…"). The requirement's table lists
   three rows and omits it entirely.
2. **"Each state carries two icon names" is false for `Unknown`**, which carries exactly one.
   `icon_name_for_style` returns `dialog-question-symbolic` for both `IconStyle::Symbolic` and
   `IconStyle::Color`, deliberately, per design D6 — a stock name costs no asset and a padlock
   during an unresolved state is the visual lie this milestone exists to prevent.
3. **The scenario's "or its symbolic variant when the host requests one" was not touched at all.**
   No such negotiation exists anywhere. `format::icon_name` resolves through `icon_style_from_env`,
   which defaults to symbolic unconditionally and is overridden only by `NOPASS_ICON_STYLE=color`
   (design §7.1). The requirement's table additionally implies the plain name is the default; the
   shipped default is the symbolic one.

Asked plainly whether this reads as a requirement weakened to excuse a shortcut: **no.** There is
no shortcut here — the code is the careful version and the spec is the careless one. It is a
leftover, not an excuse. But archive merges these sentences into `openspec/specs/`, and then a
sentence that is simply wrong about a shipped, deliberate, D6-mandated behaviour becomes project
truth — which is the failure mode the calendar defect just demonstrated at full cost, in a design
document that stated its rule backwards and had its tests written to match.

### H3 — MEDIUM-HIGH — the calendar fix's two tests skip invisibly, by the exact mechanism that hid the defect

`ce01fc8`'s message says both new tests "skip loudly where systemd-analyze is absent, since a
silent skip is how the original defect survived its own container test". The intent is right and
the implementation does not achieve it. Both tests return early after an `eprintln!`, and
`cargo test` captures the stderr of a passing test, so the skip is invisible in a normal run.
Observed directly, by running the `nopass-core` test binary with `PATH` emptied:

```
running 2 tests
test timefmt::systemd_contract::calendar_strings_are_accepted_by_systemd_analyze ... ok
test timefmt::systemd_contract::the_rfc3339_rendering_is_the_one_systemd_refuses ... ok
test result: ok. 2 passed; 0 failed; … finished in 0.00s        # exit 0
```

`ok` in 0.00 s with exit 0 — character for character the signature the first pass used to identify
G2. No gate pins `systemd-analyze`'s presence: `scripts/run-lane-b.sh` does not, gate 1 does not,
and `tests/containers/Containerfile.debian`/`.fedora` are the root lane, which runs in none of the
five. On this machine the tests execute and N4 proves they bite. On a CI image without systemd —
which is the ordinary case, and is exactly the image in which the original container test "passed
for the wrong reason" — the calendar contract silently reverts to being pinned by our own assertion
alone, which is the state that cost a milestone.

The smallest honest fix is for the skip to be visible in the gate's own exit status or its default
output: a `#[ignore]` with a runner that un-ignores when the tool is present, a hard failure when
an environment variable declares systemd should be available, or a structural test asserting that
some gate script requires `systemd-analyze`.

### H4 — MEDIUM — S4's "or the watch is lost" half is unimplemented, and its warning is unobserved

The requirement reads: *"If `/run/nopass/` does not exist when the tray starts **or the watch is
lost**, the tray MUST fall back to reconciliation-only operation, **show a warning**, and retry
establishing the watch on each 60 s tick."* Three clauses; the retry is now implemented and pinned,
the other two are not.

**"or the watch is lost."** `maybe_retry_watch` returns immediately when `self.watch.is_some()`, so
a watch that has gone dead is never re-established. The code states its reason in its own doc
comment: *"`/run/nopass/` cannot un-exist once created, so a held watch is never replaced."* That
is an assertion about the outside world, not a property of this program. `/run/nopass` is an
ordinary tmpfs directory created by `nopass.tmpfiles.conf`; nothing prevents root removing it, and
the helper's own `statefile::ensure_run_dir` would then recreate it on a fresh inode, leaving the
tray's inotify watch attached to the old one and permanently silent. That is the same failure the
module's header warns about for watching the file instead of the directory, one level up. The
practical exposure is small — the 60 s tick still reconciles, so the cost is latency, not a wrong
state — which is why this is MEDIUM and not higher.

**"show a warning."** What the code shows is `eprintln!("nopass: could not watch … ")`. For a tray
started by a session manager that is a journal line, not something a user sees, and `tray-presence`
P4 sets the precedent that a degraded capability gets a notification. No test asserts any warning
is produced at all, in any form.

### H5 — LOW — the watch's already-held short-circuit is unpinned, and the test comment misstates why

Experiment **N2** removed `if self.watch.is_some() { return; }` from `maybe_retry_watch` and every
gate stayed green. The test that reads as covering it,
`a_tick_retries_the_watch_until_established_and_never_spawns_a_second_one`, asserts `count == 1`
after a third tick with the comment *"a second watch would double this event"*.

The comment is wrong about its own mechanism, and understanding why bounds the finding precisely.
Without the guard, `maybe_retry_watch` still executes `self.watch = Some(watch)`, and assigning
over an `Option<Watch>` **drops** the previous `Watch`, which stops its `notify` backend. So no
second watch is ever live and no event is ever doubled — the assignment already enforces
at-most-one, and the guard is not what does it. Experiment **N2b** confirms the other half: leak
the old `Watch` with `mem::forget` so a second one is genuinely live, and the same test fails
immediately. The test does catch a real duplicate; it does not catch the guard's removal, because
removing the guard does not create one.

What removing the guard actually costs is a teardown and rebuild of the inotify watch and its
debounce thread **every 60 seconds for the life of the process**, with a window during each swap in
which a state-file write raises no event. That is a real regression with a real user-visible
symptom, it is invisible to all five gates, and nothing in the repository currently describes it.
LOW because the guard is present and correct today.

### H6 — LOW — carried forward unchanged from the first pass

- **`main.rs::spawn_host_watch` is referenced by no test** (P4). The real `NameOwnerChanged`
  subscription, its match rule, and its new-owner-non-empty ⇒ `HostAppeared` logic are unproven;
  F12 covers only what `App` does once the synthetic event arrives.
- **The second instance's exit 0 is never observed at process level** (U1, the first pass's G7).
  The three `real_binary_*` tests cover exits 3, 4 and the degraded run; none launches two
  instances. Lane C item 8 covers the human-visible half and is open.
- **`PolkitReadiness` is computed at every startup and discarded** (the first pass's G6).
  `main.rs::probe_polkit_readiness` still runs the full four-step ladder, including a real
  `EnumerateActions` round trip on the system bus with a 2 s timeout, and nothing consumes
  `ActionMissing`. Proposal D4 and design §0 G3 both require the toggle to render unavailable with
  an "incomplete installation" reason instead. `main.rs:281` says so itself. No delta-spec
  requirement mentions polkit readiness, so this changes no requirement call — but it is an unmet
  decision in a document archive will preserve, and it is also the one runtime check that would
  have told a user their `.policy` file was not being read (see §7 rank 2).
- **`Event::ActivateRequested` remains dead code.** `app::handle` maps it to `tray.reassert()`;
  nothing constructs it, because `AppInterface::activate` calls `reassert` and `post` directly.
  Design D4's "adapters own no state and only send `Event`s" is violated there specifically.

---

## 6. Every finding from the first pass, and its disposition

| First-pass finding | Disposition | Evidence in this pass |
|---|---|---|
| **G1** — the inotify watch is never retried, and nothing can retry it | **Resolved, implemented not relocated** | `App` owns `run_dir`, `uid`, `Option<Watch>`; `run` retries at startup, `Event::Tick` retries thereafter; `main.rs`'s single call and its `mem::forget` are gone. N1 (delete the retry call) ⇒ test fails. N2b (leak a second watch) ⇒ test fails. Residual: H4, H5 |
| **G2** — I4's only test executes in none of the five gates | **Resolved** | The `NOPASS_DBUS_TESTS` gate is gone from `tests/reactor_responsiveness.rs`; it runs under gate 1 in **0.30 s**, not 0.00 s. N5 ⇒ it fails |
| **G3** — expiry notification fires for user-initiated disables, and its trigger is untested | **Resolved, both halves** | The `pending_escalation.is_none()` guard is in `app::reconcile`; E2 ⇒ `a_file_event_transitioning_active_to_inactive_with_no_pending_action_announces_expiry` fails; `a_file_event_during_a_pending_action_does_not_announce_expiry` reproduces the race |
| **G4** — the left-click toggle has no test anywhere | **Resolved** | `activate_over_the_real_sni_wire_raises_toggle_requested` drives `org.kde.StatusNotifierItem.Activate` over the wire; F7 ⇒ it fails. F6's sibling closes `AboutToShow` the same way |
| **G5** — nothing proves the app ever renders | **Resolved** | `RecordingTray` now records `ViewModel`s; `a_probe_driven_state_transition_calls_render_on_the_tray_port` asserts one render and its content; F18 ⇒ it fails |
| **G6** — `PolkitReadiness` computed then discarded | **Open, unchanged** | `main.rs:281`'s own comment still says so. No requirement call changes; see H6 and §7 rank 2 |
| **G7** — the second instance's exit 0 is never observed at process level | **Open, unchanged** | See H6. U1 stays Partial |
| **G8** — the watch's event-kind filter is untested | **Resolved** | Three new pure tests in `watch.rs`; E8 ⇒ two of them fail |
| **G9** — `MenuOpened` cache composition and `file_observed_at` unpinned in `App` | **Resolved** | F3 and F5 each now fail a named test. The F3 test deliberately forces `current_state = Inactive` so the `Unknown` rule cannot mask the clause — the precise reason the old test missed it |
| **C1** — the tooltip scenario's literal is Spanish | **Reconciled, correctly** | Now names `42 min` and `less than a minute`, which is exactly what `format::countdown` returns, and says why English (M2 ships one language; i18n is M3) |
| **C2** — "exactly three visual states" vs four; "when the host requests one" | **Reconciled only halfway — now H2** | The names-vs-states count is fixed; "exactly three visual states", "each state carries two icon names", and "when the host requests one" are all still false |
| **C3** — probe "on every menu open" vs the narrower shipped rule | **Reconciled, correctly** | Now "on a menu open WHOSE CACHED READING IS STALE", plus "A menu open with a fresh cache MUST NOT spawn a probe". That is what `maybe_probe` does. It is a genuine narrowing of the original requirement, but it restates design §3.4's pre-existing decision rather than inventing a licence after the fact, and it gives the cost that motivates it |
| **C4** — P5's "only when" forbids the exits U3 requires | **Reconciled, adequately** | "only when" is gone; the sentence now scopes itself to preflight refusals and names `tray-single-instance`'s exit 5 explicitly. It does not enumerate exit 1 (`KsniTray::spawn` failure, design §8 "Unexpected internal failure"), but having dropped "only when" it no longer forbids it. The recommended cross-reference to §8's full exit table was not added |
| **L1** — "No session bus at all" claims Lane A, provable only in Lane B | **Reconciled** | The scenario now names `scripts/run-lane-b.sh` and explains why an empty environment does not deny a bus |
| **L2** — "Menu remains responsive" claims Lane B, belongs in Lane A, ran in neither | **Reconciled** | The scenario now names `cargo test` and `tests/reactor_responsiveness.rs`, and records the history |

### Is any reconciled sentence an excuse for a shortcut?

Asked directly of all six reconciled sentences, including the corrected `expiry-policy` one:

- **C1, C4, L1, L2 — no.** Each replaces a false statement with a true one and gives the reason.
  L1 and L2 go further than required and record why the old lane label was wrong, which is the
  difference between a correction and a cover-up.
- **C3 — no, though it is the one that could have been.** It does narrow a MUST: "on every menu
  open" becomes "on a menu open whose cached reading is stale". A narrowing written to fit code
  that took a shortcut would be an excuse. This one is not, for two checkable reasons: design §3.4
  stated the narrower rule before the code existed, so the spec sentence was the stale artefact
  rather than the code being the deviation; and the sentence states its own cost ("each one costs a
  process") instead of hiding it. It is also now strictly more testable than before.
- **`expiry-policy`'s Transient Timer Replacement — no; it is the strongest of the six.** It
  replaces a requirement that no systemd accepts with the form systemd accepts, says plainly that
  the old sentence was wrong and what it cost, and *adds* an obligation rather than removing one:
  "A test MUST validate the rendered value with `systemd-analyze calendar`, because an argv pinned
  by string equality proves only that we emit a string, never that the callee accepts it." Two
  caveats on it, neither an excuse: the new MUST is satisfied by a test that silently no-ops
  wherever `systemd-analyze` is absent (H3), and the sentence still describes the `systemd-run`
  invocation as `--unit=… --on-calendar=… <helper> expire --uid <uid>` while the real argv also
  carries `--description=`, four `--timer-property=` flags and `--property=Type=oneshot` (§7 rank
  1). It also nests backticks inside a code span, which will not render.
- **C2 — no, but it is wrong, which is worse here.** See H2. Nothing about it excuses a shortcut;
  the code is the disciplined half and the sentence is the careless one. It should not be merged as
  written.

---

## 7. External contracts pinned by our own assertion alone

The calendar defect's shape: *a test pinned the form of an external call by field equality against
a scripted runner, and nobody ever asked the callee whether it accepts that form.* The design
stated the rule backwards, the tests were written to the design, the argv assertion passed forever,
and the only lane that touched the code path exercised its failure branch in an image where systemd
was absent — so it passed for the wrong reason too.

Below is every other place in this change where an external command, a D-Bus interface, a file
format, or an on-disk path is pinned by our own assertion alone. Ranked by what breaks if the
assumption is wrong. Where a real tool was available, I asked it — those results are marked
**[checked today]** and are evidence the assumption currently holds, not evidence that anything in
the repository would notice when it stops.

### Rank 1 — `systemd-run`'s seven non-calendar arguments — the same argv, one token over

`crates/nopass-helper/src/timer.rs:52-69`. The invocation carries `--unit=`, `--description=`,
`--timer-property=AccuracySec=1s`, `Persistent=false`, `WakeSystem=false`,
`RemainAfterElapse=false`, and `--property=Type=oneshot`. The fix validated the `--on-calendar`
*value* against systemd. **Every other token in the same argv is still pinned only by
`timer.rs`'s and `ops.rs`'s own field-equality assertions against a `ScriptedRunner`** — literally
the instrument the commit message identifies as insufficient, still holding the other seven
arguments of the exact command it was insufficient for.

If any property name or spelling is wrong, `systemd-run` exits non-zero, `timer::schedule` returns
`TimerFailed { rolled_back: false }`, `ops::enable` rolls the rule back, the helper exits 17, and
the user sees an operation that did nothing — the identical symptom, indistinguishable from a
correct refusal, that made the calendar bug survive a milestone.

**[checked today]** `systemd-analyze verify` over an equivalent `.timer`/`.service` pair carrying
all of these directives: exit 0 on both, systemd 255. The assumption holds now. It is not pinned.

The fix is the same one the calendar already got: synthesise the transient unit's `[Timer]` and
`[Service]` sections from the same constants the argv uses and run `systemd-analyze verify` over
them, in the same test file, skipping by the same rule (once H3 makes that rule honest).

### Rank 2 — the polkit policy file, which is the privilege boundary's declaration

`data/com.enfoquestic.nopass.policy`, pinned by `crates/nopass-helper/tests/data_artifacts.rs`
substring assertions: `policy.matches("<action id=").count()`, `policy.contains(r#"<action
id="com.enfoquestic.nopass.manage">"#)`, the three `<allow_*>` lines, and the `exec.path` annotate.
Every one of those is our own string against our own file. Nothing asks polkit.

If the XML is malformed, or the `policyconfig` DTD shape is rejected, polkit **silently ignores the
file**. `pkexec` then exits 127, `outcome::classify` maps that to `NotAuthorized`, and the tray
tells the user authorization failed — a completely believable message that looks like the user's
own mistake. Blast radius: every privileged action, which is the entire product.

Two things partly cover it, neither of them a gate. `main.rs::probe_polkit_readiness` does ask the
real authority via `EnumerateActions` and would return `ActionMissing("action_not_registered")` —
but its result is discarded (H6/G6), so the tray never says anything. And the package has now been
installed on a real desktop where the toggle works (§8), which is one live machine confirming
polkit accepts this file today.

### Rank 3 — the calendar fix's own tests are unpinned to their tool

Fully described as **H3**. Ranked here because it is not merely a gap in the fix; it is the
calendar defect's *mechanism* reproduced inside the calendar defect's *remedy*. On any machine
without `systemd-analyze` — the ordinary CI image, and the very kind of image in which the original
container test passed for the wrong reason — the only test that asks the real tool returns `ok` in
0.00 s and the contract reverts to being pinned by our own assertion alone.

### Rank 4 — the sudoers rule text: validated by the real parser in no gate

`nopass_core::template::render_rule` emits

```
# Generated by NoPass — do not edit manually
# nopass-user: jorge
# nopass-expires: 1789000000
#1000 ALL=(ALL) NOPASSWD: ALL
```

The operative line begins with `#`, which is a comment character almost everywhere and a uid
specification in sudoers. It is pinned by byte-equality in `template.rs`'s own tests. Real `visudo`
validates it only in `root_system.rs`'s container lane, gated on `NOPASS_ROOT_TESTS=1` **and**
euid 0, which executes in **none of the five gates**.

**[checked today]** `/usr/sbin/visudo -cf <rendered file>` as an unprivileged user: exit 0,
"análisis OK". So the real parser can be asked from an ordinary Lane A test, needing no root and no
container, and no test asks it.

Ranked below the policy file because production has a genuine second line of defence: the helper
runs `visudo -c` before the rename, so a broken rendering becomes exit 14 with a rollback rather
than a bad rule on disk. The exposure is a feature that stops working, not a privilege escalation —
but the artefact in question is the grant itself, which is why it is this high.

### Rank 5 — `nopass-cleanup.service`, the only thing that revokes a reboot-marked grant

`data/nopass-cleanup.service`, pinned by `data_artifacts.rs` substring assertions on `Type=oneshot`,
`ConditionPathExistsGlob=`, `ExecStart=` and the `Before=` ordering. Nothing runs
`systemd-analyze verify`.

If a directive name or the ordering is wrong, systemd refuses to load the unit, and stale or
reboot-marked rules survive a reboot — the single promise `Expiry::Reboot` makes. The unit is also
`Before=systemd-user-sessions.service display-manager.service` precisely so it runs before anyone
can log in; an ordering systemd rejects loses that guarantee silently.

**[checked today]** `systemd-analyze verify data/nopass-cleanup.service`: exit 0.

### Rank 6 — `nopass.tmpfiles.conf`

One line, `d /run/nopass 0755 root root -`, pinned by exact-line equality in `data_artifacts.rs`.
If systemd's tmpfiles grammar rejects it, `/run/nopass` is not created at boot. The blast radius is
now *smaller* than it was before this change, because the helper's `statefile::ensure_run_dir`
creates the directory on demand and the tray's new retry (G1) establishes the watch on the next
tick — so this is one of the few places where M2 genuinely reduced an external dependency. systemd
255 has no `--dry-run`, so validating it means `systemd-tmpfiles --create --root=<tempdir>`, which
is feasible and untried.

### Rank 7 — `data/nopass.desktop`, the only shipped artefact with no coverage of any kind

Added in `ce01fc8`. No test anywhere references it — not an integration test, not a substring
assertion, nothing. It names `Icon=nopass-locked`, which the `.deb` does install into
`usr/share/icons/hicolor/scalable/apps/`, but nothing ties the two names together, and
`format::icon_name`'s own table is likewise maintained separately from `icon_assets.rs`'s
`ICON_NAMES` array, so renaming an icon in one place leaves the other green.

**[checked today]** `desktop-file-validate data/nopass.desktop`: exit 0, with one hint —
`Categories=System;Security;Settings;` "contains more than one main category; application might
appear more than once in the application menu". Lowest functional blast radius on this list (a
missing or duplicated launcher entry), listed because zero coverage on a shipped file is a category
of its own.

### Rank 8 — the SNI and DBusMenu wire is proven against our own fake watcher

`tests/dbus_session.rs::spawn_fake_watcher` is our implementation of
`org.kde.StatusNotifierWatcher`, and `ItemProxy`/`MenuProxy` are our own `zbus` proxy declarations,
so both sides of every Lane B assertion are written by us. Two facts make this materially weaker
than the calendar case and keep it out of the top half: the *server* side is `ksni`, a real
third-party implementation, so our interface and method names must match what ksni actually emits
or registration fails outright; and the two new tests drive the real `Activate` and `AboutToShow`
methods rather than the callbacks. What no gate proves is that a real host — GNOME's AppIndicator
extension, plasmashell — accepts the registration and draws the item. That is Lane C items 1–3,
open, now with one live-desktop observation behind them (§8) and an icon rework in flight that
argues item 2 has already failed once on a dark panel.

### Rank 9 — `/usr/libexec/nopass-helper` is spelled independently in four places

`nopass_core::paths::HELPER_PATH`; the policy file's
`<annotate key="org.freedesktop.policykit.exec.path">`; `crates/nopass/Cargo.toml`'s
`[package.metadata.deb] assets` entry (`usr/libexec/`); and `nopass-cleanup.service`'s `ExecStart=`.
`data_artifacts.rs::policy_exec_path_is_default_helper_path` asserts that the policy *contains the
literal string*, rather than comparing it against `paths::HELPER_PATH` — so changing the constant
leaves that test green. Nothing at all relates the `.deb` asset path or the unit's `ExecStart` to
either. A mismatch makes polkit refuse the exec (127 ⇒ `NotAuthorized`, indistinguishable from a
failed authentication) or makes the cleanup unit fail silently at boot. The Cargo.toml comment
already notes that the Arch package will rewrite `exec.path` to `/usr/lib/nopass/`, which is
exactly the change this arrangement will not notice.

### Rank 10 — the `pkexec` and `sudo` argv, and the locale pass-through pkexec may discard

`invoke::pkexec_spec` and `probe::spec` are asserted field-by-field against a `ScriptedRunner`; no
gate asks real `pkexec` or real `sudo` to accept them. Two specific assumptions ride on that:

- **The locale pass-through may be inert.** `Locale::pairs` sets `LANG`/`LC_ALL`/`LC_MESSAGES` on
  the `pkexec` process so the polkit dialog speaks the user's language (design D9). `pkexec(1)`
  states the program runs in "a minimal known and safe environment", so whether those variables
  reach anything is a property of pkexec's sanitisation rules, not of our argv. To its credit
  design.md line 717 already carries this as an open risk in its own words, and Lane C item 4's
  pass criterion checks the dialog's Spanish message — so the contract is named, deferred, and
  waiting on a lane that is still open. Worst case is cosmetic.
- **`sudo -k -n true` touches the user's own credential cache.** It is the correct idiom for
  "ignore the cache for this one command", but no automated lane can prove it does not invalidate
  a cached sudo timestamp, which is why Lane C item 9 exists and why it is still open.

### Rank 11 — the state file across the process boundary

The helper writes it and the tray reads it, and both sides go through
`nopass_core::state::HelperStatus`, so they cannot disagree about field names by construction; the
tray's extra two-stage `schema` extraction reads raw JSON but for the same struct, and
`state_tempdir.rs` exercises real files on disk. No test in any gate feeds the helper's *own
written bytes* to the tray's reader. Listed last, and for completeness only: shared-type coupling
makes this the weakest assumption on the list, and it is the one case in this change where the
"external" counterpart is not external at all.

---

## 8. Lane C: what the two open tasks now have behind them

**Tasks 7.7 and 11.4 remain open.** Sixty-five of sixty-seven tasks in `tasks.md` are checked;
these two are not, and they are Lane C observations that only a human at a desktop can discharge. I
have no desktop session and recorded no Lane C observation; nothing in this report infers one. The
environment and result tables in `tests/manual/README.md` are still empty and are to be filled in
by the operator and transcribed here.

**Partial evidence now exists, and it is worth naming precisely.** The package has been installed
on a real desktop and the left-click toggle was confirmed to work. Taken at face value that touches
the following checklist scenarios, and no others:

| Item | Scenario | What the observation establishes | What it does not |
|---|---|---|---|
| 1 | Icon becomes visible within budget | A real SNI host drew the item, so registration succeeds against a real host, not only our fake watcher (§7 rank 8) | The **< 1 s budget** — no timing was measured or recorded |
| 4 | Enabling authenticates and probes clean | A polkit dialog appeared and authentication succeeded, so polkit is reading the `.policy` file (§7 rank 2) and `pkexec` accepts our argv (§7 rank 10) | The dialog's exact message, the `sudo -kn true` timing, and the `auth_admin_keep` second-prompt suppression — all three are separate assertions in item 4 |
| 2 | All three icon states render, symbolic and colour, light and dark | Partially, and **negatively**: the three symbolic SVGs were rewritten during this pass because a white keyhole is invisible on a dark panel. That is Lane C item 2 finding a real defect | A pass. Item 2 must be re-run after the icon rework lands |
| — | *(not a checklist item)* | Because the left-click toggle defaults to one hour, a successful toggle means `systemd-run` accepted the corrected `--on-calendar` value on a real running systemd — the only evidence anywhere that the calendar fix works end to end, as opposed to `systemd-analyze` accepting the string | That the timer **fires**. No checklist item observes a timed grant actually elapsing, which is the one symptom class the calendar defect belonged to. Worth adding as item 12 |

Items 3, 5, 6, 7, 8, 9, 10 and 11 are untouched by this observation. The checklist itself remains
sufficient for what the specs require — the first pass's assessment of `tests/manual/README.md`
stands, including its developer icon-theme install step and its result table — with the one addition
recommended above.

---

## 9. Vacuous skips: is anything in this crate skipped in every gate?

Using the method that found G2 — read each binary's own reported wall time, then account for every
conditional early return in the source.

Three skip mechanisms exist in the workspace, and only three:

1. `skip_unless_lane_b!` (`tests/dbus_session.rs:54`), on all 17 tests in that file. **Covered:**
   0.00 s under gate 1, 4.23 s under gate 5, 17 tests both times.
2. `NOPASS_ROOT_TESTS=1` **and** `geteuid().is_root()` (`root_system.rs:123`), on 15 tests.
   **Not covered by any of the five gates** — this is M1's root container lane, documented in
   `tests/containers/README.md` as lanes 3 and 4, run under podman. Out of scope for this change,
   named because it is the lane whose container test passed for the wrong reason in the calendar
   defect.
3. `systemd-analyze` absence (`nopass-core/src/timefmt.rs:82, 113`), on 2 tests. Executes here;
   silently does not elsewhere — **H3**.

**For the `nopass` crate specifically — the crate this change creates — nothing is vacuously
skipped in every gate.** All 157 lib tests, and all of `icon_assets.rs` (6), `state_tempdir.rs` (3),
`watch_inotify.rs` (6) and `reactor_responsiveness.rs` (1) execute under gate 1; all 17 of
`dbus_session.rs` execute under gate 5. There is no third conditional anywhere in
`crates/nopass/**`: the only other `std::env::var` reads in that crate are production ones
(`NOPASS_ICON_STYLE` in `format.rs`, the three locale variables in `invoke.rs`), and the crate
contains no `#[ignore]`.

---

## 10. Decision

**Archive is unblocked on the contract.** The blocking condition from the first pass — one
requirement clause with no implementation and no possible implementation site — is gone, and the
retry is pinned rather than merely present. Every coverage gap the first pass raised as blocking is
closed by a test that fails when its production line is removed.

Three things should be settled, in this order, and only the first is worth holding the archive for:

1. **H1, the ETXTBSY flake.** Small, local, and the precondition for trusting anything else in this
   report. A gate that answers differently on different runs is not a gate.
2. **H2, `tray-presence` P2.** Archive merges these sentences into `openspec/specs/`. Merging
   "exactly three visual states" and "each state carries two icon names" makes two false statements
   project truth about a deliberate, D6-mandated behaviour, and adds "when the host requests one",
   which describes a negotiation that does not exist. This is a documentation edit, not a code
   change, and it is exactly the class of error that just cost a milestone.
3. **H3 and §7 rank 1.** Both are follow-ups rather than blockers, and both are the calendar
   defect's own lesson applied where it has not been applied yet: make the skip visible, and ask
   systemd about the other seven arguments of the command it already rejected once.

H4, H5, H6 and §7 ranks 4–11 are M3 material with a recorded decision. Tasks 7.7 and 11.4 stay open
until a human executes `tests/manual/README.md` and transcribes the result table into this report;
the live-desktop observation in §8 is partial evidence for items 1, 4 and 2 and is not a substitute
for any of them.

---

## 11. Verification hygiene

- **No source file, spec file, or task file was modified by this verification.** Thirty-four
  neutering edits were applied one at a time and reverted with `git checkout -- <file>` immediately
  after each gate run. `git status --porcelain -- crates/` is empty.
- One background experiment run was killed mid-flight and left a neutered `instance.rs` in the
  tree; it was detected by a routine `git status` check and restored before any further work. One
  Lane B experiment (F8) hung and was killed at 300 s, orphaning two `dbus_session` processes,
  which were reaped by PID. The user's own running `nopass` tray (pid 592615) was not touched.
- **The three modified files under `data/icons/` are not this verification's.** They changed at
  15:33 local from outside this pass, while a `.deb` install and an editor session were running on
  the same machine. They were left as found. Nothing this pass touched lies outside
  `crates/**/*.rs`.
- Gates re-run at the end with those edits present: all five exit 0.
- No attempt token was settled, nothing was committed, and no Lane C result was inferred or
  recorded as observed.

---

## Key Learnings

1. An argv pinned by string equality against a scripted runner proves only that the program emits
   that string, never that the external tool accepts it.
2. A conditional test skip that prints to stderr and returns is invisible under `cargo test`,
   because the harness captures the output of every passing test.
3. Assigning over an `Option<T>` field drops the previous value, so a guard that prevents
   reassignment is not what enforces the at-most-one-resource property.
4. A test that fails roughly once in twelve runs silently corrupts any mutation or neutering
   experiment executed against that same gate.
5. Reconciling a spec sentence by correcting one of its false claims can leave the remaining false
   claims looking freshly reviewed and therefore trustworthy.

# Verify Report: M2 — Tray application (`m2-tray`)

Change: `m2-tray` · Project: `nopass` · Predecessor: `m1-core-helper` (archived)
Verified against: `proposal.md`, `design.md`, `tasks.md`, `specs/{tray-presence, tray-state-sync,
tray-privileged-invocation, tray-notifications, tray-single-instance}`, and the consumed M1
capabilities `openspec/specs/helper-cli/spec.md` and `openspec/specs/helper-observability/spec.md`.
Tree verified at commit `7bd28c6`, working tree clean before and after verification.

---

## Overall verdict

**Verified with gaps — not ready to archive as written.**

All five gates pass. All 20 requirements have implementing code, and 18 of the 32 scenarios are
pinned by tests that genuinely fail when the behaviour is removed. The pure core — the state
reader, the merge table, the exit-code table, the argv builders, the formatters — is the strongest
work in this repository so far: every neutering experiment aimed at it was caught, usually by more
than one test.

The weakness is uniformly at one seam: **the adapter-to-app wiring**. Seven distinct behaviours can
be deleted from `app.rs`, `tray.rs` and `watch.rs` with all 438 tests still green, and one
requirement's only test executes in none of the five gates. One requirement clause —
"retries the watch on each 60 s tick" — is not implemented at all, and the code that would host it
does not exist. Four spec sentences contradict the shipped behaviour and would become project
truth if merged into the canonical specs unchanged.

| Call | Count |
|---|---|
| Satisfied | 10 |
| Partially satisfied | 9 |
| Not satisfied | 1 |
| **Total requirements** | **20** |

Archive should be blocked until G1 (the missing watch retry) is resolved and the four contradicting
spec sentences are reconciled. The remaining gaps are coverage gaps, not behaviour defects: the
shipped behaviour is correct where I could observe it; it is simply not pinned.

---

## Gate results (observed directly, exit status read from the command itself)

Run sequentially from a clean tree at `7bd28c6`, each gate's exit status captured by the shell
rather than inferred from its output.

| # | Gate | Exit | Evidence |
|---|---|---|---|
| 1 | `cargo test --workspace` | **0** | 438 tests, 0 failed, 0 ignored |
| 2 | `cargo build --release` | **0** | release profile, LTO, `codegen-units = 1` |
| 3 | `cargo clippy --workspace --all-targets -- -D warnings` | **0** | no warnings |
| 4 | `bash scripts/assert-single-reactor.sh` | **0** | `exactly one async-io major (v2.6.0), zero tokio, no dbus/glib/gtk, zbus tokio feature disabled` |
| 5 | `bash scripts/run-lane-b.sh` | **0** | 15 tests under `dbus-run-session` with the isolated config |

Test distribution across the 438:

| Target | Tests |
|---|---|
| `nopass` unit (`src/lib.rs`) | 148 |
| `nopass` `tests/dbus_session.rs` (Lane B) | 15 |
| `nopass` `tests/icon_assets.rs` | 6 |
| `nopass` `tests/watch_inotify.rs` | 6 |
| `nopass` `tests/state_tempdir.rs` | 3 |
| `nopass` `tests/reactor_responsiveness.rs` | 1 (**never actually executes — see G2**) |
| `nopass-core` unit | 55 |
| `nopass-helper` unit + integration (M1) | 204 |

Gates 1–5 were re-run after every neutering experiment was reverted. Post-restore spot check:
`cargo test --workspace` exit 0, 438 tests; `scripts/assert-single-reactor.sh` exit 0;
`git status --short` empty.

**Note on gate 2.** The brief's warning about `cargo clean -p <pkg>` cleaning only the dev profile
is correct and was respected: gate 2 was run without any `cargo clean`, against the same target
directory the previous release build populated. A stale-artifact false positive therefore cannot be
excluded for gate 2 alone, but gate 4 ends with its own `cargo +1.85 build --release -p nopass`, and
gates 1 and 3 compile every target from the dev profile, so no source file in the change goes
uncompiled across the gate set.

---

## Verification of the claims this build makes about itself

Each was checked against the code, not the report that asserted it.

| Claim | Verdict | Evidence |
|---|---|---|
| A missing or stale state file reads as "unknown, reconcile", never "inactive" | **Confirmed** | `state.rs:47-51` — `FileReading { Parsed, Absent, Faulted }`, no `Inactive` arm, no `Default`, no `From<FileReading> for TrayState`. `reconcile.rs:35-84` — `merge` matches `Some(Passwordless)`, `Some(PasswordRequired)`, `None` by name, and within each, `Parsed`/`Absent`/`Faulted` by name. No `_` arm on either axis |
| Eighteen distinct outcomes, not a generic failure | **Confirmed** | `outcome.rs:30-80` — exactly 18 `OutcomeKind` variants. `every_documented_outcome_renders_a_distinct_summary_and_body_pair` asserts set-uniqueness over all 18 rendered `(summary, body)` pairs |
| A bare exit 17 is read as "state unknown, reconcile now", escalating on a passwordless probe | **Confirmed** | `outcome.rs::escalate` returns `UnexpiringGrant` only for `TimerUnscheduled` + `Passwordless`; every other variant is matched by name and returns `None`. `app.rs::handle_probe_finished` consumes `pending_escalation`. `TimerUnscheduled`'s text asserts neither "nothing was changed" nor a live grant, pinned by `exit_seventeen_text_asserts_neither_that_nothing_changed_nor_that_a_grant_is_active` |
| The tray never reads or writes `/etc/sudoers.d` | **Confirmed, with a caveat** | `no_production_code_in_the_crate_references_a_path_under_etc_sudoers_d`. Caveat: it is a source grep over `crates/nopass/src/*.rs` (flat, non-recursive) with everything after the first `#[cfg(test)]` excluded — a proxy for the spec's "enumerate the port surface", not the enumeration itself. A dynamically composed path would evade it |
| Exactly one production periodic timer; countdown computed on demand | **Confirmed, with a caveat** | `only_one_timer_interval_call_exists_in_the_crate` counts `Timer::interval(` occurrences in production code and asserts exactly 1. Caveat: a second periodic source written as `loop { Timer::after(..).await }` would pass. `format::countdown` is pure and takes `now` as a parameter |
| `DoNotQueue` is passed explicitly | **Confirmed, and load-bearing** | `instance.rs::acquire` uses `request_name_with_flags(.., DoNotQueue)`. Experiment **E1** replaced it with the plain `request_name`: three Lane B tests failed. The claim about zbus's default flags is real and the suite defends it |
| The watch is on the directory, never the file | **Confirmed, and load-bearing** | `watch.rs::start` calls `watcher.watch(run_dir, NonRecursive)`. `an_atomic_rename_over_an_existing_file_is_still_observed` writes a temp file and `rename(2)`s it over the target, then asserts the inode actually changed — a file watch would fail this |

---

## Per-spec requirement audit

### `tray-presence` — 5 requirements, 8 scenarios

| # | Requirement | Call | Code | Test |
|---|---|---|---|---|
| P1 | StatusNotifierItem Registration and Startup Visibility | **Partially satisfied** | `main.rs:99-112` (step 7 before step 11), `tray.rs::KsniTray::spawn` | `dbus_session.rs::sni_properties_match_the_view_model_for_each_tray_state` proves registration. The **< 1 s budget is asserted nowhere** in any automated lane; it is Lane C item 1, still open |
| P2 | Three Visual States With Distinct Icon Names | **Partially satisfied** | `format.rs::icon_name_for_style`, `tray.rs::ViewModel::from_state` | 6 `format::tests` icon cases, `icon_assets.rs` (6), `dbus_session.rs::sni_properties_match_the_view_model_for_each_tray_state`. **Gap:** "MUST switch icon on every state transition" — experiment **F18** deleted the body of `App::render` and all 438 tests stayed green |
| P3 | Tooltip Recomputed Only At Existing Wake Points | **Partially satisfied** | `format.rs::countdown/tooltip/status_line`, `event.rs::tick` | `countdown_floors_to_whole_minutes_across_the_documented_boundaries` (experiment E3 caught a ceil), `only_one_timer_interval_call_exists_in_the_crate`. **Contradiction:** the scenario's literal expected string is Spanish (`"caduca en 42 min"`); the shipped renderer produces `"42 min"` — see C1 |
| P4 | Degraded Start When No SNI Host Is Present | **Partially satisfied** | `preflight::decide` row 2, `app.rs::announce_degraded_mode`, `main.rs::spawn_host_watch` | `app::tests::no_tray_host_mode_posts_exactly_one_environment_notification_and_does_not_exit` (E-experiment F4 caught its removal), `host_appearing_later_reasserts_the_tray_without_needing_a_restart` (F12 caught). **Gap:** `main.rs::spawn_host_watch` — the real `NameOwnerChanged` subscription — has no test at all; the late-registration scenario is proven only from a synthetic `Event::HostAppeared` |
| P5 | Hard Refusal With No User-Visible Channel | **Satisfied** | `main.rs` exit 3 / exit 4, `preflight::decide` | `real_binary_with_no_session_bus_address_exits_3_with_a_stderr_message`, `real_binary_with_neither_host_nor_notifications_exits_4`, `real_binary_with_only_the_sni_host_present_keeps_running_degraded`, plus the `preflight::decide` table. **Lane mislabel** — see L1 |

### `tray-state-sync` — 5 requirements, 8 scenarios

| # | Requirement | Call | Code | Test |
|---|---|---|---|---|
| S1 | Absence or Schema Mismatch Produce Unknown, Never Inactive | **Satisfied** | `state.rs::{FileReading, parse, read}`, `reconcile.rs::merge` rows 7–8 | 9 `state::tests`, 3 `state_tempdir.rs`, `row_7_absent_file_and_no_probe_yields_unknown`, `row_8_every_fault_kind_and_no_probe_yields_unknown`. Experiments **F9** (two-stage schema ordering) and **F10** (ENOENT ⇒ Absent) were both caught |
| S2 | Live Probe Takes Precedence Over the Cached File | **Satisfied** | `reconcile.rs::merge` rows 1–2 | `probe_password_required_always_yields_inactive_regardless_of_file`, `threat_matrix_probe_overrides_a_state_file_claiming_active`, `missing_file_and_probe_passwordless_yields_active_with_no_expiry`. Experiment **E9** (trusting an expired file) was caught by two tests |
| S3 | Reconciliation Runs at Four Defined Triggers | **Partially satisfied** | `reconcile.rs::probe_required`, `app.rs::maybe_probe` | `each_trigger_invokes_the_probe_port_exactly_once_per_the_table`, `every_trigger_except_menu_opened_always_requires_a_probe_regardless_of_state` (F13 caught). **Three gaps:** experiment **F3** deleted the app's `MenuOpened` cache-staleness clause — green; **F5** deleted the `file_observed_at` update — green; **F6** stopped `menu_about_to_show` raising `MenuOpened` — Lane B green. The menu-open trigger is therefore unproven from the adapter inward. Also a **contradiction** — see C3 |
| S4 | Inotify Watch With Missing-Directory Fallback | **NOT SATISFIED** | `watch.rs::start`, `main.rs:133-136` | The watch half is well proven: 6 tests in `watch_inotify.rs`, including the rename test and the debounce test (F14 caught). **The retry clause is not implemented.** `Watch::start` has exactly one production call site, in `main.rs::boot_async`; on `Err` it prints to stderr and never tries again. `App` holds no `run_dir` and no watch handle, so `Trigger::Tick` cannot retry. `a_retry_after_the_directory_appears_succeeds_exactly_like_a_first_attempt` proves only that the *module* is retry-safe, never that anything retries — see G1 |
| S5 | No Periodic Wakeup Beyond the 60-Second Reconciliation Tick | **Satisfied** | `event.rs::{tick, tick_stream, TICK_INTERVAL_SECS}` | `only_one_timer_interval_call_exists_in_the_crate`, `tick_stream_yields_tick_events_periodically`, `ticks_keep_firing_on_schedule_while_a_live_tray_holds_the_bus`. The measured idle RSS/CPU half is Lane C item 11, still open |

### `tray-privileged-invocation` — 4 requirements, 6 scenarios

| # | Requirement | Call | Code | Test |
|---|---|---|---|---|
| I1 | pkexec Invocation for Enable and Disable | **Satisfied** | `invoke.rs::pkexec_spec`, `Locale::{from_env, pairs}` | `pkexec_spec_builds_the_exact_documented_enable_argv`, `..._disable_argv`, `pkexec_spec_never_adds_user_or_shell_flags`, `env_pass_through_is_exactly_lang_lc_all_lc_messages_and_nothing_else` (experiment **E13** injecting one extra env var failed 3 tests), `a_non_absolute_program_is_rejected_before_any_spawn_for_either_ports_spec_shape`. "MUST NOT invoke `expire`" is structural: `Action` has only `Enable`/`Disable` |
| I2 | The Tray Never Accesses /etc/sudoers.d | **Satisfied** | absence of any such path in the port surface | `no_production_code_in_the_crate_references_a_path_under_etc_sudoers_d`. Caveat noted above: source grep, not port enumeration |
| I3 | Total Exit-Code-to-Outcome Mapping | **Satisfied** | `outcome.rs::{DocumentedCode, classify, escalate, OutcomeKind::text}` | 29 `outcome::tests` including the set-uniqueness assertion over all 18 outcomes, `exit_seventeen_text_differs_from_exit_sixteen_text`, `pkexec_126_and_127_texts_are_distinguished_from_each_other_and_from_helper_texts`, plus `not_sudoer_and_visudo_rejected_payloads_are_captured_and_distinct` in Lane B. Experiment **E4** collapsing 127 into a single outcome failed 2 tests |
| I4 | pkexec Invocation Never Blocks the Reactor | **Partially satisfied** | `runner.rs::run_off_reactor`, `app.rs::{spawn_action, spawn_probe}` | The test exists and is correct — `reactor_responsiveness.rs::a_pending_privileged_action_does_not_block_other_reactor_work`. **It executes in none of the five gates** — see G2 |

### `tray-notifications` — 3 requirements, 4 scenarios

| # | Requirement | Call | Code | Test |
|---|---|---|---|---|
| N1 | Success, Failure, and Expiry Notifications | **Partially satisfied** | `app.rs::handle_action_finished` (action), `app.rs::reconcile:210-215` (expiry), `notifications.rs::{action_notification, expiry_notification}` | Success/failure proven by `successful_action_and_detected_expiry_each_produce_a_delivered_notification` and the app-level escalation tests. **Two gaps:** experiment **E2** deleted the expiry-notification trigger from `app::reconcile` entirely and all 438 tests stayed green — the Lane B test exercises the pure composer and the notifier port, never the app's *decision* to call it; and the spec's "**without a pending tray-initiated action**" guard is absent from the code — see G3 |
| N2 | Notification Body Is Distinct Per Outcome | **Satisfied** | `outcome.rs::text`, `notifications.rs::action_notification` | `every_documented_outcome_renders_a_distinct_summary_and_body_pair`, `not_sudoer_and_visudo_rejected_payloads_are_captured_and_distinct`, `visudo_rejected_and_a_hostile_username_never_leak_untrusted_text_into_the_delivered_payload` |
| N3 | Degraded Mode When No Notification Service Is Present | **Satisfied** | `notifications.rs::FreedesktopNotifier::post` (error sink, no retry) | `no_notification_service_owner_degrades_without_a_retry_loop`, `a_hostile_notification_daemon_returning_garbage_ids_cannot_affect_the_tray` |

### `tray-single-instance` — 3 requirements, 6 scenarios

| # | Requirement | Call | Code | Test |
|---|---|---|---|---|
| U1 | D-Bus Name Claim and NameTaken Exit | **Partially satisfied** | `instance.rs::{acquire, classify_request_name}`, `main.rs:78-88` | `first_instance_claims_the_name_and_a_second_gets_name_taken`, `ok_becomes_owner`, `name_taken_becomes_already_running`. Experiment **E1** (dropping `DoNotQueue`) failed 3 tests. **Gap:** "the second process exits 0, and no second SNI item is registered" is never proven at the process level — no test spawns two real binaries; the three `real_binary_*` tests cover exits 3, 4 and the degraded run only |
| U2 | Activation Nudge on a Second Launch | **Satisfied** | `instance.rs::{nudge, AppInterface::activate, accept}`, `NUDGE_TIMEOUT`, `NUDGE_RATE_LIMIT` | `second_instance_nudges_the_first_and_the_first_reacts_exactly_once`, `a_nudge_to_an_owner_that_never_replies_still_returns_within_the_bounded_timeout`, `ten_rapid_activations_produce_exactly_one_notification_and_zero_pkexec_spawns` (experiment **E7** removing the rate limit was caught), plus 4 unit tests on `accept`. Caveat: experiment **F8** removed the bound from `nudge` and the Lane B suite **hung indefinitely** rather than failing — the defect is detected, but as a hang, which in CI reads as a timeout rather than a named assertion |
| U3 | Non-NameTaken request_name Errors Are a Real Fault | **Satisfied** | `instance.rs::classify_request_name`, `main.rs:80-84` (exit 5) | `any_other_error_is_propagated_never_treated_as_a_second_instance`, `a_failure_error_is_also_propagated_never_treated_as_a_second_instance`. Caveat: exit code 5 itself is asserted by no test |

---

## Gaps, ranked by severity

### G1 — HIGH — the inotify watch is never retried, and nothing can retry it

`tray-state-sync` "Inotify Watch With Missing-Directory Fallback" requires: *"If `/run/nopass/` does
not exist when the tray starts or the watch is lost, the tray MUST fall back to reconciliation-only
operation, show a warning, and **retry establishing the watch on each 60 s tick**."*

`Watch::start` has exactly one production call site — `crates/nopass/src/main.rs:133`. On `Err` it
prints one line to stderr and the watch is never attempted again for the life of the process.
`App`'s field list contains neither `run_dir` nor a watch handle, so `Trigger::Tick` has nothing to
retry with. The successful path calls `std::mem::forget(watch)` to leak it for the process
lifetime, which also means a lost watch is never detected.

Consequence on a real machine: `/run/nopass/` is created by the helper's first `enable` (M1
`root_system.rs::real_run_nopass_directory_is_created_at_0755_when_missing_before_enable`). A tray
started before any grant has ever been made on that boot therefore runs **permanently** on the 60 s
tick alone, with no inotify. The user-visible effect is that expiry and out-of-band changes take up
to 60 s to surface instead of under 1 s — exactly the budget `tray-state-sync` "A state-file write
is observed within budget" and design §6.3 promise.

The existing test `a_retry_after_the_directory_appears_succeeds_exactly_like_a_first_attempt` reads
as coverage for this clause and is not: its own comment says it proves `Watch::start` "holds no
state across calls", which is a property of the module, not evidence that any caller retries.

### G2 — HIGH — `tray-privileged-invocation` I4's only test executes in none of the five gates

`tests/reactor_responsiveness.rs` is gated on `NOPASS_DBUS_TESTS=1`. Under gate 1
(`cargo test --workspace`) the variable is unset, so the test prints its skip message and returns —
and reports `ok` in 0.00s, indistinguishable in the summary from a real pass. Gate 5
(`scripts/run-lane-b.sh`) sets the variable but restricts cargo to `--test dbus_session`, so this
file is never built into that run.

Observed directly:

```
$ cargo test -p nopass --test reactor_responsiveness
test a_pending_privileged_action_does_not_block_other_reactor_work ... ok
test result: ok. 1 passed; ... finished in 0.00s          # skipped

$ grep -c reactor_responsiveness scripts/run-lane-b.sh
0

$ NOPASS_DBUS_TESTS=1 cargo test -p nopass --test reactor_responsiveness
test a_pending_privileged_action_does_not_block_other_reactor_work ... ok
test result: ok. 1 passed; ... finished in 0.30s          # actually ran
```

I ran it explicitly with the flag and **it passes** — the behaviour is correct. The defect is that
no gate would notice if it stopped being. The test also does not need a bus (its own module doc
says so), so gating it on `NOPASS_DBUS_TESTS` was the wrong lane assignment in the first place.

Smallest fix: drop the env gate from that file so it runs in gate 1, since it needs no bus.

### G3 — MEDIUM — the expiry notification fires for user-initiated disables, and its trigger is untested

Two separate problems in `crates/nopass/src/app.rs:210-215`.

**(a) The guard the spec names is absent.** `tray-notifications` N1 requires the expiry notification
when *"the merged state transitions from active-temporary to inactive **without a pending
tray-initiated action**"*. The implemented condition is only
`trigger == Trigger::FileEvent && was_active && new_state == Inactive`. There is no check against a
pending or just-completed action.

Reachable sequence for a user clicking Disable:

1. `handle_action_finished` posts the `Revoked` notification, sets `pending_escalation`, and calls
   `reconcile(Trigger::ActionCompleted)`. That reconcile reads the already-rewritten file but still
   holds a `Passwordless` probe in cache that passes `usable()` (its `taken_at` is not older than
   `file_observed_at`, which `ActionCompleted` does not update), so `merge` still yields `Active` —
   `current_state` stays `Active`.
2. The helper's state-file write raises inotify. `reconcile(Trigger::FileEvent)` sets
   `file_observed_at = now`, which invalidates the cached probe, so `merge` yields `Inactive`.
   `was_active` is `true`.
3. The expiry notification fires: *"Passwordless sudo has expired."* — immediately after the user
   was told *"Passwordless sudo disabled"*.

**(b) The trigger has no test.** Experiment **E2** removed the whole notification block from
`app::reconcile` and all 438 tests stayed green. The Lane B test named for this scenario,
`successful_action_and_detected_expiry_each_produce_a_delivered_notification`, calls
`expiry_notification()` — the pure composer — and posts it through the notifier port directly. It
never drives `App` through a state transition, so it cannot observe whether the app decides to emit
one, or emits one it should not.

### G4 — MEDIUM — the left-click toggle has no test anywhere

Experiment **F7** emptied `ksni::Tray::activate` in `tray.rs`, so a left click raises no event at
all. Lane B stayed green (15/15). The Lane B test that touches the menu,
`menu_labels_and_sensitivity_match_the_view_model_and_quit_raises_its_event`, asserts, per its own
name, that **Quit** raises its event — it does not exercise the toggle item's `activate` closure or
the SNI `Activate` method.

This is the behaviour PRD RF-02 leads with ("Left click toggles") and `tray-presence`'s minimal
menu depends on. Its absence would be invisible to every gate.

**F6** is the same class: emptying `menu_about_to_show` removes the `MenuOpened` trigger, one of
`tray-state-sync` S3's four named reconciliation triggers, and Lane B stayed green.

### G5 — MEDIUM — nothing proves the app ever renders

Experiment **F18** replaced the body of `App::render` so it never calls `TrayPort::render`. All 438
tests stayed green. `tray-presence` P2 requires the tray to "switch icon on every state transition";
that is proven only at the pure level (`format::icon_name`, `ViewModel::from_state`) and at the
protocol level with a manually constructed `ViewModel` (`sni_properties_match_the_view_model_for_each_tray_state`).
No test observes `App` pushing a `ViewModel` after a state change.

The three app-level ports (`RecordingTray`-style fakes) already exist in `app.rs`'s test module for
the notify path; extending one to count `render` calls would close this in a few lines.

### G6 — MEDIUM — `PolkitReadiness` is computed at every startup and then discarded

Proposal D4 and design §0 G3 both require: *"If either is missing, the toggle renders as unavailable
with an 'incomplete installation' reason rather than firing a `pkexec` that is guaranteed to fail."*

`main.rs::probe_polkit_readiness` implements the full four-step ladder, including a real
`EnumerateActions` call on the system bus with a 2 s timeout, and stores the result in `Preflight`.
`preflight::decide` deliberately does not read it (correctly — §8 says so), and **nothing else reads
it either**. `ActionMissing` never reaches the toggle. The code says so itself at `main.rs:274-280`:
*"nothing yet consumes `PolkitReadiness::ActionMissing` to disable the toggle, since no task in this
phase specifies where that wiring belongs."*

This is a design-and-proposal obligation, not a delta-spec requirement — none of the five specs
mentions polkit readiness — so it does not change a requirement call. But it is an unmet decision
carried in a document that archive will preserve, and the per-startup system-bus round trip is
currently paid for nothing.

### G7 — LOW — the second instance's exit 0 is never observed at process level

`tray-single-instance` U1's scenario says *"the second process exits 0, and no second SNI item is
registered"*. The Lane B tests prove `acquire` classifies `NameTaken` correctly and that `nudge`
returns; the `return 0` in `main.rs:87` and the absence of a second SNI item are proven by nothing.
The three `real_binary_*` tests cover exits 3, 4 and the degraded run; none launches two instances.
Lane C item 8 covers the human-visible half, and it is still open.

### G8 — LOW — the watch's event-kind filter is untested

Experiment **E8** made `watch::is_relevant` return `true` for every `EventKind`, including
`Access`. All tests stayed green. The design's stated intent — *"`Access`/`Other` are excluded so a
mere read of the file never triggers reconciliation"* — is unpinned. The cost of the regression is
small (a spurious probe when something reads the state file) and no spec sentence names it.

### G9 — LOW — the `MenuOpened` probe-cache composition and `file_observed_at` are unpinned in `App`

Experiments **F3** and **F5** each deleted one clause from `app.rs` — the `MenuOpened` cache-staleness
check and the `file_observed_at` update on a file event — and the suite stayed green in both cases.
`reconcile.rs`'s own test `each_trigger_invokes_the_probe_port_exactly_once_per_the_table`
reconstructs the composition inside the test rather than calling the app's `maybe_probe`, so it
pins the intended algebra but not the shipped wiring. `app::tests::menu_opened_with_a_stale_cache_requests_a_probe`
did not catch F3 either.

F5 matters more than it looks: `file_observed_at` is what makes design §3.3's "the probe wins is not
the oldest probe wins" rule real. With it removed, a probe taken before a file event stays usable,
which is precisely the staleness the rule exists to reject.

---

## Spec sentences that contradict the shipped behaviour

If these are merged into `openspec/specs/` unchanged at archive, each becomes project truth that the
code deliberately disobeys — the M1 failure mode the brief names.

### C1 — `tray-presence`, "Tooltip renders remaining time at minute granularity"

> THEN it renders **"caduca en 42 min"** (or the "less than a minute" form below 60 s)

The shipped renderer produces `"42 min"`, composed into `"jorge — Active (42 min)"`. The expected
string is Spanish; every user-facing string in `format.rs` is English, and the proposal states
i18n/`rust-i18n` is M3. The scenario is a leftover from the Spanish-language source PRD.

**Recommended reconciliation:** replace the literal with the shipped English form, or state the
granularity without quoting a locale-specific string.

### C2 — `tray-presence`, "Three Visual States With Distinct Icon Names"

> The tray MUST render **exactly three** visual states, each with its own icon name and `-symbolic`
> variant

The shipped tray renders **four** distinct icon names. `Unknown` renders
`dialog-question-symbolic` with SNI `Status::NeedsAttention`, mandated by design D6 and pinned by
`unknown_never_reuses_the_locked_icon`. The spec's own capability text elsewhere calls `Unknown` a
first-class state, so the two halves of the spec set disagree with each other as well as with the
code.

The same requirement's scenario says the symbolic variant is selected *"when the host requests one"*.
No such negotiation exists: `format::icon_name` defaults to symbolic unconditionally and is
overridden only by the `NOPASS_ICON_STYLE` environment variable (design §7.1). The spec's table
also implies the non-symbolic name is the default; the shipped default is the symbolic one.

**Recommended reconciliation:** state four rendered states including `Unknown` →
`dialog-question-symbolic` + `NeedsAttention`, and replace "when the host requests one" with the
`NOPASS_ICON_STYLE` override and the symbolic default.

### C3 — `tray-state-sync`, "Reconciliation Runs at Four Defined Triggers"

> The tray MUST run the `sudo -kn true` probe at startup, immediately after a completed
> enable/disable action, **on every menu open**, and on a 60-second timer

Design §3.4 deliberately narrows this: *"`MenuOpened` — only if the cached probe is unusable per
§3.3 (a menu open must not cost a 200 ms subprocess every time)"*, and the code implements the
narrower rule (`reconcile::probe_required` returns `true` for `MenuOpened` only in `Unknown`;
`app::maybe_probe` adds the cache-staleness clause). The design's reasoning is sound; the spec
sentence was never updated to match.

**Recommended reconciliation:** amend to "on a menu open when the cached probe is no longer usable".

### C4 — `tray-presence`, "Hard Refusal With No User-Visible Channel"

> The tray MUST print to stderr and exit non-zero … **only when** there is no session bus reachable
> at all, or when neither an SNI host nor a notification service is reachable.

The shipped binary has two further non-zero exits that this "only when" forbids: exit **5** when
`request_name` fails for a reason other than `NameTaken` — which `tray-single-instance` U3 *requires*
— and exit **1** when `KsniTray::spawn` fails, which design §8 lists as "Unexpected internal
failure". As written, `tray-presence` P5 and `tray-single-instance` U3 contradict each other.

**Recommended reconciliation:** scope P5's "only when" to preflight refusals, and cross-reference
design §8's full exit table.

---

## Lane assignment review

The brief asks whether any scenario claims a lane it cannot be proven in. Reviewing all 32:

| Finding | Scenario | Claimed lane | Provable where |
|---|---|---|---|
| **L1** | `tray-presence` → Hard Refusal → "No session bus at all" | `cargo test` (spawns the binary with the bus environment removed) | Its only test, `real_binary_with_no_session_bus_address_exits_3_with_a_stderr_message`, lives in `tests/dbus_session.rs` behind `skip_unless_lane_b!`. Under a bare `cargo test` it does not run. **Claims Lane A, provable only in Lane B.** The test's own comment explains why: an empty environment does not deny a session bus — zbus derives `/run/user/<uid>/bus` from `getuid()` and reaches the developer's real desktop session. Denying a bus needs an address that resolves to nothing, which is a controlled-environment concern, i.e. Lane B |
| **L2** | `tray-privileged-invocation` → "Menu remains responsive during a pending authorization" | `dbus-run-session` (fake spawn port that blocks until released) | The test needs no bus at all, as its own module doc admits, and could run in Lane A. It is gated on `NOPASS_DBUS_TESTS` anyway and then excluded from the Lane B runner. **Claims Lane B, belongs in Lane A, currently runs in neither.** Same root as G2 |
| OK | `tray-presence` → "Icon registers and becomes visible within budget" | real desktop session, additionally dbus-run-session | Honest split; the registration half is genuinely provable in Lane B and is, the < 1 s half genuinely is not |
| OK | `tray-presence` → "No timer exists solely to refresh the tooltip" and `tray-state-sync` → "Only one periodic wakeup source exists" | `cargo test` structural + real desktop for measured CPU/RSS | Honest split. The structural half is implemented as a source grep rather than the "instrumented reactor" the scenario names — a weaker instrument than described, but it does catch a second `Timer::interval` call site |
| OK | all remaining 27 | `cargo test` or `dbus-run-session` | Each is proven in the lane it names |

No scenario claims a lane that is fundamentally impossible; the two findings are both
misplacements that a one-line change to the test file or the runner script would fix.

---

## The three questions

### 1. Coverage honesty — which requirements would survive their behaviour being deleted?

I neutered 26 behaviours one at a time, ran the relevant gate, and restored via `git checkout` after
each. Nineteen were caught, usually by more than one test. **Seven were not.**

| # | Behaviour deleted | File | Gate run | Result | Requirement left unpinned |
|---|---|---|---|---|---|
| E2 | The expiry-notification block in `reconcile` | `app.rs:210-215` | `cargo test -p nopass` | **green** | `tray-notifications` N1 (expiry scenario) |
| E8 | `is_relevant`'s event-kind filter (`true` for everything) | `watch.rs` | `cargo test -p nopass` | **green** | design D8's read-does-not-trigger intent |
| F3 | The `MenuOpened` cache-staleness clause in `maybe_probe` | `app.rs` | `cargo test -p nopass` | **green** | `tray-state-sync` S3 (menu-open half) |
| F5 | The `file_observed_at = now` update on a file event | `app.rs` | `cargo test -p nopass` | **green** | design §3.3 probe-freshness ordering |
| F6 | `menu_about_to_show` raising `MenuOpened` | `tray.rs` | `run-lane-b.sh` | **green** | `tray-state-sync` S3 (menu-open trigger, end to end) |
| F7 | `activate` raising `ToggleRequested` (left click) | `tray.rs` | `run-lane-b.sh` | **green** | PRD RF-02's left-click toggle; `tray-presence`'s menu |
| F18 | `App::render`'s call to `TrayPort::render` | `app.rs` | `cargo test -p nopass` | **green** | `tray-presence` P2 ("switch icon on every state transition") |

Caught, for the record — these are the ones the suite defends well: `DoNotQueue` (3 Lane B
failures), countdown flooring, the 127 disambiguation, `NeedsAttention` on `Unknown`, the nudge rate
limit, the expiry-coherence check in `merge`, the `ActionGate`, `Unknown`'s icon, `toggle_label(Unknown)`,
the locale pass-through list, `MenuOpened`+`Unknown` forcing a probe, the degraded-mode announcement,
the two-stage schema-order check, ENOENT ⇒ `Absent`, `HostAppeared` ⇒ reassert, `ActionCompleted`
always probing, the 100 ms debounce, and the `TimerUnscheduled` escalation wiring.

One partial: **F8**, removing the bound from `instance::nudge`, caused the Lane B suite to **hang**
rather than fail. The defect is detected — the gate never returns 0 — but as a timeout rather than a
named assertion.

**The pattern.** Every uncaught behaviour sits in the same place: the seam where an adapter hands
work to `App`, or where `App` hands work back to an adapter. The pure core is exhaustively tabled and
essentially undeletable. `tray.rs`'s `ksni::Tray` impl is exercised for its *properties*
(`icon_name`, `status`, `tool_tip`, menu labels) but not for its *callbacks*. `app.rs` is exercised
through `handle(Event)` with fake ports, which covers the event→state path but never asserts the
state→port path. 438 tests is a large number honestly earned in the middle of the architecture and
thin at both edges of it.

**Cheapest closure**: a `RecordingTray` in `app.rs`'s test module that counts `render` calls closes
F18 and most of F3/F5; two Lane B assertions that drive `Activate` and `AboutToShow` on the live
`ksni` item and read the resulting `TrayEvent` close F6 and F7; one app-level test driving
`Active → FileEvent → Inactive` closes E2.

### 2. The two open manual tasks — is `tests/manual/README.md` sufficient?

**Yes for the specs; yes for the design's own Lane C list. It is sufficient.**

Exactly three scenarios across the five delta specs name a real desktop session in their
`Testable via:` line:

| Scenario | Spec | Checklist item |
|---|---|---|
| "Icon registers and becomes visible within budget" | `tray-presence` | Item 1 (cites the scenario verbatim) |
| "No timer exists solely to refresh the tooltip" (measured-CPU half) | `tray-presence` | Item 11 |
| "Only one periodic wakeup source exists" (measured RSS/CPU half) | `tray-state-sync` | Item 11 |

All three are covered. The design's own 8-item Lane C list is also fully covered, mapping onto
checklist items 1/2/3, 4, 6, 7, 8, 9, 10 and 11 — with one item beyond the design's list (item 5,
cancelling the polkit dialog, sourced from `tray-privileged-invocation`'s 126 row).

Two things the checklist gets right that are easy to get wrong:

- It carries the developer icon-theme install step
  (`mkdir -p ~/.local/share/icons/hicolor/scalable/apps`, copy `data/icons/*.svg`,
  `gtk-update-icon-cache`). Without it icon lookup by theme name resolves nothing and items 1–3
  would fail for a packaging reason rather than a code reason. Design §7.1 requires exactly this.
- It has a result table with an environment block (DE + version, session type, SNI host, distro,
  build commit, date, operator) and a per-item pass/fail + observed-value row, plus an explicit
  instruction to transcribe it into this report — which is what keeps task 11.4 an observation
  rather than an inference.

**One caveat, not a gap.** The checklist is sufficient to discharge 7.7 and 11.4 as the specs define
them, but it cannot substitute for G4 and G7. Checklist item 8 ("double-launching nudges the running
instance, as a human sees it") is the *only* place the second-instance exit-0 path is exercised at
all, and the left-click toggle (G4) is likewise only reachable through a human clicking item 4. If
Lane C is executed and passes, those behaviours are observed once, by a person, on one machine — they
are still not defended by any gate. Executing the checklist closes the two tasks; it does not close
the coverage holes, and the report should not let the one be read as the other.

**Status of the two tasks.** Both remain open. I have no desktop session in this environment and
recorded no Lane C observation; no item below is marked, and nothing in this report infers a Lane C
result. The environment and results tables in `tests/manual/README.md` are to be filled in by the
human executing them and transcribed here.

### 3. What the eleven phases never covered

Each phase verified its own slice; nothing audited the seams between them. Three things fall in
exactly those seams:

1. **The watch retry (G1).** Phase 4 built `Watch` and proved the module is retry-safe. Phase 10
   wired it into `main.rs` and never built the retry. Neither phase's tests could notice, because
   Phase 4's scope ended at the module boundary and Phase 10's scope treated the watch as already
   done. This is the only clause in the whole contract with no implementation at all.

2. **The polkit readiness result (G6).** Phase 10 built the full four-step ladder that the design
   spent a page specifying, then had no task telling it where the result goes — and said so in a
   code comment rather than leaving it silent, which is to its credit. The consumer side belongs to
   `tray.rs`'s menu (`toggle_label` would need a third input), which was Phase 7's file, already
   closed by then.

3. **The adapter callbacks (G4, G5, F6).** Phase 7 built `ksni::Tray`'s `activate` and
   `menu_about_to_show` and tested the properties around them. Phase 10 built the consumer of the
   events they raise and tested it with synthetic `Event`s. Neither phase owned the wire between
   them, so the wire was never tested, and cutting it today changes no gate.

Two smaller items with no phase owner:

4. **The `Event::ActivateRequested` variant is dead code.** `app::handle` maps it to
   `tray.reassert()`, but nothing constructs it: `AppInterface::activate` calls `reassert` and
   `post` directly rather than sending an event. Harmless, but it makes the event enum read as if
   the nudge flows through the single-owner loop, and it does not. Design D4's "adapters own no
   state and only send `Event`s" is violated here specifically — `AppInterface` holds a `Mutex`
   rate-limit and calls two ports itself.

5. **`std::mem::forget(watch)`** (`main.rs:134`) is the mechanism keeping the watch alive. It works,
   but it is also why a lost watch can never be detected — there is no handle left to check. Storing
   the `Watch` in `App` would serve both this and G1.

---

## Recommended disposition

**Do not archive yet.** Minimum set before archive:

1. Implement the G1 retry (store `run_dir` and the `Watch` in `App`; retry on `Trigger::Tick` when
   absent). This is the one unimplemented requirement clause.
2. Reconcile C1–C4 in the delta specs so the archived canonical specs describe the shipped tray.
3. Fix L2/G2 by removing the `NOPASS_DBUS_TESTS` gate from `tests/reactor_responsiveness.rs`, or by
   adding it to `scripts/run-lane-b.sh`. One line either way.
4. Add the guard and the test for G3 — a user-initiated disable must not be announced as an expiry.

Worth doing in the same pass, cheap and high value: the `RecordingTray` render counter (G5), and two
Lane B assertions covering the `ksni` callbacks (G4).

Deferrable to M3 with a recorded decision: G6 (polkit readiness consumer), G7 (two-binary
single-instance test), G8 (the watch event-kind filter).

Tasks 7.7 and 11.4 stay open until a human executes `tests/manual/README.md` on a real desktop and
transcribes the result table into this report.

---

## Verification hygiene

- No source file, spec file, or task file was modified by this verification. Twenty-six neutering
  edits were applied one at a time and reverted with `git checkout -- <file>` immediately after each
  gate run.
- Final tree state: `git status --short` empty, `git diff --stat` empty, `HEAD` at `7bd28c6`.
- Post-restore gate spot check: `cargo test --workspace` exit **0**, 438 tests;
  `bash scripts/assert-single-reactor.sh` exit **0**.
- No attempt token was settled, nothing was committed, and no Lane C result was inferred.

# Tasks: M2 — Tray application (`crates/nopass`)

Source: `proposal.md` (11-slice delivery plan), `design.md` (§0–§9, Architecture Decisions D1–D9,
File Changes, Threat Matrix), `specs/{tray-presence,tray-state-sync,tray-privileged-invocation,
tray-notifications,tray-single-instance}`. Eleven phases map 1:1 onto the proposal's eleven
delivery slices. TDD: every behaviour task names its RED test(s) and lane before its GREEN
implementation step. Three lanes: **A** = `cargo test --workspace`, unprivileged, no bus, no
desktop; **B** = headless `dbus-run-session` (`NOPASS_DBUS_TESTS=1`), proves name ownership, SNI
registration and notification payloads; **C** = real desktop session, manual, recorded in the
verify report — the only lane that can prove human-visible rendering and polkit authentication.

G1 (single reactor), G2 (`notify-rust` 4.17 resolution) and G3 (polkit `EnumerateActions`
unprivileged, confirmed by proxy via `pkaction`) already PASSED in the orchestrator's scratch-
workspace gate run. Phase 1 re-runs the same assertions against the real `crates/nopass` crate,
because the scratch run never touched this repository's tree. Under the workspace's 1.85 pin the
resolver selects `zbus` **5.13.2**, not the 5.19.0 the exploration/proposal name — tasks below
assert 5.13.x. `notify-rust` resolves to 4.17.0 even bare; the `~4.17` pin stays for the reason
D2 already gives. Exit-17 handling (Phase 5) is written against the design's "bare 17 = state
unknown, reconcile now" rule, independent of the in-flight M1 fix to `rolled_back`.

## Review Workload Forecast

| # | Slice / Phase | Impl | Tests | Total |
|---|---|---|---|---|
| 1 | Crate skeleton, workspace member, dependency pins, feature-tree gate, `notify-rust` check | 60 | 40 | 100 |
| 2 | State reader: parse, `schema != 1` rejection, missing ⇒ `Unknown` | 120 | 180 | 300 |
| 3 | Reconciliation: `sudo -kn true` port, precedence, the merge state machine | 150 | 220 | 370 |
| 4 | inotify watcher, reactor bridge, missing-directory fallback, 60 s tick | 140 | 160 | 300 |
| 5 | `pkexec` invocation and the full exit-code → outcome table | 130 | 230 | 360 |
| 6 | Icon assets (3 + 3 symbolic) and theme-aware name resolution | 100 | 20 | 120 |
| 7 | SNI item: three states, tooltip/countdown, minimal menu, left-click toggle | 220 | 150 | 370 |
| 8 | Notifications: success, error, expiry; degraded-notify path | 110 | 120 | 230 |
| 9 | Single instance: name request, `NameTaken` ⇒ exit 0, `Activate` + nudge | 100 | 110 | 210 |
| 10 | Startup preflight, degraded modes, refusal rules, `main` wiring | 130 | 140 | 270 |
| 11 | Manual desktop lane: checklist + headless `dbus-run-session` harness | 60 | 60 | 120 |
| | **Total** | **1,320** | **1,430** | **2,750** |

Test lines are counted as their own column, per the discipline M1 was missing every phase forecast
it overran. Every slice individually stays at or under the 400 changed-line review budget (largest:
slices 3 and 7 at 370), so the per-PR risk is mitigated by chaining. The **total** (2,750) exceeds
`state.yaml`'s `attempt_ledger.default_max_changed_lines: 2500`, carried over from M1. That ceiling
must be raised deliberately for M2, or the eleven slices split across two `sdd-apply` attempts —
before apply begins, not discovered mid-apply, per `design.md`'s own migration note.

```text
Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High
```

Rationale for `High` despite no single slice exceeding 400 lines: the raw total is 6.9× the per-PR
budget, and `High` here (matching M1's own convention) reflects that scale before chaining
mitigates it — chaining is the mitigation, not evidence the risk was never there. `Decision needed
before apply: No` follows directly from the cached `auto-chain` delivery strategy, exactly as in
M1's tasks.md; the orchestrator proceeds with the first slice using `stacked-to-main` and does not
block on a chain-strategy question. The attempt-ledger overage above is a separate, pre-apply
decision the orchestrator must still resolve (raise the ledger cap or split into two attempts) —
it is not covered by the `auto-chain` mapping and is called out again under `## Next Step` risk.

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | Crate skeleton + gate scripts (Phase 1) | PR 1 | `cargo test -p nopass` | N/A — no runtime behaviour yet | delete `crates/nopass/`, `scripts/assert-single-reactor.sh`, revert the workspace member line |
| 2 | State reader (Phase 2) | PR 2 | `cargo test -p nopass state::` | N/A — pure logic + `TempDir` | delete `state.rs`, `tests/state_tempdir.rs` |
| 3 | Reconciliation + `CommandRunner` port (Phase 3) | PR 3 | `cargo test -p nopass probe:: reconcile:: runner::` | N/A — `ScriptedRunner` only | delete `probe.rs`, `reconcile.rs`, `runner.rs` |
| 4 | inotify watcher + reactor bridge (Phase 4) | PR 4 | `cargo test -p nopass --test watch_inotify` | real inotify against a `TempDir`, no bus | delete `watch.rs`, `tests/watch_inotify.rs`, revert `event.rs`'s tick source |
| 5 | `pkexec` invocation + outcome table (Phase 5) | PR 5 | `cargo test -p nopass invoke:: outcome::` | `dbus-run-session` for the reactor-responsiveness scenario only | delete `invoke.rs`, `outcome.rs` |
| 6 | Icon assets + theme resolution (Phase 6) | PR 6 | `cargo test -p nopass --test icon_assets` | N/A — static SVG assertions | delete `data/icons/`, revert `format.rs`'s `icon_name` |
| 7 | SNI item (Phase 7) | PR 7 | `cargo test -p nopass format::` | `dbus-run-session` (`NOPASS_DBUS_TESTS=1`); real desktop session for §7.7 rendering | delete `tray.rs`, revert `format.rs`'s tooltip/countdown functions |
| 8 | Notifications (Phase 8) | PR 8 | `NOPASS_DBUS_TESTS=1 cargo test -p nopass --test dbus_session` | `dbus-run-session`, fake `org.freedesktop.Notifications` | delete `notifications.rs` |
| 9 | Single instance (Phase 9) | PR 9 | `NOPASS_DBUS_TESTS=1 cargo test -p nopass --test dbus_session` | `dbus-run-session`, two connections on one private bus | delete `instance.rs` |
| 10 | Preflight + `main` wiring (Phase 10) | PR 10 | `cargo test -p nopass preflight::` | `dbus-run-session` for exit codes; real desktop session for the <1 s startup budget | delete `preflight.rs`, `app.rs`; revert `main.rs` to the Phase-1 stub |
| 11 | Manual lane + container harness (Phase 11) | PR 11 | `NOPASS_DBUS_TESTS=1 podman run … cargo test --workspace` | real desktop session (Lane C, recorded in the verify report) | delete `tests/containers/Containerfile.dbus`, `tests/manual/README.md` |

Delivery strategy: `auto-chain`, `stacked-to-main`, 400 changed lines per PR. Rationale: `crates/
nopass` is additive to a working M1 (no deployed users of the tray yet), and each slice reverts
independently per `design.md`'s Migration/Rollout section — Phase 2's state-reader type has no
consumer until Phase 3 exists, so stacking to `main` after each green slice carries no more risk
than a long-lived tracker branch and iterates faster.

---

## Phase 1: Crate Skeleton, Workspace Member, Dependency Pins, Feature-Tree Gate

*(Design §0 G1–G2, §1; Proposal slice 1)*

- [x] 1.1 Modify `Cargo.toml` (workspace root) — add `"crates/nopass"` to `members`; add to
      `[workspace.dependencies]`: `ksni = "=0.3.6"` (`default-features = false, features =
      ["async-io"]`), `zbus = "5"`, `notify-rust = "~4.17"`, `notify = { version = "8",
      default-features = false }`, `async-io = "2"`, `async-channel = "2"`, `futures-lite = "2"`
      (design §1).
- [x] 1.2 Create `crates/nopass/Cargo.toml` — the six deps above plus `nopass-core` (path), `nix`
      (workspace, unchanged features), `serde_json`, `thiserror`; dev-dep `tempfile`.
- [x] 1.3 Create `crates/nopass/src/main.rs` (`#![forbid(unsafe_code)]`, `exit(boot())` stub) and
      `crates/nopass/src/lib.rs` (`#![forbid(unsafe_code)]`, empty module declarations) — design
      §1, §2 `main`.
- [x] 1.4 RED (Lane A): `cargo test --workspace` compiles and passes with zero `nopass` tests —
      baseline gate, mirrors M1's task 1.6.
- [x] 1.5 Create `scripts/assert-single-reactor.sh` — the G1 gate: `cargo tree -i tokio|async-io|
      dbus|glib|gtk|zbus`, the `zbus feature "tokio"` grep, `cargo +1.85 build --release -p
      nopass`; exit non-zero on any violation (design §0 G1).
- [x] 1.6 GREEN: run `scripts/assert-single-reactor.sh` and `cargo +1.85 build --release -p
      nopass` against the real crate. Assert exactly one `async-io` major, zero `tokio`, `zbus`
      resolves to `5.13.x` (not `5.19.x` — MSRV 1.85 selects 5.13.2), `notify-rust` resolves to
      `4.17.0`. Commit `Cargo.lock`.
- [x] 1.7 Modify `openspec/config.yaml` — add the `crates/nopass` test/verify command entries and
      record `scripts/assert-single-reactor.sh` as a required pre-Phase-2 gate.

---

## Phase 2: State Reader — the type that makes "inactive from absence" unrepresentable

*(Design §3.1, D3; Spec: tray-state-sync "Absence or Schema Mismatch Produce Unknown, Never
Inactive"; Proposal slice 2 — the highest-value task in this change: every later phase's
`TrayState` depends on this type, so it lands early and nothing downstream compiles around it)*

- [x] 2.1 Create `crates/nopass/src/state.rs` — `enum FileReading { Parsed(HelperStatus), Absent,
      Faulted(ReadFault) }` and `enum ReadFault { Io, Malformed, UnsupportedSchema{found:u32} }`
      with **no** `Default`, `is_active()`, or `From<FileReading> for TrayState` (design §3.1,
      D3). `SUPPORTED_SCHEMA` aliases `nopass_core::state::SCHEMA_VERSION`.
- [x] 2.2 RED (Lane A) `state.rs`: two-stage `parse` goldens — valid schema-1; `schema: 0`;
      `schema: 2` ⇒ `UnsupportedSchema{found:2}` **not** `Malformed`; truncated JSON; empty file;
      non-UTF-8 bytes; `active:true` with `expires:null` (tray-state-sync "Schema mismatch is
      treated identically to absence"). GREEN: implement `parse` — deserialize `{schema:u32}`
      first, then the full `HelperStatus`.
- [x] 2.3 RED (Lane A) `crates/nopass/tests/state_tempdir.rs` via `Layout::under(TempDir)`: file
      absent ⇒ `Absent`; unreadable dir ⇒ `Faulted(Io)`; rename-in-place while reading ⇒ never a
      torn parse (tray-state-sync "Missing state file never reads as inactive"). GREEN: implement
      `read` — `ENOENT` ⇒ `Absent`, other I/O ⇒ `Faulted(Io)`.
- [x] 2.4 RED (Lane A): threat-matrix "State misrepresentation" cases against `FileReading` alone
      — absent, `schema != 1`, truncated/empty/non-UTF-8, `active:true`+expired,
      `active:true`+`expires:null` each assert the file-derived value never claims activity
      (design threat matrix row 1). GREEN: none beyond 2.2–2.3 — a named test pins the
      type-level guarantee on its own.

---

## Phase 3: Reconciliation — the probe port and the merge state machine

*(Design §3.2–§3.3, §4.1–§4.2, D3; Spec: tray-state-sync "Live Probe Takes Precedence",
"Reconciliation Runs at Four Defined Triggers"; Proposal slice 3)*

- [x] 3.1 Create `crates/nopass/src/runner.rs` — `CommandSpec`, `SpawnOutcome`, `RunnerError`,
      `CommandRunner` trait, `SystemRunner`, `run_off_reactor` (design §4.1–4.2). `ScriptedRunner`
      under `#[cfg(test)]`.
- [x] 3.2 RED (Lane A) `runner.rs`: non-absolute `program` rejected before any spawn (threat
      matrix "External command composition"); `status: None` (signalled) ⇒
      `RunnerError::Signaled`, never treated as success. GREEN: implement `SystemRunner`
      (`env_clear`, `Stdio::null()` stdin, no shell).
- [x] 3.3 Create `crates/nopass/src/probe.rs` — `enum Probe { Passwordless, PasswordRequired }`,
      `enum ProbeError`, `probe::spec` (`/usr/bin/sudo -k -n true`, env `LANG=C, LC_ALL=C`),
      `interpret`, `ProbeCache`, `MAX_PROBE_AGE_SECS = 90` (design §3.3, §4.3).
- [x] 3.4 RED (Lane A) `probe.rs`: exact `CommandSpec` equality for `probe::spec` via
      `ScriptedRunner`; `ProbeCache::usable` — probe older than the file ⇒ `None`; age 89/90/91 s
      boundaries (tray-state-sync "Probe resolves an Unknown state"). GREEN: implement `spec`,
      `interpret`, `ProbeCache::usable`.
- [x] 3.5 Create `crates/nopass/src/reconcile.rs` — `enum TrayState { Active{user,expiry},
      Inactive, Unknown }`, `merge(file, probe, now)`, `enum Trigger`, `probe_required` (design
      §3.2, §3.4).
- [x] 3.6 RED (Lane A) `reconcile.rs`: **all 8 merge-table rows exhaustively**, both probe values
      × every `FileReading` variant × expired/unexpired `now`; named test
      `missing_file_and_probe_passwordless_yields_active_with_no_expiry` (design §3.2 row 1;
      tray-state-sync "Missing state file never reads as inactive", "Probe overrides a stale
      active file"). GREEN: implement `merge` per the 8-row table.
- [x] 3.7 RED (Lane A) `reconcile.rs`: `probe_required` — full `Trigger` × `TrayState` table,
      `Unknown` forces a probe on entry regardless of trigger; a fake probe-port invocation
      counter records exactly one call per `Startup`/`FileEvent`/`ActionCompleted`/60 s `Tick`,
      and only when the cache is unusable for `MenuOpened` (tray-state-sync "Reconciliation Runs
      at Four Defined Triggers"). GREEN: implement `probe_required`.
- [x] 3.8 RED (Lane A): threat matrix "Stale or forged observation source" — a state file
      claiming `active:true` while the probe says `PasswordRequired` renders `Inactive` (design
      threat matrix row 6; `/run/nopass` `0755 root:root` documented as a comment, not a test).
      GREEN: covered by 3.6.

---

## Phase 4: inotify Watch, Reactor Bridge, Missing-Directory Fallback, 60 s Tick

*(Design §4.2, D8; Spec: tray-state-sync "Inotify Watch With Missing-Directory Fallback", "No
Periodic Wakeup Beyond the 60-Second Reconciliation Tick"; Proposal slice 4)*

- [ ] 4.1 Create `crates/nopass/src/watch.rs` — `struct Watch`, `Watch::start(run_dir, uid, tx)`,
      `enum WatchError { DirMissing, Io }`, `DEBOUNCE_MS = 100`; watches the **directory**, never
      the file (design D8 — the helper replaces the state file by `rename`).
- [ ] 4.2 RED (Lane A) `crates/nopass/tests/watch_inotify.rs` — **real inotify against a
      `TempDir`**: create/rename/delete of `<uid>.state` each yield exactly one debounced event;
      an unrelated sibling file yields none; missing directory ⇒ `WatchError::DirMissing`
      (tray-state-sync "A state-file write is observed within budget", "Missing run directory
      falls back to reconciliation only"). GREEN: implement `Watch::start`, bridging `notify`'s
      thread to the reactor via `async_channel`, never a blocking `recv` (design §4.2).
- [ ] 4.3 Create `crates/nopass/src/event.rs` (partial) — `enum Event { FileChanged, Tick,
      ProbeFinished(...), ActionFinished(...), ... }` scaffolding to drive `watch`/probe results
      into a channel (design §2 `event`; full wiring completes in Phase 10).
- [ ] 4.4 RED (Lane A): an instrumented reactor enumerates registered periodic timers — exactly
      one, firing every 60 s; no dedicated countdown timer (tray-presence "No timer exists solely
      to refresh the tooltip"; tray-state-sync "Only one periodic wakeup source exists"). GREEN:
      implement the 60 s `Timer::interval` source feeding `Event::Tick`.

---

## Phase 5: `pkexec` Invocation and the Exit-Code → Outcome Table

*(Design §4.3–§4.4, §5, §5.1, D5, D9; Spec: tray-privileged-invocation (all); Proposal slice 5)*

- [ ] 5.1 Create `crates/nopass/src/invoke.rs` — `pkexec_spec(pkexec, helper, action, locale)`,
      `struct ActionGate` (design §4.3–4.4, D9: `env_clear()` then only `LANG`/`LC_ALL`/
      `LC_MESSAGES` pass through — deliberately unlike M1's `LANG=C` forcing).
- [ ] 5.2 RED (Lane A) `invoke.rs`: exact `pkexec` argv — `pkexec /usr/libexec/nopass-helper
      enable --until <epoch>` / `disable`, no `--user`, no shell; env pass-through list is exactly
      `LANG`/`LC_ALL`/`LC_MESSAGES` and nothing else (tray-privileged-invocation "Enable
      invocation uses the exact documented argv"; threat matrix "External command composition").
      GREEN: implement `pkexec_spec`.
- [ ] 5.3 RED (Lane A) `invoke.rs`: second `ActionGate::try_begin()` while one is in flight ⇒
      `None` (design §4.4). GREEN: implement `ActionGate`.
- [ ] 5.4 Create `crates/nopass/src/outcome.rs` — `enum Action`, `enum OutcomeKind` (one variant
      per §5 row plus `UnexpiringGrant`), `enum Severity`, `classify(action, status,
      helper_present)`, `escalate(prev, probe)` (design §5, §5.1).
- [ ] 5.5 RED (Lane A) `outcome.rs`: one case per code 0/1/2/10–17/126/127/spawn-failure/
      signalled; 127 × `helper_present` both ways (`HelperMissing` vs `NotAuthorized` via
      `access(HELPER_PATH, X_OK)`); a uniqueness assertion over all rendered `(summary, body)`
      pairs (tray-privileged-invocation "Each documented code produces its own message", "Exit 17
      is distinguished...", "pkexec 126 and 127 are distinguished..."). GREEN: implement
      `classify`, the 127 disambiguation.
- [ ] 5.6 RED (Lane A) `outcome.rs` — §5.1 escalation, written against "a bare 17 is state
      unknown, reconcile now" and independent of whether M1's in-flight `rolled_back` fix has
      landed: `TimerUnscheduled` + probe `Passwordless` ⇒ `Some(UnexpiringGrant)`;
      `TimerUnscheduled` + probe `PasswordRequired` ⇒ `None`; no other `OutcomeKind` escalates
      under either probe value; assert the `TimerUnscheduled` text claims neither "nothing was
      changed" nor an active grant (design §5.1; threat matrix "A failure code that may accompany
      a live grant"). GREEN: implement `escalate`.
- [ ] 5.7 RED (Lane A): threat matrix "External command composition" completion — `CommandSpec`
      equality for `pkexec` and `sudo` together, env allow-list equality, non-absolute program ⇒
      `NonAbsoluteProgram` with zero spawns (one table-driven test spanning both `probe.rs` and
      `invoke.rs` ports). GREEN: covered by 3.1–3.2, 5.1–5.2.
- [ ] 5.8 RED (Lane A): the tray's I/O port surface (`/run/nopass/` read+watch, `pkexec`,
      `sudo -kn true`) contains no port that accepts or resolves a path under `/etc/sudoers.d/` —
      a structural assertion over the port trait definitions (tray-privileged-invocation "The
      tray's I/O surface excludes /etc/sudoers.d entirely"). GREEN: none — pins a compile-time
      property; the test documents it.
- [ ] 5.9 RED (Lane B, `dbus-run-session`): a pending `pkexec` (fake spawn port blocking until
      released) does not prevent the tray from handling a menu-open or `Quit` request
      (tray-privileged-invocation "Menu remains responsive during a pending authorization").
      GREEN: proven by the off-reactor thread bridge (design §4.2) under a real event loop, not
      `run_off_reactor` in isolation.

---

## Phase 6: Icon Assets and Theme-Aware Name Resolution

*(Design §7.1; Spec: tray-presence "Three Visual States With Distinct Icon Names"; Proposal
slice 6)*

- [ ] 6.1 Create `data/icons/nopass-{locked,unlocked,unlocked-timed}.svg` and the three
      `-symbolic` variants — six SVG assets, no embedded pixmaps, no `<script`, no `http`-scheme
      external reference (design §7.1).
- [ ] 6.2 RED (Lane A) `crates/nopass/tests/icon_assets.rs` — dependency-free `str` assertions:
      all six present, each starts with an `<svg` root, has a `viewBox`, contains no `<script`,
      no `http`-scheme reference (same pattern as M1's `data_artifacts.rs`, no XML crate). GREEN:
      satisfied by 6.1.
- [ ] 6.3 Create `crates/nopass/src/format.rs` (new file) — `icon_name(state) -> &'static str`,
      `-symbolic` default, `NOPASS_ICON_STYLE=color|symbolic` override (design §7.1).
- [ ] 6.4 RED (Lane A) `format.rs`: `icon_name` full table — `Inactive`⇒`nopass-locked[-symbolic]`,
      `Active{At}`⇒`nopass-unlocked-timed[-symbolic]`, `Active{Never|Reboot|None}`⇒`nopass-
      unlocked[-symbolic]`, `Unknown`⇒`dialog-question-symbolic` (stock, no new asset), both
      styles (tray-presence "Icon name follows the merged state exactly"). GREEN: implement
      `icon_name`.

Note: the design places **no** desktop entry (`data/nopass.desktop`) in M2 — the proposal's Out
of Scope section defers it to M4 packaging alongside `.deb`/`.rpm`/AUR. Nothing is created here.

---

## Phase 7: SNI Item — Three States, Tooltip, Countdown, Minimal Menu

*(Design §2 `tray`, §7.2, D6, D7; Spec: tray-presence (remaining scenarios); Proposal slice 7)*

- [ ] 7.1 RED (Lane A) `format.rs`: `countdown` at 0/1/59/60/61/3599/3600/28800 s and past-epoch;
      `Never`⇒"no expiry"; `Reboot`⇒"until reboot"; past `At`⇒"expired";
      `Active{expiry:None}`⇒"remaining time unknown" — floor, never round (D7; tray-presence
      "Tooltip renders remaining time at minute granularity"). GREEN: implement `countdown`
      (floor to whole minutes).
- [ ] 7.2 RED (Lane A) `format.rs`: `tooltip`/`status_line` render `<user> — <state>
      (<remaining>)`; `toggle_label(Inactive)`⇒"Enable passwordless sudo",
      `toggle_label(Active)`⇒"Disable passwordless sudo", `toggle_label(Unknown) == None` (D6;
      tray-presence tooltip scenario). GREEN: implement `tooltip`, `status_line`, `toggle_label`.
- [ ] 7.3 Create `crates/nopass/src/tray.rs` — `trait TrayPort { render, reassert }`, `struct
      ViewModel`, `struct KsniTray` — the only `ksni`-aware module (design §2 `tray`).
- [ ] 7.4 RED (Lane B, `dbus-run-session`) `crates/nopass/tests/dbus_session.rs` (gated
      `NOPASS_DBUS_TESTS=1`) — SNI registration against a **fake `org.kde.StatusNotifierWatcher`**:
      `RegisterStatusNotifierItem` called; `IconName`/`Status`/`ToolTip`/`Menu` properties read
      back match the `ViewModel` for each `TrayState`, including `dialog-question-symbolic` +
      `NeedsAttention` for `Unknown` (tray-presence "Icon name follows the merged state exactly").
      GREEN: implement `KsniTray::render`.
- [ ] 7.5 RED (Lane B) `dbus_session.rs`: menu — `Status: <user> — <state> (<remaining>)`
      insensitive label; toggle item labelled per `toggle_label`, insensitive with "Checking…"
      when `None`; `Quit` exits 0 (design §7.2 minimal-menu table). GREEN: wire the menu into
      `KsniTray`.
- [ ] 7.6 RED (Lane B) `dbus_session.rs`: no periodic timer other than the 60 s tick is
      registered on the real reactor under a live bus connection (tray-presence "No timer exists
      solely to refresh the tooltip", structural half). GREEN: covered by 4.4; re-verified here
      under the full stack.
- [ ] 7.7 (Lane C, manual — recorded in the verify report) A panel actually draws the icon and
      its tooltip within 1 s of process start; symbolic vs colour variant legibility on light and
      dark themes (tray-presence "Icon registers and becomes visible within budget"). Requires
      the developer icon-theme install step to `~/.local/share/icons/hicolor/scalable/apps/`
      documented in `tests/manual/README.md` (Phase 11) — real installation is M4 packaging.

---

## Phase 8: Notifications — Success, Error, Expiry, Degraded Mode

*(Design §2 `notifications`, §7.3; Spec: tray-notifications (all); Proposal slice 8)*

- [ ] 8.1 Create `crates/nopass/src/notifications.rs` — `trait NotifyPort { post }`, `enum
      Category { Action, Expiry, Environment }`, `struct FreedesktopNotifier` — the only
      `notify-rust`-aware module; one retained `NotificationHandle` per `Category`, updated in
      place; `Urgency::Normal` always (design §7.3).
- [ ] 8.2 RED (Lane B, `dbus-run-session`) `dbus_session.rs`: successful enable/disable emits one
      confirmation notification; a detected external expiry (active-temporary ⇒ inactive with no
      pending tray action) emits its own notification (tray-notifications "Success, Failure, and
      Expiry Notifications"). GREEN: wire `NotifyPort::post` into the app loop's outcome/merge
      results.
- [ ] 8.3 RED (Lane B) `dbus_session.rs`: exit-12 (not a sudoer) and exit-14 (visudo rejected)
      payloads captured and asserted unequal (tray-notifications "Notification Body Is Distinct
      Per Outcome"). GREEN: covered by `format::outcome_text` (5.5) feeding `FreedesktopNotifier`.
- [ ] 8.4 RED (Lane B) `dbus_session.rs`: no owner for `org.freedesktop.Notifications` ⇒ icon/
      tooltip still update, a message is written to stderr, no notification call retried in a
      loop (tray-notifications "Degraded Mode When No Notification Service Is Present"). GREEN:
      implement the degraded-notify path.
- [ ] 8.5 RED (Lane B) `dbus_session.rs`: threat matrix "Untrusted text reaching a user surface"
      — a `VisudoRejected` outcome carrying attacker-shaped stderr renders only the constant
      `format` text, never the stderr; a username containing markup/control characters renders
      literally, length-capped, no markup (design threat matrix row 5). GREEN: assert
      `notifications.rs` composes bodies only from `format::outcome_text` plus username/countdown.
- [ ] 8.6 RED (Lane B): threat matrix "Privilege-request initiation over D-Bus" (notification
      leg) — a hostile fake `org.freedesktop.Notifications` publishing garbage never changes
      `TrayState` (design threat matrix row 4). GREEN: structural — `NotifyPort` is write-only
      from the tray's perspective.

---

## Phase 9: Single Instance — Name Ownership, `NameTaken`, Activation Nudge

*(Design §2 `instance`, §6.4, D3; Spec: tray-single-instance (all); Proposal slice 9)*

- [ ] 9.1 Create `crates/nopass/src/instance.rs` — `acquire(conn)`, `enum Acquisition { Owner,
      AlreadyRunning }`, `nudge(conn)` (bounded, infallible by design), `struct AppInterface`
      serving `org.freedesktop.Application` at `/com/enfoquestic/nopass` (design §2 `instance`,
      §6.4).
- [ ] 9.2 RED (Lane A): `request_name` failing with an error other than `NameTaken` (fake
      connection port) ⇒ stderr + exit non-zero, never the RF-10 exit-0 path (tray-single-instance
      "Non-NameTaken request_name Errors Are a Real Fault"). GREEN: implement the non-`NameTaken`
      branch of `acquire`.
- [ ] 9.3 RED (Lane B, `dbus-run-session`) `dbus_session.rs`: no existing owner ⇒ first instance
      becomes owner; a second connection on the same private bus ⇒ `NameTaken`, second process
      exits 0, no second SNI item registered (tray-single-instance "First instance claims the
      name", "Second instance exits 0 without a second icon"). GREEN: implement
      `Acquisition::Owner`/`AlreadyRunning` dispatch.
- [ ] 9.4 RED (Lane B) `dbus_session.rs`: on `NameTaken`, the second instance calls `Activate` on
      the first within a bounded timeout then exits 0 regardless of success/failure/timeout (fake
      owner that never replies) (tray-single-instance "Second instance nudges the first before
      exiting", "A failed or timed-out nudge still exits 0"). GREEN: implement `nudge`.
- [ ] 9.5 RED (Lane B) `dbus_session.rs`: the first instance's `Activate` handler re-asserts SNI
      registration and emits exactly one status notification (tray-single-instance "The first
      instance reacts to a received nudge"). GREEN: wire `AppInterface::activate` to
      `TrayPort::reassert` + `NotifyPort::post(Category::Environment, ...)`.
- [ ] 9.6 RED (Lane B) `dbus_session.rs`: threat matrix "Privilege-request initiation over D-Bus"
      — ten `Activate` calls in one second produce exactly one notification and zero `pkexec`
      spawns (design threat matrix row 4; nudge rate limit "at most one nudge per 5 s"). GREEN:
      implement the rate limiter in `AppInterface::activate`.

---

## Phase 10: Startup Preflight, Degraded Modes, Refusal Rules, `main` Wiring

*(Design §2 `preflight`/`app`, §6.1, §8, D4; Spec: tray-presence (refusal rows), degraded rows of
tray-state-sync/tray-notifications; Proposal slice 10)*

- [ ] 10.1 Create `crates/nopass/src/preflight.rs` — `struct Preflight`, `enum ServicePresence`,
      `enum PolkitReadiness { Ready, ActionMissing, Indeterminate }`, `enum StartDecision`, `enum
      Mode`, `enum Refusal`, pure `decide(p)` (design §0 G3, §8).
- [ ] 10.2 RED (Lane A) `preflight.rs`: the 4-row `sni_host`×`notifications` decision table
      (`Full`/`NoTrayHost`/`NoNotifications`/`Refuse(NoUserVisibleChannel)`) plus `NoSessionBus`;
      `Indeterminate` polkit never changes the decision (tray-presence "Degraded Start When No SNI
      Host Is Present", "Hard Refusal With No User-Visible Channel"; proposal D4). GREEN:
      implement `decide`.
- [ ] 10.3 RED (Lane A): polkit ladder — `NameHasOwner` absent ⇒ `ActionMissing(no_authority)`;
      `EnumerateActions` present/absent/error-or-timeout falling back to the policy-file stat;
      unresolved ⇒ `Indeterminate` (design §0 G3 — already confirmed PASS via `pkaction` by the
      orchestrator; this pins the ladder's logic against fakes, not a real bus). GREEN: implement
      the polkit ladder.
- [ ] 10.4 Create `crates/nopass/src/app.rs` — `struct App` (single owner of all mutable state),
      `async fn run(App) -> i32`; finish `crates/nopass/src/event.rs`'s `Event` enum (design §2
      `app`/`event`, §6).
- [ ] 10.5 RED (Lane B, `dbus-run-session`) `dbus_session.rs`: no session bus at all ⇒ exit 3;
      neither host nor notification service present ⇒ exit 4, distinct from the SNI-host-only
      degraded case which does not exit (tray-presence "No session bus at all", "Neither host nor
      notification service reachable"). GREEN: implement the exit-code table (0/1/3/4/5, design
      §8).
- [ ] 10.6 RED (Lane A): startup binary spawn with `DBUS_SESSION_BUS_ADDRESS` unset and
      unresolvable ⇒ stderr + non-zero exit (tray-presence "No session bus at all", the
      cargo-test half). GREEN: covered by 10.5's exit-3 path exercised as a real subprocess.
- [ ] 10.7 Wire `crates/nopass/src/main.rs` — `boot()`: `getuid`, `Layout::system()`, connect
      session bus, serve `AppInterface`, `request_name`, run preflight, render `Unknown` and
      register the SNI item **before** any probe (step 7 < step 11 — the <1 s NFR budget),
      subscribe `NameOwnerChanged`, start the inotify watch, spawn the 60 s tick, read the state
      file and spawn the first probe (design §6.1 steps 1–11).
- [ ] 10.8 RED (Lane A): host-absent-at-startup degraded path claims the D-Bus name, emits the
      degraded notification, and does not exit (tray-presence "Host absent at startup, tray still
      runs"); host appears later via `NameOwnerChanged` completes registration without a restart
      in the same process (tray-presence "Host appears later..."). GREEN: covered by 10.2/10.4
      wiring; re-asserted here at the `app::run` level.

---

## Phase 11: Manual Desktop Lane and Headless `dbus-run-session` Harness

*(Design §9 Lane B/C, §0 G3; Proposal slice 11)*

- [ ] 11.1 Create `tests/containers/Containerfile.dbus` — headless `dbus-run-session` image,
      alongside M1's images (design §9 Lane B).
- [ ] 11.2 Modify `tests/containers/README.md` (existing, from M1) — document the
      `dbus-run-session` container invocation; wire `dbus_session.rs`'s gate to skip with a clear
      message when `dbus-run-session` is absent or `NOPASS_DBUS_TESTS` is unset.
- [ ] 11.3 Create `tests/manual/README.md` — Lane C checklist: §10.1 panel rendering; §10.2/§10.3
      polkit dialog + `auth_admin_keep` + `sudo -kn true` returning 0 within 1 s; visible
      notifications; keyboard menu navigation; the double-clicked-launcher path; `sudo -kn true`
      not destroying cached sudo credentials (typed-password item); `pkaction --action-id
      com.enfoquestic.nopass.manage` as the unprivileged user (G3 already PASS by proxy — this
      item converts it into a recorded desktop-session observation); idle RSS < 30 MB and idle
      CPU ≈ 0 over ≥ 1 h with no wakeup source other than the 60 s tick (design §9 Lane C).
- [ ] 11.4 (Lane C, manual — recorded in the verify report, not a gate) Execute the
      `tests/manual/README.md` checklist on this machine's desktop session; record environment
      name/version and the observed result per item, never an inference (design §9 "Provable
      nowhere automatically").

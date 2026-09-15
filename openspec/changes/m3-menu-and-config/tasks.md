# Tasks: M3 — Context menu, durations, user config and i18n

Source: `proposal.md` (8-deliverable scope), `design.md` (§0–§9, Architecture Decisions D1–D6, File
Changes, Threat Matrix), `specs/{tray-menu,activation-consent,user-config,autostart-entry,
localization,tray-privileged-invocation,helper-cli,privilege-admission}`. Eleven phases. TDD: every
behaviour task names its RED assertion and lane before its GREEN implementation step. Three lanes,
unchanged from M2: **A** = `cargo test --workspace`, unprivileged, no bus, no desktop; **B** =
headless `dbus-run-session` (`NOPASS_DBUS_TESTS=1`); **C** = real desktop session, manual, recorded
in the verify report.

**Facts this plan is built against, not re-litigated:**
- **G4 is proven** (design §0). `toml_edit =0.25.15` (`parse,display`) builds at pinned rustc
  1.85.1, adds exactly one package (`toml_writer 1.1.2`), and the DOM API's tolerant-read behaviour
  was smoke-tested. Phase 1 does the ordinary `cargo add`; no verification slice, no fallback ladder.
- **`podman` is absent on this machine; `docker` 29.8.0 is present and its daemon runs.** Phase 10's
  container lane detects the runtime rather than hardcoding `podman`.
- **`systemd-analyze` and `pkaction`/`pkcheck` are present today.** Phase 9's Rank 1 gate runs in
  Lane A now, no container needed.
- **The spec/design lane mismatch this checklist originally flagged has been RESOLVED in the
  specs, not carried as a note.** The two hardening deltas said `Testable via:
  scripts/run-lane-b.sh`, which is the headless session-bus lane and is wrong for both gates:
  Rank 1 needs no bus at all, and Rank 2 needs a SYSTEM bus with `polkitd`, which that lane does
  not run. `helper-cli` now says `cargo test --workspace` (Lane A) and `privilege-admission` says
  `scripts/run-lane-polkit.sh`, matching `design.md` §6. Spec, design and tasks now name the same
  two commands; there is nothing left for apply to reconcile.
- Both hardening gates share one "fail loudly, never skip" rule: `nopass-core::toolgate::require`
  (design §6, reusing the `timefmt.rs` H3 remedy pattern). One shared implementation, not two copies.

## Review Workload Forecast

| # | Phase | Impl | Tests | Total |
|---|---|---|---|---|
| 1 | Foundation: toolgate, atomicfile, duration, `toml_edit` dep | 80 | 100 | 180 |
| 2 | User config | 120 | 160 | 280 |
| 3 | Consent type | 140 | 180 | 320 |
| 4 | Autostart entry | 90 | 110 | 200 |
| 5 | Localization | 160 | 140 | 300 |
| 6 | Menu tree — pure logic | 150 | 140 | 290 |
| 7 | Menu tree — tray wiring, dbus, keyboard | 80 | 90 | 170 |
| 8 | App wiring: consent routing, polkit readiness | 150 | 130 | 280 |
| 9 | Rank 1 gate — real `systemd-analyze` | 70 | 130 | 200 |
| 10 | Rank 2 gate — real polkit, container | 90 | 120 | 210 |
| 11 | Docs, manual lane, carry-forwards | 60 | 20 | 80 |
| | **Total** | **1,190** | **1,320** | **2,510** |

Every individual phase stays at or under the 400-line review budget (largest: Phase 3 at 320,
Consent, split from menu on purpose to stay under budget after an earlier single-phase draft of the
menu work landed at ~420). The **total** (2,510) is 6.3× the per-PR budget, so `High` reflects scale
before chaining mitigates it, matching M2's own convention for this project.

```text
Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High
```

`Decision needed before apply: No` follows from the cached `auto-chain` delivery strategy: the
orchestrator proceeds with Phase 1 under `stacked-to-main` without blocking on a chain-strategy
question. `stacked-to-main` follows M2's precedent for the same reason: M3 is additive to a working
M2, no deployed users of the M3 surface exist yet, and per `design.md`'s Migration/Rollout every new
artifact is user-owned and inert to an M2 binary — reverting any slice restores exactly the prior
working state.

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | Foundation (Phase 1) | PR 1 | `cargo test -p nopass-core toolgate:: && cargo test -p nopass duration:: atomicfile::` | N/A — pure logic + `TempDir` | revert `toml_edit` dep; delete `toolgate.rs`, `atomicfile.rs`, `duration.rs`; revert `timefmt.rs` delegation |
| 2 | User config (Phase 2) | PR 2 | `cargo test -p nopass config::` | N/A — `TempDir` + `XDG_CONFIG_HOME` | delete `config.rs` |
| 3 | Consent type (Phase 3) | PR 3 | `cargo test -p nopass consent:: outcome::` | N/A — `ScriptedRunner` | delete `consent.rs`; `outcome.rs`'s `Action::Enable` change is consumed by Phases 6–8 — revert those first if they have already merged |
| 4 | Autostart (Phase 4) | PR 4 | `cargo test -p nopass autostart::` | N/A — `TempDir` | delete `autostart.rs` |
| 5 | Localization (Phase 5) | PR 5 | `cargo test -p nopass format::` | N/A | revert `format.rs` to its M2 English-only body |
| 6 | Menu tree, pure logic (Phase 6) | PR 6 | `cargo test -p nopass menu::` | N/A — bus-free `MenuNode` | delete `menu.rs`; `tray.rs` is not yet wired to it |
| 7 | Menu tray wiring (Phase 7) | PR 7 | `NOPASS_DBUS_TESTS=1 cargo test -p nopass --test dbus_session` | `dbus-run-session`; real desktop session for keyboard nav (7.3) | revert `tray.rs`'s `menu()`/`TrayEvent` changes to M2's three-item menu |
| 8 | App wiring (Phase 8) | PR 8 | `cargo test -p nopass app:: preflight::` | `dbus-run-session` for the `NameOwnerChanged` re-run scenario | revert `app.rs`/`preflight.rs`/`event.rs` to M2's hardcoded-hour toggle — this is the point where consent+config+menu become load-bearing |
| 9 | Rank 1 gate (Phase 9) | PR 9 | `cargo test -p nopass-helper --test systemd_unit_contract` | real `systemd-analyze` (Lane A, present today) | delete `systemd_unit_contract.rs`; revert `timer.rs`'s `UNIT_PROPERTIES` refactor (argv bytes unchanged, so this is an independent revert per design Migration/Rollout) |
| 10 | Rank 2 gate (Phase 10) | PR 10 | `scripts/run-lane-polkit.sh` | detected container runtime (`docker` on this machine) running `dbus-daemon --system` + `polkitd` | delete `Containerfile.polkit`, `run-lane-polkit.sh`, `polkit_contract.rs`; revert `config.yaml`'s `gate_commands` addition |
| 11 | Docs, manual lane, carry-forwards (Phase 11) | PR 11 | N/A — docs only | real desktop session (Lane C, recorded in the verify report) | revert doc edits |

---

## Phase 1: Foundation — Shared Toolgate, Atomic File Writer, Duration Enum, TOML Dependency

*(Design §0 G4, §2 `duration`, §4 D4 `atomicfile`, §6 `toolgate`; consumed by every later phase)*

- [ ] 1.1 Modify `crates/nopass/Cargo.toml` — `cargo add --package nopass toml_edit@=0.25.15
      --no-default-features --features parse,display` (G4 proven; ordinary add, no fallback ladder).
- [ ] 1.2 RED (Lane A): `cargo tree -i toml_edit --workspace` shows exactly one version 0.25.15;
      `scripts/assert-single-reactor.sh` still passes; `git diff --stat Cargo.lock` shows exactly one
      new `[[package]]` stanza (`toml_writer 1.1.2`). GREEN: satisfied by 1.1; commit `Cargo.lock`.
- [ ] 1.3 Create `crates/nopass-core/src/toolgate.rs` — `pub fn require(tool: &str, why: &str)`:
      `Command::new(tool).arg("--version")....expect(msg)` panics (never `eprintln!`-and-return) when
      the tool is absent — the single shared "fail, don't skip" rule for both hardening gates.
- [ ] 1.4 RED (Lane A) `toolgate.rs`: `require` panics for a nonexistent binary name; passes silently
      for a known-present one. GREEN: implement `require`.
- [ ] 1.5 Modify `crates/nopass-core/src/timefmt.rs` — its private `require_systemd_analyze`
      delegates to `toolgate::require`, replacing its standalone `.expect(...)`. RED/GREEN: existing
      `timefmt.rs` tests (67–105) pass unchanged — a delegation refactor, no new test.
- [ ] 1.6 Create `crates/nopass/src/atomicfile.rs` — `write(path, bytes, mode)`: `O_CREAT|O_EXCL` tmp
      → write → `fchmod` → `fsync` → `rename` → best-effort parent `fsync`; every failure from step 1
      unlinks the tmp.
- [ ] 1.7 RED (Lane A) `atomicfile.rs`, `TempDir`: tmp unlinked on every injected failure; an
      interrupted write never leaves a torn final file; a pre-planted symlink at the target causes
      `O_EXCL` to refuse, not follow (threat matrix "Executable-file authoring"). GREEN: implement
      `write`.
- [ ] 1.8 Create `crates/nopass/src/duration.rs` — `enum GrantDuration { Minutes15, Hour1, Hours4,
      Hours8, UntilReboot, Permanent }`, `const ALL: [GrantDuration; 6]`, `expiry`, `args`,
      `config_key`, `parse`.
- [ ] 1.9 RED (Lane A) `duration.rs`: `args()` table over `ALL` — never both `--until`/
      `--until-reboot`, exactly one variant with neither; `config_key`/`parse` round-trip for all six
      plus an unknown string ⇒ `None` (tray-privileged-invocation "All six durations render their
      documented argv with no collision"). GREEN: implement `expiry`, `args`, `config_key`, `parse`.
- [ ] 1.10 RED (Lane A): a named test documents `GrantDuration::ALL` as the **single** ordering
      constant in the crate, consumed later by Phase 6's submenu build and Phase 7's
      `RadioGroup::select` index lookup — no second ordering may exist (design Open Questions).
      GREEN: none — pins the single-source guarantee `duration.rs` already provides.

---

## Phase 2: User Config

*(Design §4 D4; Spec: user-config (all))* — depends on Phase 1. Parallel-safe with Phases 4, 5.

- [ ] 2.1 Create `crates/nopass/src/config.rs` — `ConfigFault { Io, Malformed }`, `ConfigReading {
      Parsed(Config), Absent, Faulted(ConfigFault) }`, `Config { default_duration: GrantDuration,
      warning_acknowledged: bool }`, `read`, `resolve`, `write`. The persisted TOML key is
      `warn_before_activation` (user-config schema) and maps to `!warning_acknowledged` on
      read/write — name and negation are both intentional; do not let them drift.
- [ ] 2.2 RED (Lane A) `config.rs`, `TempDir` + `XDG_CONFIG_HOME`: path resolves
      `$XDG_CONFIG_HOME/nopass/config.toml` when set, `~/.config/nopass/config.toml` otherwise
      (user-config "Config Path Honors XDG_CONFIG_HOME"). GREEN: implement path resolution.
- [ ] 2.3 RED (Lane A) `config.rs`: missing file ⇒ `default_duration=Hour1`,
      `warn_before_activation=true`, without creating the file (user-config "A missing file resolves
      to schema defaults"). GREEN: implement `Absent` handling in `resolve`.
- [ ] 2.4 RED (Lane A) `config.rs`: malformed TOML, non-UTF-8, truncated, unknown key, and an
      unrecognized `default_duration="3h"` all degrade to defaults plus an observable warning, never
      refuse to start; an unrecognized `default_duration` preserves a valid `warn_before_activation`
      in the same document (user-config "Tolerant Parsing Never Blocks Startup"; threat matrix
      "Config as foreign input"). GREEN: implement per-field tolerant `resolve` over the `toml_edit`
      DOM.
- [ ] 2.5 RED (Lane A) `config.rs`: `write` uses `atomicfile::write` — temp file in the same
      directory, renamed over the target; a write failing after temp-file creation leaves the prior
      file (or its absence) untouched (user-config "Atomic Write"). GREEN: implement `write`.
- [ ] 2.6 RED (Lane A) `config.rs`: a write against a `Faulted` reading first renames the existing
      file to `config.toml.bak` (best-effort) before writing fresh. GREEN: implement the `.bak`
      rename.
- [ ] 2.7 RED (Lane A) `config.rs`: a hostile `default_duration` value (`"; rm -rf /"`) never reaches
      argv — only `GrantDuration::parse`'s closed six-arm match output does (threat matrix "Config as
      foreign input"). GREEN: covered by 1.9 + 2.4 composition; this test pins the composition.

---

## Phase 3: Consent — the Type That Makes an Unconsented Grant Unrepresentable

*(Design §3 D3; Spec: activation-consent (all))* — depends on Phases 1, 2. **MUST complete before
Phase 6** — the invariant is a type, not a per-caller check, and only exists once this lands.

- [ ] 3.1 Create `crates/nopass/src/consent.rs` — `pub struct Granted(())` (no `Default`/`new`/
      `Clone`/`Copy`/`From`/public field — unconstructible outside this module); `pub struct
      ConsentState { acknowledged: bool, pending: Option<GrantDuration> }`; `from_config`, `grant`,
      `arm`, `confirm`, `cancel`, `branch`.
- [ ] 3.2 Modify `crates/nopass/src/outcome.rs` — `Action::Enable` loses its public constructor;
      `pub struct EnableRequest { duration, at, granted: Granted }` with private fields and
      `EnableRequest::new(d, now, g: Granted)` as the only constructor; `Action::kind() ->
      ActionKind`.
- [ ] 3.3 RED (Lane A): a module-doc note on `consent.rs` records `Granted(())` as unconstructible
      outside the module (asserted by review + the private field, no `trybuild` dependency, matching
      M2's declined precedent). GREEN: none — pins a compile-time property.
- [ ] 3.4 RED (Lane A) `consent.rs`: `ConsentState::grant()` returns `None` while unacknowledged;
      `arm(d)` records a pending duration with **zero** `ScriptedRunner` invocations
      (activation-consent "A menu-triggered activation with no recorded consent dispatches
      nothing"). GREEN: implement `grant`, `arm`.
- [ ] 3.5 RED (Lane A) `consent.rs`: every activation-raising `Event` (menu toggle, SNI
      left-click/keyboard, single-instance nudge), fed to an unacknowledged `ConsentState`, produces
      zero invocations — table-driven over every known caller (activation-consent "A non-menu
      activation path...", "...activation nudge never itself dispatches an enable"; threat matrix
      "Consent bypass"). GREEN: none beyond 3.1–3.2 — the type makes the alternative a compile error;
      this test pins the guarantee per caller.
- [ ] 3.6 RED (Lane A) `consent.rs`: `confirm(persist: false)` returns `Some((d, Granted))` exactly
      once per `arm`; `cancel()` clears `pending` with no state change (activation-consent "Confirming
      the branch grants exactly once", "Cancelling the branch grants nothing"). GREEN: implement
      `confirm`, `cancel`.
- [ ] 3.7 RED (Lane A) `consent.rs`: `confirm(persist: true)` sets `acknowledged = true` only after a
      successful config write; a failing write (fake config port) leaves `acknowledged` false and the
      next `grant()` still returns `None` (activation-consent "Don't-Warn-Again Persists Consent", "A
      Failed Consent Write Re-Warns Rather Than Silently Granting"). GREEN: implement the
      write-then-acknowledge sequencing.
- [ ] 3.8 RED (Lane A) `consent.rs`: `branch()` returns `None` once acknowledged, `Some(ConsentBranch)`
      while pending. GREEN: implement `branch` — consumed by Phase 6's menu render.

---

## Phase 4: Autostart Entry

*(Design §4 D5; Spec: autostart-entry (all))* — depends on Phase 1. Parallel-safe with Phases 2, 3, 5.

- [ ] 4.1 Create `crates/nopass/src/autostart.rs` — `enum AutostartState { Enabled, Disabled,
      Indeterminate }`, `path()`, `read(p)`, `enable(p)`, `disable(p)`, `const TEMPLATE: &str =
      include_str!("../../../data/nopass.desktop")`.
- [ ] 4.2 RED (Lane A) `autostart.rs`: path resolves `$XDG_CONFIG_HOME/autostart/nopass.desktop` when
      set, `~/.config/autostart/nopass.desktop` otherwise (autostart-entry "Autostart Path Honors
      XDG_CONFIG_HOME"). GREEN: implement `path`.
- [ ] 4.3 RED (Lane A) `autostart.rs`, `TempDir`: `enable` creates a missing `autostart/` directory and
      writes bytes byte-equal to `TEMPLATE`; `O_EXCL` refuses a pre-planted symlink at the target
      (autostart-entry "Create Writes the Template Verbatim"; threat matrix "Executable-file
      authoring" — `Exec=` is a compile-time constant, never composed from `config.toml`). GREEN:
      implement `enable` via `atomicfile::write`.
- [ ] 4.4 RED (Lane A) `autostart.rs`: `disable` unlinks only `nopass.desktop`, leaves a sibling
      `other-app.desktop` untouched, and succeeds when already absent (autostart-entry "Remove Deletes
      Only the NoPass Entry", "Removing an already-absent entry does not error"). GREEN: implement
      `disable`.
- [ ] 4.5 RED (Lane A) `autostart.rs`: `read` 4-row table — absent ⇒ `Disabled`; `Hidden=true` or
      `X-GNOME-Autostart-enabled=false` present ⇒ `Disabled`; present otherwise ⇒ `Enabled`; other I/O
      error ⇒ `Indeterminate` (autostart-entry "Checkbox State Reflects On-Disk Truth"). GREEN:
      implement `read`.
- [ ] 4.6 RED (Lane A): `data/nopass.desktop` and the crate's packaging manifest install no file under
      any `autostart/` directory (autostart-entry "Packaging Ships No Autostart Entry By Default").
      GREEN: none — asserts existing packaging state; fails only if M3 accidentally adds one.

---

## Phase 5: Localization

*(Design §0 D2; Spec: localization (all))* — rewrites `format.rs`'s body only; public signatures
unchanged. Parallel-safe with Phases 2, 3, 4.

- [ ] 5.1 Modify `crates/nopass/src/format.rs` — replace the body with `enum Lang { En, Es }`,
      `Lang::from_env`, `from_locale_string`, `enum Msg` (one arm per user-facing string, both
      languages required to compile), `Msg::text(self, lang)`, process-wide `OnceLock<Lang>` `lang()`,
      and `_in`-suffixed testable variants of every existing public fn. No call site outside
      `format.rs` changes.
- [ ] 5.2 RED (Lane A) `format.rs`: `Lang::from_locale_string` table — `LC_ALL` > `LC_MESSAGES` >
      `LANG`, first non-empty Spanish wins; `LC_ALL=en_US` beats `LC_MESSAGES=es_ES`; all-unset ⇒
      English (localization "LC_ALL takes precedence...", "No locale variable set resolves to
      English"). GREEN: implement `from_env`/`from_locale_string`.
- [ ] 5.3 RED (Lane A) `format.rs`: every `Msg` arm renders both languages, and the rendered pair is
      distinct per arm (M2's existing distinctness rule, extended). GREEN: implement the `Msg` table
      — an arm omitting `Es` fails to compile by construction.
- [ ] 5.4 RED (Lane A) `format.rs`: `status_line`, `tooltip`, `toggle_label` (`_in` variants) return
      Spanish text under `Lang::Es` with the same call signature as English; no caller in `tray.rs`/
      `app.rs` changes (localization "A Spanish locale changes format.rs output without touching
      callers"). GREEN: covered by 5.1–5.3.
- [ ] 5.5 RED (Lane A) `crates/nopass-core`: `render_rule`'s header output is byte-identical to its
      pinned constant under a Spanish process locale (test harness env, not `set_var`) —
      `nopass_core::{header, template}` accept no locale input (localization "The rendered header is
      byte-identical under a Spanish locale"). GREEN: none — pins existing structural isolation.
- [ ] 5.6 RED (Lane A): `data/com.enfoquestic.nopass.policy` XML fixture parse asserts an
      `xml:lang="es"` variant of `<description>` and `<message>` (localization "The installed policy
      carries a Spanish description and message"). GREEN: modify `data/com.enfoquestic.nopass.policy`
      to add the `es` variants (content-only, not a gate).

---

## Phase 6: Menu Tree — Pure Logic

*(Design §1, §2 `tray`; Spec: tray-menu (all))* — depends on Phases 1–5, **especially Phase 3**: no
menu item exists yet that can construct an `Action::Enable`.

- [ ] 6.1 Create `crates/nopass/src/menu.rs` — bus-free `MenuNode { label, enabled, checked, kind,
      children }`, `MenuModel` (extends M2's `ViewModel` with config/consent/autostart/availability),
      `menu_tree(&MenuModel) -> Vec<MenuNode>` — pure function, no `ksni` import.
- [ ] 6.2 RED (Lane A) `menu.rs`: the full item tree in order — toggle, "Activate during…", "Default
      duration", "Current rule", "Start with session", "About", "Quit" — for any merged state other
      than `Unknown` (tray-menu "The full item tree is present in the exported menu"). GREEN:
      implement the top-level `menu_tree` skeleton.
- [ ] 6.3 RED (Lane A) `menu.rs`: "Activate during…" and "Default duration" each render exactly the
      six durations, same fixed order, sourced from `GrantDuration::ALL` — the array 1.10 pinned as
      the single ordering source (tray-menu "Both submenus render the same six items in the same
      order"). GREEN: implement both submenus from `ALL`.
- [ ] 6.4 RED (Lane A) `menu.rs`: "Default duration" marks exactly the entry matching
      `config.default_duration`; selecting a different entry moves the marker on the next build and
      persists via `config::write` (tray-menu "The marker follows the configured default", "Selecting
      a new default moves the marker and persists it"). GREEN: implement the marker + write-through.
- [ ] 6.5 RED (Lane A) `menu.rs`: "Current rule" renders insensitive items (path, user, expiry,
      remaining) for an active grant, one insensitive "No active rule" item when inactive, and an
      insensitive "Rule details unavailable" item when the probe says active but the state file is
      `Absent`/`Faulted` (tray-menu "Current Rule Renders As Insensitive Reference Items"; resolves
      the proposal's "view current rule" open risk). GREEN: implement the submenu from `status`.
- [ ] 6.6 RED (Lane A) `menu.rs`: "Start with session" checked state comes from `autostart::read(disk)`
      at every build, never cached; checking it invokes `autostart::enable` and the next build shows
      it checked (tray-menu "Start-With-Session Checkbox Toggles the Autostart Entry"; autostart-entry
      "An externally deleted entry is reflected on the next menu build"). GREEN: wire the checkbox to
      `autostart`.
- [ ] 6.7 RED (Lane A) `menu.rs`: the first unconsented "Activate for ⟨d⟩" replaces rows 3–4 with the
      two-step branch (warning, "I understand — activate", "I understand — activate and don't warn me
      again", "Cancel") and dispatches **nothing** (activation-consent "First Activation Branches the
      Menu Instead of Granting"). GREEN: implement `ConsentState::branch()` rendering.

---

## Phase 7: Menu Tree — Tray Wiring, Bus Integration, Keyboard

*(Design §1, §2 `tray`; Spec: tray-menu (bus/keyboard scenarios))* — depends on Phase 6.

- [ ] 7.1 Modify `crates/nopass/src/tray.rs` — `menu()` renders `MenuNode` into `ksni::MenuItem`
      (`SubMenu`, `CheckmarkItem`, `RadioGroup`, `StandardItem{enabled}`); new `TrayEvent` variants for
      duration selection, consent confirm/cancel, default-duration selection, autostart toggle.
      `tray.rs` remains the only module that imports `ksni`.
- [ ] 7.2 RED (Lane B, `dbus-run-session`): the exported menu over a fake `StatusNotifierWatcher`
      matches `menu.rs`'s tree item-for-item, including `RadioGroup.selected` index and submenu
      nesting (tray-menu "The full item tree is present..." bus half). GREEN: wire `KsniTray::menu()`
      to `menu_tree`.
- [ ] 7.3 (Lane C, manual — recorded in the verify report) Every item and submenu entry is reachable
      via keyboard-only navigation on a real desktop session; no item depends on a mouse-only gesture
      (tray-menu "Every item exposes standard keyboard-navigation properties").

---

## Phase 8: App Wiring — Consent Routing, Config/Autostart Ownership, Polkit Readiness

*(Design §2 `app`/`preflight`, §3 D3 "Paths that cannot show a menu", §0 D6; Spec: activation-consent
(dispatch), tray-privileged-invocation, privilege-admission (readiness))* — depends on Phase 7.

- [ ] 8.1 Modify `crates/nopass/src/app.rs` — `App` owns `Config`, `ConsentState`, `PolkitReadiness`,
      `last_warned_fault: Option<ConfigFault>`; `handle_toggle` routes every activation path through
      `ConsentState::grant()` before constructing `EnableRequest::new(...)`, replacing the hardcoded
      `now + 3600` (`app.rs:328-344`) with `duration` taken from the menu selection or
      `config.default_duration` for a left click.
- [ ] 8.2 RED (Lane A) `app.rs`: left click, keyboard `Activate`, and the second-instance nudge, fed
      with an unacknowledged `ConsentState`, each post exactly one `Category::Environment`
      notification ("Open the NoPass menu to activate for the first time") and take no privileged
      action (design §3 D3; activation-consent "A non-menu activation path...", "...activation nudge
      never itself dispatches an enable"). GREEN: implement the `grant() == None` branch for non-menu
      callers.
- [ ] 8.3 RED (Lane A) `app.rs`: `Trigger::MenuOpened` re-reads `config.toml` and `autostart` from
      disk; a config fault notifies only on transition (`last_warned_fault`), matching commit
      `29ebe32`'s rule applied to a second surface. GREEN: implement the re-read + transition-only
      warning.
- [ ] 8.4 Modify `crates/nopass/src/preflight.rs` — D6: `ToggleAvailability { OfferEnable,
      OfferDisable, Unavailable(UnavailableReason) }`, `UnavailableReason { StateUnknown,
      InstallationIncomplete, ActionInFlight }`; `App` stores the preflight `PolkitReadiness`, posts
      one startup notification on `ActionMissing(reason)`, re-runs the ladder on
      `org.freedesktop.PolicyKit1`'s `NameOwnerChanged`.
- [ ] 8.5 RED (Lane A) `preflight.rs`: `probe_polkit_readiness`'s `EnumerateActions` result is
      consumed — an enumeration missing `com.enfoquestic.nopass.manage` yields `ActionMissing`, never
      assumed-ready (privilege-admission "probe_polkit_readiness consumes the enumeration result").
      GREEN: implement the consuming ladder step (replaces the discarded result, verify-report
      H6/G6).
- [ ] 8.6 RED (Lane B, `dbus-run-session`): `ActionMissing` renders the toggle
      `Unavailable(InstallationIncomplete)`, insensitive, with a localized reason label; a later
      `NameOwnerChanged` recovers to `OfferEnable`/`OfferDisable` without a restart. GREEN: wire the
      ladder re-run to `NameOwnerChanged`.
- [ ] 8.7 Modify `crates/nopass/src/event.rs` — finish `Event`/`TrayEvent` wiring: `DurationSelected`,
      `ConsentConfirmed{persist}`, `ConsentCancelled`, `DefaultDurationSelected`, `AutostartToggled`.
- [ ] 8.8 RED (Lane A): threat matrix "External command composition" completion — `EnableRequest`/
      `Action::Enable` argv for all six durations matches `duration::args()` exactly, `pkexec_spec`
      still contains no `sh`/`-c` (tray-privileged-invocation "All six durations render their
      documented argv with no collision"). GREEN: covered by 1.9 + 8.1's wiring; this test pins the
      end-to-end composition.

---

## Phase 9: Rank 1 Hardening Gate — `systemd-run` Argv Validated By Real systemd

*(Design §6; Spec: helper-cli (real-tool scenarios))* — depends on Phase 1 (`toolgate::require`).
Independent of Phases 2–8; `systemd-analyze` is present today, so this runs in Lane A now.

- [ ] 9.1 Modify `crates/nopass-helper/src/timer.rs` — `enum UnitProperty { Timer(&'static str),
      Service(&'static str) }`, `const UNIT_PROPERTIES: [UnitProperty; 5]` (the five
      `AccuracySec=1s`/`Persistent=false`/`WakeSystem=false`/`RemainAfterElapse=false`/`Type=oneshot`
      spellings), `property_args()`, `synthesize_unit(uid, epoch)` — the **only** place these
      spellings exist; argv bytes produced by `property_args()` unchanged from the current literal
      list.
- [ ] 9.2 RED (Lane A) `timer.rs`: `schedule_builds_the_exact_pinned_systemd_run_argv_in_order`
      (existing, `timer.rs:125`) still passes unchanged after the refactor. GREEN: satisfied by 9.1's
      construction.
- [ ] 9.3 Create `crates/nopass-helper/tests/systemd_unit_contract.rs` —
      `the_production_unit_properties_are_accepted_by_systemd_analyze`: `toolgate::require
      ("systemd-analyze", ...)`, `synthesize_unit` into a `TempDir`, `systemd-analyze verify
      <tmp>/x.timer <tmp>/x.service`, assert no diagnostic outside a narrow "`ExecStart=` command does
      not exist/is not executable" allowlist (`HELPER_PATH` deliberately not installed in Lane A)
      (helper-cli "The synthesized systemd-run unit is accepted by real systemd").
- [ ] 9.4 RED (Lane A): **negative control** —
      `a_corrupted_property_token_is_rejected_by_systemd_analyze`: the same synthesis with
      `AccuracySec=1s` mutated to `AccuracySecc=1s` MUST be rejected by `systemd-analyze verify`
      (helper-cli "A corrupted property token fails the gate"). GREEN: none — this test's pass/fail
      *is* the gate; if the allowlist in 9.3 ever widens enough to swallow this, the suite must fail.
- [ ] 9.5 RED (Lane A): `every_unit_property_appears_in_the_production_argv` — `property_args()`
      contains exactly one flag per `UNIT_PROPERTIES` entry and nothing else, pinning that argv and
      the gate read the same array. GREEN: covered by 9.1.
- [ ] 9.6 RED (Lane A): absence of `systemd-analyze` (binary hidden from `PATH` in a controlled
      sub-environment) makes 9.3 **fail**, not skip — `toolgate::require` panics inside the test
      (helper-cli "Absence of systemd-analyze fails the gate, not skips it"). GREEN: satisfied by
      1.3's shared `toolgate::require`, reused here rather than re-implemented.

---

## Phase 10: Rank 2 Hardening Gate — polkit Policy Validated By a Real Authority

*(Design §6; Spec: privilege-admission (real-authority scenarios))* — depends on Phase 1
(`toolgate::require`) and Phase 8 (the fake-authority half of readiness consumption, 8.5). **This
machine has `docker` 29.8.0, not `podman`** — detect the runtime, do not hardcode `podman`.

- [ ] 10.1 Create `tests/containers/Containerfile.polkit` — a plain image (rootful, **not**
      `--privileged`, **not** systemd-as-PID-1 — distinct from the out-of-scope
      `Containerfile.systemd`) whose entrypoint starts `dbus-daemon --system` and
      `/usr/lib/polkit-1/polkitd`, installs `data/com.enfoquestic.nopass.policy` into
      `/usr/share/polkit-1/actions/`, and runs `cargo test -p nopass-helper --test polkit_contract`.
- [ ] 10.2 Create `scripts/run-lane-polkit.sh` — detects the available container runtime (`docker`
      first on this machine since `podman` is absent; falls back to `podman` when present) rather than
      hardcoding one; builds and runs `Containerfile.polkit` with `NOPASS_POLKIT_TESTS=1`; exits
      non-zero when neither runtime is found.
- [ ] 10.3 Create `crates/nopass-helper/tests/polkit_contract.rs` —
      `the_installed_policy_is_enumerated_by_the_real_authority`: `pkaction --action-id
      com.enfoquestic.nopass.manage --verbose`, exit 0, stdout reports `implicit active:
      auth_admin_keep` (privilege-admission "The real polkit engine enumerates the installed
      action").
- [ ] 10.4 RED: **negative control** — `a_malformed_policy_is_not_enumerated`: install a deliberately
      broken copy under a second action id, assert it is absent from `pkaction`'s output
      (privilege-admission "A malformed policy file fails enumeration"). GREEN: none — this test's
      pass/fail *is* the gate; without it a `pkaction` listing everything would pass 10.3 vacuously.
- [ ] 10.5 RED: `the_exec_path_annotation_matches_the_shipped_helper_path` — `pkaction --verbose`'s
      `org.freedesktop.policykit.exec.path` equals `nopass_core::paths::HELPER_PATH`. GREEN: satisfied
      by the unchanged policy file; test pins the constant match.
- [ ] 10.6 RED: absence of a reachable polkit authority (neither runtime present, or the container's
      `polkitd` fails to start) makes `scripts/run-lane-polkit.sh` exit non-zero, never a silent skip
      (privilege-admission "Absence of a polkit authority fails the gate, not skips it"). GREEN:
      satisfied by 10.2's runtime-detection exit path.
- [ ] 10.7 Modify `openspec/config.yaml` — add `scripts/run-lane-polkit.sh` to `verify.gate_commands`
      so `sdd-verify` executes it.
- [ ] 10.8 **Pre-agreed fallback, do not improvise** — if the container lane cannot be stood up inside
      this change's budget: degrade to (a) a Lane C manual checklist item recording verbatim
      `pkaction` output in the verify report, and (b) relabel `data_artifacts.rs`'s existing substring
      assertions in their own module doc as "shape, not acceptance — this is not the Rank 2 gate".
      Never degrade to a substring-only assertion presented as the gate.

---

## Phase 11: Docs, Manual-Lane Updates, Carry-Forwards

*(Proposal Out of Scope; design §7 Lane C row; house-style carry-forward)*

- [ ] 11.1 Modify `tests/manual/README.md` — add M3 Lane C items: full-tree keyboard navigation
      (7.3); first activation observed **once** on a real desktop, never re-presented after "don't
      warn again"; autostart entry surviving a real logout/login (autostart-entry "A fresh install has
      no autostart entry", Lane C half); Spanish rendering under `LANG=es_ES.UTF-8` for menu and
      notifications.
- [ ] 11.2 (Lane C, manual — recorded in the verify report, not a gate) Execute the updated
      `tests/manual/README.md` checklist; record environment name/version and the observed result per
      item, never an inference.
- [ ] 11.3 Carry forward, unchecked, from the M2 archive — do not re-attempt in this change: archived
      M2 task **7.7** (panel rendering within 1 s, symbolic/colour legibility, Lane C) and task
      **11.4** (execute the M2 manual checklist), both still open per
      `openspec/changes/archive/2026-09-15-m2-tray/tasks.md` (read-only reference).
- [ ] 11.4 Carry forward, unchecked — `tests/containers/Containerfile.systemd` has never run end to
      end (no rootful privileged podman on this machine); it stays out of scope for M3 and is not a
      dependency of Phase 10's `Containerfile.polkit`, which needs no systemd-as-PID-1.
- [ ] 11.5 Modify `docs/PRD_NoPass_Linux.md` — mark RF-02/RF-03/RF-06/RF-09 as delivered if the
      document carries per-requirement status markers; read the file first and skip if it does not.

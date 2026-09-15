# Design: M3 — Context menu, durations, user config and i18n

Change: `m3-menu-and-config` · Project: `nopass` · Inputs: `proposal.md`, archived `m2-tray/design.md` + `verify-report.md` §7/H3, the ten canonical specs in `openspec/specs/`, `docs/PRD_NoPass_Linux.md` v1.1 · Date: 2026-09-15

## Technical Approach

M2 proved the hexagon; M3 fills it in without widening it. Four new **pure** modules (`duration`, `config`, `consent`, `autostart`) sit beside `reconcile`/`format`/`outcome`, behind the four existing ports (`CommandRunner`, `TrayPort`, `NotifyPort`, `Probe`). No new port, no new adapter, no new reactor work, no GUI toolkit.

Three properties drive the change, and each is enforced structurally rather than by a remembered rule:

1. **An unconsented grant must be unrepresentable**, not merely guarded. `Action::Enable` loses its public constructor and gains a consent token whose only producer is the consent state machine (§2).
2. **"Permanent = neither flag" must be expressed by construction**, not remembered. One enum owns the XOR (§3).
3. **A gate that re-spells production's constants proves nothing.** Both hardening gates read the *same array* production builds argv from, and both FAIL — never skip — when the real tool is absent (§6).

M3 touches no helper subcommand, no `PKEXEC_UID` handling, and no polkit action. The only boundary-adjacent change is *which* duration argv the tray sends — a variant the helper already accepts and tests.

---

## 0. Verification status — what was proven here and what was not

This executor had **no shell/execution tool** (Read/Write/Grep/Glob/memory only), exactly as the M2 design phase did. **No `cargo` command was run.** Nothing below is reported as observed that was not observed. Every dependency claim is either a fact read out of this repository or an explicit slice-1 gate with an exact command and a pre-agreed fallback.

### Verified from repository contents (read directly)

| Claim | Evidence |
|---|---|
| `toml_edit 0.25.15+spec-1.1.0` is **already resolved into this workspace's `Cargo.lock`** | `Cargo.lock:1138-1148`, reached from `zbus_macros 5.13.2` and `zvariant_derive 5.9.2` via `proc-macro-crate 3.5.0` (`Cargo.lock:874-880, 1586-1592, 1632-1638`) |
| Its entire subtree is **already in the lock** — adding it as a direct dependency adds **zero new packages** | `toml_edit` deps are exactly `indexmap 2.14.2`, `toml_datetime 1.1.1`, `toml_parser 1.1.3`, `winnow 1.0.4` — all four already present (`Cargo.lock:509, 1130, 1151, 1543`) |
| The workspace resolver runs under the 1.85 pin and **selected 0.25.15 anyway** | `rust-toolchain.toml` pins `1.85`; `Cargo.toml:2` `resolver = "3"` is MSRV-aware, and it demoted `zbus` 5.19→5.13.2 and `notify-rust` 4.18→4.17 for exactly that reason (archived design G2, tasks.md:16-17). It did **not** demote `toml_edit` |
| Its subtree contains no tokio, dbus, glib or gtk | Four named deps above; `scripts/assert-single-reactor.sh:49-52` is already green with all four in the lock |
| Enabling `toml_edit`'s `serde` feature **would** add a package | `serde_spanned` is absent from `Cargo.lock`; `toml_edit`'s current dep list carries no `serde_core` edge. The DOM API is therefore the cheaper surface, not just the nicer one |
| `ksni` 0.3.6 exposes `SubMenu`, `CheckmarkItem`, `RadioGroup`/`RadioItem`, `StandardItem{enabled}` | `ksni-0.3.6/src/menu.rs:74-81, 84-129, 174-206, 248-313, 365-411, 433-461` — read from the vendored source, not from memory |
| `SubMenu::about_to_show` is **commented out** in 0.3.6 | `menu.rs:204`. Submenus cannot refresh lazily; the whole tree is rebuilt in `Tray::menu(&self)` (`ksni-0.3.6/src/lib.rs:200`) from `Inner`'s already-rendered state |
| The H3 remedy pattern exists and is the one to reuse | `crates/nopass-core/src/timefmt.rs:92-105` — `require_systemd_analyze()` uses `.expect(...)`, so an absent tool **panics the test** instead of `eprintln!`-and-return |
| `cargo test --workspace` **already requires** `systemd-analyze` | same, `timefmt.rs:67-91` module doc |

### Not executed here — slice-1 gates

**G4 — `toml_edit` builds at MSRV 1.85 as a normal (target) dependency.** What the lock proves is *resolution*, not *compilation*: `proc-macro-crate-3.5.0` and `toml_edit-0.25.15` are present in `~/.cargo/registry/cache/` but **not** extracted into `registry/src/`, so they have never been compiled on this machine. They are also build-graph (host) crates today, and M3 makes `toml_edit` a target dependency of `crates/nopass`. Same rustc, same crate, no build script, no C — low risk, but unproven **by this phase**.

> **RESOLVED by the orchestrator after this design was written.** `cargo add --package nopass
> toml_edit@=0.25.15 --no-default-features --features parse,display` then `cargo build -p nopass`
> on `rustc 1.85.1 (4eb161250 2025-03-15)` — the pinned toolchain — exits 0. `toml_edit 0.25.15`,
> `toml_parser 1.1.3`, `toml_datetime 1.1.1`, `indexmap 2.14.2` and `winnow 1.0.4` all compile.
> `scripts/assert-single-reactor.sh` exits 0 with the dependency present: "exactly one async-io
> major (v2.6.0), zero tokio, no dbus/glib/gtk, zbus tokio feature disabled".
>
> **One correction to the analysis above:** the "zero new `[[package]]` stanzas" claim is off by
> one. The `display` feature — which the DOM write path needs — pulls `toml_writer 1.1.2`, which
> was not in the lock. A lockfile diff against the baseline shows exactly that one addition and
> nothing else. It is not tokio, dbus, glib or gtk, so the gate is unaffected, but the claim as
> written was reasoning from a lock that only covered the *parse* half of the API M3 uses.
>
> The DOM API this design builds on was smoke-tested on the same toolchain, including the
> property the tolerant-read contract depends on: `doc["bogus"].as_str()` on a wrong-typed field
> returns `None` rather than failing the document, and `to_string()` after a write preserves both
> comments and unknown keys. The probe was removed; the dependency was reverted so `sdd-apply`
> adds it as part of its own slice.
>
> The fallback ladder below is therefore **not needed** and slice 1 should not spend time on it.



```bash
# slice 1, before any config.rs code:
cargo +1.85 build --release -p nopass          # after adding toml_edit to crates/nopass
cargo tree -i toml_edit --workspace            # MUST show exactly ONE version: 0.25.15
scripts/assert-single-reactor.sh               # MUST still PASS
git diff --stat Cargo.lock                     # MUST show no new [[package]] stanza
```

**Pre-agreed fallback ladder — do not improvise during apply:**

| Failure | Fallback | Cost |
|---|---|---|
| `toml_edit 0.25.15` fails to build at 1.85 | Bisect down with `cargo update -p toml_edit --precise <v>`. The `DocumentMut`/`Item`/`Value` DOM is unchanged across 0.22–0.25, so **no `config.rs` code changes** | One pin, reviewed |
| No `toml_edit` builds at 1.85 (implausible) | Hand-written line-oriented parser for the two-key subset, in `config.rs`, ~40 lines | Loses comment preservation and `[section]` tolerance; §4's tolerant-read contract is unaffected |
| Adding it introduces a second `winnow`/`indexmap` major | Accept. `assert-single-reactor.sh` asserts nothing about them and two `winnow` majors already coexist (`Cargo.lock:1534, 1543`) | None |

**G5 — the Rank 1 and Rank 2 gates run where they are claimed to run.** §6 specifies both. Neither was executed here.

---

## Architecture Decisions

### D1 — TOML: `toml_edit = "=0.25.15"`, DOM API, **no** `serde` feature

| Option | Tradeoff | Decision |
|---|---|---|
| `toml_edit = "=0.25.15"`, DOM | Already in the lock ⇒ **zero new packages**; format-preserving writes keep the user's comments; per-field tolerance is natural | **Chosen** |
| `toml = "1.1"` + serde derive | Ergonomic, but pulls `toml_parser`+`serde_spanned`+`toml_write` behind a crate **not in the lock** whose MSRV I cannot evidence; and serde fails the *whole document* on one wrong-typed field, which fights §4's tolerant read | Rejected |
| `basic-toml` / hand-rolled | Smallest tree, but `config.toml` is **foreign input the user edits by hand**. A parser that mis-handles `#` in a quoted string or a `[section]` header mis-reads a real file silently | Rejected as primary; kept as the G4 tail fallback |

Exact pin (`=`, not `~`) follows the house precedent for `ksni = "=0.3.6"`: the version named is the version the evidence is about. A caret bump silently re-opens G4.

### D2 — i18n: **no crate.** A `Lang` enum and a `match` inside `format.rs`

`format.rs`'s module doc names `rust-i18n`. Recommend against it, for five reasons that compound:

1. **The language set is two and closed.** `rust-i18n` buys runtime locale loading and YAML catalogues; nothing here is dynamic.
2. **MSRV is the binding constraint and its tree is wide** (`rust-i18n-support` → `globwalk`, `regex`, `serde_yaml`/`toml`, `normpath`, `arc-swap`, `itertools`, `siphasher`). This workspace has already been bitten twice by exactly this (zbus 5.19→5.13.2, notify-rust 4.18→4.17). Every crate in that tree is an independent 1.85 risk I cannot evidence, versus zero for a `match`.
3. **Binary-size NFR (< 5 MB):** `regex` alone is ≈1 MB.
4. **`format.rs` is already a total seam.** M2 asserted structurally that every user-facing string lives there. The catalogue has one call site by construction.
5. **A `match` gives a guarantee the crate cannot**: an untranslated key is a **compile error**, where a missing YAML key is a silent runtime fallback to the key name. That alone settles it.

The general rule this establishes, and the reason D1 and D2 land differently: **parse foreign input with a real parser; render internal strings with a match.**

Shape — it copies `format.rs`'s existing `icon_name` / `icon_name_for_style` split exactly, which is why **no call site outside `format.rs` changes**:

```rust
#[derive(Clone, Copy, PartialEq, Eq)] pub enum Lang { En, Es }
impl Lang {
    /// LC_ALL > LC_MESSAGES > LANG, first non-empty wins. Language subtag
    /// `es` ⇒ Es. Absence, `C`, `POSIX`, anything else ⇒ En.
    pub fn from_env() -> Lang;
    fn from_locale_string(s: &str) -> Option<Lang>;   // the parameterised table the tests drive
}
/// One arm per string. Both languages side by side: a new arm that omits
/// `Es` does not compile.
pub(crate) enum Msg { EnablePasswordlessSudo, /* … */ }
impl Msg { fn text(self, lang: Lang) -> &'static str }

fn lang() -> Lang { *LANG.get_or_init(Lang::from_env) }        // std::sync::OnceLock
pub fn toggle_label(s: &TrayState) -> Option<String> { toggle_label_in(lang(), s) }
fn toggle_label_in(lang: Lang, s: &TrayState) -> Option<String> { … }   // tests use this
```

`OnceLock` per process is deliberate and mirrors `NOPASS_ICON_STYLE`: this crate is `#![forbid(unsafe_code)]`, so a test cannot `set_var`, and the `_in` variants are what make both languages testable without one.

**The sudoers header is unreachable by construction, not by discipline.** `nopass_core::{header, template}` live in a crate that does not depend on `nopass`, has no `Lang`, and whose `render_rule` signature accepts no locale input. RED test: `render_rule` output is byte-identical to its pinned constant, asserted from a test binary run under `LANG=es_ES.UTF-8` (Lane A, via the test harness's own env, not `set_var`).

### D3 — Consent is a **type**, not an `if`

Left click (`tray.rs:113`), the menu toggle (`tray.rs:133`), the keyboard path and the single-instance nudge all converge on `app::handle_toggle` (`app.rs:328-344`), which constructs `Action::Enable { until }` inline. A runtime guard there is one refactor away from being forgotten — which is precisely the proposal's **High**-likelihood risk. Make it unrepresentable instead, using M2's own move (§3: "the only function that can produce `TrayState::Inactive` from an absent file requires a probe result as an argument"):

```rust
// consent.rs — the ONLY producer of a Granted anywhere in the crate.
/// Proof that the first-activation warning was shown and explicitly
/// accepted. No `Default`, no `new`, no `Clone`, no `Copy`, no public
/// field, no `From`. The unit field is private, so this type cannot be
/// named into existence outside this module.
pub struct Granted(());

/// Opaque on purpose: `Acknowledged` is NOT a public enum variant, so no
/// call site can fabricate a state that yields a `Granted`.
pub struct ConsentState { acknowledged: bool, pending: Option<GrantDuration> }

impl ConsentState {
    pub fn from_config(c: &Config) -> Self;            // acknowledged = c.warning_acknowledged
    /// The single gate. `None` ⇒ the caller MUST route to `branch()`.
    pub fn grant(&self) -> Option<Granted>;
    pub fn arm(&mut self, d: GrantDuration);           // menu: "Activate for <d>" while unwarned
    pub fn confirm(&mut self, persist: bool) -> Option<(GrantDuration, Granted)>;
    pub fn cancel(&mut self);
    pub fn branch(&self) -> Option<ConsentBranch>;     // what the menu renders instead
}

// outcome.rs — Action::Enable loses its public constructor.
pub enum Action { Enable(EnableRequest), Disable }
/// Private fields ⇒ `Action::Enable` is unconstructible without a `Granted`.
pub struct EnableRequest { duration: GrantDuration, at: u64, granted: Granted }
impl EnableRequest { pub fn new(d: GrantDuration, now: u64, g: Granted) -> Self }

/// Authority-free description, for the outcome table and the in-flight record.
#[derive(Clone, Copy)] pub enum ActionKind { Enable { expiry: Expiry }, Disable }
impl Action { pub fn kind(&self) -> ActionKind }
```

`classify` and `handle_action_finished` take `ActionKind`, so `Granted` never needs `Clone` and one grant buys exactly one invocation. `invoke::pkexec_spec` keeps its signature and gains nothing to check: it *cannot be called* for an enable without the token.

**RED tests (Lane A):** (a) every `Event` that can request activation, fed to an `App` whose `ConsentState` is unacknowledged, produces **zero** `ScriptedRunner` invocations; (b) a compile-fail note in `consent.rs`'s doc records that `Granted(())` cannot be constructed externally — asserted by review plus the private field, not by a `trybuild` dev-dependency (declined for the same reason M2 declined one).

**Paths that cannot show a menu.** Left click, keyboard `Activate`, and the second-instance nudge cannot render a branch. All three take the `grant() == None` arm, which has exactly one behaviour, in one place: post one `Category::Environment` notification ("Open the NoPass menu to activate for the first time") and take no privileged action.

### D4 — Config is read at startup **and** at every menu open

`state.rs` already answers this question shape with `FileReading { Parsed, Absent, Faulted }`; `config.rs` mirrors it:

```rust
pub enum ConfigFault { Io, Malformed }
pub enum ConfigReading { Parsed(Config), Absent, Faulted(ConfigFault) }
pub struct Config { pub default_duration: GrantDuration, pub warning_acknowledged: bool }
pub fn read(path: &Path) -> ConfigReading;
pub fn resolve(r: &ConfigReading) -> (Config, Option<ConfigFault>);
```

**One deliberate asymmetry with `state.rs`, stated so nobody reads it as backsliding.** `FileReading` forbids a `From<FileReading> for TrayState` because the state file is a claim about the *world* and absence is not evidence. `config.toml` is a claim about the *user's preferences*, where every field has a safe default — so `resolve` is legitimate here. Its defaults are the conservative ones: `default_duration = Hour1` (M2's shipped behaviour) and `warning_acknowledged = false`. **A faulted config never grants**; re-warning is the safe failure, exactly as the proposal requires.

- **Never refuses to start.** `Absent` and both `Faulted` arms yield defaults. Malformed is never fatal.
- **Warn once per degraded stretch, not once per read.** `App` holds `last_warned_fault: Option<ConfigFault>`; a fault notifies only on transition. This is commit `29ebe32`'s rule (`fix(tray): warn once per degraded stretch, not once per tick`) applied to a second surface, not a new invention.
- **Per-field tolerance.** An unknown key is ignored; a wrong-typed or unrecognised `default_duration` falls back to `Hour1` **without** discarding a valid `warning_acknowledged`. The DOM makes this the natural implementation; serde would have made it custom code.
- **Re-read at `Trigger::MenuOpened`.** `reconcile` already runs `state::read` at that wake point; one more small read is inside budget, and it removes the "cached intent vs disk truth" divergence class for config the same way the proposal already demands it for autostart. Hand-deleting `warning_acknowledged` re-arms the warning — the safe direction.
- **Writes are atomic**, in a new `crates/nopass/src/atomicfile.rs` reusing the helper's discipline (`fileops.rs`): `O_CREAT|O_EXCL` tmp → write → `fchmod 0600` → `fsync` → `rename` → best-effort parent `fsync`; every failure from step 1 unlinks the tmp. Two deliberate differences from `fileops.rs`, both because this is a user-owned preferences file and not the privilege boundary: mode `0600` not `0440`, and `create_dir_all` on `~/.config/nopass` **is** allowed (the helper refuses to create `/etc/sudoers.d`; an XDG dir legitimately may not exist). The code is duplicated rather than shared: `fileops.rs` also runs `visudo` and `fchown`s to root, and unifying them would drag both concerns across the privilege boundary. `nopass-core` is not a home for it — config.yaml pins it as "no UI or system dependencies".
- **A write against a `Faulted` reading first renames the existing file to `config.toml.bak`** (best-effort) before writing a fresh document. The user's broken hand-edit is preserved, not destroyed.
- **Write failure surfaces**: a failed `warning_acknowledged = true` write posts an error outcome and leaves the flag false in memory. The user is re-warned next time. Never a silent grant.

Location: `${XDG_CONFIG_HOME:-$HOME/.config}/nopass/config.toml`, resolved once at startup into `App`.

### D5 — Autostart state is read from disk, and the disable *markers* are honoured

```rust
pub enum AutostartState { Enabled, Disabled, Indeterminate }
pub fn path() -> PathBuf;               // ${XDG_CONFIG_HOME:-~/.config}/autostart/nopass.desktop
pub fn read(p: &Path) -> AutostartState;
pub fn enable(p: &Path) -> io::Result<()>;   // atomicfile::write(TEMPLATE), 0644
pub fn disable(p: &Path) -> io::Result<()>;  // unlink
pub const TEMPLATE: &str = include_str!("../../../data/nopass.desktop");
```

Content comes from `include_str!`, not from `/usr/share/applications/` at runtime: the tray must not depend on its own package layout, and M4 may relocate it.

**Finding the proposal did not anticipate.** File presence alone is *not* the truth. GNOME Tweaks disables an autostart entry by writing `X-GNOME-Autostart-enabled=false` into the file rather than deleting it, and the freedesktop spec's own marker is `Hidden=true`. A presence-only checkbox would show "on" for an entry the session ignores. So `read` is:

| On disk | ⇒ |
|---|---|
| absent (`ENOENT`) | `Disabled` |
| present, contains `Hidden=true` or `X-GNOME-Autostart-enabled=false` | `Disabled` |
| present, otherwise | `Enabled` |
| any other I/O error | `Indeterminate` |

Enabling from a marker-disabled file **rewrites** it from `TEMPLATE`, dropping the marker. Disabling **unlinks** — unambiguous, and what the proposal specifies. `Indeterminate` renders the checkbox insensitive, following `toggle_label(Unknown) == None`: a state whose value is underivable must not be actionable. Read at every `Trigger::MenuOpened`, never cached across menu opens.

### D6 — The polkit readiness result stops being discarded, and the toggle gains a *reason*

M2 computes `PolkitReadiness` and throws it away (verify-report H6/G6), so §0 G3's promise — "`ActionMissing` renders the toggle unavailable with an incomplete-installation reason" — was never delivered. `format::toggle_label -> Option<&'static str>` cannot carry a reason, which is also M2's open question ("the user is not left with a dead menu and no reason"). Replace it:

```rust
pub enum ToggleAvailability { OfferEnable, OfferDisable, Unavailable(UnavailableReason) }
pub enum UnavailableReason { StateUnknown, InstallationIncomplete, ActionInFlight }
```

Each reason renders its own insensitive, localized label. `App` stores the preflight `PolkitReadiness`, posts one `Category::Environment` notification at startup on `ActionMissing(reason)`, and re-runs the ladder when `org.freedesktop.PolicyKit1`'s owner changes on the system bus (polkitd restarts on package upgrade; a startup `ActionMissing` must not be permanent). `Indeterminate` still never disables anything — M2's rule is unchanged.

---

## 1. Menu tree — against `ksni` 0.3.6's actual API

`Tray::menu(&self)` rebuilds the whole tree from `Inner`'s already-rendered state (`lib.rs:200`), and `SubMenu::about_to_show` does not exist in 0.3.6 (`menu.rs:204`). Therefore **the entire tree is a pure function `menu_tree(&MenuModel) -> Vec<MenuItem<Inner>>`**, and `MenuModel` — an extension of M2's `ViewModel` — carries every already-formatted string. Lane A asserts the tree without a bus by exporting a dependency-free mirror (`MenuNode { label, enabled, checked, kind, children }`) that `menu_tree` is built from; `tray.rs` is the only module that turns `MenuNode` into `ksni::MenuItem`.

| # | Item | `ksni` type | Sensitivity |
|---|---|---|---|
| 1 | `<user> — <state> (<remaining>)` | `StandardItem` | **insensitive** (unchanged from M2) |
| 2 | — | `Separator` | — |
| 3 | **Activate for ▸** (shown iff `OfferEnable`) | `SubMenu` | sensitive |
| 3.1–3.6 | 15 minutes · 1 hour · 4 hours · 8 hours · Until reboot · Permanently | `StandardItem` ×6 | sensitive |
| 4 | **Disable passwordless sudo** (shown iff `OfferDisable`) | `StandardItem` | sensitive |
| 4′ | `Unavailable(r)` label for `r` | `StandardItem` | **insensitive** |
| 5 | — | `Separator` | — |
| 6 | **Current rule ▸** | `SubMenu` | `enabled: false` unless `Active` |
| 6.1–6.3 | `User: jorge` · `Expires: 18:42 (42 min)` · `Rule: /etc/sudoers.d/nopass-1000` | `StandardItem` ×3 | **all insensitive** |
| 6.x | `Rule details unavailable — the state file could not be read` | `StandardItem` | insensitive; the §3.2-row-1 case (probe says active, file `Absent`/`Faulted`) |
| 7 | **Default duration ▸** | `SubMenu` | sensitive |
| 7.1 | six options, `selected = index_of(config.default_duration)` | **`RadioGroup`** | sensitive |
| 8 | **Start NoPass at login** | **`CheckmarkItem`** `{ checked: autostart == Enabled, enabled: autostart != Indeterminate }` | per D5 |
| 9 | — | `Separator` | — |
| 10 | **About NoPass ▸** | `SubMenu` | sensitive |
| 10.1–10.3 | `NoPass <version>` · `Helper: /usr/libexec/nopass-helper` · `Config: ~/.config/nopass/config.toml` | `StandardItem` ×3 | **all insensitive** |
| 11 | **Quit** | `StandardItem` | sensitive |

`RadioGroup` is the right type for item 7 and is why the current-value marker needs no hand-drawn glyph: `selected: usize` *is* the marker, and `select: Box<dyn Fn(&mut T, usize)>` delivers the index. Item 6 answers the proposal's open risk — **insensitive menu items under a "Current rule" submenu, fed from the existing `state::read`, not a notification.** Every item is a real D-Bus menu node, so keyboard reachability is the host's (Lane C confirms it; Lane B asserts the exported tree).

### The consent branch

On the first `Activate for ▸ <d>` while unacknowledged, `ConsentState::arm(d)` runs, **no invocation is made**, and the next `menu()` replaces rows 3–4 with:

| Item | `ksni` type | Sensitivity |
|---|---|---|
| `⚠ Read this before activating` | `StandardItem` | insensitive |
| `Any program running as you can become root without a password until this expires.` | `StandardItem` | insensitive |
| — | `Separator` | — |
| `I understand — activate for <d>` | `StandardItem` | sensitive → `confirm(persist: false)` |
| `I understand — activate and don't warn me again` | `StandardItem` | sensitive → `confirm(persist: true)` |
| `Cancel` | `StandardItem` | sensitive → `cancel()` |

**"Don't warn again" is a third action, not a checkbox.** SNI hosts close the menu when an item is activated, so a check-then-activate flow inside one open menu is not reliably performable. Three unambiguous single-click actions carry the same three semantics with no intra-menu state. `confirm` is the only path that yields `(GrantDuration, Granted)`.

---

## 2. Durations — the XOR lives in one enum

```rust
pub enum GrantDuration { Minutes15, Hour1, Hours4, Hours8, UntilReboot, Permanent }
impl GrantDuration {
    pub const ALL: [GrantDuration; 6];
    pub fn expiry(self, now: u64) -> Expiry;     // At{now+900|3600|14400|28800} | Reboot | Never
    pub fn args(self, now: u64) -> Vec<String>;  // the ONLY place the XOR is spelled
    pub fn config_key(self) -> &'static str;     // "15m" "1h" "4h" "8h" "reboot" "permanent"
    pub fn parse(s: &str) -> Option<Self>;
}
```

| Variant | argv appended after `enable` |
|---|---|
| `Minutes15` / `Hour1` / `Hours4` / `Hours8` | `--until <now + 900 \| 3600 \| 14400 \| 28800>` |
| `UntilReboot` | `--until-reboot` |
| `Permanent` | *(nothing)* |

`Permanent ⇒ vec![]` is expressed by construction. RED test over `ALL`: no variant's `args` ever contains both `--until` and `--until-reboot`, and exactly one contains neither. `cli.rs:19` already declares `ArgGroup::new("when").multiple(false)`, so the helper enforces the same XOR from the other side — unchanged.

`app.rs:335`'s `now + 3600` becomes `EnableRequest::new(duration, now, granted)`, where `duration` is the menu selection or, for a left click, `config.default_duration`.

---

## 3. Data flow

```
startup ──▶ config::read(~/.config/nopass/config.toml)
               │ Parsed / Absent / Faulted(Io|Malformed)
               ▼
            config::resolve ──▶ Config{default_duration, warning_acknowledged}
               │                     │                       │
               │                     ▼                       ▼
               │            ConsentState::from_config   Trigger::MenuOpened re-read
               │                     │
menu open ─────┴──▶ state::read + probe ─▶ merge ─▶ TrayState ─┐
                    autostart::read(disk) ─────────────────────┤
                    ConsentState::branch() ────────────────────┼─▶ MenuModel
                    PolkitReadiness ───────────────────────────┘      │
                                                                      ▼
                                                         menu_tree ─▶ ksni::MenuItem

"Activate for 4 h"  ──▶ ConsentState::grant()
                          │ None  ─▶ arm(d); re-render with the consent branch; NO invocation
                          │ Some(Granted)
                          ▼
                    EnableRequest::new(d, now, granted) ─▶ Action::Enable
                          ▼
                    ActionGate::try_begin() ─▶ pkexec_spec ─▶ run_off_reactor
                          ▼
                    Event::ActionFinished(ActionKind, result) ─▶ classify ─▶ notify
                          └─▶ Trigger::ActionCompleted ⇒ ALWAYS probe   (M2 D5, unchanged)
```

---

## 4. File changes

| File | Action | Description |
|---|---|---|
| `crates/nopass/src/duration.rs` | Create | `GrantDuration`, `expiry`, `args`, `config_key`/`parse` |
| `crates/nopass/src/config.rs` | Create | `Config`, `ConfigReading`, `read`, `resolve`, `write` |
| `crates/nopass/src/consent.rs` | Create | `Granted`, `ConsentState`, `ConsentBranch` |
| `crates/nopass/src/autostart.rs` | Create | `AutostartState`, `read`, `enable`, `disable`, `TEMPLATE` |
| `crates/nopass/src/atomicfile.rs` | Create | write-temp → fsync → rename → fsync-parent, mode `0600`/`0644` |
| `crates/nopass/src/menu.rs` | Create | `MenuModel`, `MenuNode`, `menu_tree` — bus-free, Lane-A assertable |
| `crates/nopass/src/format.rs` | Modify | `Lang`, `Msg`, `*_in` tables; public signatures preserved |
| `crates/nopass/src/tray.rs` | Modify | `menu()` renders `MenuNode`; new `TrayEvent` variants |
| `crates/nopass/src/outcome.rs` | Modify | `Action::Enable(EnableRequest)`, `ActionKind`; `classify` takes `ActionKind` |
| `crates/nopass/src/app.rs` | Modify | owns `Config`/`ConsentState`/`PolkitReadiness`; `handle_toggle` routes through `grant()` |
| `crates/nopass/src/preflight.rs` | Modify | ladder re-run on polkit `NameOwnerChanged` |
| `crates/nopass/src/event.rs` | Modify | `DurationSelected`, `ConsentConfirmed{persist}`, `ConsentCancelled`, `DefaultDurationSelected`, `AutostartToggled` |
| `crates/nopass-helper/src/timer.rs` | Modify | `UNIT_PROPERTIES` const array + `property_args()` + `synthesize_unit()`; argv **bytes unchanged** |
| `crates/nopass-helper/tests/systemd_unit_contract.rs` | Create | Rank 1 gate (§6.1) |
| `crates/nopass-helper/tests/polkit_contract.rs` | Create | Rank 2 gate (§6.2) |
| `crates/nopass-core/src/toolgate.rs` | Create | `require(tool)` — the single "fail, don't skip" rule |
| `crates/nopass-core/src/timefmt.rs` | Modify | its private `require_systemd_analyze` delegates to `toolgate::require` |
| `tests/containers/Containerfile.polkit` | Create | dbus-daemon + polkitd image for §6.2 |
| `scripts/run-lane-polkit.sh` | Create | the §6.2 lane runner |
| `Cargo.toml`, `crates/nopass/Cargo.toml` | Modify | `toml_edit = "=0.25.15"` (tray only) |
| `openspec/config.yaml` | Modify | add `scripts/run-lane-polkit.sh` to `verify.gate_commands` |
| `data/nopass.desktop` | Unchanged | now also `include_str!`-embedded as the autostart template |
| `data/com.enfoquestic.nopass.policy` | Unchanged | content unchanged; §6.2 adds the gate |
| `crates/nopass-helper/`, `crates/nopass-core/` | Otherwise unchanged | behaviour consumed as archived |

---

## 5. Sequence: first activation (the privileged flow, per `rules.design`)

```
user ─ menu ─▶ "Activate for 4 hours"
   │ TrayEvent::DurationSelected(Hours4)
   ▼
app: ConsentState::grant() ──▶ None            ← unacknowledged
   │ ConsentState::arm(Hours4)
   │ TrayPort::render(MenuModel{ consent: Some(FirstActivation{Hours4}), .. })
   ▼  NOTHING IS INVOKED. ActionGate untouched. No pkexec. No helper.
user ─ menu ─▶ "I understand — activate and don't warn me again"
   │ TrayEvent::ConsentConfirmed{ persist: true }
   ▼
app: config.warning_acknowledged = true; config::write(...)   ← atomic; failure ⇒ error outcome,
   │                                                             flag stays false, user re-warned
   │ ConsentState::confirm(true) ──▶ Some((Hours4, Granted))
   │ ActionGate::try_begin() ──▶ Some(ticket)   (None ⇒ ignore, M2 §4.4 unchanged)
   │ EnableRequest::new(Hours4, now, granted) ──▶ Action::Enable
   ▼
run_off_reactor ─▶ /usr/bin/pkexec /usr/libexec/nopass-helper enable --until <now+14400>
   │                    └─▶ polkitd ─▶ agent ─▶ [human] ─▶ helper(root) ─▶ sudoers.d + state file
   ▼
Event::ActionFinished(ActionKind::Enable{expiry:At}, result)
   │ classify ─▶ notification            Trigger::ActionCompleted ⇒ ALWAYS probe
   ▼ merge ⇒ render      (M2 §5's "an exit code is never evidence of state" is unchanged)
```

Disable and expire are byte-for-byte M2's flows; neither is touched.

---

## 6. The two hardening gates

Both obey one rule, taken verbatim from the H3 remedy already in the tree (`timefmt.rs:67-105`): **when the real tool is absent the test panics.** No `eprintln!`-and-return — `cargo test` captures a passing test's stderr, which is how H3 produced `ok` in 0.00 s with exit 0. Both share one implementation so the rule cannot be softened in one copy:

```rust
// nopass-core/src/toolgate.rs — std only; nopass-core keeps zero system deps.
pub fn require(tool: &str, why: &str) -> ();   // Command::new(tool).arg("--version") … .expect(msg)
```
`timefmt.rs`'s private `require_systemd_analyze` is rewritten to call it. Two copies of a "fail loudly" rule is how one of them gets softened; H3 is exactly that defect class.

### 6.1 Rank 1 — `systemd-run`'s seven arguments · **Lane A**

The drift problem is the whole problem: a gate that re-spells `AccuracySec=1s` proves that we can type it twice. So **the spellings move into one array that production argv and the gate both read**:

```rust
// timer.rs — the ONLY place these spellings exist.
pub enum UnitProperty { Timer(&'static str), Service(&'static str) }
pub const UNIT_PROPERTIES: [UnitProperty; 5] = [
    UnitProperty::Timer("AccuracySec=1s"),
    UnitProperty::Timer("Persistent=false"),
    UnitProperty::Timer("WakeSystem=false"),
    UnitProperty::Timer("RemainAfterElapse=false"),
    UnitProperty::Service("Type=oneshot"),
];
/// `schedule` pushes exactly this. Renders Timer(x) ⇒ "--timer-property=x",
/// Service(x) ⇒ "--property=x".
pub fn property_args() -> Vec<String>;
/// The gate renders the SAME array into unit text. Argv bytes are unchanged
/// by this refactor — `schedule_builds_the_exact_pinned_systemd_run_argv_in_order`
/// (timer.rs:125) passes untouched and is the proof of that.
pub fn synthesize_unit(uid: u32, epoch: u64) -> (String, String);
```

`synthesize_unit` writes, into a `TempDir`, `<unit_name(uid)>.timer` and `<unit_name(uid)>.service` — so `--unit=` and `--description=` are validated too (an illegal unit name becomes an illegal filename; `Description=` becomes a `[Unit]` directive), together with `OnCalendar=format_systemd_calendar(epoch)` and `ExecStart=<HELPER_PATH> expire --uid <uid>`. All seven tokens, one array, one source.

`crates/nopass-helper/tests/systemd_unit_contract.rs`, three tests:

1. `the_production_unit_properties_are_accepted_by_systemd_analyze` — `toolgate::require("systemd-analyze", …)`, then `systemd-analyze verify <tmp>/x.timer <tmp>/x.service`. Assert no diagnostic outside a **narrow** allowlist of the "ExecStart= command does not exist / is not executable" family, since `HELPER_PATH` is deliberately not installed in Lane A.
2. **`a_corrupted_property_token_is_rejected_by_systemd_analyze`** — the negative control, and the reason test 1's allowlist is safe: the same synthesis with `AccuracySec=1s` → `AccuracySecc=1s` MUST be rejected. If the allowlist ever widens enough to swallow a real error, this test goes green-when-it-should-be-red and fails the suite.
3. `every_unit_property_appears_in_the_production_argv` — `property_args()` contains one flag per `UNIT_PROPERTIES` entry and nothing else, pinning that the gate and argv really do read the same array.

**Lane and tool absence:** Lane A, `cargo test --workspace`. A machine without `systemd-analyze` **fails**, never skips — and `cargo test --workspace` already requires it for `nopass-core` today, so this widens no requirement. `verify.test_command` and `gate_commands` are unchanged.

### 6.2 Rank 2 — the polkit policy file · **new container lane**

**`EnumerateActions`'s result stops being discarded** — that half is D6 and runs in Lane A/B against fakes.

**Validating the file itself needs the real engine, and there is no honest unprivileged way to get it.** Stated plainly rather than papered over: polkit parses action files inside `polkitd` at startup from a compiled-in directory, with a hand-written GMarkup parser that does *not* consult the DTD. `pkaction`/`pkcheck` read only *installed* actions through the running authority. So an unprivileged, uninstalled-file check would either be a DTD validation (a different parser, plus an HTTP DOCTYPE fetch in CI) or another substring assertion — i.e. the defect being closed, wearing a costume. **Rejected.**

The gate therefore runs where it can be real: `tests/containers/Containerfile.polkit` — a plain image (rootful podman, **not** `--privileged`, and **not** systemd-as-PID-1, so it is not the out-of-scope `Containerfile.systemd`) whose entrypoint starts `dbus-daemon --system`, starts `/usr/lib/polkit-1/polkitd`, installs `data/com.enfoquestic.nopass.policy` into `/usr/share/polkit-1/actions/`, and runs `cargo test -p nopass-helper --test polkit_contract`. Three tests:

1. `the_installed_policy_is_enumerated_by_the_real_authority` — `pkaction --action-id com.enfoquestic.nopass.manage --verbose`, exit 0, and stdout reports `implicit active: auth_admin_keep`. That asserts polkit *parsed and accepted* the file, which is the thing no substring can prove.
2. **`a_malformed_policy_is_not_enumerated`** — the negative control: install a deliberately broken copy under a second action id, assert it is absent from `pkaction`'s output. Without it, an image where `pkaction` lists everything would pass test 1 vacuously.
3. `the_exec_path_annotation_matches_the_shipped_helper_path` — `pkaction --verbose` output's `org.freedesktop.policykit.exec.path` equals `nopass_core::paths::HELPER_PATH`, read from the constant.

**Lane and tool absence:** a new lane, run by `scripts/run-lane-polkit.sh` and added to `openspec/config.yaml`'s `verify.gate_commands`, so `sdd-verify` executes it. Inside the container, `NOPASS_POLKIT_TESTS=1` is set and **every** failure to reach `pkaction` or the system bus is a test failure, never a skip. Outside it, the script exits non-zero when `podman` is missing. There is no `eprintln!`-and-return anywhere in this lane.

**Pre-agreed fallback if the container lane cannot be stood up inside M3's budget** — and this is the part that must not be improvised: it does **not** degrade to a substring assertion. It degrades to (a) a Lane C manual checklist item whose *observed* `pkaction` output is recorded verbatim in the verify report, and (b) relabelling `data_artifacts.rs`'s existing assertions in their own module doc as **"shape, not acceptance — this is not the Rank 2 gate"**, so no future reader mistakes them for one. A named, visible gap beats an invisible pass.

---

## 7. Testing strategy

| Layer | What | Approach |
|---|---|---|
| Unit (Lane A) | `GrantDuration::args` over `ALL`; never both flags, exactly one neither | table test |
| Unit (Lane A) | `config::read`/`resolve`: valid, absent, truncated, non-UTF-8, unknown key, wrong-typed `default_duration` (⇒ `Hour1`, `warning_acknowledged` preserved), unknown duration string | `TempDir` + `XDG_CONFIG_HOME` |
| Unit (Lane A) | `atomicfile`: tmp unlinked on every failure; interrupted write never leaves a torn final file; `.bak` created only from a `Faulted` reading | `TempDir` |
| Unit (Lane A) | **consent**: every activation event × unacknowledged ⇒ **zero** `ScriptedRunner` invocations; `confirm` is the only producer of `Granted`; `confirm(persist:true)` with a failing write leaves the flag false | `ScriptedRunner` exhaustion panic |
| Unit (Lane A) | `autostart::read` 4-row table incl. `Hidden=true` and `X-GNOME-Autostart-enabled=false`; `enable` output equals `TEMPLATE`; `disable` unlinks | `TempDir` |
| Unit (Lane A) | `menu_tree` per `(TrayState × consent × config × autostart × availability)`: item order, insensitivity, `RadioGroup.selected` index | `MenuNode` mirror, bus-free |
| Unit (Lane A) | `format`: `Lang::from_locale_string` table; both languages for every `Msg`; distinctness of rendered pairs (M2's existing rule, extended) | table test |
| Unit (Lane A) | sudoers header byte-identical under `LANG=es_ES.UTF-8` | `nopass-core`, pinned constant |
| Gate (Lane A) | **Rank 1** — §6.1, three tests, `systemd-analyze` required | real tool |
| Integration (Lane B) | exported menu tree over a fake `StatusNotifierWatcher`: every item published, labels match `MenuModel`, submenu nesting correct | `dbus-run-session` |
| Gate (new lane) | **Rank 2** — §6.2, three tests, real `polkitd` | container |
| Manual (Lane C) | keyboard navigation of the full tree; first activation observed once on a real desktop; autostart survives a real logout/login; Spanish rendering under `LANG=es_ES.UTF-8` | checklist, recorded |

---

## Threat Matrix

The reference matrix targets VCS/PR automation; all five reference rows are marked, and the four real boundaries are expanded beneath them.

| Boundary | Minimum adversarial cases | Applicability | Design response | Planned RED tests |
|---|---|---|---|---|
| Documentation-like paths | `requirements.txt`, executable Markdown, `README.sh` | **N/A** — M3 classifies and executes no repository file | — | — |
| Git repository selection | `git -C`, relative/absolute paths | **N/A** — no VCS invocation | — | — |
| Commit state | staged, `commit -a`, empty index | **N/A** — no VCS invocation | — | — |
| Push state | tracking branch, first push, refspec | **N/A** — no VCS invocation | — | — |
| PR commands | `--head`, env prefix, composed commands | **N/A** — no PR automation | — | — |
| **Executable-file authoring** (`autostart.rs` writes a `.desktop` with `Exec=`) | `Exec=` altered to another binary; a relative `Exec`; a symlink at the target path; a pre-existing file | **Applicable** | `TEMPLATE` is `include_str!`-embedded and never composed from runtime input; `Exec=nopass` is a compile-time constant; `atomicfile` opens the tmp with `O_CREAT\|O_EXCL` so a planted symlink fails `EEXIST`; mode `0644`, never `+x` | written file byte-equals `TEMPLATE`; `O_EXCL` refuses a pre-planted symlink; no value from `config.toml` ever reaches the written bytes |
| **Config as foreign input** (`config.toml` is user-editable) | `default_duration = "; rm -rf /"`, huge file, non-UTF-8, `[section]`, duplicate keys | **Applicable** | `GrantDuration::parse` is a closed 6-arm match returning `Option`; an unparsed value **never** reaches argv, only `Hour1`; the value is never interpolated into a command | unknown/hostile `default_duration` ⇒ `Hour1` and the `ScriptedRunner` sees the default argv; non-UTF-8 ⇒ `Faulted(Malformed)` ⇒ defaults + one warning |
| **External command composition** (durations extend `pkexec` argv) | both duration flags; a negative/overflowing epoch; a shell wrapper | **Applicable** | `args()` is the single XOR site; `CommandSpec` only, never a shell string; absolute program enforced pre-spawn (M2 §4.1, unchanged) | `ALL`-variant argv table; never both flags; `pkexec_spec` still contains no `sh`/`-c` |
| **Consent bypass** (a new activation path reaching `enable`) | left click, keyboard `Activate`, second-instance nudge, a future path | **Applicable** | `Action::Enable` is unconstructible without a `Granted` whose only producer is `ConsentState::confirm` (D3) — a compile error, not a runtime check | every activation event × unacknowledged ⇒ zero invocations; `Granted` has no public constructor |

---

## Migration / Rollout

No data migration. M3 is additive to a working M2 and every new artifact is user-owned and inert to an M2 binary: `~/.config/nopass/config.toml` is ignored by it, and `~/.config/autostart/nopass.desktop` launches an M2 tray perfectly well. No system path is written, so no revert can leave a stale sudoers rule or timer. The two gates are test-only and revert independently of every feature slice. Packaging still ships no autostart entry (`crates/nopass/Cargo.toml:61-62`, unchanged).

---

## Open Questions

- [x] **G4 is proven.** No `cargo` was available to this phase, so the orchestrator ran it after the fact: `toml_edit =0.25.15` with features `parse,display` builds on rustc 1.85.1 and the reactor gate stays green, adding exactly one package (`toml_writer 1.1.2`). See §0. The fallback ladder is not needed.
- [ ] **The Rank 2 container lane's real cost is unmeasured.** Standing up `dbus-daemon --system` + `polkitd` in a rootful, non-privileged image is believed straightforward but was not attempted here. If it exceeds the slice budget, take §6.2's named fallback — never a substring assertion.
- [ ] **`systemd-analyze verify`'s exit status on a missing `ExecStart` target varies by systemd version** (255 was observed by the M2 verifier on a different assertion). Test 1's allowlist must be written against the version in the lane and test 2 must stay red-on-corruption; if the allowlist cannot be kept narrow, install a stub executable at a temp path and pass it via a `synthesize_unit` parameter rather than widening the filter.
- [ ] `toml_edit`'s exact feature set at 0.25 (whether `display` is on by default, or must be named) is not transcribed here on purpose — a feature name guessed from memory is a build failure. Slice 1 names it against the compiled pin, as M2 did for `ksni`'s API.
- [ ] Re-reading `config.toml` at every `MenuOpened` is one extra small read per menu open. If Lane C shows menu-open latency regressing, fall back to startup-only plus a re-read after each write; the autostart read stays per-open regardless, because that file has a second writer.
- [ ] `RadioGroup`'s `select` callback delivers an index, not a variant. `GrantDuration::ALL` is the index basis and must be the single ordering used by both the menu and `selected`; a second ordering anywhere reintroduces exactly the drift class §6.1 exists to kill.

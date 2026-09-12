# Design: M2 — Tray application (`crates/nopass`)

Change: `m2-tray` · Project: `nopass` · Inputs: `proposal.md`, `exploration.md`, `state.yaml`, archived `m1-core-helper/design.md`, `openspec/specs/helper-{cli,observability}/spec.md`, `docs/PRD_NoPass_Linux.md` v1.1 · Date: 2026-09-12

## Technical Approach

The same hexagonal split M1 proved, applied to a process that can observe almost nothing. `crates/nopass` has a pure core that decides *what is true* from bytes and exit statuses alone — state-file parsing, the `(file, probe)` merge, the exit-code → outcome table, and every user-facing string — and a thin adapter ring that touches D-Bus, inotify, and subprocesses. Every external program goes through a `CommandRunner` port so exact argv is asserted without a privileged environment, exactly as `nopass-helper` does.

Two properties drive the whole crate:

1. **The tray cannot read the truth.** `/etc/sudoers.d` is `0750 root:root`. It reads two proxies and merges them. The merge is a pure function, exhaustively tabled.
2. **"Inactive because the file was missing" must be unrepresentable.** Not "discouraged" — unrepresentable. The state reader's return type has no inactive arm reachable from absence, and the only function that can produce `TrayState::Inactive` from an absent file requires a probe result as an argument.

M2 is purely additive. It does not edit `nopass-core`, `nopass-helper`, `rust-toolchain.toml`, or the MSRV.

---

## 0. Verification status — what was proven and what was not

The phase brief required three checks to be *run*, not assumed. This executor had no shell/execution tool available (Read/Write/Grep/Glob/memory only), so **none of the three could be executed here.** Nothing below is reported as observed that was not observed. Each is converted into a slice-1 gate with an exact command, an exact pass criterion, and a pre-agreed fallback, so that no design decision depends on an unverified guess.

### Verified from repository contents (real evidence, read directly)

| Claim | Evidence |
|---|---|
| Workspace uses `resolver = "3"`, `rust-version = "1.85"`, edition 2024 | `Cargo.toml` lines 2–7 |
| `nix` is already a workspace dependency with `features = ["fs", "user", "dir"]` | `Cargo.toml` line 16 — the tray needs no new crate for `getuid`/`access` |
| The exact `nopass-core` surface the tray reuses exists and is `pub` | `state.rs` (`SCHEMA_VERSION = 1`, `HelperStatus` with `Deserialize`), `expiry.rs` (`Expiry`, `is_expired`), `paths.rs` (`Layout::system`, `state_path`, `HELPER_PATH = "/usr/libexec/nopass-helper"`) |
| Helper exit codes are exactly 0, 1, 2, 10–17, and each cause is distinct | `crates/nopass-helper/src/error.rs` `exit_code()` + its `every_failure_cause_maps_to_a_distinct_code` test; 2 is clap-owned (`cli.rs` tests) |
| `HelperStatus` deserializes (not just serializes) — the tray can parse the state file with no new type | `#[derive(... Serialize, Deserialize)]`, `state.rs` line 19 |
| **Exit 17 is reachable only from `enable` with an `At` expiry**, and its `rolled_back` flag is not evidence | `ops.rs` 233–239: the `TimerFailed` return is inside `if let Expiry::At { epoch } = expiry`, so `Never`/`Reboot` never reach the scheduler. Line 236 constructs `rolled_back: true` **unconditionally**, while `rollback_rule` (274–278) discards the unlink result (`let _ = std::fs::remove_file(&path)`). Step 15 returns early, so the state file (step 16) is never written on this path |

### Not executed here — slice-1 gates

**G1 — single reactor, single D-Bus stack.** The proposal asks for `cargo tree -e features`. That assertion is *too weak*: two `async-io` **major** versions in the tree mean two reactor threads even with no tokio anywhere, and `notify-rust` may pull a different `zbus` major than `ksni`. The gate is therefore sharper than the proposal's:

```bash
cargo tree -i tokio       --workspace   # MUST match no packages
cargo tree -i async-io    --workspace   # MUST show exactly ONE version
cargo tree -i dbus        --workspace   # MUST match no packages (libdbus C backend)
cargo tree -i glib        --workspace   # MUST match no packages (NFR: no GTK/Qt)
cargo tree -i gtk         --workspace   # MUST match no packages
cargo tree -i zbus        --workspace   # more than one version = stop and review
cargo tree -e features -p nopass | grep -F 'zbus feature "tokio"'   # MUST be empty
cargo +1.85 build --release -p nopass
```

Shipped as `scripts/assert-single-reactor.sh` (exit non-zero on any violation) so it is one command in slice 1 and again at verify. Expressing the invariant as a *tree* assertion rather than a feature-name assertion is deliberate: it catches a wrong `notify-rust` backend feature without the design having to name a feature flag it could not compile.

**Pre-agreed fallback if G1 fails** — do not improvise during apply:

| Failure | Fallback | Cost |
|---|---|---|
| `ksni` `async-io` feature does not compile or panics against zbus | **Fallback B (all-tokio)**: `ksni` at defaults, `zbus = { version = "5", default-features = false, features = ["tokio"] }`, and `#[tokio::main(flavor = "current_thread")]` | The RSS objection in D1 was against tokio's *multi-thread* scheduler; the current-thread flavor is one thread, so the NFR survives. This is the better-travelled path and is not a crisis |
| Two `async-io` majors (mismatched `zbus` majors between `ksni` and `notify-rust`) | Align majors by bumping/pinning whichever crate is behind; if impossible, take Fallback B, which collapses both onto one tokio runtime | One dependency bump, reviewed |
| `notify-rust` resolves the `dbus` (libdbus C) backend | `default-features = false` plus the zbus backend feature, named from the crate's own docs at that moment | Trivial |

**G2 — `notify-rust` version selection.** Registry facts from explore (4.18.0 requires rustc 1.89.0; 4.17.0 requires 1.63.0) are taken as given. **The design pins `~4.17` explicitly and does not rely on resolver 3 picking it.** This is a decision, not a failure to verify (see Decision D2 below): MSRV-aware resolution is a *preference* that is silently defeated by `resolver.incompatible-rust-versions = "allow"` in any `.cargo/config.toml`, by `cargo update --precise`, by `--ignore-rust-version`, and by any third crate requiring `>=4.18`. Its failure surfaces as a compile error in someone else's packaging environment, not here. A tilde requirement converts that into a resolution-time error in this repository. Gate: `cargo tree -i notify-rust` shows `4.17.x`.

**G3 — polkit `EnumerateActions` unprivileged.** Not executed. Strong prior, *not* proof: `pkaction(1)` is an unprivileged user tool whose whole job is calling `EnumerateActions` on `org.freedesktop.PolicyKit1.Authority`, and polkit's shipped bus policy grants `send_interface="org.freedesktop.PolicyKit1.Authority"` in the `context="default"` policy. **The design does not depend on the answer.** The preflight result is three-valued and only a *definite* negative constrains the UI:

```rust
pub enum PolkitReadiness { Ready, ActionMissing, Indeterminate(&'static str) }
```

Resolution ladder, first conclusive answer wins:

1. `NameHasOwner("org.freedesktop.PolicyKit1")` on the **system** bus. Absent ⇒ `ActionMissing` reason `no_authority`.
2. `EnumerateActions("")`, 2 s timeout, scan for `com.enfoquestic.nopass.manage`. Present ⇒ `Ready`; conclusively absent ⇒ `ActionMissing`.
3. If (2) errors, is denied, or times out — **the fallback the proposal asked for** — stat `/usr/share/polkit-1/actions/com.enfoquestic.nopass.policy` (world-readable `0644` by convention). Present ⇒ `Ready`; absent ⇒ `ActionMissing`.
4. Anything still unresolved ⇒ `Indeterminate`.

**`Indeterminate` never disables the toggle.** A probe that failed is not evidence that the product is broken; disabling the only action over a failed probe is a worse outcome than firing a `pkexec` that renders exit 127 with a good message. Only `ActionMissing` renders the toggle unavailable with an "incomplete installation" reason. `pkaction` verification is folded into the desktop lane checklist so the prior becomes an observation rather than staying a belief.

---

## 1. Workspace and Cargo

```
crates/nopass/
├── Cargo.toml
└── src/{main,lib,state,probe,reconcile,outcome,format,runner,invoke,watch,tray,notifications,instance,preflight,event,app}.rs
data/icons/nopass-{locked,unlocked,unlocked-timed}[-symbolic].svg
scripts/assert-single-reactor.sh
```

Root `Cargo.toml`: add `"crates/nopass"` to `members`, and to `[workspace.dependencies]`:

```toml
ksni          = { version = "=0.3.6", default-features = false, features = ["async-io"] }
zbus          = "5"                                   # own defaults = async-io; never default-features = false
notify-rust   = "~4.17"                               # see G2 / D2
notify        = { version = "8", default-features = false }
async-io      = "2"
async-channel = "2"
futures-lite  = "2"
```

`crates/nopass` dependencies: the six above plus `nopass-core` (path), `nix` (workspace, unchanged features), `serde_json`, `thiserror`. Dev: `tempfile`. `main.rs` and `lib.rs` both start with `#![forbid(unsafe_code)]`.

Notes that are load-bearing, not preference:

- **`ksni` is pinned `=0.3.6`, exactly.** Pre-1.0 with a documented zbus-interaction panic class; a caret bump is a silent re-open of G1.
- **`zbus` must never be given `default-features = false`** without simultaneously naming a runtime feature — that disables `async-io` and produces a connection that nothing drives. The fallback path in G1 is the only place `default-features = false` is legal, and it names `tokio` in the same breath.
- **`async-channel` and `futures-lite` are declared directly, not borrowed transitively.** They are used in our own code; relying on them being somewhere inside zbus's tree is exactly the unverified-assumption class this milestone exists to eliminate.
- `notify = { default-features = false }` keeps the cross-platform extras out; the inotify backend is unconditional on Linux. If that combination fails to build, take defaults and record it in slice 1 — it is not worth a redesign.
- **A library crate (`lib.rs`) plus a thin `main.rs`**, for the same reason `nopass-helper` has one: integration tests under `crates/nopass/tests/` cannot otherwise reach the modules.
- `notifications.rs`, **not** `notify.rs`: a module named `notify` in a crate that also depends on the `notify` crate is a path-ambiguity trap in edition 2024.

### What is reused from `nopass-core` rather than reimplemented

| Item | Use |
|---|---|
| `state::HelperStatus` (`Deserialize`) | The parsed state file. The tray defines **no** parallel wire type |
| `state::SCHEMA_VERSION` | The `schema != 1` rejection compares against this constant, never a literal `1` |
| `expiry::Expiry`, `Expiry::is_expired(now)` | Countdown and the "file claims active but its own expiry has passed" contradiction check |
| `paths::Layout::system().state_path(uid)` | The only `Layout` method the tray calls. `rule_path`/`rule_tmp_path`/`lock_path` are helper-only |
| `paths::HELPER_PATH` | The `pkexec` target argv[1] and the exit-127 disambiguation stat |

`Layout::under(&TempDir)` is what makes the state-reader and inotify tests run unprivileged — the same injection point M1 built.

---

## 2. Module map and public API

| Module | Public API |
|---|---|
| `state` | `const SUPPORTED_SCHEMA: u32 = nopass_core::state::SCHEMA_VERSION` · `enum ReadFault { Io, Malformed, UnsupportedSchema { found: u32 } }` · `enum FileReading { Parsed(HelperStatus), Absent, Faulted(ReadFault) }` · `FileReading::parsed(&self) -> Option<&HelperStatus>` · `fn parse(bytes: &[u8]) -> Result<HelperStatus, ReadFault>` · `fn read(path: &Path) -> FileReading` |
| `probe` | `enum Probe { Passwordless, PasswordRequired }` · `enum ProbeError { Spawn, Signaled, BinaryMissing }` · `fn spec(sudo: &Path) -> CommandSpec` · `fn interpret(o: Result<SpawnOutcome, RunnerError>) -> Result<Probe, ProbeError>` · `struct ProbeCache { value: Probe, taken_at: u64 }` · `ProbeCache::usable(&self, file_observed_at: u64, now: u64) -> Option<Probe>` · `const MAX_PROBE_AGE_SECS: u64 = 90` |
| `reconcile` | `enum TrayState { Active { user: Option<String>, expiry: Option<Expiry> }, Inactive, Unknown }` · `fn merge(file: &FileReading, probe: Option<Probe>, now: u64) -> TrayState` · `enum Trigger { Startup, FileEvent, ActionCompleted, MenuOpened, Tick }` · `fn probe_required(state: &TrayState, trigger: Trigger) -> bool` |
| `outcome` | `enum Action { Enable { until: u64 }, Disable }` · `enum OutcomeKind { … one variant per §5 row, plus `UnexpiringGrant` (§5.1) }` · `enum Severity { Success, Info, Error }` · `fn classify(action: Action, status: Option<i32>, helper_present: bool) -> OutcomeKind` · `OutcomeKind::severity(&self) -> Severity` · `fn escalate(prev: OutcomeKind, probe: Probe) -> Option<OutcomeKind>` (§5.1 rule 3) |
| `format` | `fn countdown(expiry: Expiry, now: u64) -> String` · `fn tooltip(user: &str, s: &TrayState, now: u64) -> ToolTip` · `fn status_line(user: &str, s: &TrayState, now: u64) -> String` · `fn toggle_label(s: &TrayState) -> Option<&'static str>` · `fn icon_name(s: &TrayState) -> &'static str` · `fn outcome_text(k: OutcomeKind) -> (String, String)` · `struct ToolTip { title: String, body: String }` |
| `runner` | `struct CommandSpec { program: PathBuf, args: Vec<String>, env: Vec<(String, String)> }` · `struct SpawnOutcome { status: Option<i32>, stdout: Vec<u8>, stderr: Vec<u8> }` · `enum RunnerError { NonAbsoluteProgram, Spawn, Signaled }` · `trait CommandRunner: Send + Sync + 'static` · `struct SystemRunner` · `async fn run_off_reactor(runner: Arc<dyn CommandRunner>, spec: CommandSpec) -> Result<SpawnOutcome, RunnerError>` |
| `invoke` | `fn pkexec_spec(pkexec: &Path, helper: &Path, action: Action, locale: &Locale) -> CommandSpec` · `struct ActionGate` (`try_begin() -> Option<Ticket>`, at most one in-flight privileged action) |
| `watch` | `struct Watch` · `Watch::start(run_dir: &Path, uid: u32, tx: Sender<Event>) -> Result<Watch, WatchError>` · `enum WatchError { DirMissing, Io }` · `const DEBOUNCE_MS: u64 = 100` |
| `tray` | `trait TrayPort { fn render(&self, v: &ViewModel); fn reassert(&self); }` · `struct ViewModel { icon: &'static str, attention: bool, tooltip: ToolTip, status_line: String, toggle: Option<&'static str> }` · `struct KsniTray` — **the only `ksni`-aware module** |
| `notifications` | `trait NotifyPort { fn post(&self, category: Category, summary: &str, body: &str); }` · `enum Category { Action, Expiry, Environment }` · `struct FreedesktopNotifier` — **the only `notify-rust`-aware module** |
| `instance` | `async fn acquire(conn: &Connection) -> Result<Acquisition, zbus::Error>` · `enum Acquisition { Owner, AlreadyRunning }` · `async fn nudge(conn: &Connection) -> ()` (bounded, infallible by design) · `struct AppInterface` serving `org.freedesktop.Application` |
| `preflight` | `struct Preflight { session_bus, sni_host, notifications, polkit }` · `enum ServicePresence { Present, Absent }` · `enum PolkitReadiness { Ready, ActionMissing, Indeterminate(&'static str) }` · `enum StartDecision { Run(Mode), Refuse(Refusal) }` · `enum Mode { Full, NoTrayHost, NoNotifications }` · `enum Refusal { NoSessionBus, NoUserVisibleChannel }` · `fn decide(p: &Preflight) -> StartDecision` (pure) |
| `event` | `enum Event { FileChanged, Tick, MenuOpened, ToggleRequested, ActivateRequested, HostAppeared, HostVanished, ProbeFinished(Result<Probe, ProbeError>), ActionFinished(Action, Result<SpawnOutcome, RunnerError>), Quit }` |
| `app` | `struct App` — the single owner of all mutable state · `async fn run(App) -> i32` |
| `main` | `fn main() { std::process::exit(nopass::boot()) }` |

---

## 3. `tray-state-sync`: making the lie unrepresentable

### 3.1 The reader type

```rust
/// What the state FILE says — and nothing more.
///
/// There is deliberately NO `Default`, NO `is_active()`, NO
/// `unwrap_or(Inactive)`, and NO `From<FileReading> for TrayState`.
/// Absence, an I/O error, malformed bytes, and `schema != 1` are all
/// arms that carry no activity claim at all: the type has no way to
/// express "inactive because the file was missing". The ONLY route from
/// `Absent`/`Faulted` to a non-`Unknown` `TrayState` is `merge`, whose
/// signature demands a probe result.
pub enum FileReading { Parsed(HelperStatus), Absent, Faulted(ReadFault) }
```

`parse` is two-stage, and the order is a requirement: deserialize a `{ schema: u32 }` probe struct **first**, so a future schema-2 file reports `UnsupportedSchema { found: 2 }` and not `Malformed`; only then deserialize the full `HelperStatus`. A reader that reports a version mismatch as corruption sends a user to the wrong problem.

`read` maps: `ENOENT` ⇒ `Absent`; any other I/O error ⇒ `Faulted(Io)`; non-UTF-8 or bad JSON ⇒ `Faulted(Malformed)`; `schema != SUPPORTED_SCHEMA` ⇒ `Faulted(UnsupportedSchema)`.

### 3.2 The merge (exhaustive, pure, total)

| # | `FileReading` | `probe` | ⇒ `TrayState` |
|---|---|---|---|
| 1 | any | `Some(Passwordless)` | `Active { user, expiry }` where `user`/`expiry` come from the file **only if** it is `Parsed`, `active == true`, and its expiry has not passed; otherwise `Active { user: None, expiry: None }` |
| 2 | any | `Some(PasswordRequired)` | `Inactive` |
| 3 | `Parsed{active:true, expires:Some(e)}`, `!e.is_expired(now)` | `None` | `Active { user, expiry: Some(e) }` |
| 4 | `Parsed{active:true, expires:Some(e)}`, `e.is_expired(now)` | `None` | **`Unknown`** — the file contradicts itself; the helper's timer should already have rewritten it |
| 5 | `Parsed{active:true, expires:None}` | `None` | **`Unknown`** — the helper never writes this combination; treat as corrupt, never as active-forever |
| 6 | `Parsed{active:false}` | `None` | `Inactive` |
| 7 | `Absent` | `None` | **`Unknown`** |
| 8 | `Faulted(_)` (any fault, including `schema != 1`) | `None` | **`Unknown`** |

Rows 1 and 2 are RF-07's "live probe wins". Row 2 is §10.11: root deletes the rule out of band, the state file still says active, and the probe corrects it within 60 s. Rows 7 and 8 are the inherited M1 contract, and row 1's expiry clause is why `Active { expiry: None }` exists as a real value: **active, remaining time unknown** — the honest rendering when the probe confirms a grant whose state file is gone.

### 3.3 Probe freshness — "the probe wins" is not "the oldest probe wins"

A probe taken 30 s before an inotify event is not evidence about the world after that event. `merge` stays pure and total by taking `Option<Probe>`; the policy lives in one testable function:

```rust
ProbeCache::usable(&self, file_observed_at: u64, now: u64) -> Option<Probe>
// Some(v) iff  taken_at >= file_observed_at  &&  now - taken_at <= MAX_PROBE_AGE_SECS (90)
```

`MAX_PROBE_AGE_SECS = 90` is 1.5 × the 60 s tick: one missed tick is tolerated, two are not. A probe ruled unusable yields `None`, which for an `Absent`/`Faulted` file means `Unknown` — and `Unknown` always schedules a probe.

### 3.4 The four reconciliation triggers

`probe_required(state, trigger)` is a pure table:

| Trigger | Probe? |
|---|---|
| `Startup` | Always |
| `FileEvent` | Always (the cached probe is now older than the file) |
| `ActionCompleted` | Always — **an exit code is never evidence of state** (§5) |
| `MenuOpened` | Only if the cached probe is unusable per §3.3 (a menu open must not cost a 200 ms subprocess every time) |
| `Tick` (60 s) | Always |

`Unknown` additionally forces a probe on entry regardless of trigger.

---

## 4. Ports: subprocesses from an async context

### 4.1 `CommandRunner`, mirroring M1

Same shape as `nopass-helper::runner`, minus `Expect` (the tray inspects every status itself) and plus `Send + Sync + 'static` for cross-thread use. `SystemRunner` uses `std::process::Command` with `.env_clear()`, `.stdin(Stdio::null())`, `.output()`; a non-absolute `program` is rejected **before** any spawn, for the same glibc `execvp`/`confstr(_CS_PATH)` reason `nopass-helper` documents. No shell, ever. `ScriptedRunner` (in `#[cfg(test)]`) asserts `CommandSpec` equality field-by-field and panics on `Drop` if its script is unexhausted.

### 4.2 Off-reactor execution — the polkit dialog waits on a human

```rust
pub async fn run_off_reactor(runner: Arc<dyn CommandRunner>, spec: CommandSpec)
    -> Result<SpawnOutcome, RunnerError>
{
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || { let _ = tx.send_blocking(runner.run(&spec)); });
    rx.recv().await.unwrap_or(Err(RunnerError::Spawn))
}
```

One dedicated OS thread per invocation, joined implicitly when it ends; the result crosses back over an async channel the reactor is already polling. **No blocking call ever executes on the reactor thread.** A shared worker pool is rejected: a `pkexec` waiting on a human for 60 s would occupy a pool slot the 60 s probe also needs. Thread cost is bounded by design — one per user action plus one per 60 s tick, each short-lived except the dialog.

`notify-rust`'s **async** API is used from the reactor; its blocking wrapper is never called from inside the executor. `notify`'s inotify thread is bridged to the reactor through the same `async_channel`, never a blocking `recv`.

### 4.3 Exact argv

**Privileged action** — `invoke::pkexec_spec`:

```
/usr/bin/pkexec /usr/libexec/nopass-helper enable --until <now + 3600>
/usr/bin/pkexec /usr/libexec/nopass-helper disable
```

No `--user`, no `--disable-internal-agent`, no shell. `pkexec` sets `PKEXEC_UID` to the calling uid itself, which is precisely the context the helper's `uid::resolve` demands.

Environment: `env_clear()`, then pass through **only** `LANG`, `LC_ALL`, `LC_MESSAGES` when present in the tray's own environment. This deliberately departs from M1's `LANG=C` forcing, and the rationale is the inversion of M1's: the helper forces `C` because it *parses* subprocess output, while the tray parses nothing from `pkexec` but an integer status — the only consumer of locale on this path is the human-facing authentication dialog, and forcing `C` would guarantee an English dialog for a Spanish user. The privileged child re-clears its own environment before every subprocess it runs (M1 §3), so a locale value cannot reach `visudo` or `sudo` parsing.

**Ground-truth probe** — `probe::spec`:

```
/usr/bin/sudo -k -n true          env: LANG=C, LC_ALL=C
```

`-k` is load-bearing: without it a user who ran `sudo` two minutes ago gets a **false positive** from the timestamp cache rather than from NOPASSWD. Per `sudo(8)`, `-k` *with* a command ignores the cached credentials for that invocation and does not update them — it does not invalidate the user's stored timestamp, so the probe does not repeatedly destroy the user's sudo session. That claim is a desktop-lane checklist item (§9 lane C), not an assumption: it needs a typed password to observe.

Binary resolution reuses M1's ordered-absolute-candidate discipline: `pkexec` → `/usr/bin/pkexec`, `/bin/pkexec`; `sudo` → `/usr/bin/sudo`, `/bin/sudo`. First existing wins, resolved lazily, no `PATH` lookup ever.

### 4.4 One privileged action at a time

`ActionGate::try_begin()` returns `None` while a `pkexec` is in flight. The toggle menu item goes insensitive for the duration and the SNI `Activate` (left click) is ignored. Without this, a user clicking twice stacks two polkit dialogs and two competing helper transactions that race on M1's `flock` and produce a gratuitous exit 15.

---

## 5. `tray-privileged-invocation`: the exit-code → outcome table

**Rule above the table: an exit code is never evidence of state.** Exit 0 means "the request was accepted", not "a grant exists". Every terminal outcome — success included — is followed by `Trigger::ActionCompleted`, which always probes. The table produces *messages*, never `TrayState`.

| Code | Source | Meaning | `OutcomeKind` | User-facing outcome | Sev |
|---|---|---|---|---|---|
| 0 | helper | accepted | `Granted` / `Revoked` | "Passwordless sudo enabled until HH:MM" / "Passwordless sudo disabled" | Success |
| 1 | helper | internal error or a required binary is missing | `InternalError` | "NoPass could not complete the change: a required system program is missing or the helper failed internally. Nothing was changed." | Error |
| 2 | helper | clap rejected the argv the tray built | `VersionSkew` | "The installed helper does not understand this request — the tray and helper versions do not match. Reinstall NoPass." | Error |
| 10 | helper | invocation-context violation | `ContextViolation` | "The authorization did not carry your user identity. Do not run NoPass as root." | Error |
| 11 | helper | uid rejected (unknown / root / outside `[UID_MIN, UID_MAX]`) | `UidRejected` | "This account is not eligible for passwordless sudo (a system account, or outside the normal user id range)." | Error |
| 12 | helper | not a sudoer | `NotSudoer` | "Your user is not allowed to use sudo, so passwordless sudo cannot be granted." | Error |
| 13 | helper | duration/until invalid | `BadDuration` | "The requested expiry time was rejected. Check that the system clock is correct." — M2 hard-codes 1 h, so a 13 can only mean the tray's clock and the helper's disagree | Error |
| 14 | helper | `visudo` rejected the generated rule | `VisudoRejected` | "The generated sudo rule was rejected as invalid. Nothing was changed. Please report this." | Error |
| 15 | helper | lock busy | `LockBusy` | "Another NoPass operation is already running. Try again in a moment." | Info |
| 16 | helper | filesystem / atomic-write failure | `FsFailure` | "NoPass could not write the rule file. Nothing was changed." | Error |
| 17 | helper | `enable` with an `At` expiry **only**: `systemd-run` could not schedule the expiry timer, and the helper attempted to roll the rule back | `TimerUnscheduled` | "The expiry timer could not be scheduled, so the timed grant was withdrawn. NoPass is re-checking whether any grant is currently active." — deliberately claims **neither** outcome; see §5.1 | Error |
| 126 | `pkexec` | the user dismissed the authentication dialog | `Cancelled` | "Authorization cancelled. Nothing was changed." | Info |
| 127 | `pkexec` | not authorized / authorization unobtainable / **exec failed** | `NotAuthorized` **or** `HelperMissing` | Disambiguated below | Error |
| — | tray | spawn failed (`pkexec` absent) | `SpawnFailed` | "`pkexec` is not installed, so NoPass cannot request authorization. Install `polkit`." | Error |
| — | tray | `status: None` (killed by a signal) | `Interrupted` | "The authorization was interrupted. NoPass will re-check the current state." | Error |

### 5.1 Exit 17 — one code, two worlds, and why the tray must not pick one

Exit 17 is `HelperError::TimerFailed { rolled_back }`: `systemd-run` could not schedule the expiry timer. It is reachable **only** from `enable` carrying an `At` expiry — `Never` and `Reboot` never reach the scheduler — and the helper then attempts to unlink the rule it just wrote. Two materially different worlds sit behind that single code:

| World | Reality | What the user must be told |
|---|---|---|
| Rollback confirmed | No rule on disk, no timer. Nothing was granted | "The timed grant could not be scheduled. Nothing was changed." |
| Rollback **not** confirmed | The rule may still be on disk **with no timer scheduled** — that is a **permanent** grant the user was told had failed, after asking for a time-boxed one | "Passwordless sudo may be active with **no expiry**. Disable it when you are done." |

**The tray cannot distinguish them from the exit code alone**, and today's helper cannot help: `ops.rs` hardcodes `rolled_back: true` while `rollback_rule` discards the unlink result, so the flag is not evidence even where it is visible. It is also invisible to the tray — the flag never crosses the process boundary, and step 16 (the state-file write) is never reached on this path, so the observability channel carries nothing new either.

**Therefore the safe reading of a bare 17 is "state unknown, reconcile now"** — precisely the discipline the missing-state-file contract already imposes, applied to an exit code instead of an absent file. Concretely:

1. The notification for `TimerUnscheduled` asserts only what is certain: the timer could not be scheduled, and NoPass is re-checking. It never says "nothing was changed" and never says "a grant is live", because neither is known.
2. `Trigger::ActionCompleted` forces a probe, as it does for every outcome (D5). The probe is ground truth and resolves the ambiguity the exit code cannot.
3. **Escalation rule**: if the probe that follows a `TimerUnscheduled` returns `Passwordless`, the tray posts a second, distinct notification — `OutcomeKind::UnexpiringGrant`, the only `Severity::Error` message in M2 that reports a *live* grant: "Passwordless sudo is active with no expiry, because its timer could not be scheduled. Use Disable when you are done." The toggle in that state already reads "Disable passwordless sudo", so the remedy is one click away. The tray never auto-disables: it takes no privileged action the user did not ask for.
4. The rendered state needs no special case. The state file was not rewritten, so `merge` row 1 (probe `Passwordless`, file not corroborating) yields `Active { user: None, expiry: None }` — the plain unlocked icon and "remaining time unknown". That is already the honest rendering of an unexpiring grant.

This is the design's answer to a class, not to one bug: **an exit code that covers two states is read as neither, and resolved by the probe.** It stays correct whether or not M1's `rolled_back` flag is ever fixed.

**Exit 127 disambiguation.** `pkexec` returns 127 both for "could not authorize" and for "could not exec the target". The tray separates them without guessing: on 127 it calls `access(HELPER_PATH, X_OK)` — unprivileged, cheap, and only on failure.

- Helper missing/not executable ⇒ `HelperMissing`: "NoPass is not completely installed: the privileged helper is missing at `/usr/libexec/nopass-helper`."
- Helper present ⇒ `NotAuthorized`: "Authorization failed. There may be no polkit authentication agent running in this session, or your user is not permitted to perform this action."

Because **no shell is ever involved**, 126/127 can only originate from `pkexec` itself and never from a shell's `command not found` convention — which is exactly why M1's "no `sh -c`" rule is what keeps this table unambiguous.

**Distinctness is a test, not a promise.** One table-driven test renders every `OutcomeKind` and asserts the set of `(summary, body)` pairs has no duplicates — the same shape as M1's `every_failure_cause_maps_to_a_distinct_code`.

---

## 6. Event loop and sequences

One async task owns every piece of mutable state. Adapters own no state and only send `Event`s; the loop is the sole writer and the sole caller of `TrayPort::render`. There is no `Mutex<TrayState>` anywhere, because there is only one writer.

```
  ksni item ─Activate/MenuOpen──┐
  notify(inotify) ─thread──────┐│
  Timer::interval(60s) ────────┼┼──▶ async_channel<Event> ──▶ app::run (single owner)
  zbus Activate handler ───────┘│                                 │
  run_off_reactor results ──────┘                                 │
                                                                  ├─▶ merge(file, probe, now) ─▶ TrayState
                                                                  ├─▶ format::* ─▶ ViewModel ─▶ TrayPort::render
                                                                  └─▶ NotifyPort::post
```

### 6.1 Startup (NFR: icon visible < 1 s)

```
main
 │ 1 uid = getuid();  layout = Layout::system();  state_path = layout.state_path(uid)
 │ 2 connect session bus ─────────────────── fail ⇒ stderr + exit 3
 │ 3 serve org.freedesktop.Application at /com/enfoquestic/nopass
 │ 4 request_name("com.enfoquestic.nopass")
 │      NameTaken ⇒ nudge (§6.4) ⇒ exit 0
 │      other error ⇒ stderr + exit 5
 │ 5 preflight: NameHasOwner(StatusNotifierWatcher), NameHasOwner(Notifications),
 │              polkit ladder (§0 G3) on the SYSTEM bus
 │ 6 decide(): neither host nor notifier ⇒ stderr + exit 4
 │ 7 RENDER Unknown AND register the SNI item  ← before any probe; this is the <1s budget
 │ 8 subscribe NameOwnerChanged (late host / late notifier)
 │ 9 start inotify watch on /run/nopass  (missing ⇒ Unavailable, retried each tick)
 │10 spawn 60 s Timer::interval
 │11 read state file + spawn first probe  ──▶ first real render
```

Step 7 before step 11 is the whole reason `Unknown` is a first-class state: the honest first frame is "I do not know yet", and the NFR forbids waiting for a subprocess before drawing.

### 6.2 Toggle (the privileged flow)

```
user click / menu / keyboard
      │ ActionGate::try_begin() ─ None ⇒ ignore (dialog already open)
      │ toggle item ⇒ insensitive
      ▼
  app ──run_off_reactor──▶ [dedicated thread] ──▶ /usr/bin/pkexec /usr/libexec/nopass-helper enable --until N
      │                                                     │
      │  reactor stays live: icon, menu, inotify, tick       ├─▶ polkitd ─▶ session agent ─▶ [human types password]
      │                                                     └─▶ helper (root) ─▶ /etc/sudoers.d + /run/nopass/<uid>.state
      ▼
  Event::ActionFinished(action, result)
      │ outcome::classify(action, status, helper_present)  ─▶ notification (RF-08)
      │ Trigger::ActionCompleted ⇒ ALWAYS probe          ─▶ state comes from the probe, never from the code
      ▼
  merge ⇒ render
```

Two independent paths usually deliver the same truth here: the helper's state-file write fires inotify, and the forced probe returns. Whichever lands first renders; the second confirms. If the state-file write failed (M1 `enable` §4.1 step 16 returns 0 anyway), only the probe lands — and row 1 of the merge table renders `Active { expiry: None }`, never `Inactive`. That is the inherited contract, honoured structurally.

### 6.3 Expiry by timer (§10.8)

```
systemd nopass-expire-<uid>.timer ─▶ helper expire --uid ─▶ unlink rule, rewrite state file
                                                                   │
      inotify (<100 ms debounce) ─▶ Event::FileChanged ─▶ probe ─▶ merge ⇒ Inactive
                                                                   └─▶ NotifyPort::post(Expiry, "…has expired")
```

The 60 s countdown granularity (D2, settled) does not delay this: the transition is inotify-driven and lands inside the 1 s budget. Coarse countdown, prompt transition.

### 6.4 Second instance (RF-10, D3)

```
instance #2: request_name ⇒ NameTaken
             │ Activate() on com.enfoquestic.nopass, 2 s timeout
             │ ignore success, failure, and timeout alike
             ▼ exit 0   (always — RF-10 is satisfied whatever the nudge did)

instance #1: Event::ActivateRequested
             │ rate limit: at most one nudge per 5 s (§ threat matrix)
             │ TrayPort::reassert()      (re-register the SNI item)
             └ NotifyPort::post(Environment, "NoPass is already running — <status line>")
```

---

## 7. Presentation

### 7.1 Icons

| `TrayState` | `IconName` | SNI `Status` |
|---|---|---|
| `Inactive` | `nopass-locked` | `Active` |
| `Active { expiry: Some(At{..}) }` | `nopass-unlocked-timed` | `Active` |
| `Active { expiry: Some(Never\|Reboot) }` or `Active { expiry: None }` | `nopass-unlocked` | `Active` |
| `Unknown` | **`dialog-question-symbolic`** (stock freedesktop name, no new asset) | `NeedsAttention` |

`Unknown` deliberately does **not** reuse `nopass-locked`. A padlock-closed glyph during an unresolved state is the exact visual lie this milestone exists to prevent, even for the sub-second window before the first probe returns. A stock question icon is present in every mainstream theme, costs no asset, and cannot be mistaken for "inactive".

`format::icon_name` resolves the `-symbolic` variant by default — panel tray icons are monochrome by convention on both GNOME and KDE, and symbolic icons recolor correctly against light and dark panels. `NOPASS_ICON_STYLE=color|symbolic` overrides it, so the desktop lane compares both without a rebuild. Icon lookup is by **theme name**; assets must be installed into the icon theme, which is M4 packaging — the desktop lane carries a documented developer install step into `~/.local/share/icons/hicolor/scalable/apps/`. Embedding pixmaps is rejected: it inflates the binary against the < 5 MB NFR and defeats theme and symbolic switching, which is the point of shipping six SVGs.

### 7.2 Countdown, tooltip, menu

`format::countdown` **floors** to whole minutes and renders "less than a minute" below 60 s. Flooring, not rounding: rounding up claims time the user does not have, and this is a security countdown. `Never` ⇒ "no expiry"; `Reboot` ⇒ "until reboot"; a past `At` ⇒ "expired"; `Active { expiry: None }` ⇒ "remaining time unknown".

The minimal menu (deliberate subset of RF-03, per the proposal):

| Item | Behaviour |
|---|---|
| `Status: <user> — <state> (<remaining>)` | Insensitive label. The only keyboard-reachable state surface; never renders a state the tooltip does not |
| toggle | `format::toggle_label(state)`: `Inactive` ⇒ "Enable passwordless sudo"; `Active` ⇒ "Disable passwordless sudo"; **`Unknown` ⇒ `None`** |
| `Quit` | Exit 0 |

`toggle_label` returning `None` makes the item insensitive and labelled "Checking…". This is a type-level consequence, not a UI preference: the toggle's label is *state-dependent*, so in `Unknown` it is underivable — the code cannot know whether to offer Enable or Disable. Offering a mislabeled privileged action is worse than offering none for a window bounded by the 2 s probe timeout. Rejected alternative: defaulting to "Enable" while unknown, which would show "Enable" to a user who already has a live grant — the same class of lie as the locked icon.

Every user-facing string lives in `format`. M3 replaces that module's body with `rust-i18n` without touching a single call site.

### 7.3 Notifications (RF-08)

One `NotificationHandle` is retained **per `Category`** and updated in place rather than posting a new notification each time, so a flurry of events cannot stack eight toasts. Urgency is `Normal` for every category — never `Critical`, which on several daemons produces an undismissable banner, hostile behaviour for a tray utility. Bodies are composed only from `format::outcome_text` constants plus the username and countdown; **raw subprocess stderr is never interpolated into a notification**, extending M1's audit-record rule to the user-facing surface. No markup.

---

## 8. Preflight, degraded modes, and the tray's own exit codes

`preflight::decide` is pure and table-tested:

| `sni_host` | `notifications` | Decision |
|---|---|---|
| Present | Present | `Run(Full)` |
| Absent | Present | `Run(NoTrayHost)` — claim the name, post one notification explaining that no tray host was found (with the GNOME AppIndicator hint), subscribe `NameOwnerChanged`, register the item the moment a host appears |
| Present | Absent | `Run(NoNotifications)` — outcomes degrade to icon, tooltip, and stderr; RF-08 documented as unmet here |
| Absent | Absent | `Refuse(NoUserVisibleChannel)` |

No session bus at all ⇒ `Refuse(NoSessionBus)`. `PolkitReadiness` never enters `decide` — it only constrains the toggle, per §0 G3.

Tray exit codes, each distinct from RF-10's 0:

| Exit | Meaning |
|---|---|
| 0 | Normal quit; **or** a second instance after its nudge (RF-10) |
| 1 | Unexpected internal failure |
| 3 | No session bus |
| 4 | Session bus present, but neither a tray host nor a notification service — no possible user-visible channel |
| 5 | `request_name` failed for a reason other than `NameTaken` |

---

## 9. Testing strategy — three lanes

### Lane A — `cargo test --workspace`, unprivileged, no bus, no desktop

| Area | Cases |
|---|---|
| `state::parse` | valid schema-1; `schema: 0`; `schema: 2` ⇒ `UnsupportedSchema{found:2}` **not** `Malformed`; truncated JSON; empty file; non-UTF-8 bytes; `active:true` with `expires:null` |
| `state::read` via `Layout::under(TempDir)` | file absent ⇒ `Absent`; unreadable dir ⇒ `Faulted(Io)`; rename-in-place while reading ⇒ never a torn parse |
| `reconcile::merge` | **all 8 rows exhaustively**, both probe values × every `FileReading` variant × expired/unexpired `now`. Named test: missing file + probe `Passwordless` ⇒ `Active{expiry:None}` — the milestone's headline risk |
| `ProbeCache::usable` | probe older than the file ⇒ `None`; age 89/90/91 s boundaries |
| `probe_required` | full `Trigger` × `TrayState` table |
| `outcome::classify` | one case per 0/1/2/10–17/126/127/spawn/signal; 127 × `helper_present` both ways; **uniqueness assertion over all rendered `(summary, body)` pairs** |
| `outcome::escalate` (§5.1) | `TimerUnscheduled` + `Passwordless` ⇒ `Some(UnexpiringGrant)`; `TimerUnscheduled` + `PasswordRequired` ⇒ `None`; no other `OutcomeKind` escalates under either probe value. Assert the `TimerUnscheduled` text claims neither "nothing was changed" nor an active grant |
| `format` | countdown at 0/1/59/60/61/3599/3600/28800 s and past-epoch; `Never`/`Reboot`/`None`; `toggle_label(Unknown) == None`; `icon_name` full table incl. both styles |
| argv | `pkexec_spec` and `probe::spec` exact `CommandSpec` equality via `ScriptedRunner`; non-absolute program rejected before spawn; env pass-through list is exactly `LANG`/`LC_ALL`/`LC_MESSAGES` and nothing else |
| `preflight::decide` | the 4-row table + `NoSessionBus`; `Indeterminate` polkit never changes the decision |
| `watch` | **real inotify against a `TempDir`** (no bus, no desktop): create/rename/delete of `<uid>.state` each yield exactly one debounced event; an unrelated sibling file yields none; missing directory ⇒ `WatchError::DirMissing` |
| `ActionGate` | second `try_begin` while in flight ⇒ `None` |
| icon assets | dependency-free assertions over `data/icons/*.svg` — all six present, each starts with an `<svg` root, has a `viewBox`, contains no `<script`, and no `http`-scheme external reference. Same pattern as M1's `data_artifacts.rs`, no XML crate |

### Lane B — headless `dbus-run-session` (gated by `NOPASS_DBUS_TESTS=1`; skips with a clear message when `dbus-run-session` is absent)

Container recipe `tests/containers/Containerfile.dbus` alongside M1's images. This lane proves the D-Bus protocol, never rendering.

- Name ownership; a second process gets `NameTaken` and **exits 0**; the first receives `Activate` and emits exactly one nudge (§10.12 at the protocol level).
- Nudge rate limiting: ten `Activate` calls in one second produce one notification.
- SNI registration against a **fake `org.kde.StatusNotifierWatcher`**: `RegisterStatusNotifierItem` is called; `IconName`, `Status`, `ToolTip`, `Menu` properties read back match the `ViewModel` for each `TrayState`, including `dialog-question-symbolic` + `NeedsAttention` for `Unknown`.
- Host absent at startup ⇒ degraded start; host appears later ⇒ `NameOwnerChanged` ⇒ late registration.
- Notification payloads against a **fake `org.freedesktop.Notifications`**: summary/body per `OutcomeKind`, one retained id per category (an update, not a new id), never `Critical`.
- Neither fake service present ⇒ exit 4. No session bus ⇒ exit 3.

### Lane C — real desktop session, manual, recorded in the verify report

Checklist in `tests/manual/README.md`; the verify report records the desktop environment, its version, and the observed result per item — never an inference.

- §10.1 a panel actually draws the icon and its tooltip; symbolic vs colour variant legibility on light and dark themes.
- §10.2/§10.3 the polkit dialog appears, authenticates, and `auth_admin_keep` suppresses the second prompt; `sudo -kn true` returns 0 within 1 s of confirmation.
- Notifications visibly rendered by the session's own daemon.
- Keyboard navigation of the menu (accessibility NFR).
- The double-clicked-launcher path end to end (§10.12 as a human sees it).
- `sudo -kn true` does **not** destroy the user's cached sudo credentials (§4.3) — requires a typed password.
- `pkaction --action-id com.enfoquestic.nopass.manage` run as the unprivileged user — converts §0 G3's prior into an observation.
- Idle RSS < 30 MB and idle CPU ≈ 0 over a continuous run of ≥ 1 h with no wakeup source other than the 60 s tick.

### Provable nowhere automatically

Polkit authentication itself. It requires a registered agent and a human typing a password; there is no unprivileged, headless substitute. It stays a human step. Likewise, that a *specific* panel renders a *legible* icon is a judgement only a person looking at a screen can make — Lane B can prove the correct name was published, never that it looked right.

---

## Gate Results — executed by the orchestrator

The design phase had no shell, so its three "prove, not assume" checks shipped as unexecuted
slice-1 gates. They were run before `sdd-tasks`, in a scratch workspace outside the repository.
All three passed, and one produced a fact the design did not anticipate.

**G1 — the `async-io` combination compiles and stays single-reactor. PASS.**
A scratch crate with `ksni = { version = "0.3", default-features = false, features = ["async-io"] }`,
`zbus = "5"`, `notify-rust = "~4.17"` and `notify = "8"` resolves, and `cargo build` exits 0.
Against the design's sharper assertion, not the proposal's weaker one:

| Assertion | Observed |
|---|---|
| exactly one `async-io` major in the lockfile | 1 |
| no `tokio` anywhere in the tree | 0 occurrences |

Fallback B (all-tokio, `current_thread`) is not needed. Slice 1 keeps its planned shape.

**G2 — resolver 3 selects `notify-rust` 4.17 unaided. PASS, and the tilde pin still stands.**
With a bare `notify-rust = "4"` requirement the resolver picked 4.17.0 on its own, declining
4.18.0 because it requires rustc 1.89 above the workspace's 1.85. So MSRV-aware resolution does
work here. Keep the `~4.17` pin anyway, for the reason the design already gave: MSRV-aware
resolution is a preference that a packager's cargo config can silently override, and every way it
can be defeated surfaces as a compile error in M4 rather than here.

**G3 — polkit action enumeration works unprivileged. PASS, by proxy.**
`dbus-send --system` could not be used on this host: a Homebrew `dbus` earlier in `PATH` resolves
the system bus socket to a path that does not exist. That is an environment artifact, not a polkit
answer. `pkaction(1)`, which calls `EnumerateActions` and nothing else, ran as uid 1000 and listed
actions. The startup installation-completeness check can rely on it. `PolkitReadiness` keeps its
three-valued shape regardless, since `Indeterminate` never disables the toggle.

**Unanticipated finding: the MSRV pin also constrains `zbus`.**
`zbus` 5.19.0, the current release and the version the exploration and proposal both name,
requires rustc 1.87. Under the workspace's 1.85 pin the resolver selects **5.13.2** instead, along
with older `zvariant`, `zbus_names` and `zbus_macros`. The combination compiles and satisfies both
reactor assertions, so nothing in the design changes. But the design, the tasks and any future
dependency audit must refer to zbus 5.13.x, not 5.19.x, and a later crate that requires zbus 5.19
would force the same MSRV decision the design already took for `notify-rust`. Record it rather
than rediscover it.


## Architecture Decisions

### D1 — `async-io`, one reactor, with a pre-agreed fallback and a sharper assertion

**Choice**: `ksni` `default-features = false, features = ["async-io"]`, `zbus` at its own defaults, one executor. Carried from the proposal, with two refinements. First, the invariant to assert is **one `async-io` major version and zero tokio**, not merely "no zbus tokio feature" — two `async-io` majors (from `ksni` and `notify-rust` pulling different `zbus` majors) produce two reactor threads with no tokio anywhere, and the proposal's assertion would pass. Second, the fallback is chosen now, not during apply: all-tokio with `flavor = "current_thread"`.
**Alternatives considered**: tokio by default (`ksni`'s own default, but pairing it with crates that may still pull zbus's `async-io` is the documented panic configuration); `ksni`'s `blocking` feature on a dedicated thread (a second D-Bus connection and a second ticking context for no gain).
**Rationale**: a single epoll loop is the cheapest configuration against the idle-CPU and < 30 MB RSS NFRs, and removes the "blocking zbus call inside a foreign async context" panic class entirely. The fallback is not a crisis: D1's RSS argument was aimed at tokio's *multi-threaded* scheduler, and the current-thread flavour is a single thread, so taking it costs a dependency tree and not an NFR. **Unverified here — G1 gates it.**

### D2 — Pin `notify-rust = "~4.17"` explicitly, rather than trusting resolver 3

**Choice**: a tilde requirement, unconditionally.
**Alternatives considered**: `"4"` plus reliance on resolver 3's MSRV-aware selection, which explore's evidence says *should* pick 4.17.0 from `rust-version = "1.85"`.
**Rationale**: MSRV-aware resolution is a **preference**, not a constraint. It is silently defeated by `resolver.incompatible-rust-versions = "allow"` in any user or CI `.cargo/config.toml`, by `cargo update --precise`, by `--ignore-rust-version`, and by any third crate that requires `>= 4.18`. Every one of those defeats surfaces as a compile error under someone else's toolchain — a distro packager's, in M4 — rather than here. `~4.17` converts a silent preference into a hard resolution-time error in this repository, for the price of one character. The resolver may well behave; the design simply refuses to depend on it. `cargo tree -i notify-rust` showing `4.17.x` remains a slice-1 gate.

### D3 — Absence is a variant of the reader's type, not a flag on a status

**Choice**: `FileReading::{Parsed, Absent, Faulted}` with no `Default`, no `is_active()`, no `From<FileReading> for TrayState`, and a single `parsed()` accessor.
**Alternatives considered**: (a) `Result<HelperStatus, ReadError>` with callers deciding; (b) `Option<HelperStatus>` plus a `bool unknown` flag; (c) returning a synthesized `HelperStatus { active: false }` on absence.
**Rationale**: (c) is the exact bug M1 documented in advance — `enable` returns 0 when only the state-file write fails, so a synthesized inactive status tells a user with live passwordless sudo that they have none. (b) makes the wrong thing merely inconvenient: `status.active` still compiles and still reads `false`. (a) invites `unwrap_or_default()`. Only a type with no inactive arm reachable from absence makes the failure a compile error rather than a code review. The guarantee is completed by `merge`'s signature: producing `Inactive` from `Absent` requires passing a `Probe`, which a caller cannot fabricate without running one.

### D4 — The reconciliation loop is a single-owner actor, not shared state behind a lock

**Choice**: one async task owns every mutable field; adapters send `Event`s and own nothing; the loop is the only caller of `TrayPort::render`.
**Alternatives considered**: `Arc<Mutex<TrayState>>` shared with the `ksni` handler and the inotify callback.
**Rationale**: with five asynchronous event sources (inotify, tick, menu, action completion, D-Bus `Activate`), shared mutable state means interleaving, and the interleaving that matters is precisely the one that renders a stale `Inactive` over a fresh `Active`. Serialising every state transition through one channel makes ordering total and observable, makes the whole loop testable by feeding it a scripted `Vec<Event>`, and removes lock-ordering from a program whose failure mode is a security-relevant lie. It also removes any temptation to render from an adapter callback.

### D5 — A completed privileged action never sets state; it only schedules a probe

**Choice**: `outcome::classify` returns a message kind. `TrayState` is produced exclusively by `merge(file, probe, now)`.
**Alternatives considered**: on exit 0 from `enable`, optimistically set `Active` with the requested expiry.
**Rationale**: the optimistic path is wrong in both directions, and exit 17 proves it is wrong in *both directions at once*. Exit 0 from `enable` does **not** guarantee an observable grant — M1 §4.1 step 16 returns 0 when the state-file write failed. Exit 17 does **not** guarantee the absence of one: the rollback is best-effort and its success is never observed (§5.1), so a code the tray would read as "failed" can accompany a live, unexpiring grant. A tray that infers state from a code renders "active until 15:00" after a 0-with-no-grant, and "nothing changed" after a 17-with-a-permanent-grant — the second being the more dangerous of the two. The one-way rule costs one extra `sudo -kn true` per user action, which is already a funded RF-07 trigger, and it is what lets §5.1's escalation exist at all.

### D6 — `Unknown` is rendered with a stock question icon, and the toggle goes insensitive

**Choice**: `dialog-question-symbolic` + `Status::NeedsAttention`; `toggle_label(Unknown) == None`.
**Alternatives considered**: (a) render `Unknown` as `nopass-locked`; (b) add a fourth asset; (c) keep the toggle sensitive, defaulting its label to "Enable".
**Rationale**: (a) is the precise visual lie the milestone exists to prevent, even transiently. (b) buys a bespoke asset the PRD never budgeted for a state that is bounded by a 2 s probe. (c) would show "Enable" to a user who already holds a live grant, and clicking it would silently re-grant with a fresh hour — reintroducing at the action layer the lie removed at the display layer. A state whose label cannot be derived must not be actionable.

### D7 — Countdown floors to the minute

**Choice**: integer-floor minutes; "less than a minute" below 60 s.
**Alternatives considered**: rounding to nearest; per-second display.
**Rationale**: per-second is D2's settled rejection (~28,800 wakeups and property updates per 8 h grant, animating a tooltip nobody is hovering, against an NFR that funds no such timer). Between floor and round, floor is the only safe direction for a security countdown: rounding up advertises time the user does not have. Worst-case staleness equals the 60 s reconciliation period, which equals the display granularity, so the user cannot observe the difference.

### D8 — Keep the `notify` crate for inotify, with a named trigger to revisit

**Choice**: `notify` 8 with its own thread bridged to the reactor by `async_channel`, as the proposal settled. **Watch the directory `/run/nopass/`, never the file** — the helper replaces the state file by `rename`, so a file watch follows a dead inode and goes permanently silent after the first update. Events are filtered to `<uid>.state` and debounced 100 ms.
**Alternatives considered**: `nix::sys::inotify` (already a workspace dependency) registered directly with `async_io::Async`, which would eliminate both the extra thread and the `notify` dependency and put the watch on the same epoll loop.
**Rationale**: the `nix` route is very likely better on every NFR axis, and this executor could not compile it. Overriding a settled proposal decision in favour of an alternative that was not built is exactly the assumption class this milestone is eliminating. It is recorded with a concrete trigger instead: revisit if the G1 tree assertion shows `notify` dragging in a thread pool, or if the Lane C idle-RSS measurement misses the 30 MB budget.

### D9 — Locale is passed through to `pkexec`, in deliberate contrast to M1

**Choice**: `env_clear()`, then pass only `LANG`/`LC_ALL`/`LC_MESSAGES` when present. `LANG=C`/`LC_ALL=C` is still forced on the `sudo` probe.
**Alternatives considered**: force `C` everywhere for consistency with M1; inherit the full environment.
**Rationale**: M1 forces `C` because it *parses* subprocess output and needs it stable. The tray parses nothing from `pkexec` but an integer status; the only locale consumer on that path is a human-facing authentication dialog, so forcing `C` would guarantee an English prompt for a Spanish user and buy nothing. The privileged child re-clears its own environment before every subprocess it spawns, so a locale value cannot reach `visudo` or `sudo` parsing. The probe keeps `C` because its stderr is logged.

---

## Data Flow

```
                 /etc/sudoers.d/90-nopass-<uid>      0750 root:root — INVISIBLE to the tray
                                 ▲
                                 │ (helper only)
  ┌── crates/nopass (unprivileged, one reactor) ─────────────────────────────┐
  │                                                                          │
  │  watch(inotify /run/nopass) ──┐                                          │
  │  Timer::interval(60 s) ───────┤                                          │
  │  ksni menu / Activate ────────┼─▶ Event ─▶ app (single owner)            │
  │  zbus Application.Activate ───┤              │                            │
  │  run_off_reactor results ─────┘              │                            │
  │                                              ├─ state::read ◀── /run/nopass/<uid>.state (0644, root-written)
  │                                              ├─ probe ─▶ [thread] ─▶ /usr/bin/sudo -k -n true
  │                                              ├─ merge(file, probe, now) ─▶ TrayState
  │                                              ├─ format ─▶ ViewModel ─▶ TrayPort (ksni) ─▶ SNI host
  │                                              └─ outcome ─▶ NotifyPort ─▶ org.freedesktop.Notifications
  │                                                        │
  │  invoke ─▶ [dedicated thread] ─▶ /usr/bin/pkexec ──────┘
  └──────────────────────────────────┬───────────────────────────────────────┘
                                     ▼
                      polkitd ─▶ session agent ─▶ [human]
                                     ▼
                nopass-helper (root) ─▶ rule + state file + journald
```

Dependency direction is one-way: `nopass → nopass-core`. Nothing in M2 links `nopass-helper`.

---

## File Changes

| File | Action | Description |
|---|---|---|
| `Cargo.toml` | Modify | Add `crates/nopass` member; add `ksni`, `zbus`, `notify-rust`, `notify`, `async-io`, `async-channel`, `futures-lite` to `[workspace.dependencies]` |
| `Cargo.lock` | Modify | Committed resolution; the `notify-rust` 4.17 and single-`async-io` evidence |
| `crates/nopass/Cargo.toml` | Create | Tray crate manifest |
| `crates/nopass/src/main.rs` | Create | `#![forbid(unsafe_code)]`, `exit(boot())` |
| `crates/nopass/src/lib.rs` | Create | Module declarations; exists so `tests/` can reach them |
| `crates/nopass/src/state.rs` | Create | `FileReading`, `ReadFault`, two-stage `parse`, `read` |
| `crates/nopass/src/probe.rs` | Create | `sudo -k -n true` spec, interpretation, `ProbeCache` freshness |
| `crates/nopass/src/reconcile.rs` | Create | `TrayState`, the 8-row `merge`, `probe_required` |
| `crates/nopass/src/outcome.rs` | Create | `OutcomeKind`, `classify`, 127 disambiguation |
| `crates/nopass/src/format.rs` | Create | Every user-facing string; the M3 i18n seam |
| `crates/nopass/src/runner.rs` | Create | `CommandRunner` port, `SystemRunner`, `run_off_reactor`, `ScriptedRunner` (cfg-test) |
| `crates/nopass/src/invoke.rs` | Create | `pkexec` argv construction, `ActionGate` |
| `crates/nopass/src/watch.rs` | Create | Directory-level inotify adapter + debounce |
| `crates/nopass/src/tray.rs` | Create | `TrayPort`, `ViewModel`, the only `ksni`-aware module |
| `crates/nopass/src/notifications.rs` | Create | `NotifyPort`, per-category handle reuse; the only `notify-rust`-aware module |
| `crates/nopass/src/instance.rs` | Create | Name ownership, `org.freedesktop.Application`, nudge + rate limit |
| `crates/nopass/src/preflight.rs` | Create | Service probes, polkit ladder, pure `decide` |
| `crates/nopass/src/event.rs` | Create | The `Event` enum and channel types |
| `crates/nopass/src/app.rs` | Create | Single-owner event loop |
| `crates/nopass/tests/state_tempdir.rs` | Create | `Layout::under(TempDir)` reader tests |
| `crates/nopass/tests/watch_inotify.rs` | Create | Real inotify against a `TempDir` |
| `crates/nopass/tests/icon_assets.rs` | Create | Dependency-free SVG assertions |
| `crates/nopass/tests/dbus_session.rs` | Create | Lane B, gated by `NOPASS_DBUS_TESTS=1` |
| `data/icons/nopass-{locked,unlocked,unlocked-timed}.svg` | Create | Three colour assets |
| `data/icons/nopass-{locked,unlocked,unlocked-timed}-symbolic.svg` | Create | Three symbolic variants |
| `scripts/assert-single-reactor.sh` | Create | The G1 gate: one `async-io`, no tokio, no libdbus, no GTK/Qt, `notify-rust` 4.17 |
| `tests/containers/Containerfile.dbus` | Create | Headless `dbus-run-session` lane image |
| `tests/manual/README.md` | Create | Lane C checklist, including the developer icon-theme install step |
| `openspec/config.yaml` | Modify | Record the tray crate's test/verify commands and the single-reactor gate |

`openspec/specs/` is untouched. `crates/nopass-core/`, `crates/nopass-helper/`, and `rust-toolchain.toml` are untouched.

---

## Threat Matrix

The tray is unprivileged, but it decides *what to ask the helper for* and it renders state a user acts on. A tray showing "inactive" while a grant is live is a security failure, not a cosmetic one. The reference matrix targets VCS/PR automation; every reference row is marked, and the four real boundaries are expanded beneath them.

| Boundary | Minimum adversarial cases | Applicability | Design response | Planned RED tests |
|---|---|---|---|---|
| Documentation-like paths | `requirements.txt`, executable Markdown, `README.sh` | **N/A** — the tray classifies and executes no repository file; it reads one fixed path and spawns two fixed absolute binaries | — | — |
| Git repository selection | `git -C`, relative/absolute paths | **N/A** — no VCS invocation in M2 | — | — |
| Commit state | staged, `commit -a`, empty index | **N/A** — no VCS invocation | — | — |
| Push state | tracking branch, first push, refspec | **N/A** — no VCS invocation | — | — |
| PR commands | `--head`, env prefix, composed commands | **N/A** — no PR automation | — | — |
| **State misrepresentation** (the primary boundary) | state file absent; `schema != 1`; truncated/empty/non-UTF-8; `active:true` with a passed expiry; `active:true` with `expires:null`; probe spawn failure; a probe older than the file; rule deleted out of band by root | **Applicable** | `FileReading` cannot express inactive-from-absence (D3); all eight merge rows resolve to `Unknown` or a probe-backed answer; a probe *error* is `None`, never `PasswordRequired`; `ProbeCache::usable` rejects a probe older than the file | Each listed case asserts the result is `Unknown` or `Active`, and **never** `Inactive`; root deletes the rule ⇒ probe ⇒ `Inactive` within one tick |
| **A failure code that may accompany a live grant** | exit 17 with the rollback confirmed; exit 17 with the rollback silently failed, leaving a rule on disk and no timer — an unexpiring grant the user was told had failed | **Applicable** | §5.1: a bare 17 is read as "state unknown, reconcile now" and its message asserts neither world; `Trigger::ActionCompleted` always probes; a following `Passwordless` escalates to the `UnexpiringGrant` warning. The tray never auto-disables | 17 ⇒ probe `PasswordRequired` ⇒ one `TimerUnscheduled` message, no escalation, `Inactive`; 17 ⇒ probe `Passwordless` ⇒ `TimerUnscheduled` **plus** `UnexpiringGrant`, state `Active { expiry: None }`, zero unrequested `pkexec` spawns |
| **External command composition** | argv injection; `PATH` hijack; shell metacharacters; env leakage into a setuid-root target; death by signal | **Applicable** | `CommandSpec` with `PathBuf` program and `Vec<String>` args, never a string; no `sh -c` anywhere; absolute candidate lists only, non-absolute program rejected before spawn; `env_clear()` with a three-variable pass-through allow-list (D9); `status: None` ⇒ `Interrupted`, never success | Exact `CommandSpec` equality for `pkexec` and `sudo`; env allow-list equality; non-absolute program returns `NonAbsoluteProgram` with zero spawns |
| **Privilege-request initiation over D-Bus** | any session peer calling our `Activate`; a flood of `Activate`; a hostile peer owning `org.kde.StatusNotifierWatcher`; a hostile `org.freedesktop.Notifications` | **Applicable** | **No D-Bus method we export may initiate a privileged action.** `Activate` only re-asserts SNI registration and posts one status notification; nudges are rate-limited to one per 5 s; `ActionGate` allows at most one in-flight `pkexec`; a hostile host only *receives* our published data and can never inject state — state comes solely from a root-written file and a local subprocess | `Activate` × 10 in 1 s ⇒ exactly one notification and zero `pkexec` spawns; a fake watcher publishing garbage changes no `TrayState` |
| **Untrusted text reaching a user surface** | helper/`pkexec` stderr in a notification; markup injection; a username with control characters or markup | **Applicable** | Notification bodies compose only `format` constants plus the username and countdown; raw stderr goes to stderr/log only (extending M1's audit rule); no markup; the username originates from a root-written file already passed through M1's `sanitize_username`, and is length-capped before display | A `VisudoRejected` outcome with attacker-shaped stderr renders the constant text and never the stderr; a username containing markup renders literally |
| **Stale or forged observation source** | a user process creating `/run/nopass/<uid>.state` | **Applicable** | `/run/nopass` is `0755 root:root` (M1 tmpfiles), so an unprivileged peer cannot create or replace the file; and the file is never sufficient on its own — every path that could render `Inactive` from it is corroborated or superseded by the probe | Documented as an environment invariant plus the merge tests above; a state file claiming `active:true` while the probe says `PasswordRequired` renders `Inactive` |

---

## Migration / Rollout

No migration — additive to a working M1. `cargo build` installs nothing system-wide and the tray writes to no system path; every mutation goes through the helper, so a reverted M2 slice leaves no residue. Reverting the M2 commits removes `crates/nopass/`, `data/icons/`, `scripts/assert-single-reactor.sh`, and the workspace member line, restoring the archived M1 tree exactly.

Delivery is `auto-chain`, `stacked-to-main`, 400 changed lines per PR; the proposal's eleven slices hold, with **slice 1 owning the three gates in §0** before any feature code. Carried risk, already flagged in the proposal and unresolved here: the forecast total (~2,750 lines) exceeds `attempt_ledger.default_max_changed_lines: 2500`. That ceiling must be raised deliberately or the work split across two attempts *before* apply begins — discovering it mid-apply is how M1's forecasts failed.

Manual rollback after exercising the tray on a real session is unchanged from M1: `pkexec /usr/libexec/nopass-helper disable`, or `systemctl stop 'nopass-expire-*.timer'` plus `rm -f /etc/sudoers.d/90-nopass-*` and `rm -rf /run/nopass/` as root.

---

## Open Questions

- [ ] **G1 is unproven in this phase.** `ksni 0.3.6 + async-io`, `zbus 5` defaults, `notify-rust 4.17`, `notify 8` was **not** compiled here — no execution tool was available. Slice 1 MUST run `scripts/assert-single-reactor.sh` and `cargo +1.85 build --release -p nopass` before any adapter code, and take the pre-agreed Fallback B (all-tokio, `flavor = "current_thread"`) rather than improvising if it fails.
- [ ] **G3 is a strong prior, not an observation.** That polkit's `EnumerateActions` is callable unprivileged is inferred from `pkaction(1)` being an unprivileged tool that calls exactly it. The design is insensitive to the answer (`Indeterminate` never disables the toggle, and the policy-file stat is the fallback), so this is a checklist item for Lane C, not a blocker.
- [ ] **What the helper would have to expose for exit 17 to be disambiguable — not assumed to exist.** §5.1 is deliberately written so the tray is correct without it, but the ambiguity is resolvable at the source in three ways, in descending order of fit with M1's architecture: (a) make `rollback_rule` return its unlink result and write the state file before returning 17 when the rollback was **not** confirmed — the observability channel then carries the surviving grant, which is exactly what that channel exists for, and the exit table is untouched; (b) split the code, e.g. 17 = rollback confirmed and a new 18 = rollback unconfirmed — cheapest for the tray, but it edits `helper-cli`'s frozen exit table and every reader of it; (c) emit a machine-readable line on stdout, which `pkexec` does relay — rejected, because it invents a second status channel beside the one M1 already defines. **M2 must not edit M1**, so this belongs to the coordinator's bounded helper fix. If (a) lands, the tray needs no change at all: the escalation in §5.1 rule 3 already triggers on the probe, and a state file would merely make it arrive sooner.
- [ ] Exact `ksni` 0.3.6 call names (`spawn`, `Handle::update`, the `Status`/`OverlayIcon` property surface) are deliberately not fixed in this document. They live behind `TrayPort` in `tray.rs`; slice 7 pins them against the compiled pin. A pinned pre-1.0 API is not a contract worth transcribing from memory.
- [ ] `format::toggle_label` returning `None` leaves the only action insensitive for as long as `Unknown` persists. Bounded at ~2 s in normal operation, but if `sudo` itself is missing the probe fails every tick and the app is permanently unusable — which is arguably correct, since NoPass cannot function without `sudo`. Confirm the status line names that cause explicitly so the user is not left with a dead menu and no reason.
- [ ] A `trybuild` compile-fail test proving `FileReading` has no conversion to `Inactive` was considered and declined: a dev-dependency and a slow test to assert the absence of an API that the module deliberately does not define. The guarantee rests on the exhaustive `merge` table plus review. Revisit if a future milestone adds a second reader.
- [ ] D9's locale pass-through assumes the polkit dialog's language follows the calling process's locale. If Lane C shows the dialog localizing from the agent's own environment regardless, the pass-through is harmless but pointless and can be dropped for a strict `env_clear()`.

# Design: M1 — Core + Privileged Helper

Change: `m1-core-helper` · Project: `nopass` · Inputs: `proposal.md`, `research.md`, `exploration.md`, `docs/PRD_NoPass_Linux.md` §6–§8 · Date: 2026-09-12

## Technical Approach

Hexagonal split along the I/O boundary. `nopass-core` is a pure library (no syscalls, no subprocesses, no UI) holding every decision that can be made from bytes alone: rule rendering, header grammar, expiry algebra, path layout, `login.defs` parsing, UTC formatting, and the `HelperStatus` wire type. `nopass-helper` is the only crate that touches the system; every external program goes through a `CommandRunner` port so exact argv is asserted in unit tests without systemd. Each subcommand is a single transaction function in `ops.rs` with an explicit, testable step order and explicit rollback rules. Errors are one typed enum whose discriminant *is* the exit code.

---

## 1. Workspace and Cargo

```
nopass/
├── Cargo.toml                      # workspace root + shared profile
├── Cargo.lock                      # committed (binary workspace)
├── rust-toolchain.toml             # channel "1.85" — MSRV is the build toolchain
├── crates/nopass-core/
├── crates/nopass-helper/
├── data/
└── tests/containers/
```

**Root `Cargo.toml`**

```toml
[workspace]
resolver = "3"
members = ["crates/nopass-core", "crates/nopass-helper"]

[workspace.package]
edition      = "2024"
rust-version = "1.85"
license      = "MIT"

[workspace.dependencies]
nopass-core        = { path = "crates/nopass-core" }
serde              = { version = "1.0",  features = ["derive"] }
serde_json         = "1.0"
thiserror          = "2.0"
clap               = { version = "4.5",  features = ["derive"] }
nix                = { version = "0.30", default-features = false, features = ["fs", "user", "dir"] }
tracing            = "0.1"
tracing-subscriber = { version = "0.3",  default-features = false, features = ["registry", "std", "fmt"] }
tracing-journald   = "0.3"
tempfile           = "3"

[profile.release]
lto           = true
codegen-units = 1
strip         = true
panic         = "unwind"   # deliberate: keeps panic exit 101 distinct from SIGABRT/134
```

`nopass-core` deps: `serde`, `serde_json`, `thiserror` (+ dev `serde_json`). `nopass-helper` deps: `nopass-core`, `clap`, `nix`, `serde_json`, `thiserror`, `tracing`, `tracing-subscriber`, `tracing-journald` (+ dev `tempfile`). **No `tokio`, no `regex`, no `chrono`/`time`, no `glob` anywhere in M1.** `crates/nopass-core/src/lib.rs` and `crates/nopass-helper/src/main.rs` both start with `#![forbid(unsafe_code)]`.

`tracing-subscriber` needs `"fmt"` in addition to `"registry"`/`"std"`: the stderr fallback in §9 uses `tracing_subscriber::fmt`, which is feature-gated and would otherwise fail with `E0433`.

**Version status** (registry read 2026-09-12): latest are nix 0.31.3, clap 4.6.6, thiserror 2.0.20, tracing-journald 0.3.2, tracing-subscriber 0.3.23, serde_json 1.0.151, tempfile 3.27.0. Every caret pin above except `nix` already resolves to that latest version (`"4.5"` → 4.6.6, `"2.0"` → 2.0.20, `"0.3"` → 0.3.23, `"1.0"` → 1.0.151, `"3"` → 3.27.0), so no edit is needed for them. **`nix` deliberately stays at `"0.30"`** (`^0.30` does not reach 0.31): a fresh-context validator compiled the exact API surface this design uses against **nix 0.30.1** with `features = ["fs", "user", "dir"]` — `fcntl::Flock`, `FlockArg::LockExclusiveNonblock`, `fcntl::renameat`, `unistd::fsync`, `unistd::User::from_uid`, `unistd::getgrouplist` all resolve. Bumping to 0.31 would re-open signature risk on `Flock`/`renameat` for no gain; revisit only if 0.30 is yanked or a needed API lands in 0.31.

`nix` feature rationale: `fs` → `open`, `fchmod`, `fchown`, `fsync`, `renameat`, `unlink`, `Flock`; `user` → `User::from_uid`, `Group`, `getgrouplist`, `getuid`, `geteuid`; `dir` → `Dir` for the `/etc/sudoers.d` sweep and directory `fsync`. The validator confirmed `getgrouplist` is gated on `user` alone — no `process` feature is required.

---

## 2. Module map and public API

### `nopass-core`

| Module | Public API |
|---|---|
| `expiry` | `const MIN_DURATION_SECS: u64 = 60` · `const MAX_DURATION_SECS: u64 = 28_800` · `enum Expiry { Never, Reboot, At { epoch: u64 } }` · `Expiry::is_expired(now: u64) -> bool` · `Expiry::is_expired_at_boot(now: u64) -> bool` · `validate_until(now: u64, until: u64) -> Result<u64, DurationError>` · `enum DurationError { NotInFuture, TooShort { secs: u64 }, TooLong { secs: u64 } }` |
| `template` | `fn sanitize_username(raw: &str) -> String` · `fn render_rule(uid: u32, user: &str, expires: Expiry) -> String` · `const BANNER: &str` |
| `header` | `struct RuleHeader { user: String, expires: Expiry }` · `fn parse(content: &str) -> Result<RuleHeader, HeaderError>` · `fn is_nopass_owned(content: &str) -> bool` · `enum HeaderError { MissingBanner, MissingUser, MissingExpires, MalformedUser, MalformedExpires }` |
| `paths` | `struct Layout { sudoers_dir: PathBuf, run_dir: PathBuf }` · `Layout::system()` · `Layout::under(root: &Path)` · `rule_path(uid)` · `rule_tmp_path(uid)` · `state_path(uid)` · `state_tmp_path(uid)` · `lock_path()` · `const RULE_PREFIX: &str = "90-nopass-"` · `fn uid_from_rule_filename(name: &str) -> Option<u32>` · `const HELPER_PATH: &str = "/usr/libexec/nopass-helper"` |
| `state` | `const SCHEMA_VERSION: u32 = 1` · `struct HelperStatus { schema, uid, user, active, expires: Option<Expiry>, rule_path: String, updated_at: u64 }` · `HelperStatus::active_from(uid, header, rule_path, now)` · `HelperStatus::inactive(uid, user, rule_path, now)` · `to_json_line(&self) -> String` |
| `logindefs` | `struct UidRange { min: u32, max: u32 }` · `const UID_FLOOR: u32 = 1000` · `const DEFAULT_MAX: u32 = 60_000` · `fn parse(content: &str) -> UidRange` (missing/malformed → defaults; `min` clamped to `max(parsed, UID_FLOOR)`) · `UidRange::admits(uid) -> bool` |
| `timefmt` | `fn format_utc_rfc3339(epoch: u64) -> String` → `YYYY-MM-DDTHH:MM:SSZ` |

`Layout` is the single injection point that lets every file-operation test run unprivileged in a `tempfile::TempDir`; production code constructs `Layout::system()` exactly once in `main`.

### `nopass-helper`

| Module | Responsibility / key API |
|---|---|
| `main` | `fn main() { std::process::exit(run()) }`; init journal layer, parse CLI, dispatch, map `HelperError` → `i32` |
| `cli` | clap derive types (below) |
| `error` | `enum HelperError` + `fn exit_code(&self) -> i32` |
| `runner` | `CommandSpec`, `CommandOutcome`, `trait CommandRunner`, `struct SystemRunner`, `enum RunnerError` |
| `bins` | `struct Binaries` — lazy first-existing-wins absolute-path resolution · `Binaries::system()` · `Binaries::from_candidates(map: &[(&'static str, &[&Path])]) -> Binaries` (test injection, analogous to `Layout::under`) · `fn resolve(&self, name: &'static str) -> Result<&Path, HelperError>` |
| `uid` | `enum InvocationContext { Pkexec(u32), SystemRoot }` · `fn resolve(subcommand) -> Result<InvocationContext, HelperError>` |
| `checks` | `fn lookup_user(uid) -> Result<String, HelperError>` · `fn admit_uid(uid, &UidRange) -> Result<(), UidRejection>` · `enum UidRejection { Unknown, Root, BelowMin { min: u32 }, AboveMax { max: u32 } }` · `fn in_admin_group(user, uid) -> bool` (advisory) · `fn is_sudoer(&dyn CommandRunner, &Binaries, user) -> Result<(), HelperError>` |
| `lock` | `struct LockGuard` (RAII) · `fn acquire(&Layout) -> Result<LockGuard, HelperError>` |
| `fileops` | `fn write_rule_atomic(&Layout, &dyn CommandRunner, &Binaries, uid, &str) -> Result<(), HelperError>` · `fn remove_rule(&Layout, uid) -> Result<bool, HelperError>` · `fn read_rule(&Layout, uid) -> Result<Option<String>, HelperError>` · `fn list_rule_uids(&Layout) -> Result<Vec<u32>, HelperError>` |
| `timer` | `fn stop(&dyn CommandRunner, &Binaries, uid)` (infallible/tolerant) · `fn schedule(&dyn CommandRunner, &Binaries, uid, epoch) -> Result<(), HelperError>` · `fn unit_name(uid) -> String` |
| `statefile` | `fn write(&Layout, &HelperStatus) -> Result<(), HelperError>` · `fn remove(&Layout, uid)` |
| `journal` | `fn init()` · `fn audit(event, uid, user, outcome, expires, exit_code)` |
| `ops` | `fn enable(..)`, `fn disable(..)`, `fn status(..)`, `fn expire(..)` — the four transactions |

**CLI types**

```rust
#[derive(Parser)]
#[command(name = "nopass-helper", version, disable_help_subcommand = true)]
pub struct Cli { #[command(subcommand)] pub cmd: Cmd }

#[derive(Subcommand)]
pub enum Cmd {
    #[command(group(ArgGroup::new("when").multiple(false).args(["until", "until_reboot"])))]
    Enable { #[arg(long)] until: Option<u64>, #[arg(long = "until-reboot")] until_reboot: bool },
    Disable,
    Status,
    #[command(group(ArgGroup::new("target").required(true).multiple(true).args(["uid", "boot"])))]
    Expire { #[arg(long)] uid: Option<u32>, #[arg(long)] boot: bool },
}
```

No `allow_external_subcommands`, no `allow_hyphen_values`, no `trailing_var_arg`. Unknown subcommand/flag → clap exits 2 before any handler.

---

## 3. `CommandRunner` port and binary resolution

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args:    Vec<String>,
    pub env:     Vec<(String, String)>, // applied after env_clear()
    pub expect:  Expect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect { Zero, Any }  // Any = caller inspects status, never fails on non-zero

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome { pub status: Option<i32>, pub stdout: Vec<u8>, pub stderr: Vec<u8> }

pub trait CommandRunner {
    fn run(&self, spec: &CommandSpec) -> Result<CommandOutcome, RunnerError>;
}
```

`SystemRunner` uses `std::process::Command::new(&spec.program)` with `.args()`, `.env_clear()`, `.envs(&spec.env)`, `.stdin(Stdio::null())`, `.output()`. **No shell, no `PATH` inheritance, no `sh -c`.** `status: None` means killed by signal and is always treated as failure. Every spec sets `env = [("LANG","C"), ("LC_ALL","C")]` and nothing else — `PKEXEC_UID` is never propagated to a child.

`ScriptedRunner` (in `#[cfg(test)]` of `runner.rs`, exported via `pub(crate)`): holds a `VecDeque<(CommandSpec, Result<CommandOutcome, RunnerError>)>`; each `run` pops the front, asserts `spec == expected` field-by-field (argv equality is the assertion), returns the scripted result, and records the spec; `Drop` panics if the script is not exhausted. This is how every `visudo`/`sudo`/`systemctl`/`systemd-run` argv is pinned and how failure injection (exit 1 from `systemd-run`) is driven.

**Absolute-path candidate lists** (ordered, first existing wins, resolved lazily per binary on first use):

| Binary | Candidates | Covers |
|---|---|---|
| `visudo` | `/usr/sbin/visudo`, `/sbin/visudo`, `/usr/bin/visudo` | Debian/Ubuntu/Mint + Fedora use `/usr/sbin`; Arch merged-usr ships `/usr/bin/visudo` |
| `sudo` | `/usr/bin/sudo`, `/bin/sudo` | all three families |
| `systemctl` | `/usr/bin/systemctl`, `/bin/systemctl` | all three families |
| `systemd-run` | `/usr/bin/systemd-run`, `/bin/systemd-run` | all three families |
| `sh` | `/bin/sh`, `/usr/bin/sh` | probe argument for `sudo -l -U <user> /bin/sh` |

Resolution is `nix::sys::stat::stat` on each candidate in order; exhausting the list yields `HelperError::BinaryMissing { name }` → exit 1. **Lazy** resolution is deliberate: `status` and `disable` must work on a host without `systemd-run` installed.

`Binaries::system()` holds the table above. `Binaries::from_candidates` accepts an arbitrary candidate table so a test can point every name at paths inside a `TempDir` — including an **empty** candidate list, which is how the "no candidate binary path exists on the host" scenario is exercised unprivileged under `cargo test`. Production code calls `Binaries::system()` exactly once in `main`, next to `Layout::system()`.

---

## 4. Transaction ordering and rollback

### 4.1 `enable` (sequence)

```
tray/CLI ──pkexec──▶ helper                 visudo     systemd-run   journald
                     │ 1 journal init
                     │ 2 clap parse ─────────────────────────────────── exit 2
                     │ 3 PKEXEC_UID ctx ───────────────────────────────  exit 10
                     │ 4 duration validate ────────────────────────────  exit 13
                     │ 5 getpwuid + UID range ─────────────────────────  exit 11
                     │ 6 sudoer probe ──────▶ sudo -n -l -U u /bin/sh ─  exit 12
                     │ 7 FLOCK /run/nopass/lock (LOCK_EX|LOCK_NB) ─────  exit 15
   ┌─── inside lock ─┤ 8 open tmp O_EXCL|O_CREAT|O_WRONLY|O_CLOEXEC 0440
   │                 │ 9 fchown 0:0 ; fchmod 0440 (umask defeat)
   │                 │10 write ; fsync ; close
   │                 │11 ───────────────────▶ visudo -cf <tmp> ───────── exit 14 (unlink tmp)
   │                 │12 renameat(tmp → 90-nopass-<uid>) ─────────────── exit 16 (unlink tmp)
   │                 │13 fsync(dir fd)                     [warn only]
   │                 │14 systemctl stop <unit>.timer       [tolerated]
   │                 │15 ─────────────────────────────────▶ systemd-run  exit 17 (unlink rule + fsync dir)
   │                 │16 statefile write (atomic)          [error log, exit stays 0]
   └─────────────────│17 ─────────────────────────────────────────────▶ audit record
                     │18 drop LockGuard  →  exit 0
```

Rollback rules, exactly:

| Failing step | Undo | Exit |
|---|---|---|
| 1–7 | nothing was created | 10/11/12/13/15/2 |
| 8–10 (tmp create/write/fsync) | `unlink(tmp)` best-effort | 16 |
| 11 `visudo -cf` non-zero | `unlink(tmp)`; final file untouched | 14 |
| 12 `renameat` | `unlink(tmp)` | 16 |
| 13 dir `fsync` | **none** — rename already took effect and the rule is valid; log `warn` and continue | 0 |
| 15 `systemd-run` | `unlink(rule)` + `fsync(dir)` + audit `rolled_back` | 17 |
| 16 statefile | **none** — the grant is real and correct; the authoritative source is `/etc/sudoers.d`, and the tray reconciles with `sudo -kn true` | 0 (log `error`) |

The `LockGuard` is released by `Drop` at the end of `ops::enable`, **after** step 17 — every rollback happens inside the lock. Steps 14–15 run only for `Expiry::At`; `Never` and `Reboot` run step 14 alone (clears a stale timer from a previous activation) and skip `systemd-run`.

### 4.2 `disable`

`PKEXEC_UID` context (exit 10; `PKEXEC_UID=0` → exit 10). **No UID-range or sudoer admission** — removing a privilege must never be blocked, and a deleted account must still be revocable; `getpwuid` failure falls back to the header's `nopass-user`, then to `""`. Then, inside the lock:

1. `flock` (exit 15 if busy)
2. `read_rule` → `header::parse` (for the audit record; a missing/unparseable file is not an error)
3. `unlink(rule)` — `ENOENT` is success (idempotent); other errors → exit 16
4. `fsync(dir)` — warn only
5. `systemctl stop nopass-expire-<uid>.timer` — tolerated, logged at `debug`
6. statefile write with `active: false` — error logged, exit stays 0
7. audit → drop `LockGuard` → exit 0

**Unlink precedes timer stop** (inverting the naive order): if the unlink fails after the timer was already stopped, the grant would become permanent with no scheduled revocation. In this order the worst case is an orphan timer that later fires `expire --uid` and finds nothing — a no-op.

### 4.3 `expire --uid <n>`

Context: `getuid() == 0` **and** `PKEXEC_UID` unset or empty; otherwise exit 10 with zero system change. Then: `flock` (15) → `read_rule` (absent → exit 0, no-op) → `header::parse`; **if `!is_nopass_owned` → exit 0 and log `warn`, never delete a foreign file** → **stale-timer guard: delete only if `expires.is_expired(now)`**, i.e. `At { epoch }` with `epoch <= now`; `Never`, `Reboot`, and future `At` → exit 0, audit `skipped_not_expired` → `unlink` → `fsync(dir)` → `systemctl stop <unit>.timer` (tolerated) → statefile `active: false` → audit → unlock.

### 4.4 `expire --boot`

Same root context. `flock` → `list_rule_uids` via `read_dir` over `/etc/sudoers.d` matching `90-nopass-<digits>` (restricted to `<n>` when `--uid` is also given) → for each: parse header, `is_expired_at_boot(now)` (**`Reboot` counts as expired here, and only here**) → unlink + `fsync(dir)`. A per-file failure is logged and does **not** abort the sweep; exit 16 if any deletion failed, else 0. Stale `/run/nopass/*.state` files without a rule are removed. **No `systemctl`/`systemd-run` call is made in `--boot` mode** — transient timers never survive a reboot and systemd is still early in boot.

### 4.5 `status`

`PKEXEC_UID` context (exit 10). **Takes no lock and writes nothing.** It reads `/etc/sudoers.d/90-nopass-<uid>` (the authoritative source, not the state cache); `rename` atomicity guarantees a torn read is impossible. Prints one `HelperStatus` JSON object + `\n` to stdout, exit 0. A missing file yields `active: false, expires: null`. `status` can therefore never return 15 or 16.

---

## 5. Data contracts

**Rule file** `/etc/sudoers.d/90-nopass-<uid>`, mode `0440 root:root`, tmp `/etc/sudoers.d/.90-nopass-<uid>.tmp`:

```
# Generated by NoPass — do not edit manually
# nopass-user: jorge
# nopass-expires: 1789000000
#1000 ALL=(ALL) NOPASSWD: ALL
```

Four lines, one trailing `\n`. The banner is a locked data contract and the anchor of NoPass ownership detection, so it MUST be reproduced byte-for-byte everywhere it appears; the em dash is U+2014 and the file is UTF-8. It is written in English so that every system-facing artifact this project installs reads in one language, and it is exempt from application i18n: the tray UI localizes from the session locale, this file never does. Localizing the banner would make rules written under one locale unrecognizable under another, so they would survive both `disable` and the boot sweep.

**Header grammar** (line-anchored; implemented with `strip_prefix` + char predicates, **not** the `regex` crate):

| Element | Regex (normative grammar) |
|---|---|
| banner | `^# Generated by NoPass — do not edit manually$` |
| user | `^# nopass-user: ([A-Za-z0-9._-]{1,32})$` |
| expires | `^# nopass-expires: (never\|reboot\|0\|[1-9][0-9]{0,18})$` |
| rule | `^#([1-9][0-9]*) ALL=\(ALL\) NOPASSWD: ALL$` |

`is_nopass_owned` ⇔ line 1 matches the banner **and** a valid user line **and** a valid expires line are present. `sanitize_username` drops every character outside `[A-Za-z0-9._-]`, truncates to 32, and falls back to `"unknown"` when the result is empty. `login.defs` grammar: `^\s*UID_MIN\s+([0-9]+)` / `^\s*UID_MAX\s+([0-9]+)`.

**`HelperStatus` JSON** — one struct, byte-identical for `status` stdout and `/run/nopass/<uid>.state` (`0644`):

```json
{"schema":1,"uid":1000,"user":"jorge","active":true,
 "expires":{"kind":"at","epoch":1789000000},
 "rule_path":"/etc/sudoers.d/90-nopass-1000","updated_at":1789000000}
```

`schema` is the version field; a reader (the M2 tray) MUST reject `schema != 1`. `expires` is `Option<Expiry>`, `#[serde(tag = "kind", rename_all = "lowercase")]` → `{"kind":"never"}` / `{"kind":"reboot"}` / `{"kind":"at","epoch":N}` / `null`.

**State-file atomic write**: `open(/run/nopass/.<uid>.state.tmp, O_EXCL|O_CREAT|O_WRONLY|O_CLOEXEC, 0644)` → `fchmod 0644` → write → `fsync` → `renameat` → `fsync(run_dir)`. If `/run/nopass` is missing (tmpfiles not yet applied), the helper creates it `0755 root:root` and continues.

---

## 6. Timers

Unit base name: `nopass-expire-<uid>` → systemd materializes `nopass-expire-<uid>.timer` and `nopass-expire-<uid>.service`.

```
/usr/bin/systemd-run
  --unit=nopass-expire-1000
  --description=NoPass expiry for uid 1000
  --on-calendar=2026-09-12T15:00:00Z
  --timer-property=AccuracySec=1s
  --timer-property=Persistent=false
  --timer-property=WakeSystem=false
  --timer-property=RemainAfterElapse=false
  --property=Type=oneshot
  /usr/libexec/nopass-helper expire --uid 1000
```

Argv order is fixed and asserted verbatim in a unit test. `--on-calendar` is always UTC RFC-3339 with an explicit `Z` (`timefmt::format_utc_rfc3339`), never a local-time or space-separated form.

> **CORRECTION, 2026-09-15.** The sentence above is wrong and shipped a defect. systemd rejects both the `T` separator and the `Z` designator; the space-separated form this sentence forbids is the only one it accepts. Verified against systemd 255 with `systemd-analyze calendar`. Consequence: no timed grant worked from M1 until a manual test found it, because every `systemd-run` refused to schedule and the helper correctly rolled the rule back, hiding the cause behind a correct refusal. The unit test this sentence cites could not catch it: it asserts the argv we produce, never that systemd accepts it. Fixed by `timefmt::format_systemd_calendar` plus a test that consults `systemd-analyze` itself. This block is left in place rather than edited away, because an archived design is a record of what was decided, and what was decided here was wrong. `RemainAfterElapse=false` lets the transient timer be garbage-collected after firing. `Persistent=false` is correct because reboot recovery is owned by `nopass-cleanup.service`, and transient units do not survive a reboot anyway.

Replacement: `/usr/bin/systemctl stop nopass-expire-<uid>.timer` runs with `Expect::Any` before every `systemd-run`. **Any** non-zero status (systemd ≥ 249 exits 0 with a warning for a non-loaded unit; older versions exit 5) is tolerated and logged at `debug` with captured stderr; a missing `systemctl` binary during `disable`/`expire` is logged and does not fail the operation. Real problems surface as a `systemd-run` failure, which is exit 17 with rollback.

---

## 7. `data/` install artifacts

`data/com.enfoquestic.nopass.policy` → `/usr/share/polkit-1/actions/`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE policyconfig PUBLIC "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
<policyconfig>
  <vendor>NoPass</vendor>
  <action id="com.enfoquestic.nopass.manage">
    <description>Manage passwordless sudo for the current user</description>
    <description xml:lang="es">Activar o desactivar sudo sin contraseña para el usuario actual</description>
    <message>Authentication is required to modify sudo rules</message>
    <message xml:lang="es">Se requiere autenticación para modificar las reglas de sudo</message>
    <icon_name>nopass</icon_name>
    <defaults>
      <allow_any>no</allow_any>
      <allow_inactive>no</allow_inactive>
      <allow_active>auth_admin_keep</allow_active>
    </defaults>
    <annotate key="org.freedesktop.policykit.exec.path">/usr/libexec/nopass-helper</annotate>
    <annotate key="org.freedesktop.policykit.exec.allow_gui">true</annotate>
  </action>
</policyconfig>
```

One action covers `enable`, `disable`, and `status` (PRD §12 Q5 resolved: no second unprivileged action in M1). The `exec.path` value is rewritten at packaging time for Arch (`/usr/lib/nopass/nopass-helper`); M1 ships the `/usr/libexec` default.

`data/nopass-cleanup.service` → `/usr/lib/systemd/system/`:

```ini
[Unit]
Description=NoPass — remove expired passwordless sudo rules
ConditionPathExistsGlob=/etc/sudoers.d/90-nopass-*
RequiresMountsFor=/etc /run
After=systemd-tmpfiles-setup.service
Before=systemd-user-sessions.service display-manager.service

[Service]
Type=oneshot
RemainAfterExit=no
ExecStart=/usr/libexec/nopass-helper expire --boot
ProtectHome=yes
PrivateTmp=yes
NoNewPrivileges=yes

[Install]
WantedBy=multi-user.target
```

`ConditionPathExistsGlob` makes the unit a no-op on the overwhelmingly common boot with no NoPass rule. `Before=systemd-user-sessions.service` is the load-bearing ordering: no user session can begin before a `reboot`-scoped or expired grant is removed. `ProtectSystem` is deliberately **not** set — the unit must write to `/etc/sudoers.d`. The environment is systemd-clean, so `PKEXEC_UID` is absent; the helper additionally treats an empty `PKEXEC_UID` as unset.

`data/nopass.tmpfiles.conf` → `/usr/lib/tmpfiles.d/nopass.conf`:

```
d /run/nopass 0755 root root -
```

---

## 8. Testing strategy (split strategy B, concrete)

| Layer | Location | What | Gate |
|---|---|---|---|
| Unit — core | `crates/nopass-core/src/*.rs` `#[cfg(test)]` | template goldens, `sanitize_username` table, header parse ok/err table, expiry boundaries (59/60/28800/28801/past/future), `login.defs` parse + clamp (incl. missing file, `UID_MIN 0`), `format_utc_rfc3339` vectors (epoch 0, leap day, 2038, 1789000000), path builders, `HelperStatus` exact-JSON golden + round-trip | `cargo test --workspace` |
| Unit — helper argv | `crates/nopass-helper/src/{checks,timer,fileops}.rs` with `ScriptedRunner` | exact argv of `sudo -n -l -U <u> /bin/sh`, `visudo -cf <tmp>`, `systemctl stop <unit>.timer`, `systemd-run …`; `env_clear` + `LANG=C`; failure injection for exit 14 and exit 17 rollback | `cargo test --workspace` |
| Unit — CLI/errors | `crates/nopass-helper/src/{cli,error}.rs` | `Cli::try_parse_from` rejections (unknown subcommand/flag, `--until` + `--until-reboot`, `expire` with neither flag), `HelperError → exit_code` exhaustive table | `cargo test --workspace` |
| Unit — binary resolution | `crates/nopass-helper/src/bins.rs` with `Binaries::from_candidates` | first-existing-wins order across the `/usr/sbin` → `/sbin` → `/usr/bin` `visudo` list; a later candidate wins when the first is absent; an **empty** candidate list yields `BinaryMissing` → exit 1; laziness (resolving `systemd-run` is never attempted during `status`) | `cargo test --workspace` |
| Unit — file ops (unprivileged) | `crates/nopass-helper/tests/fileops_tempdir.rs` with `Layout::under(TempDir)` | `O_EXCL` collision, mode `0440` under `umask(0o077)`, atomic rename, tmp cleanup on `visudo` failure, `flock` busy → exit 15, `expire --boot` directory sweep incl. foreign-file and `.bak` suffix rejection | `cargo test --workspace` |
| Unit — install artifacts | `crates/nopass-helper/tests/data_artifacts.rs` | reads `data/com.enfoquestic.nopass.policy` via `concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/…")` and asserts, with plain `str` searches and **no XML or regex dependency**: exactly one occurrence of `<action id=`, that id is `com.enfoquestic.nopass.manage`, `<allow_any>no</allow_any>`, `<allow_inactive>no</allow_inactive>`, `<allow_active>auth_admin_keep</allow_active>`, and the `org.freedesktop.policykit.exec.path` annotation is present with the `/usr/libexec/nopass-helper` value. Also asserts the tmpfiles line and the cleanup unit's `Type=oneshot`, `ConditionPathExistsGlob`, and `Before=systemd-user-sessions.service` | `cargo test --workspace` |
| Integration — root only | `crates/nopass-helper/tests/root_system.rs` | real `/etc/sudoers.d` write, real `root:root` ownership, real `/usr/sbin/visudo -cf` accept **and** reject, real `getpwuid`/`getgrouplist`, real `/run/nopass` state file | gated: skips unless `NOPASS_ROOT_TESTS=1` **and** `geteuid().is_root()` |
| Manual — full systemd | `tests/containers/Containerfile.systemd` | actual timer firing, `nopass-cleanup.service` on boot, `journalctl -t nopass-helper` | documented, **not** a gate |

Root lane containers: `tests/containers/Containerfile.debian` (`debian:12-slim` + `sudo`, `rustup`) and `tests/containers/Containerfile.fedora` (`fedora:40` + `sudo`, `rustup`). No systemd as PID 1 is required for this lane.

**Lane reconciliation — this table supersedes the spec labels for three scenarios.** `sudoers-rule-lifecycle`'s "lock busy" scenario, `expiry-policy`'s "boot sweep" scenario, and `helper-cli`'s "no candidate binary path exists" scenario are labeled root-only/container in the delta specs, but none of them needs root: `flock` contention, directory sweeping, and candidate-path resolution are all fully reproducible against a `TempDir` through `Layout::under` and `Binaries::from_candidates`. `sdd-tasks` MUST follow the design and place all three in the unprivileged `cargo test --workspace` lane; the root lane is reserved for what genuinely requires uid 0 (real `/etc/sudoers.d` writes, real `root:root` ownership, real `visudo -cf`, real `getpwuid`/`getgrouplist`).

```bash
cargo test --workspace                                     # default gate, unprivileged
cargo build --release                                      # MSRV/edition + profile gate
podman build -f tests/containers/Containerfile.debian -t nopass-test-debian .
podman run --rm -e NOPASS_ROOT_TESTS=1 nopass-test-debian cargo test --workspace
```

**Promotion for `strict_tdd`** (first work unit, before any feature code): with `Cargo.toml` and a passing `cargo test --workspace`, set in `openspec/config.yaml`: `strict_tdd: true`, `testing.strict_tdd_effective: true`, `rules.apply.tdd: true`, `rules.apply.test_command: "cargo test --workspace"`, `rules.verify.test_command: "cargo test --workspace"`, `rules.verify.build_command: "cargo build --release"`.

---

## 9. Errors, exit codes and journald

```rust
#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    #[error("internal error: {0}")]        Internal(String),
    #[error("binary not found: {name}")]   BinaryMissing { name: &'static str },
    #[error("invalid invocation context")] Context(&'static str),
    #[error("target uid rejected")]        UidRejected(checks::UidRejection),
    #[error("target user is not a sudoer")] NotSudoer,
    #[error("invalid duration")]           Duration(#[from] nopass_core::expiry::DurationError),
    #[error("visudo rejected the rule")]   VisudoRejected { stderr: String },
    #[error("lock busy")]                  LockBusy,
    #[error("filesystem failure: {0}")]    Fs(String),
    #[error("timer scheduling failed")]    TimerFailed { rolled_back: bool },
}
```

| Variant / condition | Exit | Notes |
|---|---|---|
| `Ok(())` | 0 | includes all idempotent no-ops |
| `Internal`, `BinaryMissing` | 1 | |
| clap parse failure | 2 | clap-owned, never reached by `exit_code` |
| `Context` | 10 | missing/invalid/zero `PKEXEC_UID`; `expire` not uid 0 or with `PKEXEC_UID` set |
| `UidRejected` | 11 | `UidRejection::{Unknown, Root, BelowMin, AboveMax}` — no `getpwuid` entry, uid 0, outside `[UID_MIN, UID_MAX]`; the variant is carried for the audit record only and does not split the exit code |
| `NotSudoer` | 12 | `sudo -n -l -U <u> /bin/sh` exit ≠ 0 |
| `Duration` | 13 | outside `[60, 28800]` s, or `--until` not in the future |
| `VisudoRejected` | 14 | tmp unlinked, nothing renamed |
| `LockBusy` | 15 | `flock` `LOCK_EX|LOCK_NB` returned `EWOULDBLOCK` |
| `Fs` | 16 | open/write/fsync/rename/unlink failure; `--boot` partial sweep failure |
| `TimerFailed` | 17 | rule rolled back before returning |

Journal init:

```rust
match tracing_journald::layer() {
    Ok(layer) => tracing_subscriber::registry()
        .with(layer
            .with_syslog_identifier("nopass-helper".to_string())   // 0.3.2 takes String, not &str
            .with_field_prefix(Some("NOPASS".to_string())))
        .init(),
    Err(_) => tracing_subscriber::registry()                        // needs the "fmt" feature
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init(),
}
```

Both `with_syslog_identifier` and `with_field_prefix` take `String` in tracing-journald 0.3.2; passing a `&str` literal is `E0308`. If the journald socket is unavailable the helper falls back to the stderr layer and continues — logging never fails an operation. Audit fields per event: `NOPASS_EVENT` (`enable|disable|expire|status`), `NOPASS_UID`, `NOPASS_USER`, `NOPASS_OUTCOME` (`ok|rejected|skipped_not_expired|rolled_back|error`), `NOPASS_EXPIRES` (`never|reboot|<epoch>`), `NOPASS_EXIT` (i32), `NOPASS_REASON` (short stable token, never raw subprocess text), plus `SYSLOG_IDENTIFIER=nopass-helper` and `MESSAGE`. `sudo`/`visudo` stderr is captured into the log only, never into a rule file or stdout.

---

## Architecture Decisions

### Decision: `expire --boot` sweeps the directory instead of per-uid boot units

**Choice**: a single `nopass-cleanup.service` running `expire --boot`, which enumerates `/etc/sudoers.d/90-nopass-*` and removes anything `Reboot` or past-`At`.
**Alternatives**: (a) one persistent per-uid systemd unit installed at `enable` time; (b) `systemd-run --on-startup`; (c) a separate `cleanup` subcommand.
**Rationale**: (a) and (b) require writing to `/etc/systemd/system` per user — a second privileged write surface with its own failure and rollback modes, for a job the filesystem already describes. One unit with `ConditionPathExistsGlob` costs nothing on a normal boot. (c) would add a fifth subcommand that shares 90 % of `expire`'s code and context rules; a flag keeps the privileged surface at four verbs.

### Decision: hand-parsed line grammar instead of the `regex` crate

**Choice**: `strip_prefix` + char-class predicates in `nopass-core::header`; regexes in this document are the normative grammar, not the implementation.
**Alternatives**: (a) `regex` crate; (b) structured comment (JSON or `key=value` blob inside `#`).
**Rationale**: `regex` pulls `regex-automata` + `aho-corasick` + `memchr` into a privileged binary for four anchored prefix matches — a poor dependency trade at this trust boundary and against the < 5 MB NFR. (b) would be more extensible but breaks the human-auditable requirement: an administrator running `cat /etc/sudoers.d/90-nopass-1000` must immediately see who and until when. The grammar is fixed and closed, so a parser combinator of four `strip_prefix` calls is exhaustively testable.

### Decision: `flock` on `/run/nopass/lock`, not an `O_EXCL` lock file

**Choice**: `nix::fcntl::Flock` with `FlockArg::LockExclusiveNonblock` on a regular file at `/run/nopass/lock`, released by RAII `Drop`.
**Alternatives**: (a) `O_EXCL` sentinel file; (b) a lock inside `/etc/sudoers.d/`.
**Rationale**: `flock` is released automatically by the kernel when the process dies, so a crashed or `SIGKILL`ed helper cannot wedge the system; an `O_EXCL` sentinel leaks on crash and needs stale-PID heuristics that are themselves a security surface. (b) is rejected outright: no NoPass-managed lock artifact may ever live in a directory that `sudo` parses. `/run` is tmpfs, so the lock is reset on every boot for free. Non-blocking acquisition (fail fast with exit 15) is chosen over blocking so a hung helper cannot stall a polkit dialog indefinitely.

### Decision: real-time calendar timer, not a monotonic `--on-active` timer

**Choice**: `systemd-run --on-calendar=<UTC ISO-8601 Z>`.
**Alternatives**: `--on-active=<n>s`, which needs no date formatting at all.
**Rationale**: NFR "the temporary activation expires even if the machine suspends, hibernates or reboots". systemd monotonic timers run on `CLOCK_MONOTONIC`, which does not advance across suspend — an 8-hour grant would survive a 10-hour suspend. A realtime calendar timer fires on resume once the wall-clock instant has passed. The cost is one date-formatting function, which is far cheaper than the correctness loss.

### Decision: hand-rolled UTC formatter instead of `chrono`/`time`

**Choice**: `nopass_core::timefmt::format_utc_rfc3339` using the `civil_from_days` algorithm (~25 lines, pure, no leap seconds needed).
**Alternatives**: `chrono` or `time`.
**Rationale**: we need exactly one direction of exactly one conversion, with no timezone database, no parsing, and no locale. Either crate adds a transitive tree to a privileged binary for a function that is fully pinned by a table of known epoch → string vectors. Revisit only if M2/M3 needs human-facing local-time formatting in the tray — which is a different crate and a different trust level.

### Decision: `Expiry::At { epoch }` struct variant, not `At(u64)`

**Choice**: struct variant.
**Alternatives**: the tuple variant named in the proposal, plus a manual `Serialize`/`Deserialize` impl or a `#[serde(untagged)]` shim.
**Rationale**: serde's internally-tagged representation (`tag = "kind"`) is what produces the locked wire format `{"kind":"at","epoch":N}`. A tuple variant `At(u64)` **compiles fine but fails at runtime**: internal tagging needs a map to inject the tag into, and a newtype variant wrapping an integer has no map, so the first `serde_json::to_string` of an `At` value returns `Error("cannot serialize tagged newtype variant Expiry::At containing an integer")`. That is the worst possible failure mode for this type — the contract breaks only when a real activation is written, not at build time. The struct variant moves the guarantee into the type system. A manual `Serialize`/`Deserialize` impl would work but would put the locked JSON contract in hand-written code instead of in the derive. This is a mechanical refinement of the proposal's `At(u64)` notation; the wire contract is unchanged.

### Decision: `CommandRunner` port instead of testing against real binaries

**Choice**: a trait with a `SystemRunner` and a `ScriptedRunner` asserting exact `CommandSpec` equality.
**Alternatives**: `assert_cmd` against real `sudo`/`systemctl`, or PATH-shadowed fake binaries in a test fixture directory.
**Rationale**: the security invariant being tested is *the exact argv we construct*, not the behavior of systemd. PATH-shadowed fakes are directly incompatible with invariant 10 (absolute paths only, no `PATH` trust) — they would require weakening the very property under test. The scripted runner also drives failure injection (`systemd-run` exit 1 → exit 17 with rollback) that is essentially unreproducible against real binaries.

### Decision: the workspace contains only the two M1 crates

**Choice**: `members = ["crates/nopass-core", "crates/nopass-helper"]`; `crates/nopass` (tray) is added in M2.
**Alternatives**: scaffold all three crates now per PRD §7.2.1.
**Rationale**: an empty third member either fails to build or ships a dead `fn main()` that inflates `cargo test --workspace`, `cargo build --release`, and the review diff of every M1 slice with code nobody is reviewing. Adding a member to an existing workspace in M2 is a two-line change.

---

## Data Flow

```
  tray / CLI ──pkexec(policy: com.enfoquestic.nopass.manage)──▶ nopass-helper (root)
                                                                    │
                     ┌── nopass-core (pure) ──────────────┐         │ ports
                     │ template · header · expiry         │◀────────┤
                     │ paths(Layout) · state · timefmt    │         │
                     └────────────────────────────────────┘         │
                                                                    ├─▶ CommandRunner ─▶ visudo | sudo | systemctl | systemd-run
                                                                    ├─▶ fileops ──────▶ /etc/sudoers.d/90-nopass-<uid>
                                                                    ├─▶ lock ─────────▶ /run/nopass/lock  (flock)
                                                                    ├─▶ statefile ────▶ /run/nopass/<uid>.state
                                                                    └─▶ journal ──────▶ journald (SYSLOG_IDENTIFIER=nopass-helper)

  nopass-expire-<uid>.timer ──▶ expire --uid <n> ──┐
  nopass-cleanup.service ─────▶ expire --boot ─────┴─▶ (root, no PKEXEC_UID) ─▶ same fileops/lock/statefile/journal
```

Dependency direction is one-way: `nopass-helper → nopass-core`. `nopass-core` compiles with zero system dependencies and is the M2 tray's read-side contract for `/run/nopass/<uid>.state`.

---

## File Changes

| File | Action | Description |
|---|---|---|
| `Cargo.toml` | Create | Workspace, `workspace.package` (edition 2024, MSRV 1.85), `workspace.dependencies`, release profile |
| `Cargo.lock` | Create | Committed; binary workspace |
| `rust-toolchain.toml` | Create | Pin `channel = "1.85"` so MSRV is enforced by the build, not by review |
| `crates/nopass-core/Cargo.toml` | Create | serde, serde_json, thiserror |
| `crates/nopass-core/src/lib.rs` | Create | `#![forbid(unsafe_code)]`, module re-exports |
| `crates/nopass-core/src/{expiry,template,header,paths,state,logindefs,timefmt}.rs` | Create | Pure logic + inline unit tests |
| `crates/nopass-helper/Cargo.toml` | Create | nopass-core, clap, nix, serde_json, thiserror, tracing*, dev tempfile |
| `crates/nopass-helper/src/main.rs` | Create | `#![forbid(unsafe_code)]`, dispatch, exit-code mapping |
| `crates/nopass-helper/src/{cli,error,runner,bins,uid,checks,lock,fileops,timer,statefile,journal,ops}.rs` | Create | Privileged implementation + `ScriptedRunner` tests |
| `crates/nopass-helper/tests/fileops_tempdir.rs` | Create | Unprivileged file-op tests via `Layout::under` |
| `crates/nopass-helper/tests/data_artifacts.rs` | Create | Dependency-free assertions over `data/` (polkit action shape, tmpfiles line, cleanup unit ordering) |
| `crates/nopass-helper/tests/root_system.rs` | Create | Root-only lane, gated by `NOPASS_ROOT_TESTS=1` |
| `tests/containers/Containerfile.debian` | Create | `debian:12-slim` root test image |
| `tests/containers/Containerfile.fedora` | Create | `fedora:40` root test image |
| `tests/containers/Containerfile.systemd` | Create | Optional manual full-systemd lane (not a gate) |
| `tests/containers/README.md` | Create | How to run each lane |
| `data/com.enfoquestic.nopass.policy` | Create | polkit action (§7) |
| `data/nopass-cleanup.service` | Create | Boot cleanup unit (§7) |
| `data/nopass.tmpfiles.conf` | Create | `d /run/nopass 0755 root root -` |
| `openspec/config.yaml` | Modify | Promote `strict_tdd`/`tdd` and fill test/build commands (§8) |
| `.gitignore` | Create | `/target` |

---

## Threat Matrix

The reference matrix targets VCS/PR automation; only the subprocess-composition boundary applies here. Every listed row is marked, and the applicable boundary is expanded below it.

| Boundary | Minimum adversarial cases | Applicability | Design response | Planned RED tests |
|---|---|---|---|---|
| Documentation-like paths | `requirements.txt`, `CMakeLists.txt`, executable Markdown, `README.sh` | **N/A** — the helper never classifies or executes repository files; it reads exactly one fixed path per uid under `/etc/sudoers.d` | — | — |
| Git repository selection | `git -C`, relative/absolute paths | **N/A** — no VCS invocation anywhere in M1 | — | — |
| Commit state | staged, `commit -a`, empty index | **N/A** — no VCS invocation | — | — |
| Push state | tracking branch, first push, refspec | **N/A** — no VCS invocation | — | — |
| PR commands | `--head`, env prefix, composed commands | **N/A** — no PR automation | — | — |
| **External command composition** (added — the real boundary) | argv injection via username; `PATH` hijack; shell metacharacters; env leakage; signal death | **Applicable** | `CommandSpec` with `PathBuf` program + `Vec<String>` args, never a string; `env_clear()` + only `LANG`/`LC_ALL`; absolute-path candidate lists only; no `sh -c`; `status: None` (signal) is failure | `ScriptedRunner` argv equality for all four binaries; `PKEXEC_UID` absent from every child env; `sh`-metacharacter username sanitized before it ever reaches a `CommandSpec` |
| **Privileged invocation context** | `PKEXEC_UID` absent / non-numeric / `0` / negative / huge / empty; `expire` under `PKEXEC_UID`; `enable` as plain root | **Applicable** | `uid::resolve` per subcommand; `expire` requires `getuid()==0` **and** unset-or-empty `PKEXEC_UID` | One RED test per listed case asserting exit 10 **and** zero filesystem mutation |
| **Rule-file target selection** | foreign file in `/etc/sudoers.d`; symlink at the rule path; pre-existing tmp; uid from a filename like `90-nopass-01000` or `90-nopass-1000.bak` | **Applicable** | `is_nopass_owned` required before any delete; `O_EXCL` refuses to follow/overwrite; `uid_from_rule_filename` accepts only canonical `[1-9][0-9]*` with no suffix | Foreign file survives `expire --boot`; symlink attack yields exit 16 with no write; `90-nopass-1000.bak` is never swept |
| **Stale revocation** | an obsolete timer firing after a newer `enable` | **Applicable** | `expire --uid` re-reads the header under the lock and deletes only when `is_expired(now)` | Rule with future `At` + `expire --uid` → exit 0, file intact |

---

## Migration / Rollout

No migration — greenfield. Nothing is installed to the system by `cargo build`; `data/` artifacts are inert files until M4 packaging. Manual rollback after a test install: `systemctl stop 'nopass-expire-*.timer'`, `rm -f /etc/sudoers.d/90-nopass-*`, `rm -rf /run/nopass`, `rm -f /usr/share/polkit-1/actions/com.enfoquestic.nopass.policy`, `systemctl disable nopass-cleanup.service`. Because every write passes `visudo -cf` before `rename`, no rollback path can leave `sudo` broken.

Delivery is `auto-chain` against a 400-line review budget; the proposal's ten slices hold, with slice 1 owning the `strict_tdd` promotion before any feature code.

## Open Questions

- [x] **Closed** — dependency pins are registry-verified as of 2026-09-12 and the API surface was compiled by a fresh-context validator against nix 0.30.1. `nix` stays at `"0.30"` by choice; every other caret pin already resolves to the current latest. See "Version status" in §1.
- [x] **Closed** — `getgrouplist` is gated on the `user` feature alone; no `process` feature is needed and the advisory group pre-check is kept.
- [ ] `enable` exits 0 when only the `/run/nopass/<uid>.state` write fails (§4.1 step 16). This is deliberate — the grant is real — but it means the M2 tray MUST treat a missing/stale state file as "unknown" and reconcile with `sudo -kn true` rather than as "inactive".
- [ ] Slice 1 should still commit `Cargo.lock` and re-run `cargo build --release` on the target MSRV toolchain: the validator compiled the API surface, not the full release profile under Rust 1.85.

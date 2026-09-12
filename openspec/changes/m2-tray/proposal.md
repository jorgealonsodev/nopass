# Proposal: M2 — Tray application (`crates/nopass`)

Change: `m2-tray` · Project: `nopass` · Predecessor: `m1-core-helper` (archived) · Source of truth: `docs/PRD_NoPass_Linux.md` v1.1 · Date: 2026-09-12

## Intent

M1 made passwordless sudo grantable, revocable and auditable — from a terminal, by a human who
knows the helper's subcommands and its seventeen exit codes. The product the PRD describes is a
one-click tray icon; today none of that surface exists. M2 builds the user-facing process: the
StatusNotifierItem, the click that toggles through `pkexec`, and the state machine that keeps the
icon honest about a directory the process is not allowed to read.

The hard part is not drawing an icon. It is that `/etc/sudoers.d/` is `0750 root:root`, so the
tray can never observe the truth directly. It observes two proxies — a world-readable state file
and a live `sudo -kn true` probe — and M1 deliberately leaves that state file absent in two
documented cases. A tray that reads "absent" as "inactive" tells the user they have no
passwordless sudo while they do. That single rule is the centre of this milestone.

## Scope

### In Scope (PRD requirement IDs)

| ID | Delivered in M2 |
|---|---|
| RF-01 | SNI item, three visual states, tooltip with user/state/remaining, `-symbolic` variants |
| RF-02 | Left click toggles; `pkexec` authorization; result notification. Default duration hard-coded to 1 h (PRD §12 Q1); the first-activation warning dialog is M3 (it needs `config.toml`) |
| RF-06 | **Display only** — render remaining time from `Expiry::At { epoch }`; no duration chooser |
| RF-07 | State-file read with `schema != 1` rejection, inotify on `/run/nopass/`, `sudo -kn true` reconciliation at startup / after each action / on menu open / every 60 s, live probe wins |
| RF-08 | Success, error and expiry desktop notifications via `org.freedesktop.Notifications` |
| RF-10 | D-Bus name `com.enfoquestic.nopass`, `NameTaken` → exit 0 |
| NFR | Idle < 30 MB RSS, CPU ≈ 0 % except the 60 s reconciliation; icon visible < 1 s; no GTK/Qt |

Plus: a deliberately minimal menu (below), the three icon assets with symbolic variants, and the
tray-side mapping of every helper exit code to a distinct user-facing outcome.

### The minimal menu (deliberate subset of RF-03, not a partial RF-03)

| Item | Why it is the minimum |
|---|---|
| `Enable` / `Disable passwordless sudo` (one state-dependent item) | Not every SNI host delivers `Activate` on left click, and the accessibility NFR requires every action be reachable by keyboard. Without it the app is unusable on those panels |
| `Status: <user> — <state> (<remaining>)` (insensitive label) | The tooltip is not keyboard-reachable and some hosts do not render tooltips at all. This is the only accessible surface for RF-01's state text |
| `Quit` | A tray process with no visible way to exit is a defect on every panel |

Everything else RF-03 lists — `Activate for…`, default-duration submenu, `View current rule`,
`Start with session`, `About` — is M3.

### Out of Scope

- RF-03 full context menu and RF-09 autostart — **M3** (PRD §11 delivery table; the exploration
  brief said M2 and was wrong).
- User config `~/.config/nopass/config.toml`, the first-activation warning dialog, i18n /
  `rust-i18n` — M3. M2 ships English strings behind a single formatting module so M3 swaps the
  backend, not the call sites.
- `.deb` / `.rpm` / AUR packaging, `data/nopass.desktop`, uninstall scripts, the target-distro QA
  matrix, suspend/resume and reboot scenarios — M4.
- Any change to `nopass-core`, `nopass-helper`, `rust-toolchain.toml`, or the MSRV. M2 consumes
  M1; it does not edit it.
- XEmbed fallback (PRD §12 Q4: not in v1).

## Capabilities

### New Capabilities

- `tray-presence`: SNI registration, the three visual states and their icon names, tooltip
  content and refresh points, symbolic variants, startup visibility budget, and behaviour when no
  StatusNotifierItem host is present.
- `tray-state-sync`: state-file parsing with schema rejection, the `Unknown` state and its
  "reconcile, never assume inactive" rule, inotify wiring, the four reconciliation triggers, and
  the precedence of the live probe over the cached file.
- `tray-privileged-invocation`: `pkexec` argv construction, and the total mapping of helper exit
  codes 0/1/2/10–17 plus `pkexec` 126/127 plus spawn failure onto distinct user-facing outcomes.
- `tray-notifications`: RF-08 success/error/expiry notifications, their content, and degraded
  behaviour when no notification service is on the bus.
- `tray-single-instance`: RF-10 name ownership, `NameTaken` → exit 0, and the activation nudge.

### Modified Capabilities

None. M2 consumes `helper-cli` and `helper-observability` exactly as archived; every reader-side
obligation those specs imply (schema rejection, missing-means-unknown, one outcome per exit code)
becomes a requirement of the new tray capabilities rather than an edit to M1's frozen specs.

## Decisions

### D1 — Async runtime: `async-io`, one reactor, no tokio

`ksni` defaults to tokio; `zbus` 5 defaults to `async-io` and needs no tokio; `notify` runs its
own inotify thread regardless. Choosing tokio means enabling zbus's `tokio` feature while
`notify-rust` and `ksni` may still pull zbus's default `async-io` — two reactors in one process,
and precisely the mixed configuration in which `ksni` documents a zbus-interaction panic.

**Decision**: build on `async-io`. `ksni` with `default-features = false, features = ["async-io"]`,
`zbus` at its own defaults, one executor, one epoll loop. Rationale against the NFRs:

- **Idle CPU**: neither reactor spins; both block in `epoll_wait`. The idle cost is timers, not
  the reactor — and D2 removes every timer except the 60 s reconciliation the NFR already funds.
  Over an eight-hour idle session that is ~480 wakeups total.
- **Idle RSS (< 30 MB)**: tokio's default multi-thread scheduler starts one worker thread per
  core with its own stack, for a workload that is one D-Bus connection and one inotify fd.
  `async-io` runs a single reactor thread. Fewer threads, smaller binary (NFR: < 5 MB), one less
  dependency tree.
- **Panic risk**: with a single reactor there is no "blocking zbus call inside a foreign async
  context" to hit. Two further rules follow and belong in the spec: `notify-rust`'s **async** API
  is used (never its blocking wrapper from inside the executor), and `notify`'s inotify thread
  hands events to the reactor through an async channel rather than a blocking `recv`.
- **`pkexec` must never run on the reactor.** The polkit dialog waits on a human and can take a
  minute. It is spawned on a dedicated thread and its result is delivered back through the same
  async channel.

Cost accepted: `async-io` is the less-travelled `ksni` path, so the design phase must confirm the
feature combination compiles and that no transitive dependency re-enables zbus's tokio feature
(`cargo tree -e features` assertion, not a hope).

### D2 — Countdown: computed on demand, minute granularity, no ticking timer

`Expiry::At { epoch }` lets the tray compute remaining time whenever it is asked, for free. A
per-second tooltip needs a 1 Hz timer: ~28,800 wakeups and 28,800 D-Bus property updates across
an eight-hour grant, animating a tooltip that is invisible unless the pointer is hovering it.
The NFR budgets no such timer.

**Decision**: the countdown is rendered at **minute** granularity (`caduca en 42 min`, and "less
than a minute" below 60 s) and recomputed only at moments the tray already wakes: an inotify
event, a completed action, a menu open, and the 60 s reconciliation tick. No dedicated timer.

Worst-case staleness therefore equals the reconciliation period, 60 s, which equals the display
granularity — the user cannot observe the difference. Expiry itself is *not* affected: the helper
rewrites the state file when the timer fires, inotify delivers that in well under a second, and
the icon flips to inactive with its notification inside O4's 1 s budget. Coarse countdown, prompt
transition.

### D3 — Second instance: exit 0, after nudging the first

RF-10 is explicit and stays satisfied: the second instance exits **0** and no second icon
appears. But "exit 0 silently" is the wrong behaviour for the case that actually produces a
double-click — the user launches again *because they cannot see the icon*, which is exactly the
GNOME-without-extension failure the PRD already calls out. Silently vanishing makes a working
program look broken.

**Decision**: on `Error::NameTaken`, the second instance calls
`org.freedesktop.Application.Activate` on the existing owner with a short bounded timeout, then
exits 0 — whether the call succeeded, failed, or timed out. The first instance's handler
re-asserts its SNI registration and emits one notification stating the current status. Rationale:
`org.freedesktop.Application` is a standard freedesktop interface, so this invents no private
API; the handler reuses the notification path M2 already builds for RF-08; and the failure mode
is contained, because a nudge that fails still produces RF-10's exit 0 and never a hang.
`request_name` errors that are *not* `NameTaken` are a real fault and exit non-zero.

### D4 — Degraded environments: start degraded, refuse only with no user-visible surface

The two dependencies fail independently and silently today. They are treated separately.

**StatusNotifierItem host.** Probed at startup with `NameHasOwner("org.kde.StatusNotifierWatcher")`
on the session bus — the exact mitigation PRD §9 already prescribes. If absent the tray **starts
degraded**: it claims its D-Bus name (RF-10 still holds), emits one notification explaining that
no tray host was found and how to install the AppIndicator extension on GNOME, and subscribes to
`NameOwnerChanged` so it registers the item the moment a host appears. Refusing to start would be
worse: the user gets nothing at all, and panels legitimately appear late.

**Notification service.** Probed the same way. If absent, outcomes degrade to the icon, the
tooltip and stderr, and RF-08 is documented as unmet in that environment.

**The one hard refusal**: no session bus at all, or *neither* a tray host *nor* a notification
service — a process with no possible user-visible channel is not a tray. It prints to stderr and
exits non-zero, with a code distinct from RF-10's 0.

**polkit.** There is no reliable unprivileged probe for "an authentication agent is registered",
so the tray does **not** refuse to start over it. Two mechanisms instead:

1. *Startup, installation-completeness only*: check that `org.freedesktop.PolicyKit1.Authority`
   is on the system bus and that action `com.enfoquestic.nopass.manage` is installed. If either
   is missing, the toggle renders as unavailable with an "incomplete installation" reason rather
   than firing a `pkexec` that is guaranteed to fail. The design phase MUST verify that
   `EnumerateActions` is callable unprivileged; if it is not, fall back to checking the policy
   file's presence.
2. *At failure time*: `pkexec` 126 (dialog dismissed) and 127 (not authorized / no agent) are
   each their own user-facing outcome, distinct from each other and from every helper code.

**`/run/nopass/` missing** (incomplete install): PRD RF-07 is explicit — fall back to
reconciliation only, show a warning, and retry establishing the watch on each 60 s tick.

## Approach

One new crate, `crates/nopass`, layered so the interesting logic is testable without a desktop:

- **Pure core (no I/O)** — the merge of `(state file, live probe)` into
  `Active { expiry } | Inactive | Unknown`, tooltip and countdown formatting, and the exit-code →
  outcome table. Exhaustively unit-tested.
- **Ports** — `pkexec` and `sudo -kn true` behind a `CommandRunner`-shaped trait, reusing M1's
  proven pattern so argv is asserted without a privileged environment.
- **Adapters** — ksni item, zbus connection and name, `notify` watcher, `notify-rust` sender.

The inherited contract is enforced structurally, not by convention: the state reader's return
type has no way to express "inactive because the file was missing". Absence and `schema != 1`
both produce `Unknown`, and `Unknown` can only be resolved by a probe.

## Affected Areas

| Area | Impact | Description |
|---|---|---|
| `Cargo.toml` (workspace) | Modified | Add `crates/nopass` member and shared dependency versions |
| `crates/nopass/` | New | `state`, `reconcile`, `outcome`, `format`, `runner`, `tray`, `notify`, `instance`, `preflight`, `main` |
| `data/icons/` | New | `nopass-{locked,unlocked,unlocked-timed}.svg` + `-symbolic` variants |
| `openspec/specs/` | Unchanged | M1's five capabilities are consumed, not modified |

## Dependencies

| Crate | Version | Note |
|---|---|---|
| `ksni` | pin exact `0.3.x` | `default-features = false`, `features = ["async-io"]`. Pre-1.0: pin exact, review the changelog before any bump |
| `zbus` | 5.x | Default features (`async-io`). No tokio feature anywhere in the tree |
| `notify-rust` | `~4.17` | **Settled**: 4.18 requires rustc 1.89 and the workspace pins 1.85. Resolver 3 should pick 4.17 from `rust-version`; design MUST verify and pin with a tilde requirement if it does not |
| `notify` | 8.x | inotify backend on Linux |
| `nopass-core` | path | `HelperStatus`, `Expiry`, `is_expired`, `Layout::system().state_path(uid)` |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Tray trusts inotify alone and misreports M1's documented missing-state-file case | **High** | The reader type cannot express "inactive from absence"; a dedicated test asserts missing file + active rule ⇒ `Unknown` ⇒ probe ⇒ active |
| Two async reactors in one process trigger ksni's documented zbus panic | Med | D1: single `async-io` reactor, plus a `cargo tree -e features` assertion that no tokio zbus feature is enabled |
| `pkexec` blocks the reactor while polkit waits on a human | Med | Spawned on a dedicated thread; result returns over an async channel |
| No polkit authentication agent; failure is indistinguishable from "not authorized" by code alone | Med | Startup installation check; 126/127 rendered as distinct outcomes naming the likely cause |
| GNOME without the AppIndicator extension: no icon, no feedback | Med | `NameHasOwner` probe + notification with instructions + `NameOwnerChanged` late registration (D4) |
| Collapsing helper exit codes into a generic error | Med | The outcome table is a spec requirement with one table-driven test per code |
| `ksni` 0.3.x API churn before 1.0 | Med | Exact pin; adapter isolated behind the tray port so a bump touches one module |
| Resolver picks `notify-rust` 4.18 and breaks the 1.85 build | Low | Tilde requirement plus a build under the pinned toolchain in the first slice |
| Idle CPU exceeds the NFR through an unbudgeted timer | Low | D2: no timer other than the 60 s reconciliation; measured before verify closes |
| `sudo -kn true` fills the auth log | Low | Accepted and documented in PRD §9; frequency is already bounded to 60 s |
| Icon rendering and polkit authentication cannot be proven headlessly | **High** | See "What a desktop session is required to prove" — a documented manual lane executed on this machine and recorded in the verify report |

## Rollback Plan

M2 is purely additive to a working M1. Reverting the M2 slice commits removes `crates/nopass/`,
`data/icons/` and the workspace member line, restoring the archived M1 tree exactly; `Cargo.lock`
returns with them. Nothing in M2 is installed system-wide by a build, and the tray writes to no
system path — every mutation goes through the helper, so a reverted tray leaves no residue.

Residue from *testing*, if any grant was made while exercising the tray, is removed the same way
M1 documents: `pkexec /usr/libexec/nopass-helper disable`, or
`systemctl stop 'nopass-expire-*.timer'` plus `rm -f /etc/sudoers.d/90-nopass-*` and
`rm -rf /run/nopass/` as root.

Two rollback invariants: M2 must not edit `nopass-core`, `nopass-helper`, `rust-toolchain.toml`
or the MSRV, so no revert of an M2 slice can regress the helper; and every slice is additive
within the new crate, so slices revert independently and in any order after the first.

## Success Criteria

From PRD §10, limited to what M2 can satisfy. `sudo -kn true` is the ground-truth check throughout.

- [ ] §10.1 — the icon appears in the tray on this machine's desktop environment, recorded by name
      and version in the verify report. The four-distro matrix is M4 QA.
- [ ] §10.2 — one click while inactive shows the polkit dialog and, after authentication,
      `sudo -kn true` returns 0 within 1 s of polkit confirming (O1).
- [ ] §10.3 — one click while active removes the rule and `sudo -kn true` fails again.
- [ ] §10.5 / §10.7 (tray-side extension) — a non-sudoer rejection (12), a bad duration (13), a
      `visudo` rejection (14), a busy lock (15), a dismissed dialog (126) and a missing agent (127)
      each produce a *different* message. No generic error.
- [ ] §10.8 — **partial**: the self-revert notification path is M2's, the 15-minute *choice* is
      M3's menu. Verified by granting a short expiry through the helper directly and observing the
      tray flip to inactive and notify within 1 s of the timer firing.
- [ ] §10.11 — deleting the rule file from a root terminal updates the icon within 60 s.
- [ ] §10.12 — launching the application twice produces one icon, the second process exits 0, and
      the first emits its activation nudge.
- [ ] M2 gates — `cargo test --workspace` green under the pinned 1.85 toolchain;
      `schema != 1` and a missing state file both resolve to `Unknown` and never to inactive;
      idle RSS < 30 MB and idle CPU ≈ 0 measured over a continuous run of at least one hour with
      no wakeup source other than the 60 s reconciliation.

Deferred: §10.4, §10.6, §10.10, §10.13 (M1, already met), §10.9 and §10.14 (M4).

## Delivery Plan

Strategy `auto-chain`, `stacked-to-main`, 400 changed lines per PR. Strict TDD is **on**
(`config.yaml: tdd: true`), so tests are written first and are typically the larger half of a
slice. **M1 overran every phase forecast because verification-driven test lines were never
counted. They are counted here**, as a separate column, and the totals below are what the
reviewer will actually see.

| # | Slice | Impl | Tests | Total |
|---|---|---|---|---|
| 1 | Crate skeleton, workspace member, dependency pins, feature-tree assertion, `notify-rust` 4.17 resolution check | 60 | 40 | 100 |
| 2 | State reader: parse, `schema != 1` rejection, missing ⇒ `Unknown` | 120 | 180 | 300 |
| 3 | Reconciliation: `sudo -kn true` port, precedence, the merge state machine | 150 | 220 | 370 |
| 4 | inotify watcher, reactor bridge, missing-directory fallback, 60 s tick | 140 | 160 | 300 |
| 5 | `pkexec` invocation and the full exit-code → outcome table (0/1/2/10–17/126/127/spawn) | 130 | 230 | 360 |
| 6 | Icon assets (3 + 3 symbolic) and theme-aware name resolution | 100 | 20 | 120 |
| 7 | SNI item: three states, tooltip and countdown formatting, minimal menu, left-click toggle | 220 | 150 | 370 |
| 8 | Notifications: success, error, expiry; degraded-notify path | 110 | 120 | 230 |
| 9 | Single instance: name request, `NameTaken` ⇒ exit 0, `Activate` handler and nudge | 100 | 110 | 210 |
| 10 | Startup preflight, degraded modes, refusal rules, `main` wiring | 130 | 140 | 270 |
| 11 | Manual desktop verification lane: checklist and headless `dbus-run-session` harness | 60 | 60 | 120 |
| | **Total** | **1,320** | **1,430** | **2,750** |

Each slice has a clear finish and reverts on its own: 2–5 are pure logic behind ports and land
with no UI; 6–10 each add one adapter. Slices 2 and 3 together are the milestone's highest-value
risk and should be reviewed as such even though neither renders anything.

**Budget note**: no slice exceeds 400 lines, but the forecast total of ~2,750 exceeds the
`attempt_ledger.default_max_changed_lines` of 2,500 carried over from M1. That ceiling should be
raised deliberately for M2 or the work split across two attempts — not discovered mid-apply,
which is how M1's forecasts failed.

## What a Desktop Session Is Required to Prove

This machine has a session; a headless lane will not. The dividing line is **not** the session
bus — `dbus-run-session` starts a private bus in any container — it is human-visible rendering
and polkit authentication.

**Provable headlessly** (real tests, no desktop): state parsing and schema rejection; missing-file
⇒ `Unknown`; the probe/file precedence state machine; every exit-code mapping; tooltip and
countdown strings; the exact argv of `pkexec` and `sudo`; inotify against a temporary directory;
and — under `dbus-run-session` with stub services — name ownership and the `NameTaken` path
(§10.12 at the protocol level), SNI registration and property values against a fake
`StatusNotifierWatcher`, host-absence detection and late registration via `NameOwnerChanged`, and
notification payloads against a fake `org.freedesktop.Notifications`.

**Requires a real desktop session, therefore manual on this machine**: that a panel actually draws
the icon and its tooltip (§10.1); that the polkit dialog appears, authenticates, and that
`auth_admin_keep` suppresses the second prompt (§10.2, §10.3); that notifications are visibly
rendered by the session's daemon; keyboard navigation of the menu (accessibility NFR); the
double-clicked-launcher path end to end; and the idle RSS/CPU measurement over a real session.

**Provable nowhere automatically**: polkit authentication itself — it requires a real agent and a
typed password. It stays a human step, and the verify report must record the desktop environment,
its version, and the observed result for each manual item rather than inferring them.

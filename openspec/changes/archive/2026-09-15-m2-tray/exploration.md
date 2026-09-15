# Exploration: M2 — System Tray Application

Change: `m2-tray` · Project: `nopass` · Predecessor: `m1-core-helper` (archived) · Date: 2026-09-12

## Current State

M1 is complete and archived across nine commits. The workspace has two members, `nopass-core`
and `nopass-helper`; `crates/nopass` does not exist yet. `rust-toolchain.toml` pins the channel
to `1.85` and the workspace sets `rust-version = "1.85"`, which binds anything M2 adds.

The helper is proven: 257 unprivileged tests plus 15 root-only tests that pass as real uid 0 on
Debian 12 and Fedora 40.

## What M1 Hands Over

Reusable directly from `nopass-core`:

| Item | Use in the tray |
|---|---|
| `state::HelperStatus`, `state::SCHEMA_VERSION` | The exact wire type parsed from `/run/nopass/<uid>.state`. The tray MUST add its own `schema != 1` rejection; nothing does it yet. |
| `expiry::Expiry`, `is_expired`, `is_expired_at_boot` | Countdown and expiry logic, no reimplementation needed. |
| `paths::Layout::system().state_path(uid)` | The only `Layout` method the tray needs. `rule_path` and `rule_tmp_path` are helper-only. |

Not provided, and therefore new M2 code: D-Bus and StatusNotifierItem, an inotify wrapper,
`pkexec` and `sudo -kn true` invocation, local-time countdown formatting. Config file handling
belongs to M3.

## The Helper Interface, As The Tray Sees It

Four fixed subcommands. The tray invokes `enable` and `disable` through `pkexec`, and `status`
for a read-only view. `expire --uid` requires root with no `PKEXEC_UID` and is never
tray-invocable.

Exit codes are a contract, not a detail: 0, 1, 2 and 10 through 17 each carry exactly one
meaning. Exit 17 deserves distinct messaging, because the rule genuinely does not exist despite
the authentication having succeeded. Collapsing these into one generic failure discards the
reason an operation was refused, which is the information the user needs to act.

`pkexec` adds its own codes above the helper's: 126 means the user dismissed the dialog, 127
covers the not-authorized and no-agent family. Both must be distinguished from a helper refusal.

## State Detection (RF-07)

`/etc/sudoers.d` is `0750 root:root`, so an unprivileged process cannot stat it or watch it.
Two sources, with a defined precedence:

- **inotify on `/run/nopass/`** — the fast path, fires on every helper-initiated write.
- **`sudo -kn true`** — ground truth. Fires at startup, after an action, on menu open, and on a
  60 s timer.

When they disagree, the live probe wins over the cached file. The PRD is explicit about this.

## The M1 Contract M2 Must Honour

A missing or stale state file MUST be read as "unknown, reconcile", never as "inactive". This is
written in `openspec/specs/helper-observability/spec.md` and in the archived design's §4.1
rollback row 16 and §4.4. Verified in `ops.rs`: `enable` returns exit 0 when only the state-file
write fails, because the grant is already real, and the boot sweep removes a swept rule's state
file rather than rewriting it.

Consequence: no tray code path may treat the state file as authoritative. Every inotify-driven
update either pairs with reconciliation or passes through an explicit "unknown" state first. A
tray that trusts inotify alone will tell the user they have no passwordless sudo while they do.

## Scope

Per PRD §11's own delivery table, M2 covers RF-01, RF-02, RF-06 (display only), RF-07, RF-08 and
RF-10. RF-03 (full context menu) and RF-09 (autostart) belong to M3. See "Resolved" below.

## Stack Assessment (registry-verified 2026-09-12)

| Crate | Version | MSRV | Note |
|---|---|---|---|
| `ksni` | 0.3.5 / 0.3.6 | 1.80 | Fine. Defaults to tokio but documents `async-io` and `blocking` alternatives. Docs flag a zbus-interaction panic risk under certain configs. |
| `zbus` | 5.19.0 | — | Runtime-agnostic, `async-io` default. No hard tokio requirement. |
| `notify-rust` | 4.18.0 | **1.89.0** | Exceeds the workspace's 1.85 pin. See "Resolved" below. |
| `notify` | 8.2.0 | not confirmed | inotify-backed on Linux. Heavily used by rust-analyzer and alacritty. |

`ksni` and `notify-rust` both default to `zbus`, so there is no second D-Bus stack.

## Open Questions — Findings

- **Single instance (RF-10).** `zbus::Connection::request_name` fails with `Error::NameTaken`
  when the name is owned. Mapping that to a silent exit 0 is mechanically trivial; whether it is
  the right response to a double-clicked launcher is a product question the PRD does not answer.
- **Does the tray need tokio?** No. It is `ksni`'s default, not a shared requirement of `zbus`,
  `notify` or `notify-rust`. Treat the runtime as an open architectural decision.
- **Countdown display.** `Expiry::At { epoch }` gives the tray everything needed to compute
  remaining time on demand, which costs nothing while idle. A ticking label needs its own
  periodic timer, which the NFR budget ("CPU ~ 0% except the 60 s reconciliation") does not fund.
- **`pkexec` from a tray.** Requires a running polkit authentication agent, a constraint separate
  from needing a StatusNotifierItem host. A minimal window manager can fail either one
  independently, and both currently fail silently.
- **Reconciliation cost.** All four RF-07 triggers are event-driven except the 60 s timer, which
  is the cost the NFR already accepts by name.

## Resolved By The Orchestrator

Two of the exploration's blockers are resolved here rather than carried into the proposal.

**1. `notify-rust` MSRV.** Verified against the registry: 4.18.0 requires rustc 1.89.0, while
4.17.0 and every version back to 4.12.0 require 1.63.0. The workspace pins 1.85 deliberately, so
the distro-shipped toolchains in scope can build it; raising the pin to satisfy one notification
crate inverts that priority.

Resolution: keep the 1.85 pin and take `notify-rust` 4.17. The workspace already uses
`resolver = "3"`, whose MSRV-aware resolution should select 4.17.0 on its own given
`rust-version = "1.85"`. The design phase MUST verify that it actually does, and pin explicitly
with a tilde requirement if it does not. Do not discover this at packaging time.

**2. RF-03 and RF-09 scope.** The exploration brief listed them as M2; PRD §11's delivery table
assigns the full context menu and autostart to M3. The PRD is the source of truth and the brief
was wrong. M2 scope is RF-01, RF-02, RF-06 display, RF-07, RF-08 and RF-10. M2 still needs a
minimal menu to be usable, but "minimal" must be stated in the proposal as a deliberate subset,
not left to drift into RF-03's full surface.

## Risks For The Proposal

1. A naive tray that trusts inotify alone misreports M1's own documented degraded case. This is
   the highest-value risk in M2 and the one a test must pin.
2. `pkexec` needs a polkit authentication agent, undetected today and failing silently.
3. GNOME requires a Shell extension to host a StatusNotifierItem. The PRD accepts this;
   detecting its absence and warning the user is new code.
4. The runtime choice (tokio, `async-io`, or blocking) is unresolved and shapes the whole crate.
5. Countdown refresh cadence is unspecified and can quietly cost idle CPU the NFR does not budget.
6. `ksni`'s documented zbus-interaction panic risk under certain configurations needs a
   deliberate configuration choice rather than defaults.

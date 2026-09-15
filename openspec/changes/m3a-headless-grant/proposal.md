# Proposal: M3a — Headless operation (root-context grant)

Change: `m3a-headless-grant` · Sequenced **after** `m3-menu-and-config` · Touches `crates/nopass-helper` only · Date: 2026-09-15

## Intent

NoPass cannot be operated on a server. `enable`, `disable` and `status` resolve their target uid **exclusively** from `PKEXEC_UID` (`privilege-admission/spec.md:11`), so `sudo /usr/libexec/nopass-helper status` exits **10** — confirmed by running it. The only thing that works is hand-forging the pkexec context: `sudo PKEXEC_UID=1000 … status` exits 0. That is a workaround, not an interface — nothing specifies it, nothing tests it, and it invites operators to fake an environment variable the design treats as trusted. `pkexec` over SSH is not an alternative: `data/com.enfoquestic.nopass.policy:14` sets `allow_inactive=no` and polkit classifies an SSH session as inactive, so the policy denies it. This is not a missing GUI agent.

## Scope

### In Scope

| # | Deliverable | Lane |
|---|---|---|
| 1 | A helper subcommand restricted to the `SystemRoot` invocation context `expire` already uses — real uid 0 **and** `PKEXEC_UID` unset/empty (`uid.rs:40-90`) — taking an explicit `--uid` and the same expiry flags `enable` has | A |
| 2 | The symmetric revoke/read the stated problem ("activate **or** deactivate") requires; one command with an action or three siblings is `sdd-design`'s call | A |
| 3 | journald audit coverage for those invocations, recording the root context and the explicit target | A |
| 4 | `docs/` operator section for headless hosts, writing down the desktop assumption the PRD makes end to end (tray, interactive polkit, `allow_inactive=no`) | C |
| 5 | Assertion that install → grant → revoke needs no session | B + C |

### Out of Scope

- Any remote/network interface. This is a local root command; nothing listens.
- Changing the existing action's `allow_inactive`. The desktop path stays exactly as it is.
- The tray, the menu, i18n — `m3-menu-and-config`.
- Splitting the `.deb` (see Affected Areas).

## Capabilities

### New Capabilities

- `headless-operation`: operating NoPass with no desktop session — the root invocation context as an operator interface, the no-tray-required obligation, and the operator documentation.

### Modified Capabilities

- `helper-cli`: "Fixed Subcommand and Flag Surface" pins **exactly four** subcommands (`spec.md:11`). That count grows, with the new flags closed at parse time the same way.
- `privilege-admission`: "UID Resolution by Invocation Context" makes `expire` the only `SystemRoot` consumer. It gains a second one, targeted by explicit `--uid` rather than by the invoking process.
- `helper-observability`: "Journald Audit Records" enumerates `enable`/`disable`/`expire` only; the new invocations must be journaled too.

Unchanged at spec level: `sudoers-rule-lifecycle`, `expiry-policy`. This is a new way to ask, not a new thing to do.

## Approach

A **second consumer of an admission class that is already specified, implemented and tested** — deliberately *not* a new admission path. Authorization is "you are already root", and root can write `/etc/sudoers.d` directly, so nothing is granted that was not already available. After resolving `SystemRoot`, the command enters the existing transaction unchanged: same `flock`, `visudo -cf`, atomic rename, timer, state file, audit.

**Rejected alternatives:**

- **A second polkit action with `allow_inactive=auth_admin`.** Keeps polkit in the loop over SSH, but needs `pkttyagent` and an interactive admin password on hosts that commonly disable password authentication in favour of keys. More surface, worse ergonomics, and it still fails on an unattended host.
- **Widening `enable` itself to accept a root context.** One subcommand accepting two invocation contexts destroys the invariant that `enable` is always self-targeted, which is currently a testable property (`uid.rs:177-192`).

**The widening, stated plainly:** a root-invoked grant can target **any** uid. A pkexec `enable` can only ever target its own caller. That is a real widening of what the helper can be asked to do.

It is bounded: the target still passes `admit_uid` (`checks.rs:87-102`), which rejects uid 0, enforces the configured range, and requires the user to actually exist. Every existing audit, rule-lifecycle and expiry-timer obligation applies unchanged.

## Affected Areas

| Area | Impact | Description |
|---|---|---|
| `crates/nopass-helper/src/cli.rs` | Modified | New subcommand(s) with `--uid` and the existing expiry flags |
| `crates/nopass-helper/src/uid.rs` | Modified | `SystemRoot` becomes reachable from more than `Cmd::Expire` |
| `crates/nopass-helper/src/ops.rs` | Modified | New wrapper reusing `enable_inner`/`disable_inner` |
| `crates/nopass-helper/src/checks.rs` | Unchanged | `admit_uid` is the bound, consumed as-is |
| `docs/` | New | Operator section for headless hosts |
| `data/com.enfoquestic.nopass.policy` | **Unchanged** | The desktop path is untouched |
| `crates/nopass/`, `crates/nopass-core/` | Unchanged | No tray file is touched; no collision with `m3-menu-and-config` |
| `crates/nopass/Cargo.toml:43-61`, `debian/` | Investigated, unchanged | Verdict below |

**Packaging verdict.** The `.deb` is one package shipping tray, helper, policy, icons, `.desktop` and the cleanup unit. Nothing misbehaves without a session today: no autostart entry and no `systemd-enable` of the tray are shipped (`Cargo.toml:62-63`), every session-touching `postinst` call is `|| true` (`debian/postinst:6-9`), and `nopass-cleanup.service` is root-only and boot-time. A split would therefore be cosmetic — avoiding a tray binary, icons, a desktop entry and the `policykit-1 | polkit` dependency on servers — so **it belongs in M4's packaging milestone, not here**. This change only asserts session-independence.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The new command becomes a privilege-escalation primitive for a non-root caller | Low | It resolves through the same `SystemRoot` gate as `expire`; the caller is already uid 0, which can edit `/etc/sudoers.d` regardless. Lane A asserts every non-root and `PKEXEC_UID`-present combination exits 10 with no write |
| Arbitrary-uid targeting grants an unintended user | **Med** | `admit_uid` rejects uid 0, out-of-range and non-existent uids (exit 11); the audit record names the explicit target so the grant is attributable |
| A future refactor lets `enable` accept the root context, collapsing the two paths | Med | The existing self-targeting test (`uid.rs:177-192`) stays and is extended to the new command's mirror property |
| Operators keep using the `PKEXEC_UID=…` forgery because it still works | Med | Documentation names it as unsupported; the supported command is the one that is specified and tested |
| Sequencing collision with `m3-menu-and-config` | Low | Disjoint files; that change explicitly alters no helper behaviour |
| Scope creep into a remote/daemon interface | Low | Explicitly out of scope; nothing listens |

**Risk level: High** — this is the privilege boundary, and the change is a genuine widening of what the helper can be asked to do, even though it authorizes nothing root did not already have.

## Rollback Plan

Additive and helper-local. Reverting the slice commits removes the subcommand; `enable`/`disable`/`status`/`expire` and the polkit action return to their M1 behaviour with no migration, because no on-disk format, state-file schema, rule-file format or timer unit name changes. Any rule already granted through the new command is byte-identical to a pkexec-granted one and is still revoked by `disable`, by its expiry timer, by `expire --boot`, or by `prerm`. `docs/` changes revert independently.

## Dependencies

- Sequenced after `m3-menu-and-config` merges (no file overlap, but a shared `crates/nopass-helper` history).
- Consumes archived `privilege-admission`, `sudoers-rule-lifecycle`, `expiry-policy` and `helper-observability` behaviour unchanged.
- Lane C needs a real headless host reachable over SSH with no desktop session.

## Success Criteria

- [ ] On a host with no session, a root-invoked grant for an admitted uid writes the rule, schedules the timer, updates the state file and exits 0 — Lane A for the transaction, Lane C on a real server.
- [ ] The same command run as non-root, or with `PKEXEC_UID` set to anything, exits 10 and writes nothing — Lane A.
- [ ] A root-invoked grant targeting uid 0, an out-of-range uid, or a non-existent uid exits 11 and writes nothing — Lane A.
- [ ] `enable`/`disable`/`status` still exit 10 without `PKEXEC_UID`, and `enable` remains self-targeted — Lane A (existing tests unchanged and still green).
- [ ] `journalctl -t nopass-helper` shows every new invocation with its outcome, its explicit target uid and its root context — Lane A for the record, Lane C for the real journal.
- [ ] The grant is revocable on the same headless host, and is also removed by its expiry timer and by `expire --boot` — Lane C.
- [ ] `docs/` states the desktop assumption and the supported headless procedure, and never documents `PKEXEC_UID=` forgery as an interface — Lane C.
- [ ] Install → grant → revoke completes with no desktop session and no tray process running — Lane B + Lane C.
- [ ] `cargo test --workspace`, `cargo clippy -D warnings` and `scripts/run-lane-b.sh` green under the pinned 1.85 toolchain.

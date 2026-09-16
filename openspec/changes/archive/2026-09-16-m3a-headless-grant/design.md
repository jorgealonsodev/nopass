# Design: M3a — Headless operation (root-context grant)

Change: `m3a-headless-grant` · Project: `nopass` · Inputs: `proposal.md`, the four delta specs under `specs/`, archived `m1-core-helper/design.md` §4/§8/§9, archived `m2-tray/design.md` (house style) and its `verify-report.md`, `crates/nopass-helper/src/{cli,uid,ops,checks,journal,error,main}.rs`, `tests/containers/README.md` · Date: 2026-09-15

> Size note: the sdd-design 800-word budget is deliberately exceeded, following the M1/M2 convention in this repository. This is the privilege boundary; the audit-schema and test-execution decisions below are the artifact.

## Technical Approach

Three flat sibling subcommands that **add no transaction code at all**. `grant`, `revoke`, and `inspect` are wrappers that resolve the already-specified `SystemRoot` context, admit an explicit `--uid`, and then enter `enable_inner` / `disable_inner` / `status_inner` — the same `flock`, `visudo -cf`, atomic rename, `systemd-run` timer, state file and audit that `enable`/`disable`/`status` use today. Nothing about *what is done* changes; only *who may ask, and for whom*.

The one structural addition is a type, `Subject`, that carries **where the target uid came from** from the wrapper into the transaction and out into the audit record. Today the subcommand name implies the answer. After this change it does not, and an implication that has stopped being true is the exact failure this design is built to prevent.

---

## 0. Verification status — what was proven and what was not

This executor had no shell. Nothing below is reported as observed that was not observed; every unverified claim is converted into a named gate with an exact command and a pre-agreed fallback, as M2's design did.

### Verified from repository contents (read directly)

| Claim | Evidence |
|---|---|
| `SystemRoot` = real uid 0 **and** `PKEXEC_UID` unset/empty, and is reachable only from `Cmd::Expire` | `uid.rs:80-88` |
| `admit_uid` rejects uid 0 unconditionally, then range, then `getpwuid` | `checks.rs:87-102` |
| `enable_inner` calls `admit_uid` as its **first** step, before the sudoer probe, before the lock, before any write | `ops.rs:211-215` |
| `disable_inner` runs **no** admission at all, deliberately | `ops.rs:338-344` doc comment |
| `status_inner` never returns an error and takes no lock | `ops.rs:427-443` |
| `AuditRecord` has exactly seven fields and no invocation-context field | `journal.rs:127-135` |
| `AuditEvent::Status` exists and is emitted from **nowhere** | `journal.rs:63-74` doc comment |
| `AuditRecord` has no consumer outside `crates/nopass-helper/src` | repo-wide grep for `AuditRecord`: `ops.rs`, `journal.rs`, `error.rs` (doc comment) only |
| The uid range comes from `/etc/login.defs` parsed per invocation | `ops.rs:125-126` |
| The root container lanes have **no systemd and no journald**, deliberately | `Containerfile.debian:5-10`, `tests/containers/README.md` §"Why the root containers don't run systemd" |
| `root_system.rs` executed in **no gate** during M2 | `m2-tray/verify-report.md:93` — "skipped in all five gates" |
| A test gated on an env var and left out of its lane's runner "executed in no gate at all" | `m2-tray/specs/tray-privileged-invocation/spec.md:92` |

### Not executed here — named gates

**G1 — a standalone `systemd-journald` runs in a container with no PID 1 systemd.** Required by three `helper-observability` scenarios that demand a real journald read-back.

> **RESOLVED by the orchestrator after this design was written. The answer is better than this design assumed.**
>
> On a plain `debian:12-slim`, with `apt-get install --no-install-recommends systemd`, `mkdir -p /run/systemd/journal /var/log/journal` and `/usr/lib/systemd/systemd-journald &`, journald starts and stays alive with **no PID 1 systemd, no `--privileged`, and no cgroup mount** — and on **Docker**, not podman, which matters because podman is not installed on the development machine at all.
>
> Verified in two steps rather than one, because the first only proves the easy half:
> 1. `systemd-cat -t nopass-helper` writes and `journalctl -t nopass-helper` reads it back, as does `journalctl SYSLOG_IDENTIFIER=nopass-helper`.
> 2. The half this design actually bets on: a datagram in the native protocol to `/run/systemd/journal/socket` carrying `NOPASS_CONTEXT=SystemRoot`, `NOPASS_UID=1000` and `NOPASS_OUTCOME=granted` is indexed, and **`journalctl NOPASS_CONTEXT=SystemRoot` matches it by that custom field**, returning all three in `-o json`. With a negative control: `journalctl NOPASS_CONTEXT=Pkexec`, a value never written, returns nothing.
>
> So the `CONTEXT` field decision below is not merely writable — it is queryable in exactly the shape `helper-observability` requires, which is the property that made a separate field worth adding at all.
>
> One cosmetic caveat to expect in lane output: journald prints `Failed to join audit multicast group … Ignoring` on start. It is non-fatal and the daemon runs; the lane script should not treat stderr as failure.
>
> **The fallback ladder below is not needed.** The lane's cost is one `apt-get install systemd python3` on a slim base.

```bash
podman build -f tests/containers/Containerfile.journald -t nopass-test-journald .
podman run --rm -e NOPASS_JOURNAL_TESTS=1 nopass-test-journald bash scripts/run-lane-journal.sh
# MUST exit 0, and journalctl -t nopass-helper MUST show NOPASS_CONTEXT=SystemRoot
```

Pre-agreed fallback, do not improvise during apply:

| Failure | Fallback | Cost |
|---|---|---|
| `systemd-journald` refuses to start without PID 1 / cgroup access | Move `tests/root_journal.rs` onto `Containerfile.systemd` (a real systemd, `--systemd=always --privileged`), driven by the same `scripts/run-lane-journal.sh` | The lane needs `--privileged`, so it is an on-demand named gate recorded in the verify report, not a per-commit gate |
| Neither works in the reviewer's environment | The three journald scenarios are recorded **UNPROVEN** in the verify report with the exact command that would prove them | Never marked green. M2's discipline, applied |

**G2 — `Containerfile.debian`/`Containerfile.fedora` still produce a real exit-17 rollback after G1 lands.** The journald lane is a *new fourth image*, precisely so the absence of systemd in the existing two — which is load-bearing for `real_timer_scheduling_failure_rolls_back_the_rule_when_systemd_is_unavailable` — is not disturbed. Gate: `bash scripts/run-lane-root.sh` exits 0 and that test still reports as executed, not skipped.

---

## 1. CLI surface (`cli.rs`)

Three flat variants appended to `Cmd`. No `admin` dispatch verb: the closed surface is what makes clap exit 2 before any privileged handler runs (`cli.rs:1-6`; `helper-cli` §Fixed Subcommand and Flag Surface).

```rust
#[command(group(ArgGroup::new("grant_when").multiple(false).args(["until", "until_reboot"])))]
Grant   { #[arg(long, required = true)] uid: u32,
          #[arg(long)] until: Option<u64>,
          #[arg(long = "until-reboot")] until_reboot: bool },
Revoke  { #[arg(long, required = true)] uid: u32 },
Inspect { #[arg(long, required = true)] uid: u32 },
```

`uid: u32` (not `Option<u32>`) plus `required = true` makes the flag's absence a clap error, so `--uid` is unwrapped by the type system rather than by an `expect` — unlike `Expire`, whose `Option<u32>` exists only because `--boot` is an alternative. A separate `ArgGroup` name is required: group names are global, so reusing `when` would collide with `Enable`'s.

## 2. The `ops.rs` reuse seam — where the uid comes from

New module `crates/nopass-helper/src/subject.rs`, in its own file so the seam is one `grep` away.

```rust
/// How this transaction's target uid was obtained. Derived from the
/// resolved InvocationContext, NEVER from the subcommand name — the name
/// stopped implying the answer the moment `grant` and `enable` could both
/// produce a rule for the same uid.
pub enum UidSource { Pkexec, SystemRoot }

/// The identity a transaction acts on and the authority it acts under.
/// Fields are private: the two constructors below are the only ways to
/// build one.
pub struct Subject { uid: u32, source: UidSource, event: AuditEvent }

impl Subject {
    /// Self-targeted. NOTE THE ABSENT UID PARAMETER: the uid can only be
    /// unpacked from `InvocationContext::Pkexec`, so a caller physically
    /// cannot supply one. This is what keeps enable/disable/status
    /// permanently self-targeted, at the type level rather than by test.
    pub fn pkexec(ctx: InvocationContext, event: AuditEvent) -> Result<Self, HelperError>;

    /// Root-supplied explicit target. THE ONLY FUNCTION IN THE CRATE that
    /// places a caller-supplied uid into a Subject. Refuses any context
    /// other than SystemRoot.
    pub fn root_target(ctx: InvocationContext, uid: u32, event: AuditEvent) -> Result<Self, HelperError>;

    pub fn uid(&self) -> u32;
    pub fn source(&self) -> UidSource;
    pub fn event(&self) -> AuditEvent;
}
```

`*_inner` signatures change from `uid: u32` to `subject: Subject`. Everything downstream (`fileops`, `timer`, `statefile`, `checks::admit_uid`, `layout.rule_path`) keeps taking a bare `u32` from `subject.uid()`: those are uid consumers, not authority consumers, and widening their signatures would spread the concept without adding a guarantee.

Both constructors return `Result` rather than `unreachable!()`: a mismatched pairing introduced by a future refactor fails closed as `HelperError::Context` (exit 10, no write) instead of panicking with exit 101 — a code in no table the tray or an operator can read.

`uid::resolve`'s `Cmd::Expire { .. }` arm becomes `Cmd::Expire { .. } | Cmd::Grant { .. } | Cmd::Revoke { .. } | Cmd::Inspect { .. }`, with its two `&'static str` messages generalized from "expire" to "this subcommand". Existing tests assert `exit_code() == 10`, never the message text, so they stay green unchanged.

### What each new wrapper is, in full

```rust
pub fn grant(layout, runner, binaries, uid_flag, until, until_reboot) -> Result<(), HelperError> {
    let cmd = Cmd::Grant { uid: uid_flag, until, until_reboot };
    let ctx = uid::resolve(&cmd, std::env::var("PKEXEC_UID").ok().as_deref(),
                           nix::unistd::getuid().as_raw())?;          // exit 10
    let subject = Subject::root_target(ctx, uid_flag, AuditEvent::Grant)?;
    let expiry  = resolve_expiry_audited(subject, until, until_reboot, unix_now())?;  // exit 13
    admit_root_target(subject, &uid_range)?;                          // exit 11, §3
    let raw_user = /* lookup_user_optional, same branch enable uses */;
    enable_inner(layout, runner, binaries, subject, &raw_user, &uid_range, expiry)
}
```

`revoke` is the same shape ending in `disable_inner(layout, runner, binaries, subject)`; `inspect` ends in `status_inner(layout, subject.uid(), unix_now())` plus an audit record. **No new transaction body exists anywhere in this change.** `enable_inner`, `disable_inner`, `status_inner`, `rollback_rule`, `audit_rejection`, `resolve_expiry` are edited only to carry `Subject` instead of `u32`; their step order, rollback table and lock discipline are untouched.

## 3. The widening, and its bound in code

A root-invoked `grant` may name any uid; a pkexec `enable` may name none. `admit_uid` (`checks.rs:87-102`) is the bound.

**On the path, by construction for `grant`:** `enable_inner`'s *first* step is `checks::admit_uid` (`ops.rs:211-215`), before the sudoer probe, before `LockGuard::acquire`, before any write. `grant` reuses that function unmodified, so the bound cannot be bypassed without deleting it from the path `enable` also uses — which every existing `enable` admission test would catch.

**`revoke` and `inspect` need an explicit admission, and this is the one real tension in the change.** `disable_inner` deliberately runs *no* admission ("removing a privilege must never be blocked, and a deleted account must still be revocable", `ops.rs:338-344`), and `status_inner` runs none either. But `privilege-admission` §SystemRoot Context Can Target Any Admitted UID requires all three SystemRoot subcommands to exit 11 for uid 0, out-of-range, or non-existent targets.

**Decision:** admission for the SystemRoot path lives in **one shared wrapper-level function**, `ops::admit_root_target(subject, &uid_range) -> Result<(), HelperError>`, called by all three new wrappers before they enter the transaction. It audits its own rejection with the correct event and `SystemRoot` context. `disable` and `status` on the pkexec path are **not** touched, so M1's "revocation is never blocked" property and every existing test survive exactly as written.

Consequence, stated plainly: a root `revoke --uid 0` exits 11 instead of removing a file. Root can still `rm /etc/sudoers.d/90-nopass-*` directly, `expire --boot` still sweeps, and `prerm` still purges — so nothing becomes unrecoverable. The alternative (SystemRoot revoke bypassing admission) was rejected: it would make `--uid` a primitive for naming targets the grant path can never name, which is the widening the proposal explicitly bounded.

`grant` therefore calls `admit_uid` twice — once in the wrapper, once inside `enable_inner`. Deliberate, and kept: removing the second call would move the bound out of the shared transaction, which is precisely the property §3 exists to assert. The second call is pure, has no side effect, and can only ever agree with the first.

**Range identity.** The `SystemRoot` range is the **same** range as the pkexec range: the same `/etc/login.defs` parse (`ops.rs:125-126`), no override flag, no `--force`, no root-only widening. Rationale: the delta says this "widens WHO may be named as a target, not WHAT bounds a target uid". A root-only range would be a second admission policy to keep in sync with the first — exactly the drift `admit_uid` was extracted to prevent (`checks.rs:84-86`).

## 4. Audit schema growth — one new field

`AuditRecord` gains exactly one field:

```rust
pub struct AuditRecord<'a> {
    pub event: AuditEvent,
    pub context: UidSource,   // ← new; NOPASS_CONTEXT after the journald prefix
    pub uid: u32, pub user: &'a str, pub outcome: AuditOutcome,
    pub expires: Expiry, pub exit: i32, pub reason: &'a str,
}
```

Value domain: the literal strings `Pkexec` and `SystemRoot` — the `InvocationContext` variant names the delta spec itself uses, so `journalctl NOPASS_CONTEXT=SystemRoot` matches the spec byte-for-byte. This breaks `OUTCOME`'s snake_case convention knowingly: an auditor greps what the requirement says, not what a neighbouring field's casing suggests.

**No second field for the target.** `UID` already *is* the target uid in every record the helper emits — `enable`'s uid is its target, `expire`'s uid is its target. A separate `TARGET` field would duplicate `UID` and create two places for them to disagree. `CONTEXT` answers the only question that was previously unanswerable: how that uid was chosen.

**`context` is derived from `Subject`, never from `event`.** `journal::audit` reads `subject.source()`. A future subcommand cannot mislabel its context, because the only two ways to obtain a `Subject` each fix the source from the resolved `InvocationContext`.

### What an existing consumer sees

| Consumer | Before | After |
|---|---|---|
| `journalctl -t nopass-helper` | seven `NOPASS_*` fields | eight. All seven keep their names, value domains and meanings byte-for-byte |
| A filter, e.g. `journalctl NOPASS_EVENT=enable NOPASS_OUTCOME=ok` | matches | matches, unchanged. journald fields are an unordered set; an unknown key is invisible to a reader keyed on known keys |
| PRD checklist line 329 ("cada activación aparece en `journalctl -t nopass-helper`") | true | still true |
| The stderr fallback layer's text | `EVENT="enable" UID=1000 …` | gains ` CONTEXT="Pkexec"`. Its only two consumers are `journal.rs`'s own tests, which use `contains`, not line equality — both survive and each gains a `CONTEXT` assertion |
| The `AuditRecord` struct literal | 7 fields | 8. Every construction site must add one; **all of them are in `ops.rs` and `journal.rs`'s tests** — a repo-wide grep finds no consumer outside `crates/nopass-helper/src` |
| The tray (`crates/nopass`) | reads the state file and `sudo -kn true`, never journald | unaffected. `HelperStatus` schema stays at `1` |

**No audit-schema version field is introduced.** `HelperStatus` has `schema: 1` because it is a parsed wire type where an unknown shape must be *rejected*; a journald record is a key/value set where an unknown key is simply not read. A version field would imply a migration obligation that purely additive growth does not create. The boundary is explicit: a future *removal*, *rename*, or *value-domain change* to any of the seven existing fields needs a versioning decision; this addition does not.

**Growth that closes the omission hole.** `helper-observability`'s "unaudited-by-omission" scenario is answered at compile time, not by a test that could be forgotten: a single exhaustive `fn audit_event_for(cmd: &Cmd) -> AuditEvent` with **no wildcard arm** means an eighth subcommand that ships without an `AuditEvent` fails to compile. A runtime test additionally asserts the map is total and injective over the seven.

`AuditEvent` gains `Grant`, `Revoke`, `Inspect` (strings `grant`, `revoke`, `inspect`). `AuditEvent::Status` — present since M1 and emitted from nowhere (`journal.rs:63-67`) — is finally wired: `status` and `inspect` both journal their outcome, because the modified requirement binds by "every outcome-producing subcommand", not by a name list. `journal.rs`'s stale "scopes `journal::audit` calls to enable/disable/expire only" comment must be corrected in the same commit.

`expire` also gains `CONTEXT=SystemRoot`, via the same `Subject::root_target` seam. It is a `SystemRoot` consumer and the delta names it; routing the pre-existing consumer through the new seam is what makes the seam the *only* path rather than a second one.

## 5. `inspect` versus `status`

| | `status` | `inspect --uid N` |
|---|---|---|
| Context | `Pkexec` | `SystemRoot` |
| Target | `PKEXEC_UID` | explicit `--uid` |
| Implementation | `status_inner(layout, uid, now)` | **the same `status_inner`** |
| Output | `HelperStatus::to_json_line()` | **byte-identical shape** |
| Admission | none (unchanged) | `admit_root_target` → exit 11 |
| Audit | `AuditEvent::Status`, `CONTEXT=Pkexec` | `AuditEvent::Inspect`, `CONTEXT=SystemRoot` |

**Decision: identical JSON, one implementation.** The `HelperStatus` shape is a contract with a golden test (`nopass-core/src/state.rs:73-85`) and a live consumer (the tray's state reader). A second output shape would fork that contract into a second thing to keep in sync, and hand operators a parser that disagrees with the tray's.

**Rejected: human-readable text for `inspect`.** It invents a second contract with no consumer and no golden test, and a script that today does `nopass-helper status | jq` could not be pointed at `inspect`. Operators have `jq`; the machine-readable form is strictly more useful and strictly cheaper.

`inspect` on a uid with no rule prints `"active":false` and exits **0**, exactly as `status` does. Absence is a fact, not a failure.

## 6. Exit codes — no new codes

| Code | `grant` | `revoke` | `inspect` | Source |
|---|---|---|---|---|
| 0 | granted | revoked | printed | — |
| 1 | ✓ | ✓ | — | `Internal` / `BinaryMissing` |
| 2 | ✓ | ✓ | ✓ | clap: missing `--uid`, unknown flag, both duration flags, `admin …` |
| 10 | ✓ | ✓ | ✓ | `uid::resolve` — non-root, or `PKEXEC_UID` set |
| 11 | ✓ | ✓ | ✓ | `admit_root_target`; for `grant` also `enable_inner` step 5 |
| 12 | ✓ | — | — | `checks::is_sudoer` (`enable_inner` step 6) |
| 13 | ✓ | — | — | `validate_until` |
| 14 | ✓ | — | — | `visudo -cf` |
| 15 | ✓ | ✓ | — | `LockGuard` — `status_inner` takes no lock |
| 16 | ✓ | ✓ | — | atomic write / unlink |
| 17 | ✓ | — | — | `systemd-run`, `--until` only |

No genuinely new failure exists: every way these three can fail is a way `enable`/`disable`/`status` can already fail. Inventing a code would create a value the tray's M2 outcome table (`m2-tray/design.md` §5) does not render.

## 7. Sequence — `grant` (config rule: privileged flows get a diagram)

```
operator (already uid 0, no session, no DISPLAY, no DBUS_SESSION_BUS_ADDRESS)
  │ /usr/libexec/nopass-helper grant --uid 1000 --until <epoch>
  ▼
clap ─ undeclared subcommand/flag, or missing --uid ────────────▶ exit 2, no handler
  │
uid::resolve ─ PKEXEC_UID set, or real uid ≠ 0 ─────────────────▶ exit 10, no write
  │ InvocationContext::SystemRoot
Subject::root_target(ctx, 1000, AuditEvent::Grant)   ← THE SEAM
  │ Subject { uid: 1000, source: SystemRoot, event: Grant }
resolve_expiry_audited ─ bad --until ───────────────────────────▶ exit 13 + audit
admit_root_target ─ uid 0 / out of range / no passwd entry ─────▶ exit 11 + audit
  ▼
enable_inner(subject, …)              ← UNCHANGED M1 TRANSACTION
  │ 5 admit_uid (again, deliberately)  ─▶ 11
  │ 6 is_sudoer                        ─▶ 12
  │ 7 flock                            ─▶ 15
  │ 8-13 tmp write + visudo -cf + fsync + atomic rename ─▶ 14 / 16
  │ 14 timer::stop  · 15 timer::schedule ─▶ rollback ─▶ 17
  │ 16 state file (failure logged, exit stays 0)
  │ 17 journal::audit { EVENT=grant, CONTEXT=SystemRoot, UID=1000, … }
  ▼ exit 0
```

Nothing on this path opens a bus, reads `DISPLAY`, or consults the tray — `headless-operation` §No Desktop Session Required holds because no code exists that could violate it, and the root-only test reproduces the absent session with `env_clear` rather than by finding a machine that happens to lack one.

## 8. Testing strategy, per lane — and how the root-only lane actually executes

The specs name commands, not lane letters. Placement:

| Spec scenarios | Command named | Where it lands |
|---|---|---|
| All 8 `helper-cli` scenarios | `cargo test` | `cli.rs` unit tests |
| 8 of 9 `privilege-admission` scenarios | `cargo test` | `uid.rs`, new `subject.rs`, extended `main.rs::dispatch_routes_every_subcommand…` loop |
| "grant/revoke/inspect rejected for a non-root caller" | `cargo test` | the suite already runs non-root with no `PKEXEC_UID` — that is the scenario's precondition, exactly as the delta notes |
| "unaudited-by-omission" + `CONTEXT` field mapping | `cargo test` | `journal.rs` + the wildcard-free `audit_event_for` match |
| 3 journald read-back scenarios | root-only container test | **new** `tests/root_journal.rs`, lane R-J (below) |
| 2 `headless-operation` transaction scenarios | root-only container test | `tests/root_system.rs`, lane R |
| 2 `docs/` scenarios | `cargo test` | **new** `tests/docs_headless.rs` |

### The execution gap, named and closed

`crates/nopass-helper/tests/root_system.rs` is gated on `NOPASS_ROOT_TESTS=1` **and** `geteuid().is_root()`. M2's verify report recorded it as **"skipped in all five gates"** (`verify-report.md:93`) while reporting `ok` for all 15 of its tests. Its lane exists only as a `podman` recipe in `tests/containers/README.md` that a human must retype. That is the same shape as M2's `reactor_responsiveness.rs` finding — "gated on the lane B environment variable and then left out of that lane's runner, so it executed in no gate at all". Adding assertions to `root_system.rs` and stopping there would repeat it exactly.

Three changes, together:

1. **`scripts/run-lane-root.sh`** — new, mirroring `scripts/run-lane-b.sh`. Builds `Containerfile.debian` and `Containerfile.fedora` and runs `podman run --rm -e NOPASS_ROOT_TESTS=1 <img> cargo test --workspace` for each, `set -euo pipefail`. The root lane becomes **one named command** that a verify report can list as a gate with an exit status, instead of prose a reader is trusted to have followed.
2. **`scripts/run-lane-journal.sh` + `tests/containers/Containerfile.journald`** — the journald read-back lane (G1 above, with its fallback). It is a *fourth* image on purpose: `Containerfile.debian`/`.fedora` must keep having no systemd, because that absence is what produces the genuine exit-17 rollback (G2).
3. **`crates/nopass-helper/tests/lane_wiring.rs`** — the structural guard that makes the class of failure impossible, running in the always-on `cargo test` lane. Dependency-free, same shape as `data_artifacts.rs`'s repo-file scans:
   - scan every `crates/*/tests/*.rs` for env-gate names matching `NOPASS_[A-Z_]+_TESTS`;
   - assert each name appears in at least one script under `scripts/`;
   - assert each such script is named in `tests/containers/README.md`'s checklist.

   A gated test file that no runner script names **fails `cargo test`**. That is the answer to "how do these actually get executed": not by a promise in a README, but by a guard in the lane that always runs.

`tests/containers/README.md` gains the two new script invocations in its Quick path and two checklist lines; its per-lane table gains the journald row. `openspec/config.yaml`'s `rules.verify.gate_commands` gains `scripts/run-lane-root.sh` and `scripts/run-lane-journal.sh`, G1 having resolved, so `sdd-verify` runs them rather than reading about them.

### The `docs/` assertions

`docs/` is Spanish (`docs/PRD_NoPass_Linux.md`). The new operator section follows that register; the SDD artifacts stay English. The guard therefore asserts only **language-neutral tokens** — `grant`, `revoke`, `inspect`, `--uid`, `allow_inactive=no`, `PKEXEC_UID` — which are identifiers in any prose language, not translatable sentences. A prose assertion would be brittle and would have to be rewritten the first time a paragraph is reworded.

The forgery guard is a rule, not a keyword ban: **every line in `docs/**.md` containing the literal `PKEXEC_UID=` must also contain the literal `unsupported`** (or its Spanish counterpart, fixed as a constant in the test). The workaround must stay nameable — operators are already using it — but it can never appear unlabelled.

### Unit-level additions

`subject.rs`: `pkexec` rejects a `SystemRoot` context and vice versa (exit 10, not a panic); the type has no third constructor; `Subject::pkexec` has no uid parameter, so `enable` is self-targeted at the type level. `ops.rs`: `admit_root_target` table over uid 0 / below min / above max / no passwd entry, each exit 11 with a distinct `audit_reason`. `journal.rs`: `CONTEXT` present with the right value in a `Pkexec` and a `SystemRoot` record.

## 9. Threat matrix

The generic VCS-oriented matrix is recorded row-by-row, then replaced with the boundary this change actually crosses.

| Boundary | Applicability | Reason |
|---|---|---|
| Documentation-like paths | **N/A** | `docs/**.md` is read by a test as data and never executed; nothing in this change classifies or runs a file by extension |
| Git repository selection | **N/A** | no VCS operation |
| Commit state | **N/A** | no VCS operation |
| Push state | **N/A** | no VCS operation |
| PR commands | **N/A** | no VCS/PR automation |

| Privilege boundary (this change's real matrix) | Adversarial case | Design response | Planned RED test |
|---|---|---|---|
| Invocation context | non-root caller; `PKEXEC_UID` set to anything; `PKEXEC_UID` empty | `uid::resolve`, unchanged logic, extended arm — no filesystem or process capability is in scope for it, so zero-mutation holds by construction | exit 10 for each of the three subcommands × each combination (`cargo test`) |
| Arbitrary-uid targeting | `--uid 0`, below `UID_MIN`, above `UID_MAX`, non-existent uid | `admit_root_target` before the transaction; `admit_uid` again inside it for `grant` | exit 11 + no write, per subcommand per cause |
| Authority confusion in the audit trail | a root-supplied uid recorded as if the caller had proved something to polkit | `context` derived from `Subject`, never from `event`; `Subject` has exactly two constructors | a `Pkexec` record and a `SystemRoot` record for the *same* uid are distinguishable on `CONTEXT` alone |
| Unaudited new subcommand | an eighth subcommand ships with no audit call site | wildcard-free `audit_event_for` match | compile failure, plus a totality/injectivity test |
| Subprocess argv | `--uid` reaching `visudo`/`sudo` argv or a shell | unchanged: `enable_inner` → `render_rule`/`sanitize_username`; no shell anywhere; `u32` cannot carry a metacharacter | existing `enable` argv assertions cover it; `grant` adds no new subprocess call site |
| Untested test lane | a root-only assertion that runs in no gate | `lane_wiring.rs` + two runner scripts | the guard fails `cargo test` when a gated file has no runner |

## 10. File changes

| File | Action | Description |
|---|---|---|
| `crates/nopass-helper/src/subject.rs` | Create | `Subject`, `UidSource`, two constructors — the seam |
| `crates/nopass-helper/src/cli.rs` | Modify | `Grant`/`Revoke`/`Inspect` variants + parse tests |
| `crates/nopass-helper/src/uid.rs` | Modify | `SystemRoot` arm covers four subcommands; messages generalized |
| `crates/nopass-helper/src/ops.rs` | Modify | 3 wrappers + `admit_root_target`; `*_inner` take `Subject`; `expire` routed through the seam |
| `crates/nopass-helper/src/journal.rs` | Modify | `context` field, 3 new `AuditEvent`s, `Status` wired, stale comment corrected |
| `crates/nopass-helper/src/main.rs` | Modify | dispatch arms; extended non-root dispatch test |
| `crates/nopass-helper/src/lib.rs` | Modify | `pub mod subject;` |
| `crates/nopass-helper/src/checks.rs` | **Unchanged** | `admit_uid` consumed as-is — the bound is reused, not reimplemented |
| `crates/nopass-helper/tests/docs_headless.rs` | Create | the two `docs/` structural assertions |
| `crates/nopass-helper/tests/lane_wiring.rs` | Create | every gated test file is named by a runner script |
| `crates/nopass-helper/tests/root_system.rs` | Modify | the two `headless-operation` transaction scenarios |
| `crates/nopass-helper/tests/root_journal.rs` | Create | the three journald read-back scenarios (lane R-J) |
| `tests/containers/Containerfile.journald` | Create | standalone journald image (G1) |
| `scripts/run-lane-root.sh`, `scripts/run-lane-journal.sh` | Create | the two lanes as named commands |
| `tests/containers/README.md` | Modify | quick path, lane table, checklist |
| `docs/headless.md` (+ PRD cross-link) | Create | operator section; desktop assumption; `PKEXEC_UID=` named as unsupported |
| `openspec/config.yaml` | Modify | `rules.verify.gate_commands` gains the root lane |
| `data/com.enfoquestic.nopass.policy`, `crates/nopass/`, `crates/nopass-core/`, `debian/` | **Unchanged** | the desktop path and the packaging split are untouched |

## Migration / Rollout

No migration. Purely additive: no on-disk format, rule-file format, `HelperStatus` schema, timer unit name or polkit action changes. A rule written by `grant` is byte-identical to one written by `enable` and is revoked by `disable`, by its timer, by `expire --boot`, or by `prerm`. Reverting the slice commits removes the subcommands; the audit trail loses `NOPASS_CONTEXT` and its consumers see the M1 seven-field record again.

## Open Questions

- [x] **G1 (journald without PID 1)** — PROVEN by the orchestrator, including the custom-field query with a negative control, on Docker without `--privileged`. See §G1. The fallback ladder is not needed.
- [ ] `status` becomes audited for the first time. Correct under the modified requirement's "every outcome" binding, and it finally wires `AuditEvent::Status`, but it is a behaviour change to an existing subcommand — flagged for verify rather than assumed harmless.

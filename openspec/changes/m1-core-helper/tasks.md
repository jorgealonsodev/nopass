# Tasks: M1 — Core + Privileged Helper (`nopass-core` + `nopass-helper` + install data)

Source: `proposal.md`, `design.md` (§1–§9, Architecture Decisions, File Changes, Threat Matrix), `specs/{sudoers-rule-lifecycle,privilege-admission,expiry-policy,helper-cli,helper-observability}`. Ten phases below map 1:1 onto the proposal's ten delivery slices. TDD: every behavior task lists its RED test(s) before its GREEN implementation step; REFACTOR is left to each slice's discretion after GREEN and is not itemized separately.

## Review Workload Forecast

| Field | Value |
|---|---|
| Estimated changed lines (authored, per slice) | 1: ~180 · 2: ~420 · 3: ~380 · 4: ~430 · 5: ~260 · 6: ~480 · 7: ~560 · 8: ~260 · 9: ~200 · 10: ~320 |
| Estimated total | ~3,490 authored lines (`Cargo.lock` generated, excluded) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | Originally 13 work-unit PRs: slices 2, 4, 6, 7 each split into two (2a/2b, 4a/4b, 6a/6b, 7a/7b); slices 1, 3, 5, 8, 9, 10 ship as single PRs. **Revised to 14 after Phase 4 landed:** Phase 4 authored 732 lines against a ~430 forecast, so the original unit 4b (`runner` + `bins` + `main`, 502 lines) exceeded the 400-line budget on its own and is split again into 4b (`runner.rs`, 273 lines) and 4c (`bins.rs` + `main.rs`, 229 lines). Both are now inside budget. **Revised to 17 after Phase 7 landed:** Phase 7 authored 1,265 lines against a ~560 forecast — the highest overrun of any phase — because every rollback-table row (exit 13/11/12/14/16/17) needed its own pinned `ScriptedRunner` RED test, and `ops.rs`'s four transactions share one test harness (`fresh_layout`/`fake_binaries`/spec builders, ~122 lines). The original 7a (`timer` + `ops::{enable,disable,status}`, would have been ~894 lines) is split three ways: 7a-i `timer.rs` alone (196 lines, zero `ops.rs` dependency), 7a-ii `ops::enable` + the shared test harness (~496 lines — still over budget on its own because it carries the harness's one-time cost; a maintainer preferring a strict 400-line ceiling could land the harness as its own thin scaffolding commit first), 7a-iii `ops::disable`/`ops::status`/`resolve_username` reusing that harness (~208 lines). 7b (`ops::expire`, both paths) stays a single PR as planned (~267 lines). The `main.rs`/`lib.rs` dispatch-wiring diff (98 lines) is bundled with 7b, the last-landing unit, since it completes full dispatch wiring across all four subcommands in one match statement. |
| Delivery strategy | auto-chain |
| Chain strategy | stacked-to-main |

```text
Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High
```

Rationale for stacked-to-main: the repository is greenfield (no deployed users), each slice is independently `git revert`-able per the proposal's rollback plan, and slices 4/6/7 gate on real privilege-boundary behavior that benefits from landing on `main` as soon as its own tests are green rather than waiting behind a long-lived tracker branch. Slices 2, 4, 6, 7 are pre-split into `a`/`b` PRs below because their single-slice estimate exceeds 400 lines after one honest slicing pass; no comments/tests/docs were trimmed to hit the budget.

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | Workspace skeleton + `strict_tdd` promotion | PR 1 | `cargo test --workspace` | N/A — no runtime behavior yet | delete `Cargo.toml`, `crates/`, revert `openspec/config.yaml` |
| 2a | `nopass-core::template` + `header` | PR 2a | `cargo test -p nopass-core template:: header::` | N/A — pure logic | delete `template.rs`, `header.rs` |
| 2b | `nopass-core::expiry` | PR 2b | `cargo test -p nopass-core expiry::` | N/A — pure logic | delete `expiry.rs` |
| 3 | `nopass-core::{paths,state,logindefs,timefmt}` | PR 3 | `cargo test -p nopass-core` | N/A — pure logic | delete the four modules |
| 4a | `cli` + `error` (exit-code mapping) | PR 4a | `cargo test -p nopass-helper cli:: error::` | N/A — parser/mapping only | delete `cli.rs`, `error.rs` |
| 4b | `runner` (`CommandRunner`/`SystemRunner`/`ScriptedRunner`) | PR 4b | `cargo test -p nopass-helper runner::` | spawns real short-lived children to prove env clearing and signal death | delete `runner.rs` |
| 4c | `bins` + `main.rs` dispatch wiring | PR 4c | `cargo test -p nopass-helper bins::` | N/A — candidate-path resolution only | delete `bins.rs`; revert `main.rs` to the Phase 1 stub |
| 5a | `uid` (invocation-context resolution) | PR 5a | `cargo test -p nopass-helper uid::` | N/A — injected uid params, no process spawned | delete `uid.rs` |
| 5b | `checks` (admission + sudoer probe) + `error` payload widening | PR 5b | `cargo test -p nopass-helper checks:: error::` | N/A — `ScriptedRunner`-only sudo probe | delete `checks.rs`; revert `UidRejected` to a unit variant; revert `main.rs`/`Cargo.toml` |
| 6a | `lock` (flock) | PR 6a | `cargo test -p nopass-helper lock::` | N/A — unprivileged `TempDir` flock | delete `lock.rs` |
| 6b | `fileops` + the `lib.rs` library target and the thinned `main.rs` | PR 6b | `cargo test -p nopass-helper fileops::` | N/A at this unit | delete `fileops.rs`; revert `main.rs` to its own `mod` tree and delete `lib.rs` |
| 6c | `tests/fileops_tempdir.rs`, the unprivileged integration suite | PR 6c | `cargo test -p nopass-helper --test fileops_tempdir` | `podman run … cargo test --workspace` (Debian/Fedora root lane, optional here) | delete `tests/fileops_tempdir.rs` |
| 7a-i | `timer.rs` alone (196 authored lines) | PR 7a-i | `cargo test -p nopass-helper timer::` | N/A — `ScriptedRunner` argv assertions | delete `timer.rs` |
| 7a-ii | `ops::enable` + shared `ops.rs` test harness (~496 authored lines — still over budget on its own; the harness could land as a separate thin scaffolding commit first if a strict 400-line ceiling is required) | PR 7a-ii | `cargo test -p nopass-helper ops::enable` | N/A — `ScriptedRunner` argv/rollback assertions | revert `ops.rs` to its pre-Phase-7 absence |
| 7a-iii | `ops::disable` + `ops::status` + `ops::resolve_username`, reusing 7a-ii's harness (~208 authored lines) | PR 7a-iii | `cargo test -p nopass-helper ops::disable ops::status ops::resolve_username` | N/A — `ScriptedRunner` argv assertions | revert `ops.rs` to its 7a-ii state |
| 7b | `ops::expire` (uid + boot), reusing 7a-ii's harness (~267 authored lines), bundled with the `main.rs`/`lib.rs` dispatch-wiring diff (98 lines) since it completes wiring across all four subcommands | PR 7b | `cargo test -p nopass-helper ops::expire` | N/A — boot sweep runs against `Layout::under(TempDir)` | revert `ops.rs` to its 7a-iii state; revert `main.rs`/`lib.rs` dispatch wiring |
| 8 | `statefile` + `journal`, wired into `ops` | PR 8 | `cargo test -p nopass-helper statefile:: journal::` | N/A — journald fallback path exercised via stderr layer | delete `statefile.rs`, `journal.rs`; revert `ops.rs` wiring |
| 9 | `data/` install artifacts | PR 9 | `cargo test -p nopass-helper --test data_artifacts` | N/A — static file assertions | delete the three `data/` files and `data_artifacts.rs` |
| 10 | Container test lane | PR 10 | `podman build -f tests/containers/Containerfile.debian … && podman run -e NOPASS_ROOT_TESTS=1 … cargo test --workspace` | `podman build -f tests/containers/Containerfile.fedora …` (second distro) | delete `tests/containers/`, `root_system.rs` |

---

## Phase 1: Workspace Skeleton, Baseline Test Gate, `strict_tdd` Promotion

*(Design §1, §8 "Promotion for strict_tdd"; Proposal risk row "strict_tdd is currently false")*

- [x] 1.1 Create `Cargo.toml` — `[workspace]` resolver `"3"`, members `crates/nopass-core`, `crates/nopass-helper`; `workspace.package` (edition 2024, rust-version 1.85, license MIT); `workspace.dependencies` table per design §1; `[profile.release]` (`lto=true`, `codegen-units=1`, `strip=true`, `panic="unwind"`).
- [x] 1.2 Create `rust-toolchain.toml` pinning `channel = "1.85"`.
- [x] 1.3 Create `crates/nopass-core/Cargo.toml` (serde+derive, serde_json, thiserror; dev-dep serde_json) and `crates/nopass-core/src/lib.rs` — `#![forbid(unsafe_code)]`, empty module declarations only.
- [x] 1.4 Create `crates/nopass-helper/Cargo.toml` (nopass-core, serde_json, thiserror only) and `crates/nopass-helper/src/main.rs` — `#![forbid(unsafe_code)]`, stub `fn main() { std::process::exit(0) }`.
- [x] 1.5 Create `.gitignore` with `/target`.
- [x] 1.6 RED/GREEN: `cargo test --workspace` compiles and passes with zero tests (no behavior yet — this is the baseline gate itself).
- [x] 1.7 GREEN: `cargo build --release` succeeds under the pinned 1.85 toolchain and the release profile.
- [x] 1.8 Modify `openspec/config.yaml` — promote `strict_tdd: true`, `testing.strict_tdd_effective: true`, `rules.apply.tdd: true`, `rules.apply.test_command: "cargo test --workspace"`, `rules.verify.test_command`/`build_command` per design §8.
- [x] 1.9 Commit `Cargo.lock`. Landed in the initial commit `c306f34` together with phases 1-4. The work preceded the git history, and by the time this was closed `lib.rs` and `main.rs` had already moved past their phase-1 content, so work unit 1 could not be committed in isolation. Maintainer chose one initial commit; the chained work-unit plan applies from phase 5 onward.

## Phase 2: `nopass-core` — Template, Header, Expiry

*(Spec: `sudoers-rule-lifecycle`, `expiry-policy`; Design §2, §5)*

- [x] 2.1 RED `crates/nopass-core/src/template.rs`: golden for uid 1000/`jorge`/`At(1789000000)` (sudoers-rule-lifecycle §Rule File Naming and Content Format); `sanitize_username` table — `ana.perez` preserved verbatim, space-containing name stripped to `[A-Za-z0-9._-]`, shell metacharacters (`; | & $ \``) stripped before any `CommandSpec` is built (threat matrix: External command composition, username leg). GREEN: implement `BANNER`, `sanitize_username`, `render_rule`.
- [x] 2.2 RED `crates/nopass-core/src/header.rs`: banner/user/expires grammar parse ok/err table — `HeaderError::{MissingBanner,MissingUser,MissingExpires,MalformedUser,MalformedExpires}`; `is_nopass_owned` true/false cases. GREEN: implement `parse`, `is_nopass_owned` via `strip_prefix` (no `regex`).
- [x] 2.3 RED `crates/nopass-core/src/expiry.rs`: round-trip `Never`/`Reboot`/`At(1789000000)` (expiry-policy §Expiry Model and Header Encoding); unrecognized `nopass-expires: soon` → typed error, not a silent default; duration boundaries 59/60/28800/28801/past/future via `validate_until` (expiry-policy §Temporary Duration Validation). GREEN: implement `Expiry`, `is_expired`, `is_expired_at_boot`, `validate_until`, `DurationError`.

## Phase 3: `nopass-core` — Paths, `HelperStatus` JSON, `login.defs`, UTC Format

*(Spec: `helper-observability`; Design §2, §5)*

- [x] 3.1 RED `crates/nopass-core/src/paths.rs`: `Layout::system()`/`Layout::under(tmp)` builders for `rule_path`, `rule_tmp_path`, `state_path`, `state_tmp_path`, `lock_path`; `uid_from_rule_filename` accepts canonical `90-nopass-1000`, rejects `90-nopass-01000` and `90-nopass-1000.bak` (threat matrix: Rule-file target selection). GREEN: implement `Layout`, `RULE_PREFIX`, `HELPER_PATH`.
- [x] 3.2 RED `crates/nopass-core/src/state.rs`: exact-JSON golden for active `at`-expiry (helper-observability §HelperStatus JSON Contract, "Active temporary grant") and inactive (`active:false, expires:null`); round-trip. GREEN: implement `HelperStatus`, `SCHEMA_VERSION=1`, `active_from`, `inactive`, `to_json_line`, `Expiry` as `#[serde(tag="kind", rename_all="lowercase")]` **struct-variant** `At { epoch }` (Architecture Decision: struct variant, not tuple).
- [x] 3.3 RED `crates/nopass-core/src/logindefs.rs`: default admit; `UID_MIN 0` clamps to floor; missing/unparseable file clamps to floor 1000 (privilege-admission §UID Range Admission, "Malformed or missing login.defs"). GREEN: implement `UidRange::parse`, `admits`.
- [x] 3.4 RED `crates/nopass-core/src/timefmt.rs`: `format_utc_rfc3339` vectors — epoch 0, leap day, 2038, 1789000000. GREEN: implement the hand-rolled formatter (no `chrono`/`time`, per Architecture Decision).

## Phase 4: Helper CLI, `CommandRunner` Port, Error/Exit-Code Mapping, Binary Resolution

*(Spec: `helper-cli`; Design §3, §9)*

- [x] 4.1 Extend `crates/nopass-helper/Cargo.toml` with `clap` (derive). RED `crates/nopass-helper/src/cli.rs` via `Cli::try_parse_from`: unknown subcommand → exit 2; unknown flag on `enable` → exit 2; `--until` + `--until-reboot` together → exit 2; `--until-reboot` alone → `Reboot`; `expire` with neither `--uid` nor `--boot` → exit 2 (helper-cli §Fixed Subcommand and Flag Surface, §Mutually Exclusive Duration Flags). GREEN: implement `Cli`/`Cmd` exactly as design §2 (no `allow_external_subcommands`/`allow_hyphen_values`/`trailing_var_arg`).
- [x] 4.2 RED `crates/nopass-helper/src/error.rs`: exhaustive `HelperError → exit_code` table covering 1, 10–17 (helper-cli §Typed Exit Code Mapping). GREEN: implement `HelperError`, `exit_code`.
- [x] 4.3 RED `crates/nopass-helper/src/runner.rs`: `ScriptedRunner` pops front in order, asserts `CommandSpec` field-by-field equality, panics on unexhausted script in `Drop`. GREEN: implement `CommandSpec`, `Expect`, `CommandOutcome`, `CommandRunner`, `SystemRunner` (`env_clear`+`envs`, no shell, `status: None` treated as failure), `#[cfg(test)]` `ScriptedRunner`.
- [x] 4.4 RED `crates/nopass-helper/src/bins.rs`: first-existing-wins order for the `visudo` candidate list; a later candidate wins when the first is absent; an **empty** candidate list → `BinaryMissing` exit 1; laziness — resolving `systemd-run` is never attempted during `status` (helper-cli §External Command Invocation Discipline, "No candidate binary path exists"; unprivileged per design §8 lane reconciliation, overrides the spec's root-only label). GREEN: implement `Binaries::system()`, `Binaries::from_candidates`, `resolve`.
- [x] 4.5 GREEN: wire `crates/nopass-helper/src/main.rs` to parse `Cli` and dispatch to stub handlers returning `exit_code` (real handlers land in Phase 7).

## Phase 5: UID Resolution and Admission Checks

*(Spec: `privilege-admission`; Design §2, §4.1 steps 3/5/6)*

- [x] 5.1 RED `crates/nopass-helper/src/uid.rs`: `enable`/`disable`/`status` resolve uid from `PKEXEC_UID=1000`; `PKEXEC_UID` missing → exit 10, no write; `PKEXEC_UID=abc` → exit 10; `expire --uid` with `PKEXEC_UID` set → exit 10; `expire` invoked non-root without `PKEXEC_UID` (simulated via injected uid) → exit 10 (privilege-admission §UID Resolution by Invocation Context; threat matrix "Privileged invocation context" — one RED test per listed case, zero mutation asserted). GREEN: implement `InvocationContext`, `resolve`.
- [x] 5.2 RED `crates/nopass-helper/src/checks.rs`: `admit_uid` — default-range admit; uid 0 always rejected even with `UID_MIN 0`; uid 65534 rejected (exceeds `UID_MAX`); uid absent from `getpwuid` (stubbed) rejected (privilege-admission §UID Range Admission). GREEN: implement `lookup_user`, `admit_uid`, `UidRejection`, `in_admin_group` (advisory pre-check only). **Also restore the deferred payload:** Phase 4 shipped `HelperError::UidRejected` as a unit variant because `checks.rs` did not exist yet. Now that `UidRejection` exists, widen it to `UidRejected(checks::UidRejection)` so the rejection reason survives to the Phase 8 journald audit record. Exit code 11 must not change.
- [x] 5.3 RED `checks.rs` `is_sudoer`: exact argv `LANG=C /usr/bin/sudo -n -l -U <user> /bin/sh`, no shell/`PATH`, `PKEXEC_UID` absent from the child env (threat matrix: External command composition, argv leg); probe exit 0 → admitted; probe non-zero even with `sudo`-group membership → rejected (privilege-admission §Existing-Sudoer Probe). GREEN: implement `is_sudoer` via `CommandRunner`.

## Phase 6: Atomic File Operations and `flock`

> **Carried forward from Phase 3 verification.** Under a fresh `tempfile::TempDir`, `Layout::under(root)` points at `<root>/sudoers.d` and `<root>/run/nopass`, neither of which exists yet. `/run/nopass` auto-creation is already required by the helper-observability spec, but `sudoers.d` auto-creation under `Layout::under` is specified nowhere. Decide it deliberately here: either the test fixture creates it or `fileops` does, and say which in the code.

*(Spec: `sudoers-rule-lifecycle`; Design §3–§4.1 steps 7–13)*

- [x] 6.1 RED `crates/nopass-helper/src/lock.rs` (unprivileged, `Layout::under(TempDir)`): sequential acquire/release, no overlap required; second non-blocking acquire on an already-held lock → `LockBusy`/exit 15 (sudoers-rule-lifecycle §Mutation Serialization, "Lock already held"; unprivileged per design §8 lane reconciliation, overrides the spec's root-only label). GREEN: implement `LockGuard` RAII over `nix::fcntl::Flock`/`LockExclusiveNonblock`.
- [x] 6.2 RED `crates/nopass-helper/tests/fileops_tempdir.rs`: `O_EXCL` collision on a pre-existing tmp; mode `0440` under `umask(0o077)`; atomic `renameat`; tmp cleanup (`unlink`) when scripted `visudo -cf` fails; a foreign non-NoPass file survives removal helpers; `90-nopass-1000.bak` is never treated as a managed rule (sudoers-rule-lifecycle §Atomic Rule Creation, §Rule Removal; threat matrix "Rule-file target selection"). GREEN: implement `fileops::write_rule_atomic`, `remove_rule`, `read_rule`, `list_rule_uids` using `Layout` + `CommandRunner`.
- [x] 6.3 RED `fileops.rs`: rule file externally deleted before `remove_rule` runs → `ENOENT` treated as success/idempotent no-op (sudoers-rule-lifecycle §Rule Removal, "Rule file externally deleted before disable runs"). GREEN: idempotent `unlink` handling.

## Phase 7: Timer Management and `ops::{enable,disable,status,expire}` Transactions

> **Two enforcement obligations inherited from Phase 5 verification.** Both are guarantees that Phase 5 documents but cannot enforce, because `ops.rs` is their only caller and it does not exist yet.
>
> 1. **Sanitize the username before the sudo probe.** `checks::is_sudoer` does NOT sanitize; `nopass_core::template::sanitize_username` is currently called only inside `render_rule`. `ops.rs` MUST sanitize the username it passes to `is_sudoer`. Today an unsanitized value fails closed, because `CommandSpec.args` is a `Vec<String>` with no shell so a hostile value arrives as one literal argv token bound to `-U` rather than as new flags, but nothing enforces it. Add a test pinning the sanitized value in the probe argv.
>
> 2. **Source the invocation context from the real environment.** `uid::resolve` takes `pkexec_uid` and `real_uid` as injected parameters, because `std::env::set_var` is unsafe under edition 2024 and this crate forbids unsafe. `ops.rs` MUST read `pkexec_uid` from the live `PKEXEC_UID` variable and `real_uid` from `nix::unistd::getuid()`, the REAL uid and not the effective one, exactly as `uid.rs`'s doc comment prescribes. Verification of this phase must diff the actual call site against that doc comment.
>
> 3. **The dead-code lint no longer tells you what is unwired.** Phase 6 added `crates/nopass-helper/src/lib.rs`, because a Cargo integration test is its own crate and can only see a package's public library API, which `tests/fileops_tempdir.rs` needed. Every helper module is now a `pub mod` of a crate with a lib target, so rustc never reports its items as dead: an external crate could call them, and one does. All seven `#![allow(dead_code)]` attributes were therefore removed as redundant, verified by reading clippy's exit status rather than grepping its output. The consequence is that the earlier plan of checking that those attributes "shrink to nothing by Phase 7" no longer works. Verify wiring by tracing the call graph from `main::dispatch` by hand instead, and name explicitly anything still reachable only from tests.

*(Spec: `expiry-policy`, `sudoers-rule-lifecycle`; Design §4, §6)*

- [x] 7.1 RED `crates/nopass-helper/src/timer.rs`: exact `systemd-run` argv verbatim order (`--unit`, `--description`, `--on-calendar=<UTC …Z>`, four `--timer-property` (corrected from "three" after Phase 7: design.md §6's literal argv block lists `AccuracySec`, `Persistent`, `WakeSystem` and `RemainAfterElapse`, each justified individually in its prose; design.md is authoritative over this task line), `--property=Type=oneshot`, helper path + `expire --uid`); `systemctl stop <unit>.timer` always runs first and tolerates non-zero (expiry-policy §Transient Timer Replacement). GREEN: implement `timer::stop`, `timer::schedule`, `unit_name`. Implemented with **four** `--timer-property` flags (`AccuracySec`, `Persistent`, `WakeSystem`, `RemainAfterElapse`), matching design.md §6's literal argv block and its own per-property rationale prose — this task's "three" is a miscount against design.md, which is authoritative; see the apply-phase report for this discrepancy.
- [x] 7.2 RED `crates/nopass-helper/src/ops.rs` (`enable`): full step order via `ScriptedRunner` — duration validation (exit 13 in/out of `[60,28800]`, past `--until`); admission chain (exit 11/12); `visudo -cf` reject → exit 14, tmp unlinked, no rename; rename failure → exit 16; `systemd-run` failure → rule rolled back, exit 17; `Never`/`Reboot` never call `systemd-run` (sudoers-rule-lifecycle + expiry-policy scenarios; design §4.1 rollback table). GREEN: implement `ops::enable`. Also discharges the Phase 7 blockquote's obligations 1 (`enable_inner` sanitizes `raw_user` via `sanitize_username` before it reaches `checks::is_sudoer`, pinned by `enable_inner_sanitizes_the_raw_username_before_the_sudo_probe`) and 2 (the `enable`/`disable`/`status`/`expire` production wrappers read `std::env::var("PKEXEC_UID")` and `nix::unistd::getuid()` directly, proven by four live-wrapper tests that rely on the real, unset test-process environment rather than injection).
- [x] 7.3 RED `ops.rs` (`disable`): no UID/sudoer admission required; unlink precedes timer stop; `getpwuid` failure falls back to header `nopass-user`, then `""` (sudoers-rule-lifecycle §Rule Removal). GREEN: implement `ops::disable`. The fallback chain is factored into a pure, independently tested `ops::resolve_username`, not yet called by `disable_inner` (no journal audit record exists to consume it until Phase 8 wires it) — see the apply-phase report's dead-code trace (obligation 3).
- [x] 7.4 RED `ops.rs` (`status`): no lock taken, no write; missing rule → `active:false, expires:null`; never returns 15/16 (helper-observability §HelperStatus JSON Contract, "stdout and state-file share the same shape"). GREEN: implement `ops::status`.
- [x] 7.5 RED `ops.rs` (`expire --uid`): absent rule → exit 0 no-op; foreign (non-NoPass) file → exit 0, never deleted; future/`Never`/`Reboot` header → exit 0 `skipped_not_expired`, file intact; past epoch → deleted (expiry-policy §Expiry Re-validation Before Deletion; threat matrix "Stale revocation"). GREEN: implement `ops::expire` uid path.
- [x] 7.6 RED `ops.rs` (`expire --boot`, unprivileged directory sweep via `Layout::under(TempDir)`): `Reboot` and past-`At` removed; `Never`/future `At` untouched; no `systemctl`/`systemd-run` call in boot mode; a per-file failure does not abort the sweep (expiry-policy §Boot-Time Cleanup Sweep; unprivileged per design §8 lane reconciliation, overrides the spec's root-only label). GREEN: implement `ops::expire` boot-sweep path via `list_rule_uids`. `expire_boot_inner` takes no `CommandRunner`/`Binaries` parameter at all, so "no `systemctl`/`systemd-run` call in boot mode" holds structurally, not merely by test assertion.

## Phase 8: State File and Journald Audit

*(Spec: `helper-observability`; Design §5, §9)*

- [x] 8.1 RED `crates/nopass-helper/src/statefile.rs` (unprivileged `Layout::under`): atomic write `O_EXCL`+`fchmod 0644`+`fsync`+`renameat`; auto-creates `/run/nopass` at `0755` when missing; a write failure is logged but never changes the caller's exit code (helper-observability §State File Placement and Permissions). GREEN: implement `statefile::write`, `statefile::remove`. `remove` is implemented and unit-tested but deliberately NOT wired into any `ops.rs` call site — its only design-documented caller (§4.4's "stale `/run/nopass/*.state` files without a rule are removed" boot-sweep cleanup) requires a second enumeration loop with no RED test of its own in this task list; see the Phase 8 apply-phase report.
- [x] 8.2 RED `crates/nopass-helper/src/journal.rs`: journal layer configured with `with_syslog_identifier("nopass-helper".to_string())` + `with_field_prefix`; stderr fallback layer used when journald init fails (helper-observability §Journald Audit Records, "asserts the tracing-journald layer is configured"). GREEN: implement `journal::init`, `journal::audit` (`NOPASS_EVENT/UID/USER/OUTCOME/EXPIRES/EXIT/REASON`). `audit`'s own field names are the bare, unprefixed tokens (`EVENT`, `UID`, ...); `init`'s `with_field_prefix(Some("NOPASS"))` is solely responsible for the `NOPASS_` prefix on the journald transport — see the module doc comment for why double-prefixing would otherwise result.
- [x] 8.3 Extend `crates/nopass-helper/Cargo.toml` with `nix`, `tracing`, `tracing-subscriber` (`registry`,`std`,`fmt`), `tracing-journald`, dev `tempfile` (`nix` was already present from Phase 6/7; the rest were already pinned in the workspace `[workspace.dependencies]` table and are inherited with `{ workspace = true }`). Wire `statefile`+`journal` calls into `ops::{enable,disable,expire}` (design §4.1 steps 16–17) and journal init into `main.rs`. Scope note: the success-path wiring covers `enable_inner`, `disable_inner`, and `expire_uid_inner`'s two diagrammed audit points (the actual deletion, and the `skipped_not_expired` no-op); `expire_boot_inner`/`sweep_one` are NOT wired — design.md §4.4's prose has no audit arrow, unlike §4.1-4.3 — and auditing a REJECTED enable/disable/expire outcome is also NOT wired — helper-observability's own "rejected operation is still journaled" scenario is marked root-only-container-testable, not a unit-test obligation of this phase. Both gaps are documented, not silent; see the apply-phase report.

## Phase 9: `data/` Install Artifacts

*(Spec: `privilege-admission` §Single Polkit Action; Design §7)*

- [ ] 9.1 Create `data/com.enfoquestic.nopass.policy` verbatim per design §7 — single action `com.enfoquestic.nopass.manage`, `allow_any=no`/`allow_inactive=no`/`allow_active=auth_admin_keep`, `exec.path=/usr/libexec/nopass-helper`.
- [ ] 9.2 Create `data/nopass-cleanup.service` verbatim per design §7 — `ConditionPathExistsGlob=/etc/sudoers.d/90-nopass-*`, `Before=systemd-user-sessions.service display-manager.service`, `ExecStart=/usr/libexec/nopass-helper expire --boot`.
- [ ] 9.3 Create `data/nopass.tmpfiles.conf` — `d /run/nopass 0755 root root -`.
- [ ] 9.4 RED `crates/nopass-helper/tests/data_artifacts.rs`: plain-`str` assertions (no XML/regex dep) — exactly one `<action id=`, correct id + `allow_*` + `exec.path`; tmpfiles line; cleanup unit `Type=oneshot`/`ConditionPathExistsGlob`/`Before=systemd-user-sessions.service` (privilege-admission §Single Polkit Action). GREEN: satisfied by 9.1–9.3.

## Phase 10: Container Integration Test Lane

*(Design §8)*

- [ ] 10.1 Create `tests/containers/Containerfile.debian` (`debian:12-slim` + `sudo`, `rustup`).
- [ ] 10.2 Create `tests/containers/Containerfile.fedora` (`fedora:40` + `sudo`, `rustup`).
- [ ] 10.3 Create `tests/containers/Containerfile.systemd` — manual full-systemd lane, documented as not a gate (timer firing, `nopass-cleanup.service` on boot, `journalctl -t nopass-helper`).
- [ ] 10.4 Create `tests/containers/README.md` documenting each lane and the `podman build`/`podman run -e NOPASS_ROOT_TESTS=1` invocations from design §8.
- [ ] 10.5 RED (root-only, gated `NOPASS_ROOT_TESTS=1` + `geteuid().is_root()`) `crates/nopass-helper/tests/root_system.rs`: real `/etc/sudoers.d` write + `root:root` ownership; real `visudo -cf` accept and reject; real `getpwuid`/`getgrouplist`; real `/run/nopass` state file; concurrent `enable` serialization; rename-failure rollback; boot sweep against real files (sudoers-rule-lifecycle "Successful atomic write", "Rename fails after successful validation", "Concurrent enable requests serialize"; expiry-policy "Expire deletes a genuinely past-epoch rule", "Boot sweep removes only stale and reboot-marked rules"; helper-cli "Successful enable exits 0" and the root-only rows of "Each rejection path exits its documented code"; helper-observability root-only state-file/journald scenarios). GREEN: none — this lane exercises Phases 2–9's code against real root/system state; no new production code is added.

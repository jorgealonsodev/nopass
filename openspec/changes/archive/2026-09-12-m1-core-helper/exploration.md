# Exploration: M1 — Núcleo (nopass-core + nopass-helper + install data)

Change: `m1-core-helper` · Project: `nopass` · Engram topic: `sdd/m1-core-helper/explore` · Date: 2026-09-12

## Current State

Greenfield repository: only `docs/PRD_NoPass_Linux.md` (v1.1, audited) and SDD scaffolding (`openspec/config.yaml`, `.atl/skill-registry.md`) exist. No `Cargo.toml`, no crates, no code. `openspec/config.yaml` already records the planned workspace layout (nopass-core / nopass-helper / nopass) and `strict_tdd_effective: false` until a workspace exists. This exploration maps PRD §5–§8 (M1 scope only: RF-04 through RF-07 partial, RF-11, plus install data) to concrete Rust modules and surfaces open technical questions with evidence.

## Affected Areas (paths to be created — none exist yet)

- `Cargo.toml` (workspace root) — new; defines the 2 M1 crates (`nopass-core`, `nopass-helper`); `nopass` (tray) crate is scaffolded but out of scope for M1 logic.
- `crates/nopass-core/src/{template,header,paths,state,expiry}.rs` — pure logic, no I/O, PRD §7.2.1 explicit no-UI/no-system-deps constraint.
- `crates/nopass-helper/src/{cli,uid,checks,fileops,lock,timer,statefile,journal,main}.rs` — privileged binary; every module here does real I/O and must be `#![forbid(unsafe_code)]`.
- `data/com.enfoquestic.nopass.policy`, `data/nopass-cleanup.service`, `data/nopass.tmpfiles.conf` — static install artifacts, no Rust code, but in M1 scope per PRD §11.
- `openspec/config.yaml` `rules.apply` already encodes the two hard invariants (no direct `/etc/sudoers` writes; `PKEXEC_UID`-only UID resolution) — sdd-tasks/sdd-apply must honor them.

## Requirement-to-module mapping

### `nopass-core` (no UI/system deps — the natural TDD target)

- `template::render_rule(uid: u32, expires: Expiry) -> String` — exact fixed template from RF-04 (`#uid` syntax, `nopass-user`/`nopass-expires` header comments); username appears ONLY in the comment and MUST be sanitized to `[A-Za-z0-9._-]` before interpolation (PRD §9 risk table).
- `header::parse(content: &str) -> Result<RuleHeader, ParseError>` — parses `nopass-user:` / `nopass-expires:` comment lines; `nopass-expires` MUST accept exactly three forms: epoch UTC integer, `reboot`, `never` (RF-04, RF-06).
- `expiry::Expiry` enum `{ Never, Reboot, At(u64) }` + `is_expired(&self, now: SystemTime) -> bool` + `validate_duration_secs(u64) -> Result<(), Error>` enforcing the `[60, 28800]` bound from RF-06.
- `paths` module — pure path builders: `sudoers_rule_path(uid)`, `sudoers_tmp_path(uid)`, `state_path(uid)` (`/run/nopass/<uid>.state`), `lock_path()` (`/run/nopass/lock`). Shared by `nopass-helper` (writer) and the future `nopass` tray crate (reader).
- `state::HelperStatus` — the single struct serialized both as the `status` subcommand's stdout JSON (PRD §7.2) and as the `/run/nopass/<uid>.state` file content (RF-07).

### `nopass-helper` (privileged binary, `nix` + `clap` + `tracing`/`tracing-journald`)

- `cli` — `clap` derive: `Enable { until: Option<u64>, until_reboot: bool }` (mutually exclusive via `ArgGroup`), `Disable`, `Status`, `Expire { uid: u32 }`. No `AllowExternalSubcommands`/`allow_hyphen_values` — default derive behavior already rejects unknown subcommands/flags with a non-zero exit before any handler runs.
- `uid::resolve_target()` — reads `PKEXEC_UID` ONLY (RF-04, NFR security row); `expire` handler additionally asserts `getuid() == 0 && env::var("PKEXEC_UID").is_err()`.
- `checks` — `user_exists_and_not_system(uid)` via `nix::unistd::User::from_uid` + a `UID_MIN` read from `/etc/login.defs` (regex `^UID_MIN\s+(\d+)`, fallback 1000 if missing/unparseable); `already_sudoer(username)` combining `sudo -l -U <user>` **output parsing** (never trust its exit code) with `nix::unistd::getgrouplist` membership in `sudo`/`wheel`/`admin`.
- `fileops::write_rule_atomic(uid, content)` — `nix::fcntl::open` with `O_EXCL|O_CREAT|O_WRONLY`, mode `0440`; `nix::unistd::fsync`; spawn `/usr/sbin/visudo -cf <tmp>` (absolute path); on success `renameat` to the final path, on failure `unlink` the tmp and exit non-zero.
- `lock::acquire() -> Flock<File>` on `/run/nopass/lock` via `nix::fcntl::Flock` (RF-04 serialization).
- `timer::replace_expire_timer(uid, until_epoch)` — MUST `systemctl stop nopass-expire-<uid>.timer` (ignore "not loaded" errors) **before** re-issuing `systemd-run --unit=nopass-expire-<uid> --on-calendar=<UTC ISO-8601> ...`.
- `journal` — `tracing_journald::layer()` for RF-11; setting `SYSLOG_IDENTIFIER=nopass-helper` needs explicit verification (open item).
- `main.rs` — wires clap → checks → fileops/lock/timer/statefile, typed exit codes distinguishing "rejected before any write" from internal errors.

## Investigation: open technical questions (with evidence)

1. **`clap` strictness** — confirmed: derive-based `Subcommand` enums reject any variant/flag not declared at parse time and exit with an error before program logic runs. No special configuration needed as long as `AllowExternalSubcommands`/permissive modes are never enabled.
2. **`sudo -l -U <user>` is not exit-code-reliable** — confirmed: it returns 0 even for a non-sudoer; only the printed text differs. The helper MUST parse stdout (with `LANG=C` forced) and keep the group-membership check as an independent signal.
3. **`systemd-run --on-calendar` timestamp format** — confirmed: systemd accepts `YYYY-MM-DD HH:MM:SS` and the ISO-8601/RFC-3339 `T`-separated form (`2026-09-12T15:00:00Z`). Always emit UTC with an explicit `Z` suffix.
4. **Replacing an existing transient timer of the same `--unit` name** — confirmed: a second `StartTransientUnit` with the same name behaves inconsistently across systemd versions. The safe sequence requires an explicit `systemctl stop nopass-expire-<uid>.timer` (tolerating "not found") before the new `systemd-run`.
5. **`nix` crate coverage** — confirmed: `nix::fcntl::{open, Flock, FlockArg, renameat}`, `nix::unistd::{fsync, User::from_uid, getgrouplist, unlink}` cover every POSIX call the PRD's stack table lists. No raw `libc` fallback needed.
6. **`tracing-journald` field control** — partially confirmed: the crate exposes `with_field_prefix`/`with_priority_mappings`/custom-field builders, but whether `SYSLOG_IDENTIFIER` is set automatically from the binary name or requires an explicit field is unconfirmed. **Flag for `sdd-research` or a spike** before `sdd-design` locks the logging API.
7. **Testing the helper in containers without a running systemd** — confirmed: Docker containers do not run systemd as PID 1 by default; `systemd-run`/`systemctl` need privileged mode + cgroup mounts, or Podman. This directly affects the PRD's M1 deliverable "tests de integración del helper en contenedor... incluyendo timers".

## Approaches (where there is a real fork)

### 1. State/status serialization format: JSON vs TOML vs key=value

- **JSON** — matches the PRD's already-decided requirement that `status` prints JSON to stdout; one `HelperStatus` struct via `serde_json` for both stdout and `/run/nopass/<uid>.state`. Not diff-friendly for humans, but the file is not meant for manual editing.
- **TOML** — human-friendly and already a planned dependency, but a second on-disk format for no functional gain.
- **key=value** — shell-parseable, but yet another ad-hoc parser.
- **Recommendation**: JSON, effort Low.

### 2. Testing strategy for the helper's systemd-dependent behavior (M1 deliverable)

- **A. Full systemd-in-Docker/Podman** — highest fidelity; slow, brittle in CI, needs privileged/rootful containers.
- **B. Split testing (recommended)** — a `CommandRunner` trait in `nopass-helper` so every `systemd-run`/`systemctl`/`visudo` invocation is built as explicit argv and unit-tested for exact content; separately, root-only container tests (plain Debian/Fedora, no systemd) for the security-critical file operations (`O_EXCL`, `fsync`, `visudo -cf`, atomic `rename`, `flock`, `getpwuid`/`getgrouplist`). Full-systemd container kept as an optional manual/smoke lane. Suspend/resume/reboot scenarios stay manual QA per PRD §10.
- **C. No container testing** — contradicts the PRD deliverable.
- **Recommendation**: B, effort Medium — a design-level decision to carry into `sdd-design`.

### 3. `UID_MIN` sourcing and trust boundary

- **Trust `/etc/login.defs` as-is** — literal PRD reading; a missing/tampered file could weaken the "no system users" check.
- **Trust with a clamped floor** — never accept a parsed `UID_MIN` below a sane minimum, falling back to 1000 otherwise.
- **Recommendation**: clamp with a floor, effort Low; floor value is a decision for the orchestrator.

## Recommendation

Proceed to `sdd-propose` scoped strictly to M1: Cargo workspace + `nopass-core` (fully unit-tested, no I/O) + `nopass-helper` (privileged binary, `CommandRunner`-abstracted systemd calls for testability) + the static `data/` install artifacts. Use JSON for the shared status/state schema. Adopt the split testing strategy (B). Confirm the pending product decisions below before finalizing the proposal, since two of them (permanent-mode existence, max duration) directly change `nopass-core`'s `Expiry` type and `nopass-helper`'s CLI surface.

## Risks

- `tracing-journald`'s exact mechanism for setting `SYSLOG_IDENTIFIER` is unconfirmed — needs a spike or `sdd-research` pass before `sdd-design` locks the logging API, or RF-11's acceptance criterion (`journalctl -t nopass-helper`) may silently fail.
- `sudo -l -U` text-parsing is locale- and version-sensitive; force `LANG=C` and keep the group-membership fallback.
- Transient timer replacement semantics vary across systemd versions; skipping the explicit `systemctl stop` risks two coexisting timers or an "already exists" failure.
- The M1 estimate (1.5 weeks) implicitly assumes full timer-firing container coverage; adopting split testing (B) changes what "done" means for that line item.
- No `.codegraph/` index exists and none is warranted yet; initialize once `Cargo.toml`/crates exist.

## Product decisions pending (for the orchestrator)

1. Does a **permanent** mode exist in `enable` (no `--until`/`--until-reboot`)? (PRD §12 Q1) — changes `Expiry` and the CLI surface.
2. Temporary max duration **8 h** (`[60, 28800]` s) or **24 h**? (PRD §12 Q2).
3. Separate unprivileged polkit action for `status` (`allow_active=yes`)? (PRD §12 Q5) — affects the policy file and a second non-`PKEXEC_UID` code path.
4. Clamp `UID_MIN` to a security floor; which value?
5. Confirm JSON as the single format for `status` stdout and `/run/nopass/<uid>.state`.
6. Confirm split testing strategy (B) as the meaning of the M1 container-testing deliverable.

## Ready for Proposal

Yes, conditional on the product decisions above being confirmed first.

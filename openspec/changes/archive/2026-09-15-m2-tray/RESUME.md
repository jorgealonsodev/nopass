# Resume here — nopass, M2

Written 2026-09-13 at the end of a working session. Read this first, then
`state.yaml`, then `tasks.md`.

## Where the project stands

M1 is complete, verified and archived. The privileged helper writes, validates and
revokes a single NOPASSWD rule per uid, admits callers through polkit and an
existing-sudoer probe, expires grants through transient timers and a boot sweep, and
journals every outcome including refusals. Its five capabilities are the canonical
specs in `openspec/specs/`.

M2, the tray, is most of the way through implementation. **Phases 1 to 10 of 11 are
committed and green: 438 tests, five gates.** Phase 11 is the manual and container
lanes, then formal verification, then archive.

## First thing to do on Monday

Run this. It should be green before anything else happens.

```
cargo test --workspace
cargo build --release
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/assert-single-reactor.sh
bash scripts/run-lane-b.sh
```

If lane B fails, read `tests/dbus/session-isolated.conf` before debugging anything.
A plain `dbus-run-session` fails four notification tests on any machine with a
desktop installed, because the nested session still activates the host's real
notification daemon. The script exists for that reason and is the only supported way
to run that lane.

## Then

1. **Phase 11**, the last implementation phase: the manual desktop checklist and the
   container harness for lane B. Its tasks are in `tasks.md`.
2. **Task 7.7** is deliberately unchecked and belongs with Phase 11. It is the manual
   observation of the icon in a real panel, and its prerequisite is the checklist
   Phase 11 ships. Nothing automated can satisfy it.
3. **`sdd-verify`** against the five M2 specs, then remediation if it finds anything,
   then **`sdd-archive`**.

Then M3 (context menu, durations, user config, autostart, i18n) and M4 (packaging).

## How this project works, so the next session does not relearn it

- One SDD attempt per phase. **Settle, then commit, then acquire the next.** Creating
  a commit while an attempt is open charges the whole commit to that attempt and
  cannot be undone; it cost one reset already.
- The attempt ceiling is **per attempt, not per change**. Three separate phases
  flagged the 2750-line forecast as exceeding a 2500 ceiling. It does not. See
  `state.yaml` for the evidence.
- Never use a throwaway `settle` to discover the untracked-inventory digest. When
  there is nothing to declare it settles for real and burns the attempt with a junk
  diagnosis. Probe with a deliberately invalid `--evidence-revision` and confirm the
  attempt is still `running` before the real settle.
- Every phase gets an independent verifier that probes the real functions with a
  harness rather than reading the implementer's tests. That is what found the
  security defects in M1, and the zbus name-claim footgun in M2.
- Read exit statuses from the tool, never by grepping its output. Two mistakes in
  this project came from that.
- Use `sdd-apply` only for a phase that has tasks. A bounded fix outside any phase
  goes to a delegated writer.

## Three contracts M2 must not break

1. A missing or stale state file means **unknown, reconcile**, never inactive.
   `FileReading` has no `Inactive` variant and `merge` has no wildcard arm, which is
   what keeps it that way.
2. The tray never touches `/etc/sudoers.d`. It cannot; the directory is `0750
   root:root`. Every mutation goes through the helper.
3. Each helper exit code maps to its own outcome. Eighteen distinct ones today.
   Collapsing them into a generic failure discards the reason an operation was
   refused, which is the only thing the user can act on.

## Open, carried forward

- `tests/containers/Containerfile.systemd`, M1's manual full-systemd lane, has never
  run end to end. A timer actually firing and the boot cleanup unit running at boot
  remain unobserved. Documented as not a gate.
- The attempt ledger's first row for this change records the diagnosis `probe`. It is
  wrong and cannot be amended; `state.yaml` carries the true diagnosis.

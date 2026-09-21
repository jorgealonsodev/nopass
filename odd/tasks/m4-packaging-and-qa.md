# M4 — Packaging and QA

**Feature**: `m4-packaging-and-qa`
**Started**: 2026-09-21
**Branch**: `feat/m4-packaging-and-qa` (to be created; never commit M4 work on `main`)
**Route**: Organic Driven Development (ODD). Not an SDD change — no proposal/spec/design/tasks artifacts.
**Engram mirror**: topic `odd/m4-packaging-and-qa/tasks`
**Locator**: `odd/tasks/m4-packaging-and-qa.md`

## Objective

Make NoPass installable, removable and verifiable as a real package on the three
target distributions, and close the gaps where the repository claims a property
it never proves.

## Problem

Every existing container lane builds and runs the crates directly. None installs a
package. So the security-relevant claim that removing NoPass never leaves a live
sudoers rule behind is a code-reading inference, not an observed fact. rpm and AUR
packaging do not exist at all. The `expiry-policy` spec claims suspend/resume
robustness that no scenario exercises, and nothing shipped in the package tells an
end user how to use the tool.

## Why now

M1, M2, M3 and m3a are archived and the canonical spec tree holds 16 capabilities.
The code is done; the delivery path is not.

## Decision taken (2026-09-21)

**One helper path for all three distributions**: `/usr/libexec/nopass-helper`,
shipped identically in deb, rpm and the AUR `PKGBUILD`. The Arch-specific
`/usr/lib/nopass/` rewrite described in `crates/nopass/Cargo.toml:47-48` is
**abandoned**, not deferred.

Rationale: pkexec enforces that the resolved path of the invoked program matches the
policy's `exec.path` annotation exactly. Today that path exists in four independent
places — one compiled constant (`crates/nopass-core/src/paths.rs:16`) and three
static literals (`data/com.enfoquestic.nopass.policy:17`,
`data/nopass-cleanup.service:11`, `crates/nopass/debian/prerm:9`) — with no build-time
mechanism to vary any of them. Rewriting only the policy file would ship a hard
authorization failure. A single value keeps the four copies trivially in agreement
and costs no code or build-system change. `/usr/libexec` is FHS-sanctioned and already
the Debian and Fedora default; only Arch house style is set aside.

Rejected: build-time templating via `build.rs` (high effort, one build per distro, new
Arch container lane, highest drift risk) and a cosmetic Arch symlink (invites a future
maintainer to repoint `exec.path` at it and recreate the mismatch).

## Scope

In scope: the exec.path drift defect, Debian removal hooks, rpm and AUR packaging,
package install/uninstall proof lanes, the suspend/resume spec-vs-proof gap, the
Lane C manual QA items the PRD requires, and an end-user README shipped in the package.

Out of scope: the four carried-forward Lane C human-only items (M2's 7.7 and 11.4,
M3's 7.3 and 11.2) and `Containerfile.systemd`, which has never run end to end.
Not started without a separate decision: any change to `HELPER_PATH`'s value.

## Constraints

- Strict TDD is enabled (source: global operator configuration). Runner: `cargo test`.
  Observed RED before implementation, then GREEN, then refactor. Never invent evidence.
- `cargo test` on 1.85.1 accepts only ONE positional filter.
- `cargo fmt --all -- --check` is NOT a repository gate. Do not add it to verification.
- Gates (`openspec/config.yaml`), all must exit 0: `scripts/assert-single-reactor.sh`,
  `scripts/run-lane-root.sh`, `scripts/run-lane-journal.sh`, `scripts/run-lane-polkit.sh`,
  `scripts/run-lane-b.sh`.
- `podman` is absent; `docker` 29.8.0 is present. Use the scripts' runtime detection.
- Run `docker builder prune -f` before container-lane work. Keep the Debian/Fedora/journald
  images — they are reusable.
- `cargo test --workspace` used to write real audit records into the host journal. That
  leak was fixed in `9564af3`; if it reappears, stop and fix it before continuing.

## Delivery

Strategy: `ask-on-risk` (default). Budget: about 400 authored changed lines per slice.
Forecast is well above one slice, so a chain strategy will be requested before the count
crosses the budget. Work-unit commits land on the feature branch; push, PR and merge stay
the maintainer's decision.

## Tasks

- [x] **T1 — Pin the policy exec.path test to the real constant.** DONE `aad40aa`.
  `policy_exec_path_is_default_helper_path` asserted a hardcoded literal and never referenced
  `nopass_core::paths::HELPER_PATH`, so the two could drift apart with the test still green.
  Renamed to `policy_exec_path_annotation_matches_the_compiled_helper_path` and rewritten to
  assert against the constant via `format!`, matching `polkit_contract.rs`'s style.
  Route taken: direct inline (1 file, no trigger fired).
  **Observed evidence** — neutering experiment, constant set to `/usr/lib/nopass/nopass-helper`
  with the policy file untouched:
  - old assertion under drift: `1 passed, 7 filtered out` — vacuous, confirmed.
  - new assertion under drift: `FAILED. 0 passed; 1 failed` — RED.
  - constant restored: `cargo test -p nopass-helper --test data_artifacts` → `8 passed`. GREEN.
  - `cargo clippy -p nopass-helper --all-targets -- -D warnings` → no issues.
  - `bash scripts/assert-single-reactor.sh` → PASS, exit 0.
  The task's doc-comment half of T2 landed here too, since it documents this exact test.

- [x] **T2 — Retire the abandoned Arch rewrite from `Cargo.toml`.** DONE.
  The comment said the Arch package rewrites `exec.path` to `/usr/lib/nopass/`, which the
  decision above abandons. Replaced with the reason the path is fixed: pkexec compares the
  resolved program path against the annotation, and the same path also lives in
  `HELPER_PATH`, `nopass-cleanup.service` and `debian/prerm`, so moving it for one package
  means moving it in all four or shipping an authorization failure.
  Route taken: direct inline. Documentation only — no behaviour to drive with a test, so no
  RED phase applies; the path equality itself is already covered by T1's assertion.
  **Observed evidence**: `cargo test -p nopass-helper --test data_artifacts` → `8 passed`;
  `cargo metadata --no-deps` parses the manifest.

- [x] **T3 — Debian `postrm`.** DONE. Reordered after T4 so it answered observed evidence.
  **The exploration's framing was wrong, and is corrected here.** It said `prerm` conflates
  `remove` and `purge` and that the two should be split. They should not. `prerm` MUST run on
  both: leaving a live NOPASSWD rule after a plain `apt remove` is the one uninstall outcome
  this package must never produce. And it must run in `prerm` rather than `postrm`, because
  dpkg runs it before deleting files — the only window in which the helper can still revoke
  its own rules. `prerm` is therefore unchanged.
  The real gap was elsewhere: `/run/nopass` is created by `postinst` through
  `systemd-tmpfiles`, so it is not a packaged file and dpkg never removes it. Added
  `crates/nopass/debian/postrm`, removing it on `purge` only — which is what PRD §10 line 330
  actually asks for.
  Route taken: direct inline, one new file answering a gap the lane had already pinned.
  **Observed evidence**: in T4's table — RED `FAIL (apt purge): /run/nopass still exists`,
  GREEN `OK (apt purge): /run/nopass is gone`.

- [x] **T4 — Prove deb removal leaves no live sudoers rule.** DONE. Commit `c983241`.
  New lane: `scripts/run-lane-deb.sh`, `tests/containers/Containerfile.deb`,
  `tests/containers/fixtures/deb-lifecycle.sh`. Builds the real `.deb` inside a disposable
  Debian container (`cargo deb` is NOT installed on this host; `cargo-deb` is pinned `^2`
  because 3.8.0 needs a let-chain that rustc 1.85 rejects), installs it, grants through the
  INSTALLED helper, then runs `apt remove` and `apt purge`, asserting no `90-nopass-*`
  survives either. Uses `--until-reboot`: these containers ship no systemd on purpose, so a
  timed grant would be rolled back on the spot and leave nothing to observe.
  Route taken: delegated writer, then parent correction and parent-run verification. The
  writer returned no usable report and was stopped; every result below was produced by the
  parent.
  **Two defects the parent found in the writer's output and fixed**:
  1. The lane script exported `NOPASS_LANE_INJECT_ROGUE_RULE` while the fixture read
     `NOPASS_LANE_SELFTEST_ROGUE_RULE`. The documented self-test command would have set
     nothing, the self-test would never have run, and the lane would have reported PASS — a
     non-vacuity proof that was itself vacuous. Unified on the `SELFTEST` name.
  2. `cargo install cargo-deb` sat after `COPY . .`, so every one-line fixture edit rebuilt it
     from scratch. Only `rust-toolchain.toml` is copied before the expensive layers now.
  **Observed evidence**:

  | Run | Outcome |
  |---|---|
  | clean lane | PASS, exit 0 — grant, remove, reinstall, grant, purge |
  | `NOPASS_LANE_SELFTEST_ROGUE_RULE=1` | FAIL, exit 1 — the assertion is not vacuous |
  | `assert_no_runtime_dir` added, no `postrm` | FAIL, exit 1 — `/run/nopass still exists` |
  | direct probe, independent of the lane | `/run/nopass` present after install AND after purge |
  | with `postrm` | PASS, exit 0 — `OK (apt purge): /run/nopass is gone` |
  | `cargo test --workspace` | 653 passed, 1 ignored, 24 suites |
  | `scripts/assert-single-reactor.sh` | PASS, exit 0 |
  | host journal, fake audit records | 0 — the `9564af3` leak has not returned |

  Honest limit: timers are NOT asserted here. These containers ship no systemd, so
  `systemctl list-timers` has nothing to answer; Lane C step 17 owns that on a real machine.
  The lane is deliberately NOT added to `openspec/config.yaml`'s gate_commands in this task.

- [ ] **T5 — rpm packaging.** No `.spec` or `cargo-generate-rpm` metadata exists. Same asset
  layout and same helper path as deb, plus the rpm equivalents of `postinst`/`prerm`/`postrm`.
  Route: delegated writer. Checks: rpm builds inside the Fedora container.

- [ ] **T6 — Prove rpm removal leaves no live sudoers rule.** T4's lane, for Fedora/rpm.
  Route: delegated writer. Checks: the new lane script exits 0.

- [ ] **T7 — AUR `PKGBUILD`.** Single `/usr/libexec/nopass-helper` path, no rewrite, no symlink.
  A comment must state why the path is not Arch-idiomatic, so the decision is not silently reversed.
  Route: direct inline. Checks: structural test asserting the PKGBUILD's install path matches the constant.

- [x] **T8 — Close the suspend/resume claim-vs-proof gap.** DONE.
  The Purpose line claimed robustness across suspend/resume with no requirement behind it.
  Reboot turned out to be genuinely covered by "Boot-Time Cleanup Sweep"; suspend was not.
  Added **Requirement: Grant Expiry Across Suspend and Resume** with two scenarios — one pinned
  by `cargo test`, one by Lane C step 16 — so the Purpose line is now true rather than narrowed.
  Route taken: direct inline.
  **Observed evidence**: the automated scenario cites
  `timer::tests::schedule_builds_the_exact_pinned_systemd_run_argv_in_order`, and that pin was
  proven non-vacuous by injecting `--on-active=15min` into `schedule`:
  `FAILED. 4 passed; 2 failed`, then `ok. 6 passed` once removed. The scenario's wording was
  corrected mid-task to cite that exact test instead of asserting an absence no test checked.

- [x] **T9 — Add the missing Lane C manual QA items.** DONE.
  Added steps 16 (15-min grant across a real 30-minute suspend, PRD line 325), 17 ("until
  reboot" grant absent at a TTY before desktop login, PRD line 326) and 18 (deb remove and
  purge leave no rule, no timer, no `/run/nopass`, PRD §10 line 330), each in the file's
  existing Do/Pass/Spec shape, plus their three rows in the result table.
  Route taken: direct inline. These are human-executed: writing them is the deliverable,
  running them stays the maintainer's.
  **Observed evidence**: structural readback — steps 16/17/18 present at lines 379, 401, 418;
  result table now carries rows 16–18.

- [ ] **T10 — Ship an end-user README.**
  The repository has no top-level `README.md`, so a user who installs the `.deb` and runs
  `nopass` gets nothing pointing at `docs/headless.md`, the tray's menu semantics, or how to
  report an issue. Write it and install it in all three packages.
  Route: delegated writer. Checks: structural readback; asset present in the built package.

## Acceptance criteria

1. The exec.path equality is asserted against `nopass_core::paths::HELPER_PATH`, and the
   assertion is proven non-vacuous by a neutering experiment.
2. Installing, granting, then removing and purging the package leaves no `90-nopass-*` file
   in `/etc/sudoers.d/`, observed in a container for both deb and rpm.
3. deb, rpm and AUR all install the helper at `/usr/libexec/nopass-helper`.
4. No document or comment claims a property that nothing proves.
5. All five gates exit 0.

## Progress

6/10 tasks. Branch `feat/m4-packaging-and-qa` created off `f90605b`.

| Task | Commit | Result |
|---|---|---|
| T1 | `aad40aa` | GREEN, non-vacuity proven by neutering |
| T2 | see below | GREEN, documentation only |

**Review**: RDD assess over `f90605b..d24c086` returned risk **high** (`process_boundary` /
`shell_process` in `data_artifacts.rs`), `review_due: true`. The maintainer granted consent.
Lineage `review-0316b3f603fdf3f3` ran all four lenses — risk, resilience, readability,
reliability — reduced to **approved** with no correction required, and the authority was
burned by exact acknowledgement (`gentle-ai.review-acknowledged/v1`, consumed revision
`sha256:7ceed25f…`). Review is informational; delivery stays the maintainer's decision.

**Review of `f90605b..88d4e2b`**: the stop hook opened a wider candidate covering the whole
branch. Maintainer granted consent. Lineage `review-f04a932e221945c0` ran all four lenses,
reduced to **approved** with no correction required, authority burned (consumed revision
`sha256:69e6744d…`).

**Assess of `88d4e2b..031c478`**: risk medium (`executable_change` on the spec file),
`review_due: false`, reason `under_budget` — 133 lines. Correctly ran no review.

Last reviewed boundary: `88d4e2b`. A consent envelope for `f90605b..031c478`
(`sha256:65ce8a41…`, lineage `review-ca7fd2406a9846fd`) was relayed and is still unanswered.

**Delivery budget crossed**: the branch now stands at 623 insertions / 11 deletions against
`f90605b`, past the ~400 authored-line slice budget. Strategy is `ask-on-risk`, so a chain
strategy — `stacked-to-main` or `feature-branch-chain` — must be asked for once before the
next commit.

## Next step

Ask the maintainer for a chain strategy — the delivery budget is crossed. Then T5 (rpm
packaging), which can reuse this lane's shape, followed by T6, T7 and T10.

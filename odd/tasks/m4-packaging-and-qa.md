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

- [ ] **T2 — Retire the abandoned Arch rewrite from `Cargo.toml`.**
  `crates/nopass/Cargo.toml:47-48` still says the Arch package rewrites `exec.path` to
  `/usr/lib/nopass/`, which the decision above abandons. Leaving it invites a future
  maintainer to act on it. The test's doc comment was already corrected in T1.
  Route: direct inline. Checks: `cargo test -p nopass-helper --test data_artifacts`.

- [ ] **T3 — Debian `postrm`, and split remove from purge.**
  No `postrm` exists; `prerm` does all cleanup on both `remove` and `purge`. That works but
  conflates two Debian-standard hooks.
  Route: delegated writer. Checks: covered by T4.

- [ ] **T4 — Prove deb removal leaves no live sudoers rule.**
  New container lane: install the built `.deb`, create a grant, `apt remove` and `apt purge`,
  assert `/etc/sudoers.d/` holds no `90-nopass-*`. This is the observed proof the security
  claim currently lacks.
  Route: delegated writer. Checks: the new lane script exits 0.

- [ ] **T5 — rpm packaging.** No `.spec` or `cargo-generate-rpm` metadata exists. Same asset
  layout and same helper path as deb, plus the rpm equivalents of `postinst`/`prerm`/`postrm`.
  Route: delegated writer. Checks: rpm builds inside the Fedora container.

- [ ] **T6 — Prove rpm removal leaves no live sudoers rule.** T4's lane, for Fedora/rpm.
  Route: delegated writer. Checks: the new lane script exits 0.

- [ ] **T7 — AUR `PKGBUILD`.** Single `/usr/libexec/nopass-helper` path, no rewrite, no symlink.
  A comment must state why the path is not Arch-idiomatic, so the decision is not silently reversed.
  Route: direct inline. Checks: structural test asserting the PKGBUILD's install path matches the constant.

- [ ] **T8 — Close the suspend/resume claim-vs-proof gap.**
  `openspec/specs/expiry-policy/spec.md:5` claims robustness across suspend/resume; no
  requirement or scenario exercises it, and no container can suspend. Either add a scenario
  traceable to the manual QA item from T9, or narrow the Purpose line to what is actually proven.
  Route: direct inline. Checks: spec readback.

- [ ] **T9 — Add the missing Lane C manual QA items.**
  `tests/manual/README.md` has no suspend/resume item (PRD line 325: activate 15 min, suspend
  30 min, resume), no reboot item, and no package install/uninstall item (PRD §10 line 330).
  These are human-executed; adding them is the deliverable, running them is the maintainer's.
  Route: direct inline. Checks: structural readback.

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

1/10 tasks. Branch `feat/m4-packaging-and-qa` created off `f90605b`.

| Task | Commit | Result |
|---|---|---|
| T1 | `aad40aa` | GREEN, non-vacuity proven by neutering |

Last reviewed boundary: `f90605b` (the branch point).

## Next step

T2 — remove the abandoned Arch rewrite comment from `crates/nopass/Cargo.toml:47-48`.

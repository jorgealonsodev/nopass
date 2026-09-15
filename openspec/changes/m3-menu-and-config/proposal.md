# Proposal: M3 — Context menu, durations, user config and i18n

Change: `m3-menu-and-config` · Predecessor: `m2-tray` (archived) · Source: `docs/PRD_NoPass_Linux.md` §11 M3 (line 340) · Date: 2026-09-15

## Intent

M2 ships a tray that does one thing, one way: toggle for exactly one hour (`crates/nopass/src/app.rs:335`), in English, from a three-item menu (`crates/nopass/src/tray.rs:125-148`), remembering nothing between runs. Everything that makes the grant a *choice* — RF-03's menu tree, RF-06's six durations, RF-02's config file and first-activation warning, RF-09 autostart, the bilingual NFR (PRD line 161) — is unbuilt. M3 builds it, and makes the first grant impossible until the user has been warned.

M3 also closes the two highest-ranked external-contract gaps from the M2 verify report (§7, Ranks 1-2). Both carry the defect shape that already cost a milestone: **an argv pinned by field equality against a fake proves the shape of a call, never that the callee accepts it.**

## Scope

### In Scope

| # | Deliverable | Lane |
|---|---|---|
| 1 | Full RF-03 menu: activate-during submenu, default-duration submenu with the current marker, view current rule, autostart checkbox, About, Quit; all keyboard-reachable | A + B |
| 2 | Six durations (15 min / 1 h / 4 h / 8 h / until reboot / permanent) mapped onto the **unchanged** helper CLI: `--until <epoch>`, `--until-reboot`, neither-flag ⇒ permanent (`cli.rs:19-25`, `ops.rs:137-145`) | A |
| 3 | `~/.config/nopass/config.toml`: default duration + "don't warn again"; read at startup, written on menu selection; corrupt/unknown content degrades to defaults, never refuses to start | A |
| 4 | First-activation consent as a two-step confirmation **inside the menu** (D1) | A + B + C |
| 5 | Autostart checkbox writing/deleting `~/.config/autostart/nopass.desktop` from the `data/nopass.desktop` template; packaging still ships no autostart entry (`crates/nopass/Cargo.toml:61-62`) | A + C |
| 6 | i18n es/en behind `crates/nopass/src/format.rs`; default from `LC_ALL` > `LC_MESSAGES` > `LANG`; the sudoers header is never translated (RF-04, PRD line 105) | A |
| 7 | Rank 1: `systemd-run`'s seven non-calendar arguments validated by the real tool, not by `ScriptedRunner` field equality (`timer.rs:52-69`) | A |
| 8 | Rank 2: `data/com.enfoquestic.nopass.policy` validated by real polkit, and `probe_polkit_readiness`'s `EnumerateActions` result stops being discarded | A + B |

### Out of Scope

- `tests/containers/Containerfile.systemd` — carried-forward manual check, not an M3 gate (needs privileged rootful podman).
- Archived M2 tasks 7.7 and 11.4 — open Lane C carry-forward items.
- `.rpm`/AUR packaging, uninstall scripts, distro QA matrix — M4.
- Any change to `nopass-helper` or `nopass-core` behaviour. All six durations already exist there; M3 consumes them.
- A settings window or any GUI toolkit — forbidden by the NFR (PRD line 165) and by `scripts/assert-single-reactor.sh`.

## Capabilities

### New Capabilities

- `tray-menu`: the RF-03 item tree, its state-dependent rebuild, the default-duration marker, "view current rule", and keyboard reachability of every item.
- `activation-consent`: RF-02's first-activation warning as a menu branch, its "don't warn again" persistence, and the invariant that no grant can precede it.
- `user-config`: `~/.config/nopass/config.toml` location (honouring `XDG_CONFIG_HOME`), schema, defaults, tolerant parsing, and atomic write.
- `autostart-entry`: creation/removal of `~/.config/autostart/nopass.desktop`, its content, and the checkbox reflecting on-disk truth rather than cached intent.
- `localization`: es/en catalogue behind `format.rs`, locale resolution order, fallback to English, and the untranslatable sudoers header.

### Modified Capabilities

- `tray-privileged-invocation`: the enable argv scenario currently pins the 1-hour default (`spec.md:18-23`). It becomes duration-driven — `--until <epoch>`, `--until-reboot`, and **neither flag for permanent** — with the default read from config.
- `helper-cli`: "External Command Invocation Discipline" gains the obligation that the `systemd-run` property tokens are accepted by real systemd, not only shaped by our own assertion (Rank 1).
- `privilege-admission`: "Single Polkit Action" gains the obligation that the installed policy is accepted by the real polkit engine, not only matched by substring (Rank 2).

## Decisions

### D1 — The first-activation warning is a menu branch, not a dialog or a notification

The PRD says "diálogo" (line 82). No dialog is buildable: a toolkit breaks the runtime-dependency NFR and fails `scripts/assert-single-reactor.sh`. Three candidates, one survives:

- **Desktop notification with inline actions** — not modal, and can be missed. If it is missed the grant still proceeds, so the user is warned *about* a NOPASSWD rule that already exists. That is not informed consent.
- **polkit `<message>`** — modal for free, but fires on every single invocation and cannot carry "don't show again". Wrong frequency, wrong lifetime.
- **Chosen: a two-step confirmation inside the SNI menu.** On the first activation the grant does **not** happen. The menu re-renders with a branch naming the risk plainly (any process running as the user can become root without a password) plus "I understand, activate", "don't warn again" and cancel. The grant happens only on the explicit confirm.

This is the only mechanism where the grant provably *cannot* precede the warning — it is a state transition, not a race with a notification daemon — and it adds no dependency.

### D2 — Rank 1 and Rank 2 gates ask the real tool

Rank 1: synthesise the transient unit's `[Timer]`/`[Service]` sections from the same constants `timer.rs` feeds to argv and run `systemd-analyze verify` over them. Rank 2: submit `data/com.enfoquestic.nopass.policy` to the real polkit engine and assert the action is enumerated. Both must **fail loudly when the tool is absent** rather than skipping silently — H3 in the same report is exactly that mechanism reproduced inside a remedy.

## Approach

Three new pure modules behind the existing hexagonal boundary, plus one seam replacement:

- `config` and `autostart` are pure value logic over a filesystem port, unit-testable with a temp `XDG_CONFIG_HOME`; no new reactor work, no new adapter.
- The duration set becomes one typed enum that renders to helper argv, so "permanent = neither flag" is expressed by construction rather than remembered.
- `format.rs` gains a catalogue backend behind its existing signatures — its module doc already commits to this ("M3 replaces this module's body without touching a call site outside it"). `Locale` (`invoke.rs:26-62`) is untouched: it is pkexec env passthrough, not i18n.
- The menu becomes a tree built from `(merged state, config, consent state)`, so every rendering is a pure function of observable state and is asserted without a desktop.

## Affected Areas

| Area | Impact | Description |
|---|---|---|
| `crates/nopass/src/tray.rs:125-148` | Modified | Minimal menu replaced by the RF-03 tree + consent branch |
| `crates/nopass/src/app.rs:328-344` | Modified | Hardcoded `now + 3600` reads the configured default |
| `crates/nopass/src/format.rs` | Modified | Body replaced by the i18n catalogue; call sites unchanged |
| `crates/nopass/src/config.rs`, `autostart.rs`, `duration.rs`, `consent.rs` | New | Config, autostart entry, duration set, consent state |
| `crates/nopass-helper/src/timer.rs:52-69` | Unchanged code, new gate | `systemd-analyze verify` over the synthesised unit |
| `data/com.enfoquestic.nopass.policy` | Unchanged content, new gate | Real polkit validation; `EnumerateActions` result consumed |
| `crates/nopass-helper/`, `crates/nopass-core/` | Unchanged | Behaviour consumed as archived |
| `Cargo.toml` | Modified | One TOML parser and one i18n crate; the no-tokio / no-gtk assertion still holds |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A new dependency pulls tokio, dbus, glib or gtk into the tree and breaks D1 of M2 | Med | `scripts/assert-single-reactor.sh` runs in every slice before the dependency lands, as in M2 |
| The consent branch is bypassable by a path that reaches enable directly (left click, nudge, keyboard) | **High** | Consent is checked in the single enable entry point, not per menu item; a test asserts every path through `Action::Enable` with unconsented config produces no invocation |
| "Don't warn again" persists but the config write fails silently, re-warning forever | Med | Write errors surface as an outcome; the requirement states re-warning is the safe failure, never silent grant |
| `view current rule` needs the PRD's "dialog" and has no toolkit either | Med | **Open for design**: render inside the menu as insensitive items from `status`, or as a notification body. Decided in `sdd-design`, not here |
| i18n changes strings the sudoers header depends on | Low | The header is `nopass-core` data, unreachable from `format.rs`; a test asserts byte identity under a Spanish locale |
| Autostart checkbox and on-disk file diverge (user deletes the file) | Low | Checkbox state is read from disk at menu build, never cached |
| Rank 1/Rank 2 gates skip invisibly where the tool is absent | Med | Absent tool fails the gate; skip-by-silence is the defect being closed |
| Config parsing refuses to start the tray on malformed TOML | Low | Tolerant read: defaults plus a warning; the tray always starts |

**Privilege boundary**: M3 changes no helper subcommand, no `PKEXEC_UID` handling and no polkit action. The only boundary-adjacent change is which duration argv the tray sends — a variant already accepted and tested by the helper — plus two gates that strengthen existing validation.

## Rollback Plan

M3 is additive to a working M2. Reverting the M3 slice commits restores the archived M2 tree: the menu returns to its three items, the toggle to its hardcoded hour, `format.rs` to its English body. Residue is user-owned and inert: `~/.config/nopass/config.toml` is ignored by an M2 binary, and `~/.config/autostart/nopass.desktop` is removed with `rm` (it launches an M2 tray perfectly well if left). No system path is written, so no revert can leave a stale sudoers rule or timer. The Rank 1/Rank 2 gates are test-only and revert independently of every feature slice.

## Dependencies

- A TOML parser for `config.toml` (PRD names the extension). Must not pull a second reactor.
- An i18n catalogue crate (`format.rs`'s doc names `rust-i18n`), confirmed against MSRV 1.85 before it lands.
- `systemd-analyze` and a real polkit authority available in the lane running the Rank 1/Rank 2 gates.
- Consumes archived `helper-cli`, `expiry-policy` and `privilege-admission` unchanged.

## Success Criteria

- [ ] Every RF-03 item exists, is keyboard-reachable, and the default-duration submenu marks the current value — Lane B for the exported menu tree, Lane C for real keyboard navigation.
- [ ] Each of the six durations produces its documented argv, permanent being `enable` with neither flag — Lane A.
- [ ] A first activation with no config file produces **zero** helper invocations until the explicit confirm; after "don't warn again", a later activation invokes directly — Lane A, observed once on a real desktop in Lane C.
- [ ] `config.toml` round-trips default duration and the consent flag; malformed content yields defaults and a warning, never a refusal to start — Lane A.
- [ ] The autostart checkbox creates and deletes `~/.config/autostart/nopass.desktop`, and the installed package still ships none — Lane A for the file, Lane C for surviving a real logout/login.
- [ ] Under `LANG=es_ES.UTF-8` the menu and notifications are Spanish and the rendered sudoers header is byte-identical to English — Lane A.
- [ ] A deliberately corrupted `systemd-run` property token fails the Rank 1 gate; a deliberately malformed policy file fails the Rank 2 gate; both fail (not skip) when the tool is missing — Lane A.
- [ ] `cargo test --workspace`, `cargo clippy -D warnings` and `scripts/assert-single-reactor.sh` green under the pinned 1.85 toolchain.

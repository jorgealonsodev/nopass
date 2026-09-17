# Lane C — manual desktop checklist

This is Lane C (design.md, m2-tray, §9 "Testing strategy — three lanes"):
a real desktop session, with a human watching. It is **never a gate** —
nothing here blocks landing a change — but it is the only lane that can
prove a panel actually draws the icon, that the polkit authentication
dialog appears and behaves, and that a notification is actually seen on
screen. Lane A (`cargo test --workspace`) and Lane B (`scripts/run-lane-
b.sh`, see `../containers/README.md`) prove the data that feeds those
things is correct; only a person looking at a real panel can prove the
panel itself is correct.

Follow the numbered steps in order. Each names what to do and a single,
checkable pass condition — never "looks right". Fill in the result table
at the end as you go, and transcribe it into the change's verify report
when you finish (task 11.4). Record what you actually observed, never an
inference.

## Before you start

**Environment requirements**

- A graphical session with a StatusNotifierItem host. GNOME does not host
  one on its own — install and enable the "AppIndicator and KStatusNotifierItem
  Support" Shell extension first, or the icon never appears anywhere and every
  step below will read as a false failure. KDE Plasma hosts one natively.
- A running polkit authentication agent for your session (GNOME Shell and
  Plasma each provide their own automatically; if `pkexec` ever reports "No
  authentication agent found", one is not running).
- Your own user account already able to authenticate as an administrator
  through polkit (normally: a member of the distribution's admin/wheel
  group). If it is not, every step from 4 onward will fail at the dialog
  for a reason unrelated to NoPass.

**One-time setup, this machine**

```bash
cargo build --release -p nopass -p nopass-helper

sudo install -Dm0755 target/release/nopass-helper /usr/libexec/nopass-helper
sudo install -Dm0644 data/com.enfoquestic.nopass.policy \
    /usr/share/polkit-1/actions/com.enfoquestic.nopass.policy
sudo install -Dm0644 data/nopass.tmpfiles.conf /usr/lib/tmpfiles.d/nopass.conf
sudo systemd-tmpfiles --create /usr/lib/tmpfiles.d/nopass.conf

# Developer icon-theme install step (design.md §7; task 7.7). Real
# installation into /usr/share is M4 packaging — this is the dev-only
# equivalent, into the user's own icon theme:
mkdir -p ~/.local/share/icons/hicolor/scalable/apps
cp data/icons/*.svg ~/.local/share/icons/hicolor/scalable/apps/
gtk-update-icon-cache -f ~/.local/share/icons/hicolor >/dev/null 2>&1 || true
```

Confirm `pkaction --action-id com.enfoquestic.nopass.manage` as your own
unprivileged user lists the action before continuing — if it does not, the
policy file did not install where polkit reads it, and every step from 4
onward will fail at the dialog for a reason unrelated to NoPass.

**Starting the tray under test**

```bash
./target/release/nopass
```

Run it from a terminal you keep open — its stdout/stderr is part of what
several steps below check. `NOPASS_ICON_STYLE=color` or `=symbolic` in
front of that command forces one icon variant for step 2, overriding the
symbolic-by-default resolution, without a rebuild.

**Stopping the tray when you are done**

Select **Quit** from the tray's own menu — this is the tray's own clean
shutdown path (spec `tray-presence` minimal menu: "Quit exits 0") and
releases the `com.enfoquestic.nopass` bus name. Confirm it actually exited:

```bash
pgrep -fa 'target/release/nopass' || echo "no nopass process running"
```

If it printed a process line instead of the echo, the process did not
exit cleanly — `kill <pid>` it, then re-run the check above before you
stop. Never leave a spawned `nopass` running past the end of this session.
If any step left an active grant (state file shows `active: true`), clear
it before you finish:

```bash
pkexec /usr/libexec/nopass-helper disable
```

## Checklist

### 1. Icon becomes visible within budget

- **Do**: with the tray not already running, start it (`./target/release/
  nopass`) and time from process start to the icon appearing in the panel.
- **Pass**: the icon is visible in the panel within 1 second of the
  command returning control to the shell.
- Spec: `tray-presence`, "StatusNotifierItem Registration and Startup
  Visibility" — Scenario "Icon registers and becomes visible within
  budget" (`Testable via: real desktop session (human-visible
  rendering)`).

### 2. All three icon states render, symbolic and colour, light and dark

- **Do**: with no active grant, observe the icon (Inactive). Toggle the
  tray on from its menu (Active-Temporary, 1-hour default). With that
  grant still active, run `pkexec /usr/libexec/nopass-helper enable
  --until-reboot` directly and watch the icon change without touching the
  tray again — the inotify watch on `/run/nopass/` picks up the rewritten
  state file within about 1 second (Active, no countdown). Repeat the
  three observations once with
  `NOPASS_ICON_STYLE=symbolic` and once with `NOPASS_ICON_STYLE=color`,
  and once each under your desktop's light panel theme and its dark panel
  theme.
- **Pass**: Inactive shows the closed-padlock icon (`nopass-locked` /
  `nopass-locked-symbolic`); Active-Temporary shows the unlocked-with-
  clock icon (`nopass-unlocked-timed` / `-symbolic`); plain Active shows
  the unlocked icon with no clock mark (`nopass-unlocked` / `-symbolic`).
  Each icon is legible (its shape is distinguishable from the other two
  at panel size) against both the light and the dark panel background,
  in both the colour and the symbolic variant.
- Spec: `tray-presence`, "Three Visual States With Distinct Icon Names" —
  Scenario "Icon name follows the merged state exactly"; design.md §9
  Lane C ("symbolic vs colour variant legibility on light and dark
  themes").
- Teardown for this step: run `pkexec /usr/libexec/nopass-helper disable`
  before moving on, so the reboot-scoped grant from this step does not
  carry into later steps.

### 3. Tooltip text is exactly right, including remaining time

- **Do**: with no grant, hover the icon and read the tooltip. Toggle the
  tray on (1-hour default) and immediately hover again. Wait for one full
  reconciliation tick (just over 60 s) without touching anything, then
  hover a third time.
- **Pass**: title is exactly `NoPass` in all three reads. Body with no
  grant is exactly `<your username> — Inactive`. Body immediately after
  enabling is exactly `<your username> — Active (59 min)` in virtually
  every real reading — the value floors to whole minutes and never rounds
  up, so anything but an inhuman under-1-second hover already reads 59,
  not 60. After the one-tick wait, the minute value has decreased by
  exactly 1 from the second read, never more, never less, and never
  shows seconds.
- Spec: `tray-presence`, "Tooltip Recomputed Only At Existing Wake
  Points" — Scenario "Tooltip renders remaining time at minute
  granularity".

### 4. Enabling authenticates, keeps the second prompt suppressed, and probes clean

- **Do**: with no grant, select the toggle item from the tray's menu
  (labelled exactly "Enable passwordless sudo"). When the polkit dialog
  appears, authenticate with your password. Immediately after, in a
  terminal, time `sudo -kn true`. Within polkit's admin-keep window
  (normally 5 minutes), toggle again to disable (labelled exactly
  "Disable passwordless sudo").
- **Pass**: a polkit authentication dialog appears after selecting
  "Enable…", and its message reads "Authentication is required to modify
  sudo rules" (or the Spanish translation, if your session locale is
  `es`). After you authenticate, the tray shows a success notification
  and the tooltip changes to the Active form. `sudo -kn true` exits 0 in
  under 1 second. The second toggle (Disable, within the keep window)
  completes **without** a second polkit dialog appearing.
- Spec: `tray-privileged-invocation`, "pkexec Invocation for Enable and
  Disable"; `tray-privileged-invocation`, "Total Exit-Code-to-Outcome
  Mapping" (code 0 → success); design.md §9 Lane C ("the polkit dialog
  appears, authenticates, and `auth_admin_keep` suppresses the second
  prompt; `sudo -kn true` returns 0 within 1 s of confirmation").

### 5. Cancelling the polkit dialog is handled, not hung

- **Do**: with no grant, select "Enable passwordless sudo" again. When the
  polkit dialog appears, dismiss it (its own Cancel button, or Escape)
  instead of authenticating. Immediately after, open the tray's menu and
  select Quit-adjacent items (or just re-open the menu) to confirm the
  tray is still responding.
- **Pass**: the tray shows a notification with summary "Authorization
  cancelled" and body "Authorization cancelled. Nothing was changed." No
  grant was created (menu toggle label is still exactly "Enable
  passwordless sudo" afterward). The tray's menu opens normally right
  after — it did not hang waiting on the dismissed dialog.
- Spec: `tray-privileged-invocation`, "Total Exit-Code-to-Outcome Mapping"
  (pkexec code 126) — Scenario "pkexec 126 and 127 are distinguished from
  each other and from helper codes"; `tray-privileged-invocation`,
  "pkexec Invocation Never Blocks the Reactor" — Scenario "Menu remains
  responsive during a pending authorization".

### 6. A notification is actually seen on screen

- **Do**: with no grant, enable passwordless sudo from the tray menu and
  watch the screen, without looking away, from the moment you authenticate.
- **Pass**: a desktop notification popup is visibly rendered by your
  session's own notification daemon (not just logged, not just inferred
  from the tooltip changing), with summary "Passwordless sudo enabled"
  and a body starting "Passwordless sudo enabled until the requested
  time." followed by an "Expires: <N> min." clause.
- Spec: `tray-notifications`, "Success, Failure, and Expiry Notifications"
  — Scenario "Successful enable produces a confirmation notification";
  design.md §9 Lane C ("Notifications visibly rendered by the session's
  own daemon").
- Teardown for this step: `pkexec /usr/libexec/nopass-helper disable`.

### 7. The menu is fully operable from the keyboard

- **Do**: without touching the mouse, use your desktop's panel-focus key
  (e.g. `Super`, or your panel's documented accessibility shortcut) to
  reach the tray icon, open its menu with the keyboard, navigate to the
  toggle item, and activate it with Enter. Then reopen the menu the same
  way and select Quit with the keyboard.
- **Pass**: every step above succeeds with no mouse input at any point —
  the menu opens, the toggle item is reachable and its selection state
  or label is visible while focused, activating it with Enter enables
  passwordless sudo exactly as the mouse path did, and Quit exits the
  tray.
- Spec: design.md §9 Lane C ("Keyboard navigation of the menu
  (accessibility NFR)").

### 8. Double-launching nudges the running instance, as a human sees it

- **Do**: with the tray already running, launch a second copy from a
  second terminal: `./target/release/nopass`.
- **Pass**: the second process exits on its own within a few seconds
  (`echo $?` on it reports `0`) and never creates a second icon in the
  panel — only the original icon is present throughout. The first
  instance visibly reacts: its tray item stays present (SNI re-
  registration) and a notification with summary "NoPass is already
  running" and body "NoPass is already running for <your username>."
  appears.
- Spec: `tray-single-instance`, "D-Bus Name Claim and NameTaken Exit" —
  Scenario "Second instance exits 0 without a second icon";
  "Activation Nudge on a Second Launch" — Scenarios "Second instance
  nudges the first before exiting" and "The first instance reacts to a
  received nudge"; design.md §9 Lane C ("The double-clicked-launcher path
  end to end, as a human sees it").

### 9. The reconciliation probe does not destroy cached sudo credentials

- **Do**: run `sudo -v` and type your password to cache credentials.
  Without running any other `sudo` command, wait for one tray
  reconciliation tick (or open the tray's menu, which also triggers a
  probe) so the tray's own `sudo -k -n true` probe runs. Immediately
  after, run `sudo -n true`.
- **Pass**: `sudo -n true` exits 0 with no password prompt — the tray's
  own probe did not invalidate the timestamp your `sudo -v` created.
- Spec: design.md §9 Lane C ("`sudo -kn true` does **not** destroy the
  user's cached sudo credentials — requires a typed password"); the
  behaviour is documented in `crates/nopass/src/probe.rs`'s own module
  comment.

### 10. The polkit action is enumerable as the unprivileged user

- **Do**: as your normal, unprivileged user, run:
  `pkaction --action-id com.enfoquestic.nopass.manage`
- **Pass**: the command prints exactly `com.enfoquestic.nopass.manage`
  and exits 0 — no `sudo`, no password, no error.
- Spec: design.md §9 Lane C ("`pkaction --action-id
  com.enfoquestic.nopass.manage` run as the unprivileged user — converts
  §0 G3's prior into an observation").

### 11. No wakeup source beyond the 60 s tick, over a real hour

- **Do**: with no grant active, start the tray and record its PID and
  starting values: `ps -o rss,time,etimes -p <pid>`. Leave it running,
  untouched (no menu opens, no toggles), for at least one continuous
  hour. Record the same three values again at the end.
- **Pass**: RSS (`rss`, in KiB) stays under 30,000 (30 MB) at the end.
  Cumulative CPU time (`time`) increased by no more than a few seconds
  over the full hour (`etimes`) of wall-clock elapsed time — consistent
  with waking only once a minute to do a few milliseconds of work, never
  with continuous or per-second activity.
- Spec: `tray-state-sync`, "No Periodic Wakeup Beyond the 60-Second
  Reconciliation Tick" — Scenario "Only one periodic wakeup source
  exists" (`Testable via: … real desktop session for the measured idle
  RSS/CPU over a continuous one-hour run`); `tray-presence`, "No timer
  exists solely to refresh the tooltip" (`Testable via: … real desktop
  session for the measured idle CPU`).

### 12. Full-tree keyboard navigation (M3 menu)

Item 7 above proved the minimal M2 menu is keyboard-reachable. M3 grew the
tree — two duration submenus, an insensitive "Current rule" section, an
autostart checkbox, and the first-activation consent branch — and every
new node needs the same proof; a submenu opening under the mouse but not
under `Right`/`Enter` would pass item 7 and still fail this one.

- **Do**: without touching the mouse at any point, use your desktop's
  panel-focus key to reach the tray icon and open its menu with the
  keyboard. Navigate into and out of "Activate during…" and reach each of
  its six duration entries (15 min / 1 h / 4 h / 8 h / until reboot /
  permanent). Do the same for "Default duration" and its six entries.
  Reach "Current rule" and its detail rows. Reach and toggle "Start with
  session". With consent unrecorded (fresh `config.toml`, or
  `warning_acknowledged` unset), activate the toggle item with the
  keyboard to bring up the first-activation branch, then navigate to and
  read each of its three items ("I understand — activate for…", "I
  understand — activate and don't warn me again", "Cancel") — activate
  "Cancel" to leave without granting. Finally reopen the menu and select
  Quit, all with the keyboard.
- **Pass**: every item and submenu entry named above is reachable and
  activatable using only the keyboard — no step requires a pointer
  click, hover, or drag. Each item's focus/selection state (including the
  `RadioGroup`'s current-duration marker) is visible while focused. No
  submenu or the consent branch silently fails to open under keyboard
  navigation even though it opens under the mouse.
- Spec: `tray-menu`, "Every item exposes standard keyboard-navigation
  properties"; task 7.3.

### 13. First activation is observed exactly once, never re-presented

- **Do**: start from a fresh or edited `config.toml` with consent
  unrecorded (`warn_before_activation` true, i.e. `warning_acknowledged`
  false). Activate passwordless sudo for the first time from the tray
  menu and watch for the two-step confirmation branch (the warning text,
  then "I understand — activate and don't warn me again"). Select that
  item. Once the grant completes, disable passwordless sudo, then
  activate a second time.
- **Pass**: the confirmation branch is presented exactly once, on the
  first activation — and no privileged invocation happens until "I
  understand — activate…" is explicitly selected on that first
  presentation. The second activation dispatches directly, with the
  branch never re-presented.
- Spec: `activation-consent`, "Confirming the branch grants exactly
  once", "A later activation skips the branch once consent is recorded".

### 14. Autostart entry survives a real logout and login

- **Do**: from the tray menu, enable "Start with session" (Spanish:
  "Iniciar con la sesión") and confirm
  `~/.config/autostart/nopass.desktop` now exists. Fully log out of the
  desktop session and log back in — not merely lock/unlock the screen or
  restart the tray process.
- **Pass**: the NoPass tray icon appears automatically after login,
  without manually launching `nopass`. Then disable "Start with session"
  from the menu, confirm the file is gone, and repeat a full logout/login
  — this time the icon does **not** appear on its own.
- Spec: `autostart-entry`, "A fresh install has no autostart entry" —
  Lane C half (real logout/login).

### 15. Spanish rendering — menu and notifications

Six notification strings (the first-activation consent nudge, the
polkit-unavailable toast, and the config-unreadable warning — summary and
body of each) were moved into the same catalogue the menu already uses
after a review found them hardcoded in English; this step exists to prove
that fix on a real desktop, not just under `cargo test`. Checking only the
menu would miss exactly the regression this item was added for.

- **Do**: run the tray with `LANG=es_ES.UTF-8` (and `LC_ALL`/`LC_MESSAGES`
  unset, so `LANG` is what resolves — see `localization` spec's
  resolution order). Open the menu and read every item: the toggle
  ("Activar sudo sin contraseña" / "Desactivar sudo sin contraseña"),
  "Activar durante…" and its six entries, "Duración predeterminada",
  "Regla actual", "Iniciar con la sesión", "Acerca de", "Salir". With
  consent unrecorded, trigger the first-activation branch and read its
  text ("⚠ Lee esto antes de activar", the risk statement, "Entendido:
  activar durante <d>", "Entendido: activar y no volver a advertirme",
  "Cancelar"). Then trigger and read each notification you can reasonably
  reproduce: the first-activation consent nudge (if triggered from a
  non-menu path per `activation-consent`), a successful enable, a
  successful disable, and — if reproducible in your environment — the
  polkit-unavailable toast (rename/move the installed `.policy` file
  temporarily) and the config-unreadable warning (temporarily corrupt
  `config.toml`).
- **Pass**: every menu item and submenu entry above renders in Spanish,
  matching the text quoted above exactly. The first-activation branch
  renders in Spanish. Every notification you triggered — summary and
  body — renders in Spanish, with no literal `{}` placeholder and no
  English string anywhere in the menu, tooltip, or notification text.
  **Known, out-of-scope gap — not a failure of this item**: four
  environment/degraded-mode notifications inherited from M2 ("No tray
  host found", "No notification service found", "Could not watch for
  changes", "Lost the change watch") are not yet in the catalogue and
  will still read in English; record if you observe one, but do not mark
  this item Fail solely for those four.
- Spec: `localization`, "A Spanish locale changes format.rs output
  without touching callers"; `activation-consent`; commit
  `d078207` ("move the notification text into the catalogue it belongs
  to").

## Result table

Fill in once per run. One row per checklist item above; use the exact
observed value or behaviour, never an inference ("should be fine" is not
a result).

**Environment**

| Field | Value |
|---|---|
| Desktop environment + version | |
| Session type (X11 / Wayland) | |
| SNI host (e.g. GNOME AppIndicator extension version, or "native — Plasma") | |
| Distro + version | |
| `nopass`/`nopass-helper` build (git commit or `cargo build` output) | |
| Date run | |
| Run by | |

**Results**

| # | Step | Pass / Fail | Observed value / notes |
|---|---|---|---|
| 1 | Icon visible within budget | | |
| 2 | Three icon states, symbolic/colour, light/dark | | |
| 3 | Tooltip text exact, remaining time correct | | |
| 4 | Enable authenticates; keep suppresses 2nd prompt; probe < 1 s | | |
| 5 | Cancelling the dialog is handled, not hung | | |
| 6 | Notification actually seen on screen | | |
| 7 | Menu fully operable from the keyboard | | |
| 8 | Double-launch nudges the running instance | | |
| 9 | Probe does not destroy cached sudo credentials | | |
| 10 | `pkaction` enumerable unprivileged | | |
| 11 | Idle RSS < 30 MB, idle CPU ≈ 0 over ≥ 1 h | | |
| 12 | Full-tree keyboard navigation (M3 menu) | | |
| 13 | First activation observed exactly once, never re-presented | | |
| 14 | Autostart entry survives a real logout/login | | |
| 15 | Spanish rendering — menu and notifications | | |

## Next step

Transcribe this table into the change's verify report (task 11.4;
design.md §9 "Provable nowhere automatically"). For the automated lanes
that feed this one, see `../containers/README.md` (Lanes A and B).

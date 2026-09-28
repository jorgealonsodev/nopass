# NoPass

NoPass lets you grant and revoke passwordless `sudo` for a selected user (the tray manages your current account), either from a desktop tray or through explicit root-run commands on a headless host. Choose a duration carefully: a grant can expire on a timer, at reboot, or remain until revoked.

## What NoPass does and does not do

- It manages a `NOPASSWD` sudoers rule for the selected user; the tray asks polkit to authorize changes.
- It does not run commands for you or replace `sudo`. During a grant, `sudo` commands covered by the rule do not prompt for a password.
- It does not grant permissions during package installation or authorize itself: tray changes require polkit, and headless helper commands must run as root.
- Timed grants use systemd timers. Boot-time cleanup of reboot-scoped grants is provided by `nopass-cleanup.service` (see [Autostart and cleanup](#autostart-and-cleanup)).

## Use the tray

Launch **NoPass** from your desktop's application menu, or run `nopass` in a graphical session with a system tray host and a polkit authentication agent.

- When no grant is active, choose **Enable passwordless sudo** or select a duration under **Activate during…**.
- While a grant is active, the toggle becomes **Disable passwordless sudo**. Select it to revoke the grant.
- **Current rule** shows whether a grant is active and its expiry. **Default duration** changes the duration used for a later activation.
- **Start with session** controls NoPass's per-user desktop autostart entry. **Quit** exits the tray.

The duration choices include 15 minutes, 1 hour, 4 hours, 8 hours, until reboot, and permanent. Check the selected duration before activating.

## Headless use

The tray's polkit actions need an active graphical session. For automation or administration without a desktop, invoke the helper directly as root and provide the target user's numeric UID explicitly. Run `id -u` as the target user before `sudo` so the shell supplies that user's UID.

### Grant access

```bash
uid="$(id -u)"
sudo /usr/libexec/nopass-helper grant --uid "$uid" --until-reboot
```

Use `--until <epoch-seconds>` for a timed grant. Omit duration flags only when a persistent grant is intended.

### Inspect access

```bash
sudo /usr/libexec/nopass-helper inspect --uid "$(id -u)"
```

### Revoke access

```bash
sudo /usr/libexec/nopass-helper revoke --uid "$(id -u)"
```

Do not simulate tray authorization by setting `PKEXEC_UID` yourself. See [the headless guide](docs/headless.md) for the supported procedure and its limits.

## Autostart and cleanup

Package installation does **not** create a desktop autostart entry and does **not** enable `nopass-cleanup.service`. You own both decisions:

- Enable **Start with session** in the tray menu if you want NoPass to start for your desktop user. This writes that user's autostart entry.
- The package installs `nopass-cleanup.service` but leaves it disabled. If you want reboot-scoped grants cleaned at boot, an administrator can enable it with `sudo systemctl enable nopass-cleanup.service`. Review your system policy before doing so.

## Troubleshooting

- **No tray icon:** check that your desktop provides a StatusNotifierItem host. KDE Plasma includes one; GNOME may need an AppIndicator/KStatusNotifierItem extension.
- **Authorization dialog does not appear:** the tray needs a running polkit authentication agent in an active graphical session. Headless hosts should use the helper procedure in [the headless guide](docs/headless.md), not the tray actions.
- **Helper or authorization errors:** the canonical helper path is `/usr/libexec/nopass-helper`. Confirm the package installed that executable and its polkit policy; do not move or symlink the helper to another path.
- **A grant remains active:** use **Disable passwordless sudo** or the headless `revoke` command. For boot-time cleanup of reboot-scoped grants, check that `nopass-cleanup.service` is enabled as described above.

To report an issue, open a [GitHub issue](https://github.com/jorgealonsodev/nopass/issues) and include your distribution/version, installation method, desktop/session type, what you tried, and the exact error or relevant output. Redact usernames and other private details; never include passwords or authentication secrets.

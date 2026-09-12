# Research: M1 — Núcleo (`m1-core-helper`)

Request: `m1-research-001` · Revision: 1 · Source classes: documentation, open-web · Collected: 2026-09-12 · Engram topic: `sdd/m1-core-helper/research`

## Executive summary

`journalctl -t <id>` filters on the `SYSLOG_IDENTIFIER` field. `tracing-journald` 0.3.2 exposes `Layer::with_syslog_identifier()`; without it the field defaults to the executable file name. No extra crate is needed: `with_syslog_identifier("nopass-helper")` satisfies RF-11. pkexec logs its own authorization via `syslog(LOG_AUTHPRIV)` but does not redirect the child's stderr, so the helper must log to journald directly.

For sudoer detection, `sudo -l -U <user>` **without a command** exits 0 whenever the user has *any* rule (empirically confirmed on sudo 1.9.15p5: `nobody` has narrow NOPASSWD rules on Linux Mint and exits 0). `sudo -l -U <user> <command>` has documented exit semantics (0 if permitted, 1 if not) and is the reliable probe. `sudo -l` has no JSON mode; `cvtsudoers -f json` works on rule files only. Default admin groups: Debian/Ubuntu `sudo` (legacy `admin`), Fedora/Arch `wheel`.

## Sources

| id | class | title | publisher | URL | accessed |
|---|---|---|---|---|---|
| S1 | documentation | systemd.journal-fields(7) | man7.org | https://www.man7.org/linux/man-pages/man7/systemd.journal-fields.7.html | 2026-09-12 |
| S2 | documentation | journalctl(1) | man7.org | https://www.man7.org/linux/man-pages/man1/journalctl.1.html | 2026-09-12 |
| S3 | documentation | `tracing_journald::Layer` (0.3.2) | docs.rs | https://docs.rs/tracing-journald/latest/tracing_journald/struct.Layer.html | 2026-09-12 |
| S4 | documentation | sudo(8) | sudo project (man.archlinux.org mirror) | https://man.archlinux.org/man/sudo.8.en | 2026-09-12 |
| S5 | documentation | cvtsudoers(1) | sudo project (man7.org mirror) | https://man7.org/linux/man-pages/man1/cvtsudoers.1.html | 2026-09-12 |
| S6 | open-web | polkit `pkexec.c` (mirror) | GitHub wingo/polkit | https://github.com/wingo/polkit/blob/master/src/programs/pkexec.c | 2026-09-12 |
| S7 | open-web | Debian Wiki — sudo | Debian | https://wiki.debian.org/sudo/ | 2026-09-12 |
| S8 | open-web | Sudo — ArchWiki | Arch Linux | https://wiki.archlinux.org/title/Sudo | 2026-09-12 |
| S9 | empirical | Orchestrator test on Linux Mint 22.3, sudo 1.9.15p5, systemd 255 | local | — | 2026-09-12 |

Excerpts:
- S1: "`SYSLOG_IDENTIFIER=` … the identifier string (i.e. 'tag') … usually derived from glibc's `program_invocation_short_name`."
- S2: "`-t, --identifier=SYSLOG_IDENTIFIER` Show messages for the specified syslog identifier."
- S3: "`with_syslog_identifier` … Defaults to the file name of the executable of the current process, if any. Systemd exposes it in the `SYSLOG_IDENTIFIER` journal field, and allows filtering log messages by syslog identifier with `journalctl -t`."
- S4: "If the `-l` option was specified without a command, sudo will exit with a value of 0 if the user is allowed to run sudo… `-U` … is restricted to the root user and users with either the 'list' privilege for the specified user or the ability to run any command as root or user on the current host." Also: "If a command is specified and is permitted by the security policy, the fully-qualified path is displayed … exit 1 if not permitted."
- S5: "`-f output_format` … JSON … easier for third-party applications to consume than the traditional sudoers format."
- S6: `openlog("pkexec", LOG_PID, LOG_AUTHPRIV); syslog(...)` — no redirection of the launched command's stderr before `execve`.
- S9: `LANG=C sudo -n -l -U nobody` → exit 0 and lists four `(root) NOPASSWD:` mint-specific commands. `LANG=C sudo -n -l -U nobody /bin/sh` → exit 1. `LANG=C sudo -n -l -U jorge /bin/sh` → prints `/bin/sh`, exit 0. `strings sudoers.so` contains `User %s is not allowed to run sudo on %s.`; the Spanish `sudoers.mo` has no translation for it on this system. `/etc/login.defs`: `UID_MIN 1000`, `UID_MAX 60000`; `nobody` is uid 65534.

## Claims

| Q | Verdict | Sources | Confidence |
|---|---|---|---|
| A1 | `journalctl -t` filters on `SYSLOG_IDENTIFIER`. | S1, S2 | high |
| A2 | `tracing_journald::Layer::with_syslog_identifier()` sets it; default is the executable file name. | S3 | high |
| A3 | No extra crate needed; `with_syslog_identifier("nopass-helper")` is sufficient and consistent with `tracing`. | S3 | medium |
| A4 | pkexec logs its own decision to AUTHPRIV; child stderr is inherited, not journaled. Helper must log directly to journald. | S6 | medium |
| B1 | Denial string is `User %s is not allowed to run sudo on %s.`; gettext-wrapped in principle but untranslated in the Spanish catalog on Mint 22. Not part of the documented interface — do not parse it. | S4, S9 | medium |
| B2 | `sudo -l -U <user>` (no command) exits 0 when the user has **any** rule, including narrow NOPASSWD rules for system users such as `nobody`. It does NOT mean "has broad sudo". | S4, S9 | high |
| B2b | `sudo -l -U <user> <command>` exits 0 iff the policy permits that command; exit 1 otherwise. Documented and empirically confirmed. | S4, S9 | high |
| B3 | `-U` requires root or list privilege; the helper runs as root via pkexec, so this is satisfied. | S4 | high |
| B4 | No JSON output for `sudo -l`; `cvtsudoers -f json` only converts rule files. | S5 | high |
| B5 | Admin groups: Debian/Ubuntu `sudo` (legacy `admin`); Fedora/Arch `wheel`. | S7, S8 | high |

## Design implications (for `sdd-propose` / `sdd-design`)

1. **Logging**: `tracing_journald::Layer::with_syslog_identifier("nopass-helper")`. Do not rely on stderr.
2. **"Already a sudoer" check**: define it as "may run an arbitrary command as root", probed with `LANG=C /usr/bin/sudo -n -l -U <user> /bin/sh` and its exit code (0 = permitted). Keep group membership in `sudo`/`wheel`/`admin` as a fast pre-check, but the command probe is the authoritative signal. Never parse the denial string.
3. **UID range**: check `UID_MIN <= uid <= UID_MAX` from `/etc/login.defs` (defaults 1000/60000, floor clamp 1000). `UID_MIN` alone would admit `nobody` (65534).

## Gaps

- sudo.ws (canonical publisher) returned HTTP 403; manual text taken from mirrors.
- A4 inferred from a source mirror, not an explicit polkit doc statement; moot because the helper logs directly to journald.
- Localization of the denial string on other distros not checked; irrelevant once the string is not parsed.

## Risks

- A future sudo change to `-l <command>` semantics would affect the probe; mitigated by the documented contract in sudo(8) and the group pre-check.
- `/bin/sh` probe requires the path to exist on the target (it does on all target distros via `/usr/bin/sh` merged-usr symlinks).

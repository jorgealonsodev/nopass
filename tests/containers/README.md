# Container test lanes

Four lanes. Three are gates you run before every change. One is manual and
you run it only when you need to see systemd itself do something.

## Quick path

```bash
# 1. Unprivileged gate — runs on every machine, no container needed.
cargo test --workspace
cargo build --release

# 2. Headless session bus (Lane B) — proves D-Bus name ownership, SNI
#    registration and notification payloads, with no desktop present.
#    On a machine that already has `dbus-run-session` (most desktop Linux
#    installs do), just run the script directly — no container needed:
bash scripts/run-lane-b.sh
#    Anywhere that isn't true — CI above all, since CI has no desktop and
#    usually no `dbus-run-session` either — build and run the container:
podman build -f tests/containers/Containerfile.dbus -t nopass-test-dbus .
podman run --rm nopass-test-dbus bash scripts/run-lane-b.sh

# 3. Root lane, Debian + Fedora (Lane R) — proves the same code against a
#    real root and a real /etc/sudoers.d, on both distros. Detects
#    whichever container runtime is present (docker or podman) and builds
#    + runs both images:
bash scripts/run-lane-root.sh
```

All four above must exit 0. The manual systemd lane (further down this
document) is not part of this and is never required before landing a
change.

## Details

| Lane | Containerfile | Gate? | Runs as | What it proves |
|---|---|---|---|---|
| Unprivileged | none — runs directly on the host | Yes, always | your own user | Every behavior `ScriptedRunner`/`TempDir` can simulate: core logic, CLI parsing, argv assertions, atomic file ops against a temp root, the unprivileged-reconciled lock-busy/boot-sweep/binary-resolution scenarios (see below) |
| Headless session bus (Lane B) | `Containerfile.dbus` (only needed where the host lacks `dbus-run-session`, e.g. CI) | Yes, always | your own user — no root, inside or outside a container | Everything a real private session bus can show without a desktop: `com.enfoquestic.nopass` name ownership and the `NameTaken`/nudge path, StatusNotifierItem registration and property values against a fake watcher, and notification payloads against a fake `org.freedesktop.Notifications` — never rendering, which only Lane C can prove |
| Root, Debian/Fedora | `Containerfile.debian`, `Containerfile.fedora` | Yes, both distros | root, inside a disposable container | Everything the unprivileged lane structurally cannot: real `/etc/sudoers.d` writes with real `root:root` ownership, a real `visudo -cf`, real `getpwuid`/`getgrouplist`, a real `/run/nopass` state file, real lock contention, a real rename-failure rollback, and `expire --boot` against real files |
| Manual, full systemd | `Containerfile.systemd` | **No** — never run in CI, never required before landing a change, and **never executed end to end in this repository** — see below | root, systemd as PID 1 | The one thing no gate lane can show: an actual `systemd-run` timer firing, `nopass-cleanup.service` running at real boot, and `journalctl -t nopass-helper` producing real records |

Lane C — a real desktop session with a human watching — is not a container
lane at all and has no row above. See `../manual/README.md`: it is the only
place panel rendering and polkit authentication can be proven, it is never a
gate, and its result is recorded by hand in the change's verify report.

### Why two gate lanes exist, not one

`crates/nopass-helper/tests/root_system.rs` is gated on
`NOPASS_ROOT_TESTS=1` **and** `geteuid().is_root()` — both conditions, not
either. On your own machine (unprivileged, no such env var) every test in
that file compiles and then skips, printing why:

```bash
cargo test -p nopass-helper --test root_system -- --nocapture
```

Run with `--nocapture` to see each `SKIPPED <test>: ...` line. Without it,
`cargo test` still reports every test as `ok` — a skip is not a failure —
but the skip reason is hidden the same way any other passing test's stdout
is hidden. One test in that file, `gate_requires_both_conditions_true_
before_admitting`, is *not* gated: it runs everywhere and proves the gate
itself requires both conditions together, not either alone.

Inside the Debian and Fedora containers, both conditions hold (the image
runs as root by default, and `scripts/run-lane-root.sh` sets
`NOPASS_ROOT_TESTS=1`), so every test in `root_system.rs` actually
executes — real `useradd`, real `/etc/sudoers.d` writes, real `visudo`.

### Lane B — the headless session bus

`crates/nopass/tests/dbus_session.rs` is gated on `NOPASS_DBUS_TESTS=1`,
checked per test, not on any container — the container in
`Containerfile.dbus` exists only to supply a `dbus-run-session` binary and a
D-Bus stack on hosts (CI, above all) that have neither. Always invoke it
through `scripts/run-lane-b.sh`, never a bare `dbus-run-session`:

```bash
bash scripts/run-lane-b.sh
```

That script wraps `cargo test -p nopass --test dbus_session` in
`dbus-run-session --config-file=tests/dbus/session-isolated.conf`, with
`NOPASS_DBUS_TESTS=1` set for you. `tests/dbus/session-isolated.conf`
matters and is not optional: a plain `dbus-run-session` still reads the
host's `/usr/share/dbus-1/services`, so on any machine with a desktop
installed, the *real* notification daemon gets D-Bus activated inside the
nested session and claims `org.freedesktop.Notifications` before the
in-test fake can — four tests depend on owning that name, or on nobody
owning it, and silently fail otherwise. The config file omits
`<standard_session_servicedirs/>`, so the private bus has nothing
activatable at all. `Containerfile.dbus` is the mirror-image precaution: it
installs a D-Bus client and daemon and nothing that ships a
session-activatable `.service` file, so there is nothing for that same
hazard to find even inside the container.

Two independent things can make this lane skip rather than run, and each
says so on its own:

- **`NOPASS_DBUS_TESTS` unset** — every gated test in `dbus_session.rs`
  reports `ok` (a skip is not a failure) and prints, with `--nocapture`,
  `skipping <test name>: set NOPASS_DBUS_TESTS=1 (under dbus-run-session) to
  run`. `scripts/run-lane-b.sh` always sets it, so this only shows up if you
  call `cargo test` directly instead.
- **`dbus-run-session` absent** — the wrapper binary itself, not a Rust
  test, so it fails before `cargo test` ever starts: `bash: dbus-run-session:
  command not found` (host) or the same, immediately, inside a container
  that has not installed the `dbus` package.

### Why the root containers don't run systemd

Neither `Containerfile.debian` nor `Containerfile.fedora` installs
`systemd`. This is deliberate (design.md §8: "No systemd as PID 1 is
required for this lane"), not an oversight: `crates/nopass-helper/src/
timer.rs` genuinely fails to schedule a timer inside these containers —
`systemd-run` either doesn't exist or can't reach a running systemd — which
is exactly what `root_system.rs`'s
`real_timer_scheduling_failure_rolls_back_the_rule_when_systemd_is_unavailable`
test needs: a real, unforced exit-17 rollback, not a scripted one.

### The manual systemd lane

This lane is documented, and its image builds, but **the sequence below has
never been executed end to end in this repository** — no one has run it and
watched the timer actually fire, the boot cleanup actually run, or the
`journalctl` output actually appear. Do not assume any part of it works
until you have run it yourself; treat a first run as validating the
recipe, not repeating a known-good one.

```bash
podman build -f tests/containers/Containerfile.systemd -t nopass-test-systemd .
podman run -d --name nopass-systemd --systemd=always --privileged \
    -v /sys/fs/cgroup:/sys/fs/cgroup:rw nopass-test-systemd

# Drive it by hand — this is not scripted, because the point is to watch:
podman exec nopass-systemd useradd -M demo
podman exec nopass-systemd sh -c \
    'echo "demo ALL=(ALL:ALL) ALL" > /etc/sudoers.d/00-demo && chmod 0440 /etc/sudoers.d/00-demo'
podman exec -e PKEXEC_UID=$(podman exec nopass-systemd id -u demo) \
    nopass-systemd /usr/libexec/nopass-helper enable --until 90
podman exec nopass-systemd systemctl list-timers 'nopass-expire-*'
# wait ~90s, then:
podman exec nopass-systemd journalctl -t nopass-helper --no-pager
podman exec nopass-systemd ls /etc/sudoers.d/          # the rule is gone

# Reboot cleanup, observed directly:
podman exec nopass-systemd sh -c \
    'echo "# Generated by NoPass — do not edit manually
# nopass-user: demo
# nopass-expires: reboot
#$(podman exec nopass-systemd id -u demo) ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/90-nopass-$(podman exec nopass-systemd id -u demo)'
podman restart nopass-systemd
podman exec nopass-systemd systemctl status nopass-cleanup.service
podman exec nopass-systemd ls /etc/sudoers.d/          # reboot-marked rule is gone

podman rm -f nopass-systemd
```

`--systemd=always --privileged -v /sys/fs/cgroup:/sys/fs/cgroup:rw` is
podman's standard recipe for booting a real systemd as PID 1 inside a
container. `--privileged` is normally something you'd refuse; here it is
exactly what a disposable, throwaway container is for.

## Checklist

- [ ] `cargo test --workspace` and `cargo build --release` both exit 0 on your own machine.
- [ ] `bash scripts/run-lane-b.sh` exits 0 (directly, or via `Containerfile.dbus` if your machine has no `dbus-run-session`).
- [ ] `bash scripts/run-lane-root.sh` exits 0 (builds and runs both the Debian and Fedora root-lane images).
- [ ] If you touched `timer.rs`, `fileops.rs`, `lock.rs`, `checks.rs`, or `ops.rs`, you re-ran the root lane at least once — the unprivileged lane cannot see a real `root:root` file or a real `visudo`.
- [ ] If you touched `tray.rs`, `notifications.rs`, `instance.rs`, or anything `crates/nopass/tests/dbus_session.rs` exercises, you re-ran Lane B at least once — the unprivileged lane never opens a bus connection.
- [ ] The manual systemd lane is untouched by your change unless you specifically need to verify timer/boot behavior; it is never a release blocker, and has never been run end to end — do not cite it as passing evidence.
- [ ] If your change touches `crates/nopass/` at all, Lane C's checklist (`../manual/README.md`) has been run at least once on a real desktop session and its result recorded in the verify report — it is never a gate, but it is the only place panel rendering and polkit authentication can be proven.

## Next step

Back to `crates/nopass-helper/tests/root_system.rs` for the actual
assertions each root-only scenario proves, or `../../openspec/changes/
m1-core-helper/design.md` §8 for the full testing-strategy table this
lane split implements.

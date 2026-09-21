#!/usr/bin/env bash
# T4 (m4-packaging-and-qa): deb package install/grant/remove/purge
# lifecycle, run against the real .deb built inside Containerfile.deb.
# Proves PRD §10 line 330 / tests/manual/README.md item 18's automated
# counterpart: uninstalling NoPass never leaves a live 90-nopass-* rule in
# /etc/sudoers.d/, observed for BOTH `apt remove` and `apt purge`.
#
# Runs as root inside a disposable container (scripts/run-lane-deb.sh). No
# pkexec/PKEXEC_UID is involved anywhere here -- the grant step calls the
# INSTALLED helper's root-context `grant` subcommand directly
# (crates/nopass-helper/src/uid.rs: real uid 0, PKEXEC_UID unset), the same
# invocation crates/nopass-helper/tests/root_system.rs's
# `grant_succeeds_with_no_session_environment_present` and
# `install_grant_inspect_revoke_completes_end_to_end_with_no_session` already
# prove against the unpackaged binary -- this lane proves the packaged,
# installed one.
#
# `--until-reboot`, not `--until <seconds>`: these containers ship no
# systemd deliberately (Containerfile.debian's own comment; design.md §8),
# so a timed grant's systemd-run step genuinely fails and the rule is
# rolled back on the spot (ops.rs's enable_inner step 15) -- exactly the
# behaviour root_system.rs's
# real_timer_scheduling_failure_rolls_back_the_rule_when_systemd_is_unavailable
# test pins. A reboot-scoped grant never calls systemd-run at all
# (enable_inner: "Schedule a new timer only for At"), so it is the only
# grant kind that leaves a live rule on disk for this lane to observe
# removing.
set -euo pipefail
cd /workspace

DEB_PATH=$(ls "$PWD"/target/debian/*.deb 2>/dev/null | head -n1)
if [ -z "$DEB_PATH" ]; then
    echo "deb-lifecycle: no .deb found under target/debian/ -- did the Containerfile.deb build step run?" >&2
    exit 1
fi
echo "deb-lifecycle: using package $DEB_PATH"

USER_NAME=nopasslanetest
BASELINE_SUDOERS=/etc/sudoers.d/00-nopasslanetest-baseline

cleanup_user() {
    rm -f "$BASELINE_SUDOERS"
    userdel -f "$USER_NAME" >/dev/null 2>&1 || true
}
trap cleanup_user EXIT

# A fresh, disposable, no-home test user -- mirrors
# crates/nopass-helper/tests/root_system.rs's create_test_user/
# grant_baseline_sudo exactly, including the real, visudo -cf validated
# baseline sudo grant that checks::is_sudoer's real
# `sudo -n -l -U <user> sh` probe requires. NoPass upgrades an EXISTING
# sudoer's interactive password prompt; it never creates sudo access from
# nothing, so a grant against a user with no baseline sudo entry is
# rejected before any rule is ever written.
userdel -f "$USER_NAME" >/dev/null 2>&1 || true
useradd -M "$USER_NAME"
TARGET_UID=$(id -u "$USER_NAME")

BASELINE_CHECK=$(mktemp)
printf '%s ALL=(ALL:ALL) ALL\n' "$USER_NAME" >"$BASELINE_CHECK"
if ! visudo -cf "$BASELINE_CHECK"; then
    echo "deb-lifecycle: baseline sudoers snippet failed a real visudo -cf" >&2
    rm -f "$BASELINE_CHECK"
    exit 1
fi
install -m 0440 "$BASELINE_CHECK" "$BASELINE_SUDOERS"
rm -f "$BASELINE_CHECK"

# The assertion this whole lane exists for: no NoPass-managed sudoers rule
# survives in /etc/sudoers.d/. `compgen -G` never errors on zero matches, so
# a genuinely clean directory does not trip `set -e` here.
assert_no_live_rule() {
    local phase="$1"
    local matches
    matches=$(compgen -G '/etc/sudoers.d/90-nopass-*' || true)
    if [ -n "$matches" ]; then
        echo "FAIL (${phase}): a live 90-nopass-* rule remains in /etc/sudoers.d/ after ${phase}:" >&2
        ls -la /etc/sudoers.d/ >&2
        exit 1
    fi
    echo "OK (${phase}): no 90-nopass-* rule present in /etc/sudoers.d/"
}

# PRD §10 line 330 asks purge to leave three things behind: no rule file,
# no timer, and no /run/nopass. The rule file is asserted above. Timers
# cannot be asserted here -- these containers ship no systemd on purpose
# (Containerfile.debian's own comment; design.md §8), so `systemctl
# list-timers` has nothing to answer; Lane C step 17 covers that on a real
# machine. The runtime directory CAN be asserted, and is, because postinst
# creates it with systemd-tmpfiles at install time and something has to
# remove it at purge time.
assert_no_runtime_dir() {
    local phase="$1"
    if [ -e /run/nopass ]; then
        echo "FAIL (${phase}): /run/nopass still exists after ${phase}:" >&2
        ls -la /run/nopass >&2 || true
        exit 1
    fi
    echo "OK (${phase}): /run/nopass is gone"
}

# Non-vacuity self-test (m4-packaging-and-qa T4 verification discipline: an
# assertion that can never fail proves nothing). With
# NOPASS_LANE_SELFTEST_ROGUE_RULE=1, skip the real apt lifecycle entirely
# and instead plant a rogue /etc/sudoers.d/90-nopass-selftest file by hand,
# then run assert_no_live_rule against it. It MUST fail.
#
# This cannot be done by planting the rogue file inside the real
# remove/purge flow below: ANY `90-nopass-*` file, planted or genuine, is
# also caught by crates/nopass/debian/prerm's own blind
# `rm -f /etc/sudoers.d/90-nopass-*` catch-all (verified while authoring
# this lane -- a first version planted the rogue file right after grant #1
# and the lane still reported PASS, because prerm's own cleanup, not a gap
# in it, removed the injection too). Proving `assert_no_live_rule` itself
# is not vacuous therefore has to bypass apt and prerm altogether and call
# it directly against a file it did not create.
#
# Reproduce on demand: `NOPASS_LANE_SELFTEST_ROGUE_RULE=1 bash
# scripts/run-lane-deb.sh` must fail with a `FAIL (self-test): ...`
# message; a plain run must not. Never set by run-lane-deb.sh itself.
if [ "${NOPASS_LANE_SELFTEST_ROGUE_RULE:-0}" = "1" ]; then
    echo "== self-test: planting a rogue rule the real lifecycle never runs =="
    printf '# planted by NOPASS_LANE_SELFTEST_ROGUE_RULE, not by NoPass\n' >/etc/sudoers.d/90-nopass-selftest
    chmod 0440 /etc/sudoers.d/90-nopass-selftest
    assert_no_live_rule "self-test"
    # Unreachable if assert_no_live_rule works: it exits 1 above. Reaching
    # here means the checker is vacuous -- fail loudly rather than let the
    # self-test itself report a false PASS.
    echo "deb-lifecycle: self-test rogue rule was NOT detected -- assert_no_live_rule is vacuous" >&2
    exit 1
fi

# Creates a real grant through the INSTALLED helper, then confirms the rule
# actually landed before trusting anything downstream of it.
create_grant() {
    local phase="$1"
    /usr/libexec/nopass-helper grant --uid "$TARGET_UID" --until-reboot
    if [ ! -e "/etc/sudoers.d/90-nopass-${TARGET_UID}" ]; then
        echo "FAIL (${phase}): grant reported success but /etc/sudoers.d/90-nopass-${TARGET_UID} does not exist" >&2
        exit 1
    fi
    echo "OK (${phase}): grant created /etc/sudoers.d/90-nopass-${TARGET_UID}"
}

echo "== apt-get update =="
apt-get update

echo "== install =="
apt-get install -y "$DEB_PATH"

echo "== grant #1 =="
create_grant remove

echo "== apt-get remove =="
apt-get remove -y nopass
assert_no_live_rule "apt remove"

echo "== reinstall =="
apt-get install -y "$DEB_PATH"

echo "== grant #2 =="
create_grant purge

echo "== apt-get purge =="
apt-get purge -y nopass
assert_no_live_rule "apt purge"
assert_no_runtime_dir "apt purge"

echo "deb-lifecycle: PASS -- both apt remove and apt purge leave no live 90-nopass-* rule, and purge leaves no /run/nopass"

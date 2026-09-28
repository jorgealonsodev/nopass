#!/usr/bin/env bash
# T6 (m4-packaging-and-qa): install the real RPM built by Containerfile.rpm,
# grant through its installed helper, then prove dnf removal leaves no live
# 90-nopass-* rule in /etc/sudoers.d/.
set -euo pipefail
cd /workspace

RPM_PATH=$(find target -type f -name '*.rpm' -print -quit)
if [ -z "$RPM_PATH" ]; then
    echo "rpm-lifecycle: no RPM found under target/ -- did Containerfile.rpm build it?" >&2
    exit 1
fi
echo "rpm-lifecycle: using package $RPM_PATH"

USER_NAME=nopasslanetest
BASELINE_SUDOERS=/etc/sudoers.d/00-nopasslanetest-baseline

cleanup_user() {
    rm -f "$BASELINE_SUDOERS"
    userdel -f "$USER_NAME" >/dev/null 2>&1 || true
}
trap cleanup_user EXIT

# Check the package's removal contract against the actual live sudoers
# directory. A clean directory is not an error: compgen returns no matches
# and the empty result is asserted explicitly below.
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

# Non-vacuity self-test: bypass dnf and the RPM scriptlets, plant a rogue
# rule, and require the same assertion used after package removal to catch
# it. This ensures the package's cleanup cannot make the assertion pass by
# deleting the injected file before it is inspected.
if [ "${NOPASS_LANE_SELFTEST_ROGUE_RULE:-0}" = "1" ]; then
    echo "== self-test: planting a rogue rule the real lifecycle never runs =="
    printf '# planted by NOPASS_LANE_SELFTEST_ROGUE_RULE, not by NoPass\n' \
        >/etc/sudoers.d/90-nopass-selftest
    chmod 0440 /etc/sudoers.d/90-nopass-selftest
    assert_no_live_rule "self-test"
    echo "rpm-lifecycle: self-test rogue rule was NOT detected -- assert_no_live_rule is vacuous" >&2
    exit 1
fi

# A fresh sudo-capable account matches the helper's normal grant path: NoPass
# augments an existing sudo rule and must not manufacture sudo access for a
# user who has none.
userdel -f "$USER_NAME" >/dev/null 2>&1 || true
useradd -M "$USER_NAME"
TARGET_UID=$(id -u "$USER_NAME")

BASELINE_CHECK=$(mktemp)
printf '%s ALL=(ALL:ALL) ALL\n' "$USER_NAME" >"$BASELINE_CHECK"
if ! visudo -cf "$BASELINE_CHECK"; then
    echo "rpm-lifecycle: baseline sudoers snippet failed a real visudo -cf" >&2
    rm -f "$BASELINE_CHECK"
    exit 1
fi
install -m 0440 "$BASELINE_CHECK" "$BASELINE_SUDOERS"
rm -f "$BASELINE_CHECK"

echo "== dnf install =="
dnf install -y shadow-utils "$RPM_PATH"

if [ ! -x /usr/libexec/nopass-helper ]; then
    echo "rpm-lifecycle: installed /usr/libexec/nopass-helper is missing or not executable" >&2
    exit 1
fi

# Fedora's package installation must not enable this user-owned policy. A
# systemctl query is meaningful only when the system manager responds; the
# Fedora container normally has no running systemd manager, so report that
# limitation rather than treating its absence as evidence either way.
assert_cleanup_service_not_enabled() {
    if ! command -v systemctl >/dev/null 2>&1; then
        echo "SKIP: systemctl is unavailable; cannot inspect cleanup service enablement"
        return
    fi
    if ! systemctl show-environment >/dev/null 2>&1; then
        echo "SKIP: systemd manager is unavailable; cannot inspect cleanup service enablement"
        return
    fi

    local state
    state=$(systemctl show --property=UnitFileState --value nopass-cleanup.service)
    if [ -z "$state" ]; then
        echo "FAIL: systemctl answered but returned no enablement state for nopass-cleanup.service" >&2
        exit 1
    fi
    case "$state" in
        enabled|enabled-runtime)
            echo "FAIL: nopass-cleanup.service was package-enabled (state: $state)" >&2
            exit 1
            ;;
        *)
            echo "OK: nopass-cleanup.service was not package-enabled (state: $state)"
            ;;
    esac
}
assert_cleanup_service_not_enabled

# The container has no running systemd manager, so --until-reboot is the
# grant form that leaves a live rule for RPM's pre-uninstall scriptlet to
# revoke without a timer immediately rolling it back.
echo "== grant through installed helper =="
/usr/libexec/nopass-helper grant --uid "$TARGET_UID" --until-reboot
if [ ! -e "/etc/sudoers.d/90-nopass-${TARGET_UID}" ]; then
    echo "FAIL: grant reported success but /etc/sudoers.d/90-nopass-${TARGET_UID} does not exist" >&2
    exit 1
fi
echo "OK: installed helper created /etc/sudoers.d/90-nopass-${TARGET_UID}"

echo "== dnf remove =="
dnf remove -y nopass
assert_no_live_rule "dnf remove"
echo "rpm-lifecycle: PASS -- dnf remove leaves no live 90-nopass-* rule"

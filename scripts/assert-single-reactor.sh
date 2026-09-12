#!/usr/bin/env bash
# The G1 gate (design.md §0, Architecture Decision D1): the tray's
# dependency tree stays on exactly one `async-io` reactor and pulls in no
# `tokio`, no libdbus (`dbus` crate), and no GTK/Qt (`glib`/`gtk` crates).
#
# This is deliberately sharper than "no zbus tokio feature": two `async-io`
# MAJOR versions in the tree (for example `ksni` and `notify-rust` pulling
# different `zbus` majors) would produce two reactor threads with no tokio
# anywhere, and a feature-name-only assertion would miss that entirely.
# `ksni` documents a panic for exactly that mixed-reactor configuration.
#
# Run from the workspace root: scripts/assert-single-reactor.sh
# Exit 0: every assertion holds and the release build succeeds.
# Exit non-zero: prints which assertion(s) failed and stops before build.
set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

fail=0

# `cargo tree -i <pkg>` exits non-zero with empty stdout when <pkg> is not
# in the dependency tree at all — that IS the pass condition for crates
# that must be absent, so a non-zero exit here is expected and is not
# itself a script failure. Only non-empty stdout (the package present)
# fails this assertion.
assert_absent() {
    local pkg="$1"
    local reason="$2"
    local out
    out="$(cargo tree -i "$pkg" --workspace 2>/dev/null || true)"
    if [ -n "$out" ]; then
        echo "FAIL: '$pkg' is present in the dependency tree ($reason)" >&2
        echo "$out" >&2
        fail=1
    fi
}

# Counts distinct MAJOR.MINOR.PATCH versions of <pkg> that `cargo tree -i`
# reports as tree roots (the "<pkg> vX.Y.Z" header line of each inverted
# subtree), not the number of consumers.
count_versions() {
    local pkg="$1"
    cargo tree -i "$pkg" --workspace 2>/dev/null \
        | grep -E "^${pkg} v[0-9]+\.[0-9]+\.[0-9]+" \
        | awk '{print $2}' \
        | sort -u
}

assert_absent tokio "no reactor other than async-io may be present"
assert_absent dbus "libdbus C backend must not appear"
assert_absent glib "NFR: no GTK/Qt"
assert_absent gtk "NFR: no GTK/Qt"

async_io_versions="$(count_versions async-io)"
async_io_count="$(printf '%s\n' "$async_io_versions" | grep -c . || true)"
if [ "$async_io_count" -ne 1 ]; then
    echo "FAIL: expected exactly one async-io major version in the tree, found $async_io_count" >&2
    printf '%s\n' "$async_io_versions" >&2
    fail=1
fi

# More than one zbus version is the same failure class as a second
# async-io major (mismatched zbus majors pulled by ksni vs. notify-rust) —
# design.md calls it a "stop and review" condition, treated here as a hard
# gate failure rather than a silent pass.
zbus_versions="$(count_versions zbus)"
zbus_count="$(printf '%s\n' "$zbus_versions" | grep -c . || true)"
if [ "$zbus_count" -gt 1 ]; then
    echo "FAIL: more than one zbus version in the tree, stop and review" >&2
    printf '%s\n' "$zbus_versions" >&2
    fail=1
fi

if cargo tree -e features -p nopass 2>/dev/null | grep -qF 'zbus feature "tokio"'; then
    echo 'FAIL: zbus feature "tokio" is enabled somewhere in the tree' >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "assert-single-reactor: FAILED" >&2
    exit 1
fi

echo "assert-single-reactor: exactly one async-io major ($async_io_versions), zero tokio, no dbus/glib/gtk, zbus tokio feature disabled"

if ! cargo +1.85 build --release -p nopass; then
    echo "assert-single-reactor: cargo +1.85 build --release -p nopass FAILED" >&2
    exit 1
fi

echo "assert-single-reactor: PASS"

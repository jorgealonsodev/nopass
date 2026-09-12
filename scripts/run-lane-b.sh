#!/usr/bin/env bash
# Lane B: the tests that need a real session bus but no desktop.
#
# Always use this script rather than a bare `dbus-run-session`. See
# tests/dbus/session-isolated.conf for why: a plain nested session still
# activates the host's notification daemon and four tests fail on any
# machine that has one installed.
set -euo pipefail
cd "$(dirname "$0")/.."
exec dbus-run-session --config-file=tests/dbus/session-isolated.conf -- \
    env NOPASS_DBUS_TESTS=1 cargo test -p nopass --test dbus_session "$@"

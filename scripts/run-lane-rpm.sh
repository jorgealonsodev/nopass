#!/usr/bin/env bash
# Lane RPM: builds the real Fedora RPM in Containerfile.rpm, installs it,
# creates a live grant through the installed helper, and proves dnf removal
# leaves no 90-nopass-* rule in /etc/sudoers.d/ (m4-packaging-and-qa T6).
#
# Same runtime-detection pattern as the Debian and root lanes: Docker first,
# Podman as fallback. The self-test bypasses the real lifecycle, plants a
# rogue rule, and must fail in the fixture's no-live-rule assertion.
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v docker >/dev/null 2>&1; then
    runtime=docker
elif command -v podman >/dev/null 2>&1; then
    runtime=podman
else
    echo "run-lane-rpm.sh: neither docker nor podman found on PATH" >&2
    exit 1
fi

image="nopass-test-rpm"
"$runtime" build -f tests/containers/Containerfile.rpm -t "$image" .
"$runtime" run --rm -e NOPASS_LANE_SELFTEST_ROGUE_RULE "$image" \
    bash tests/containers/fixtures/rpm-lifecycle.sh

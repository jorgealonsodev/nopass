#!/usr/bin/env bash
# Lane Deb: builds the real .deb from the workspace inside a disposable
# Debian container, installs it, creates a real grant through the installed
# helper, then proves `apt remove` and `apt purge` both leave no live
# `90-nopass-*` rule in /etc/sudoers.d/ (m4-packaging-and-qa T4; PRD §10
# line 330; tests/manual/README.md item 18's automated counterpart).
#
# Every other container lane in this tree builds and runs the crates
# directly -- none of them installs a package. Before this lane existed,
# "uninstalling NoPass never leaves a live passwordless-sudo rule behind"
# was a code-reading inference over crates/nopass/debian/prerm, never an
# observed fact. This lane observes it.
#
# Same runtime-detection pattern as scripts/run-lane-root.sh,
# scripts/run-lane-journal.sh and scripts/run-lane-polkit.sh -- docker
# first, podman as fallback, non-zero exit when neither is present.
# cargo-deb is NOT installed on the host; Containerfile.deb installs it
# inside the disposable image and bakes the .deb build itself there, so
# nothing about this lane depends on the host's own toolchain.
#
# Set NOPASS_LANE_SELFTEST_ROGUE_RULE=1 to reproduce this lane's own
# non-vacuity proof: the real apt lifecycle is skipped, a rogue
# 90-nopass-* file is planted by hand, and the lane must then FAIL. It
# cannot be planted inside the real flow instead -- prerm's own blind
# `rm -f /etc/sudoers.d/90-nopass-*` removes any such file, planted or
# genuine, so the lane would still report PASS and prove nothing. See
# tests/containers/fixtures/deb-lifecycle.sh's self-test block.
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v docker >/dev/null 2>&1; then
    runtime=docker
elif command -v podman >/dev/null 2>&1; then
    runtime=podman
else
    echo "run-lane-deb.sh: neither docker nor podman found on PATH" >&2
    exit 1
fi

image="nopass-test-deb"
"$runtime" build -f tests/containers/Containerfile.deb -t "$image" .
"$runtime" run --rm -e NOPASS_LANE_SELFTEST_ROGUE_RULE "$image" \
    bash tests/containers/fixtures/deb-lifecycle.sh

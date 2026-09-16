#!/usr/bin/env bash
# Lane R-J: the journald read-back lane (design.md §0 G1, §8 "Testing
# strategy"; task 8.2). Builds and runs Containerfile.journald, which
# starts a standalone systemd-journald with no PID 1 systemd and runs
# crates/nopass-helper/tests/root_journal.rs against it
# (NOPASS_JOURNAL_TESTS=1).
#
# Same runtime-detection pattern as scripts/run-lane-root.sh — docker
# first, podman as fallback. Never hardcode either runtime.
#
# journald prints a non-fatal `Failed to join audit multicast group ...
# Ignoring` line to stderr on start (design.md §0 G1). This script MUST
# NOT treat that stderr content as failure — only the exit code counts.
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v docker >/dev/null 2>&1; then
    runtime=docker
elif command -v podman >/dev/null 2>&1; then
    runtime=podman
else
    echo "run-lane-journal.sh: neither docker nor podman found on PATH" >&2
    exit 1
fi

image="nopass-test-journald"
"$runtime" build -f tests/containers/Containerfile.journald -t "$image" .
"$runtime" run --rm -e NOPASS_JOURNAL_TESTS=1 "$image"

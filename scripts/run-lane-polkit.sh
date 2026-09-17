#!/usr/bin/env bash
# Rank 2 hardening gate — the real-polkit-authority lane for
# data/com.enfoquestic.nopass.policy (design.md §6.2; task 10.2;
# privilege-admission "The real polkit engine enumerates the installed
# action"). Builds and runs Containerfile.polkit, which starts a private
# system bus and a real polkitd and runs
# crates/nopass-helper/tests/polkit_contract.rs
# (NOPASS_POLKIT_TESTS=1).
#
# Same runtime-detection pattern as scripts/run-lane-root.sh and
# scripts/run-lane-journal.sh — docker first, podman as fallback,
# non-zero exit when neither is present. This is half of
# privilege-admission "Absence of a polkit authority fails the gate, not
# skips it"; the other half is Containerfile.polkit's own baked-in
# polkitd liveness check before it ever invokes cargo test.
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v docker >/dev/null 2>&1; then
    runtime=docker
elif command -v podman >/dev/null 2>&1; then
    runtime=podman
else
    echo "run-lane-polkit.sh: neither docker nor podman found on PATH" >&2
    exit 1
fi

image="nopass-test-polkit"
"$runtime" build -f tests/containers/Containerfile.polkit -t "$image" .
"$runtime" run --rm -e NOPASS_POLKIT_TESTS=1 "$image"

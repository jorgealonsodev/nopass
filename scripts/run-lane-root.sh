#!/usr/bin/env bash
# Lane R: the root-only tests that need a real root and a real
# /etc/sudoers.d — crates/nopass-helper/tests/root_system.rs, gated on
# NOPASS_ROOT_TESTS=1 (design.md §8 "The execution gap, named and
# closed"). Before this script existed, that gate was named by no runner
# script at all, so every one of its 15 tests reported `ok` in a lane that
# never executed. Builds and runs both distro images so the root lane is
# one named command with an exit status, not a podman recipe a human has
# to retype from tests/containers/README.md.
#
# Detects the available container runtime — docker first, podman as
# fallback, since either accepts the same `build`/`run` flags this script
# needs. Never hardcode either runtime.
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v docker >/dev/null 2>&1; then
    runtime=docker
elif command -v podman >/dev/null 2>&1; then
    runtime=podman
else
    echo "run-lane-root.sh: neither docker nor podman found on PATH" >&2
    exit 1
fi

# `--test root_system`, NOT `cargo test --workspace`, even though
# tests/containers/README.md said --workspace for two milestones. That
# command was never actually run, so nobody discovered it cannot pass
# here, for two independent reasons:
#
#   1. Running the whole workspace as root asserts that every test passes
#      under root. That is a different claim, and a false one: some tests
#      exist precisely to pin NON-root behaviour. `state_tempdir.rs`'s
#      an_unreadable_directory_reads_as_faulted_io makes a directory
#      unreadable and expects the read to fault — root bypasses the
#      permission check, so the test correctly reports a different result.
#      A lane that fails on it is punishing a test for being right.
#   2. `Containerfile.debian`/`.fedora` deliberately ship no systemd, and
#      that absence is load-bearing: it is what produces the genuine
#      exit-17 rollback in real_timer_scheduling_failure_rolls_back_the_
#      rule_when_systemd_is_unavailable. But nopass-core's
#      timefmt::systemd_contract tests require `systemd-analyze` and fail
#      loudly when it is missing (verify-report.md H3, deliberately). Both
#      behaviours are correct; they just cannot share one lane.
#
# So this lane runs exactly what it exists for: the tests that need a real
# root and a real /etc/sudoers.d.
for distro in debian fedora; do
    image="nopass-test-${distro}"
    "$runtime" build -f "tests/containers/Containerfile.${distro}" -t "$image" .
    "$runtime" run --rm -e NOPASS_ROOT_TESTS=1 "$image" \
        cargo test -p nopass-helper --test root_system
done

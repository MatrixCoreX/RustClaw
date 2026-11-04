#!/usr/bin/env bash
set -euo pipefail

CAPACITY_TEST_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$CAPACITY_TEST_ROOT/scripts/shell_compat.sh"

# The same available-memory/CPU inputs must work for every target architecture.
[[ "$(cargo_jobs_for_host_capacity 14680064 10485760 8)" == 4 ]]
[[ "$(cargo_jobs_for_host_capacity 14680064 9437184 8)" == 2 ]]
[[ "$(cargo_jobs_for_host_capacity 14680064 6291456 8)" == 1 ]]
[[ "$(cargo_jobs_for_host_capacity 33554432 25165824 8)" == 8 ]]
[[ "$(cargo_jobs_for_host_capacity 33554432 6291456 16)" == 1 ]]
[[ "$(cargo_jobs_for_host_capacity 67108864 41943040 64)" == 16 ]]
[[ "$(cargo_jobs_for_host_capacity 134217728 100663296 64)" == 38 ]]
[[ "$(cargo_jobs_for_host_capacity 33554432 25165824 3)" == 3 ]]
[[ "$(cargo_jobs_for_host_capacity 33554432 25165824 1)" == 1 ]]
[[ "$(cargo_jobs_for_host_capacity 2097152 25165824 8)" == 1 ]]

for invalid_capacity in '33554432 0 8' '33554432 25165824 0' '33554432 invalid 8'; do
  # Word splitting here is deliberate: each fixture contains three scalar inputs.
  if cargo_jobs_for_host_capacity $invalid_capacity >/dev/null 2>&1; then
    echo "invalid capacity unexpectedly accepted: $invalid_capacity" >&2
    exit 1
  fi
done

for architecture in x86_64 aarch64 armv7; do
  (
    detect_host_arch() { printf '%s\n' "$architecture"; }
    cargo_host_memory_kb() { printf '%s\n' 33554432; }
    cargo_host_available_memory_kb() { printf '%s\n' 25165824; }
    cargo_host_cpu_count() { printf '%s\n' 8; }
    [[ "$(cargo_jobs_for_small_host)" == 8 ]]
    unset CARGO_BUILD_JOBS
    configure_cargo_build_jobs_for_small_host >/dev/null
    [[ "$CARGO_BUILD_JOBS" == 8 ]]

    cargo_host_available_memory_kb() { printf '%s\n' 1048576; }
    unset CARGO_BUILD_JOBS
    configure_cargo_build_jobs_for_small_host >/dev/null
    [[ "$CARGO_BUILD_JOBS" == 1 ]]
  )
done

(
  cargo_jobs_for_small_host() { return 1; }
  unset CARGO_BUILD_JOBS
  configure_cargo_build_jobs_for_small_host >/dev/null
  [[ "$CARGO_BUILD_JOBS" == 1 ]]
)

for explicit_jobs in 3 -1; do
  (
    cargo_jobs_for_small_host() {
      echo "explicit override must not invoke resource detection" >&2
      exit 1
    }
    CARGO_BUILD_JOBS="$explicit_jobs"
    export CARGO_BUILD_JOBS
    configure_cargo_build_jobs_for_small_host >/dev/null
    [[ "$CARGO_BUILD_JOBS" == "$explicit_jobs" ]]
  )
done

echo "SHELL_CAPACITY_TESTS ok"

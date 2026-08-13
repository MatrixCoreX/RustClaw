#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=shell_compat.sh
source "$ROOT_DIR/scripts/shell_compat.sh"

[[ "$(cargo_jobs_for_host_capacity x86_64 14680064 10485760 8)" == "4" ]]
[[ "$(cargo_jobs_for_host_capacity x86_64 14680064 9437184 8)" == "2" ]]
[[ "$(cargo_jobs_for_host_capacity x86_64 14680064 6291456 8)" == "1" ]]
[[ "$(cargo_jobs_for_host_capacity aarch64 33554432 25165824 8)" == "1" ]]
if cargo_jobs_for_host_capacity x86_64 33554432 25165824 8 >/dev/null; then
  echo "large host unexpectedly received a small-host Cargo jobs override" >&2
  exit 1
fi

[[ "$(default_macos_deployment_target "13.7.8")" == "13.0" ]]
[[ "$(default_macos_deployment_target "15.2")" == "15.0" ]]
[[ "$(default_macos_deployment_target "10.15.7")" == "10.15" ]]
if default_macos_deployment_target "invalid" >/dev/null 2>&1; then
  echo "invalid macOS version unexpectedly produced a deployment target" >&2
  exit 1
fi

unset MACOSX_DEPLOYMENT_TARGET APP_MACOS_DEPLOYMENT_TARGET
unset CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS
unset CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS
configure_macos_deployment_target macos 13.7.8
[[ "$MACOSX_DEPLOYMENT_TARGET" == "13.0" ]]
[[ "$CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS" == *"-mmacosx-version-min=13.0"* ]]
[[ "$CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS" == *"-mmacosx-version-min=13.0"* ]]

MACOSX_DEPLOYMENT_TARGET=12.0
export MACOSX_DEPLOYMENT_TARGET
unset CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS
unset CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS
configure_macos_deployment_target macos 13.7.8
[[ "$MACOSX_DEPLOYMENT_TARGET" == "12.0" ]]
[[ "$CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS" == *"-mmacosx-version-min=12.0"* ]]

unset MACOSX_DEPLOYMENT_TARGET
APP_MACOS_DEPLOYMENT_TARGET=11.3
export APP_MACOS_DEPLOYMENT_TARGET
export CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS="-C target-cpu=native"
unset CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS
configure_macos_deployment_target macos 13.7.8
[[ "$MACOSX_DEPLOYMENT_TARGET" == "11.3" ]]
[[ "$CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS" == *"-C target-cpu=native"* ]]
[[ "$CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS" == *"-mmacosx-version-min=11.3"* ]]
unset MACOSX_DEPLOYMENT_TARGET APP_MACOS_DEPLOYMENT_TARGET
unset CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS
unset CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS

TEST_ROOT="$(mktemp -d)"
DETACHED_TEST_PID=""
trap '[[ -n "$DETACHED_TEST_PID" ]] && kill "$DETACHED_TEST_PID" 2>/dev/null || true; find "$TEST_ROOT" -type f -delete 2>/dev/null || true; rmdir "$TEST_ROOT/.cargo" "$TEST_ROOT" 2>/dev/null || true' EXIT
mkdir -p "$TEST_ROOT/.cargo"

launch_detached_process "$TEST_ROOT/detached.log" sh -c 'sleep 30'
DETACHED_TEST_PID="$DETACHED_PROCESS_PID"
kill -0 "$DETACHED_TEST_PID"
kill "$DETACHED_TEST_PID"
wait "$DETACHED_TEST_PID" 2>/dev/null || true
DETACHED_TEST_PID=""

PATH_HEAD="$TEST_ROOT/rustup-bin"
PATH_TAIL="$TEST_ROOT/homebrew-bin"
mkdir -p "$PATH_HEAD" "$PATH_TAIL"
SAVED_PATH="$PATH"
PATH="$PATH_HEAD"
append_existing_command_path "$PATH_TAIL"
[[ "$PATH" == "$PATH_HEAD:$PATH_TAIL" ]]
append_existing_command_path "$PATH_TAIL"
[[ "$PATH" == "$PATH_HEAD:$PATH_TAIL" ]]
PATH="$SAVED_PATH"
export PATH
rmdir "$PATH_HEAD" "$PATH_TAIL"

USER_CARGO_BIN="$TEST_ROOT/.cargo/bin"
mkdir -p "$USER_CARGO_BIN"
PATH="/usr/bin:/bin"
CARGO_HOME="$TEST_ROOT/.cargo"
export PATH CARGO_HOME
configure_platform_command_path
[[ "$PATH" == "$USER_CARGO_BIN:/usr/bin:/bin" ]]
configure_platform_command_path
[[ "$PATH" == "$USER_CARGO_BIN:/usr/bin:/bin" ]]
PATH="$SAVED_PATH"
export PATH
rmdir "$USER_CARGO_BIN"

unset RUSTC_WRAPPER CARGO_INCREMENTAL CI
HOME="$TEST_ROOT"
CARGO_HOME="$TEST_ROOT/.cargo"
export HOME CARGO_HOME
cd "$TEST_ROOT"

cat >"$CARGO_HOME/config.toml" <<'EOF'
[build]
rustc-wrapper = "/usr/bin/sccache"
EOF
configure_cargo_build_environment >/dev/null
[[ -z "${CARGO_INCREMENTAL:-}" ]]
[[ "$CARGO_PROFILE_DEV_INCREMENTAL" == "false" ]]
[[ "$CARGO_PROFILE_TEST_INCREMENTAL" == "false" ]]
[[ "$CARGO_PROFILE_RELEASE_INCREMENTAL" == "false" ]]
[[ "$CARGO_PROFILE_BENCH_INCREMENTAL" == "false" ]]

cat >"$CARGO_HOME/config.toml" <<'EOF'
[build]
target-dir = "target"
EOF
unset CARGO_INCREMENTAL
configure_cargo_build_environment >/dev/null
[[ "$CARGO_INCREMENTAL" == "1" ]]

RUSTC_WRAPPER="/opt/homebrew/bin/sccache"
CARGO_INCREMENTAL=1
export RUSTC_WRAPPER CARGO_INCREMENTAL
configure_cargo_build_environment >/dev/null
[[ -z "${CARGO_INCREMENTAL:-}" ]]

echo "SHELL_COMPAT_TESTS ok"

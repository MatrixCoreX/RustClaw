#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=shell_compat.sh
source "$ROOT_DIR/scripts/shell_compat.sh"

[[ "$(default_macos_deployment_target "13.7.8")" == "13.0" ]]
[[ "$(default_macos_deployment_target "15.2")" == "15.0" ]]
[[ "$(default_macos_deployment_target "10.15.7")" == "10.15" ]]
if default_macos_deployment_target "invalid" >/dev/null 2>&1; then
  echo "invalid macOS version unexpectedly produced a deployment target" >&2
  exit 1
fi

unset MACOSX_DEPLOYMENT_TARGET RUSTCLAW_MACOS_DEPLOYMENT_TARGET
configure_macos_deployment_target macos 13.7.8
[[ "$MACOSX_DEPLOYMENT_TARGET" == "13.0" ]]

MACOSX_DEPLOYMENT_TARGET=12.0
export MACOSX_DEPLOYMENT_TARGET
configure_macos_deployment_target macos 13.7.8
[[ "$MACOSX_DEPLOYMENT_TARGET" == "12.0" ]]

unset MACOSX_DEPLOYMENT_TARGET
RUSTCLAW_MACOS_DEPLOYMENT_TARGET=11.3
export RUSTCLAW_MACOS_DEPLOYMENT_TARGET
configure_macos_deployment_target macos 13.7.8
[[ "$MACOSX_DEPLOYMENT_TARGET" == "11.3" ]]
unset MACOSX_DEPLOYMENT_TARGET RUSTCLAW_MACOS_DEPLOYMENT_TARGET

TEST_ROOT="$(mktemp -d)"
trap 'find "$TEST_ROOT" -type f -delete 2>/dev/null || true; rmdir "$TEST_ROOT/.cargo" "$TEST_ROOT" 2>/dev/null || true' EXIT
mkdir -p "$TEST_ROOT/.cargo"

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

#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=shell_compat.sh
source "$ROOT_DIR/scripts/shell_compat.sh"

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

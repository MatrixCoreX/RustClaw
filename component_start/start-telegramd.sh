#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/component_start/common.sh"
component_start_init "$SCRIPT_DIR" "${1:-}" "./component_start/start-telegramd.sh"

BIN_NAME="telegramd"
BIN_PATH="$(component_require_binary "$BIN_NAME")"

component_exec_binary "$BIN_NAME" "$BIN_PATH"

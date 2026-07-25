#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/component_start/common.sh"
component_start_init "$SCRIPT_DIR" "${1:-}" "./component_start/start-feishud.sh"

BIN_NAME="feishud"
BIN_PATH="$(component_require_binary "$BIN_NAME")"

# Config path: same as feishud default, explicit for scripts
export FEISHU_CONFIG_PATH="${FEISHU_CONFIG_PATH:-$COMPONENT_CHANNEL_CONFIG_DIR/feishu.toml}"

component_exec_binary "$BIN_NAME" "$BIN_PATH"

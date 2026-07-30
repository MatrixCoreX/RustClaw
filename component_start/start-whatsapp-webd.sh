#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/component_start/common.sh"
component_start_init "$SCRIPT_DIR" "${1:-}" "./component_start/start-whatsapp-webd.sh"

BIN_NAME="whatsapp_webd"
BIN_PATH="$(component_require_binary "$BIN_NAME")"

bash "$COMPONENT_ROOT/scripts/whatsapp_web_bridge_deps.sh" --ensure
component_exec_binary "$BIN_NAME" "$BIN_PATH"

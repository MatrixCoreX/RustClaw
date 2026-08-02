#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/component_start/common.sh"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/scripts/shell_compat.sh"
configure_platform_command_path
configure_python3_with_tomllib
component_start_init "$SCRIPT_DIR" release "./component_start/start-wa-web-bridge.sh"

enabled="$(
python3 "$SCRIPT_DIR/scripts/whatsapp_web_config_state.py" \
  "$APP_CHANNEL_CONFIG_DIR/whatsapp-web.toml"
)"

if [[ "$enabled" != "1" ]]; then
  echo "whatsapp_web.enabled=false, skip starting wa-web-bridge." # zh: whatsapp_web.enabled=false，跳过启动。
  exit 0
fi

BRIDGE_DIR="$SCRIPT_DIR/services/wa-web-bridge"
if [[ ! -d "$BRIDGE_DIR" ]]; then
  echo "bridge dir missing: $BRIDGE_DIR"
  exit 1
fi

if component_pid_is_running "wa-web-bridge" "$BRIDGE_DIR/index.js"; then
  echo "wa-web-bridge is already running, skip." # zh: wa-web-bridge 已在运行，跳过。
  exit 0
fi

bash "$SCRIPT_DIR/scripts/whatsapp_web_bridge_deps.sh" --ensure

component_write_pid_file "wa-web-bridge" "$$"
exec node "$BRIDGE_DIR/index.js"

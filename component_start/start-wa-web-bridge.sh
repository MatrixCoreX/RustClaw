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
python3 - <<'PY'
import os
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path(os.environ["APP_CONFIG_PATH"]).read_text(encoding="utf-8"))
channel_dir = Path(os.environ["APP_CHANNEL_CONFIG_DIR"])
for name in ("whatsapp.toml", "whatsapp-web.toml"):
    extra = channel_dir / name
    if extra.exists():
        cfg.update(tomllib.loads(extra.read_text(encoding="utf-8")))
print("1" if bool(cfg.get("whatsapp_web", {}).get("enabled", False)) else "0")
PY
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

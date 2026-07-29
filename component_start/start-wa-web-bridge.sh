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

if ! command -v node >/dev/null 2>&1; then
  echo "node not found. Please install Node.js 18+." # zh: 未找到 node，请先安装 Node.js 18+
  exit 1
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "npm not found. Please install npm." # zh: 未找到 npm，请先安装 npm
  exit 1
fi

enabled="$(
python3 - <<'PY'
import os
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path(os.environ["RUSTCLAW_CONFIG_PATH"]).read_text(encoding="utf-8"))
channel_dir = Path(os.environ["RUSTCLAW_CHANNEL_CONFIG_DIR"])
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

if [[ ! -d "$BRIDGE_DIR/node_modules" ]]; then
  echo "Installing wa-web-bridge dependencies..." # zh: 正在安装 wa-web-bridge 依赖...
  if [[ -f "$BRIDGE_DIR/package-lock.json" ]]; then
    npm --prefix "$BRIDGE_DIR" ci --omit=dev
  else
    npm --prefix "$BRIDGE_DIR" install --omit=dev
  fi
fi

component_write_pid_file "wa-web-bridge" "$$"
exec node "$BRIDGE_DIR/index.js"

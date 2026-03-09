#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Enable colored log tags on interactive terminals unless overridden.
if [[ -t 1 && -z "${RUSTCLAW_LOG_COLOR:-}" ]]; then
  export RUSTCLAW_LOG_COLOR=1
fi

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
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
extra = Path("configs/channels/whatsapp.toml")
if extra.exists():
    cfg.update(tomllib.loads(extra.read_text(encoding="utf-8")))
print("1" if bool(cfg.get("whatsapp_web", {}).get("enabled", False)) else "0")
PY
)"

if [[ "$enabled" != "1" ]]; then
  echo "whatsapp_web.enabled=false, skip starting wa-web-bridge." # zh: whatsapp_web.enabled=false，跳过启动。
  exit 0
fi

if pgrep -f 'services/wa-web-bridge/index.js|start-wa-web-bridge.sh' >/dev/null 2>&1; then
  echo "wa-web-bridge is already running, skip." # zh: wa-web-bridge 已在运行，跳过。
  exit 0
fi

BRIDGE_DIR="$SCRIPT_DIR/services/wa-web-bridge"
if [[ ! -d "$BRIDGE_DIR" ]]; then
  echo "bridge dir missing: $BRIDGE_DIR"
  exit 1
fi

if [[ ! -d "$BRIDGE_DIR/node_modules" ]]; then
  echo "Installing wa-web-bridge dependencies..." # zh: 正在安装 wa-web-bridge 依赖...
  npm --prefix "$BRIDGE_DIR" install
fi

exec node "$BRIDGE_DIR/index.js"

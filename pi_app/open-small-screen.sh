#!/usr/bin/env bash
# 在小屏（480x320）上全屏打开 RustClaw 状态页（网页版）。可放桌面双击运行。
# 需先启动 clawd（8787）。用法: ./open-small-screen.sh

set -euo pipefail
# pi_app 的上级目录为 RustClaw 根
RUSTCLAW_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASE="${BASE:-http://127.0.0.1:8787}"
URL="${BASE}/small-screen.html"

run_kiosk() {
  local exe="$1"
  "$exe" --kiosk --window-size=480,320 --window-position=0,0 --app="$URL" &
  disown
}

if command -v chromium-browser &>/dev/null; then
  run_kiosk chromium-browser
elif command -v chromium &>/dev/null; then
  run_kiosk chromium
else
  xdg-open "$URL" 2>/dev/null || echo "请安装: sudo apt install chromium-browser，或浏览器打开: $URL"
  exit 0
fi

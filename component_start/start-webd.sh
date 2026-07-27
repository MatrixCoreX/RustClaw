#!/usr/bin/env bash
# zh: 启动 webd 并由它提供已构建的 UI；clawd 必须已在本机运行。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/component_start/common.sh"
component_start_init "$SCRIPT_DIR" "${1:-}" "./component_start/start-webd.sh"

if [[ ! -d "$SCRIPT_DIR/UI/dist" || ! -f "$SCRIPT_DIR/UI/dist/index.html" ]]; then
  echo "UI assets not built. Build first: cd UI && npm ci && npm run build"
  exit 1
fi
if ! pgrep -f 'target/release/clawd|target/debug/clawd|cargo run -p clawd' >/dev/null 2>&1; then
  echo "clawd is not running. Start clawd before webd."
  exit 1
fi

WEBD_BIN="$SCRIPT_DIR/target/$COMPONENT_PROFILE/webd"
if [[ ! -x "$WEBD_BIN" ]]; then
  echo "webd binary not found: $WEBD_BIN"
  exit 1
fi

export RUSTCLAW_UI_DIST="$SCRIPT_DIR/UI/dist"
echo "Using UI assets at: $RUSTCLAW_UI_DIST"
echo "Starting webd ($COMPONENT_PROFILE)..."
exec "$WEBD_BIN"

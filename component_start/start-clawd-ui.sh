#!/usr/bin/env bash
# zh: 启动 clawd 并使用已构建的 UI 资源；适合本地快速打开 Web 控制台。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/component_start/common.sh"
component_start_init "$SCRIPT_DIR" "${1:-}" "./component_start/start-clawd-ui.sh"

if [[ ! -d "$SCRIPT_DIR/UI/dist" || ! -f "$SCRIPT_DIR/UI/dist/index.html" ]]; then
  echo "UI assets not built. Build first: cd UI && npm ci && npm run build"
  exit 1
fi
export RUSTCLAW_UI_DIST="$SCRIPT_DIR/UI/dist"
echo "Using UI assets at: $RUSTCLAW_UI_DIST"
echo "Starting clawd ($COMPONENT_PROFILE)..."
exec "$SCRIPT_DIR/component_start/start-clawd.sh" "$COMPONENT_PROFILE"

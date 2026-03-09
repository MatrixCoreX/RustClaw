#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Enable colored log tags on interactive terminals unless overridden.
if [[ -t 1 && -z "${RUSTCLAW_LOG_COLOR:-}" ]]; then
  export RUSTCLAW_LOG_COLOR=1
fi

PROFILE="${1:-${RUSTCLAW_START_PROFILE:-release}}"
case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-clawd-ui.sh [release|debug]"
    exit 1
    ;;
esac

if ! command -v npm >/dev/null 2>&1; then
  echo "npm is required. Please install Node.js/npm first."
  exit 1
fi

if [[ ! -d "$SCRIPT_DIR/UI/dist" || ! -f "$SCRIPT_DIR/UI/dist/index.html" ]]; then
  echo "UI assets not built. Build first: cd UI && npm install && npm run build"
  exit 1
fi
export RUSTCLAW_UI_DIST="$SCRIPT_DIR/UI/dist"
echo "Using UI assets at: $RUSTCLAW_UI_DIST"
echo "Starting clawd ($PROFILE)..."
exec "$SCRIPT_DIR/start-clawd.sh" "$PROFILE"

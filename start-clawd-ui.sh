#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PROFILE="${1:-${RUSTCLAW_START_PROFILE:-debug}}"
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

echo "Building UI assets..."
if [[ ! -d "$SCRIPT_DIR/UI/node_modules" ]]; then
  (cd "$SCRIPT_DIR/UI" && npm install)
fi
(cd "$SCRIPT_DIR/UI" && npm run build)

export RUSTCLAW_UI_DIST="$SCRIPT_DIR/UI/dist"
echo "Using UI assets at: $RUSTCLAW_UI_DIST"
echo "Starting clawd ($PROFILE)..."
exec "$SCRIPT_DIR/start-clawd.sh" "$PROFILE"

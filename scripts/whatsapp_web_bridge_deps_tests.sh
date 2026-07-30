#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

mkdir -p "$FIXTURE_ROOT/scripts" "$FIXTURE_ROOT/services/wa-web-bridge/node_modules"
cp "$ROOT_DIR/scripts/whatsapp_web_bridge_deps.sh" "$FIXTURE_ROOT/scripts/"
cp "$ROOT_DIR/services/wa-web-bridge/package.json" "$FIXTURE_ROOT/services/wa-web-bridge/"
cp "$ROOT_DIR/services/wa-web-bridge/package-lock.json" "$FIXTURE_ROOT/services/wa-web-bridge/"

if APP_WORKSPACE_ROOT="$FIXTURE_ROOT" bash "$FIXTURE_ROOT/scripts/whatsapp_web_bridge_deps.sh" --check >/dev/null 2>&1; then
  echo "Dependency check accepted an unstamped node_modules directory." >&2
  exit 1
fi

if APP_WORKSPACE_ROOT="$FIXTURE_ROOT" bash "$FIXTURE_ROOT/scripts/whatsapp_web_bridge_deps.sh" invalid >/dev/null 2>&1; then
  echo "Dependency check accepted an invalid action." >&2
  exit 1
fi

echo "WHATSAPP_WEB_BRIDGE_DEPS_TESTS ok"

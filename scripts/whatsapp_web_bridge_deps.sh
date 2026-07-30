#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="${APP_WORKSPACE_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
BRIDGE_DIR="$ROOT_DIR/services/wa-web-bridge"
MODE="${1:---check}"
STAMP_FILE="$BRIDGE_DIR/node_modules/.runtime-deps.sha256"

case "$MODE" in
  --check|--ensure) ;;
  *)
    echo "Usage: $0 [--check|--ensure]" >&2
    exit 2
    ;;
esac

[[ -f "$BRIDGE_DIR/package.json" ]] || {
  echo "WhatsApp Web bridge package manifest is missing: $BRIDGE_DIR/package.json" >&2
  exit 1
}
[[ -f "$BRIDGE_DIR/package-lock.json" ]] || {
  echo "WhatsApp Web bridge lockfile is missing: $BRIDGE_DIR/package-lock.json" >&2
  exit 1
}
command -v node >/dev/null 2>&1 || {
  echo "WhatsApp Web requires Node.js 18 or newer." >&2
  exit 1
}
command -v npm >/dev/null 2>&1 || {
  echo "WhatsApp Web requires npm." >&2
  exit 1
}

NODE_MAJOR="$(node -p 'Number(process.versions.node.split(".")[0])' 2>/dev/null || true)"
case "$NODE_MAJOR" in
  ''|*[!0-9]*)
    echo "Unable to determine the installed Node.js version." >&2
    exit 1
    ;;
esac
if (( NODE_MAJOR < 18 )); then
  echo "WhatsApp Web requires Node.js 18 or newer; found $(node --version)." >&2
  exit 1
fi

EXPECTED_DIGEST="$(python3 - "$BRIDGE_DIR/package.json" "$BRIDGE_DIR/package-lock.json" <<'PY'
import hashlib
from pathlib import Path
import sys

digest = hashlib.sha256()
for raw in sys.argv[1:]:
    path = Path(raw)
    digest.update(path.name.encode("utf-8"))
    digest.update(b"\0")
    digest.update(path.read_bytes())
    digest.update(b"\0")
print(digest.hexdigest())
PY
)"

deps_ready() {
  [[ -d "$BRIDGE_DIR/node_modules" ]] || return 1
  [[ -f "$STAMP_FILE" ]] || return 1
  [[ "$(tr -d '[:space:]' < "$STAMP_FILE")" == "$EXPECTED_DIGEST" ]] || return 1
  npm --prefix "$BRIDGE_DIR" ls --omit=dev --depth=0 >/dev/null 2>&1
}

if deps_ready; then
  echo "WhatsApp Web bridge dependencies are ready (Node $(node --version))."
  exit 0
fi

if [[ "$MODE" == "--check" ]]; then
  echo "WhatsApp Web bridge dependencies are missing or stale; run: bash scripts/whatsapp_web_bridge_deps.sh --ensure" >&2
  exit 1
fi

echo "Installing locked WhatsApp Web bridge dependencies..."
npm --prefix "$BRIDGE_DIR" ci --omit=dev --no-audit --no-fund
printf '%s\n' "$EXPECTED_DIGEST" > "$STAMP_FILE.tmp"
mv "$STAMP_FILE.tmp" "$STAMP_FILE"
deps_ready || {
  echo "WhatsApp Web bridge dependency verification failed after npm ci." >&2
  exit 1
}
echo "WhatsApp Web bridge dependencies installed and verified."

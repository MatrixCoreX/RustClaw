#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

if [[ -t 1 && -z "${RUSTCLAW_LOG_COLOR:-}" ]]; then
  export RUSTCLAW_LOG_COLOR=1
fi

PROFILE="${1:-${RUSTCLAW_START_PROFILE:-release}}"
case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-wechatd.sh [release|debug]"
    exit 1
    ;;
esac

BIN_NAME="wechatd"
BIN_PATH="$SCRIPT_DIR/target/$PROFILE/$BIN_NAME"
if [[ ! -x "$BIN_PATH" ]]; then
  echo "Binary missing: $BIN_PATH"
  echo "Copy built binary to target/$PROFILE/ or run: cargo build -p $BIN_NAME ${PROFILE:+--release}"
  exit 1
fi

export WECHAT_CONFIG_PATH="${WECHAT_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/wechat.toml}"

if python3 - <<'PY'
import tomllib
from pathlib import Path

path = Path("configs/channels/wechat.toml")
if not path.exists():
    print("configs/channels/wechat.toml not found, skip starting wechatd.")
    raise SystemExit(2)
cfg = tomllib.loads(path.read_text(encoding="utf-8"))
wechat = cfg.get("wechat", {}) or {}
if not bool(wechat.get("enabled", False)):
    print("wechat.enabled=false, skip starting wechatd.")
    raise SystemExit(2)
print("WeChat preflight passed.")
PY
then
  :
else
  code=$?
  if [[ "$code" == "2" ]]; then
    exit 0
  fi
  exit "$code"
fi

if pgrep -f 'target/(debug|release)/wechatd|cargo run -p wechatd' >/dev/null 2>&1; then
  echo "Detected wechatd already running on this host. Stop old instance first."
  exit 1
fi

exec "$BIN_PATH"

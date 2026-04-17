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
  release)
    ;;
  *)
    echo "Usage: ./start-wechatd.sh [release]"
    exit 1
    ;;
esac

BIN_NAME="wechatd"
BIN_PATH="$SCRIPT_DIR/target/$PROFILE/$BIN_NAME"
if [[ ! -x "$BIN_PATH" ]]; then
  echo "Binary missing: $BIN_PATH"
  echo "Copy built binary to target/$PROFILE/ or run: cargo build -p $BIN_NAME --release"
  exit 1
fi

export WECHAT_CONFIG_PATH="${WECHAT_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/wechat.toml}"

if pgrep -f 'target/release/wechatd|cargo run -p wechatd' >/dev/null 2>&1; then
  echo "Detected wechatd already running on this host. Stop old instance first."
  exit 1
fi

exec "$BIN_PATH"

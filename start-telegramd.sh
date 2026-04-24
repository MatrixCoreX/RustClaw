#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/scripts/version_info.sh"
print_rustclaw_version "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

# Enable colored log tags on interactive terminals unless overridden.
if [[ -t 1 && -z "${RUSTCLAW_LOG_COLOR:-}" ]]; then
  export RUSTCLAW_LOG_COLOR=1
fi

PROFILE="${1:-${RUSTCLAW_START_PROFILE:-release}}"
case "$PROFILE" in
  release)
    ;;
  *)
    echo "Usage: ./start-telegramd.sh [release]" # zh: 用法：./start-telegramd.sh [release]
    exit 1
    ;;
esac

BIN_NAME="telegramd"
BIN_PATH="$SCRIPT_DIR/target/$PROFILE/$BIN_NAME"
if [[ ! -x "$BIN_PATH" ]]; then
  echo "Binary missing: $BIN_PATH" # zh: 未找到二进制：$BIN_PATH
  echo "Copy built binary to target/$PROFILE/ or run: cargo build -p $BIN_NAME --release"
  exit 1
fi

# Preflight 1: avoid local duplicate polling workers.
if pgrep -f 'target/release/telegramd|cargo run -p telegramd' >/dev/null 2>&1; then
  echo "Detected telegramd already running on this host. Stop old instance first to avoid polling conflicts." # zh: 检测到本机已有 telegramd 在运行。请先停止旧实例，避免轮询冲突。
  echo "You can run: pkill -f 'target/release/telegramd|cargo run -p telegramd'" # zh: 可执行: pkill -f 'target/release/telegramd|cargo run -p telegramd'
  exit 1
fi

exec "$BIN_PATH"

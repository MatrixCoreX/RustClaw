#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$SCRIPT_DIR"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/scripts/version_info.sh"
print_rustclaw_version "$SCRIPT_DIR"

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
    echo "Usage: ./component_start/start-feishud.sh [release]" # zh: 用法：./component_start/start-feishud.sh [release]
    exit 1
    ;;
esac

BIN_NAME="feishud"
BIN_PATH="$SCRIPT_DIR/target/$PROFILE/$BIN_NAME"
if [[ ! -x "$BIN_PATH" ]]; then
  echo "Binary missing: $BIN_PATH"
  echo "Copy built binary to target/$PROFILE/ or run: cargo build -p $BIN_NAME --release"
  exit 1
fi

# Config path: same as feishud default, explicit for scripts
export FEISHU_CONFIG_PATH="${FEISHU_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/feishu.toml}"

if pgrep -f 'target/release/feishud|cargo run -p feishud' >/dev/null 2>&1; then
  echo "Detected feishud already running on this host. Stop old instance first." # zh: 检测到本机已有 feishud 在运行，请先停止旧实例。
  exit 1
fi

exec "$BIN_PATH"

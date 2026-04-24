#!/usr/bin/env bash
# zh: 单独启动 Lark 渠道服务；通常由 start-all.sh 统一调度。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
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
    # zh: 目前只支持 release 模式启动已编译二进制。
    echo "Usage: ./start-larkd.sh [release]"
    exit 1
    ;;
esac

BIN_NAME="larkd"
BIN_PATH="$SCRIPT_DIR/target/$PROFILE/$BIN_NAME"
if [[ ! -x "$BIN_PATH" ]]; then
  echo "Binary missing: $BIN_PATH"
  echo "Copy built binary to target/$PROFILE/ or run: cargo build -p $BIN_NAME --release"
  exit 1
fi

# Config path: Lark international, separate from feishu.toml
export LARK_CONFIG_PATH="${LARK_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/lark.toml}"

if pgrep -f 'target/release/larkd|cargo run -p larkd' >/dev/null 2>&1; then
  echo "Detected larkd already running on this host. Stop old instance first."
  exit 1
fi

exec "$BIN_PATH"

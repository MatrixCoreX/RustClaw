#!/usr/bin/env bash
# zh: 单独启动 WeChat 渠道服务；通常由 start-all.sh 统一调度。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/component_start/common.sh"
component_start_init "$SCRIPT_DIR" "${1:-}" "./component_start/start-wechatd.sh"

BIN_NAME="wechatd"
BIN_PATH="$(component_require_binary "$BIN_NAME")"

export WECHAT_CONFIG_PATH="${WECHAT_CONFIG_PATH:-$COMPONENT_CHANNEL_CONFIG_DIR/wechat.toml}"

component_exec_binary "$BIN_NAME" "$BIN_PATH"

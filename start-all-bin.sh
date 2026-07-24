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

RUNTIME_ENV_SCRIPT="${RUSTCLAW_RUNTIME_ENV_SCRIPT:-$HOME/runtime_env_filled.sh}"
if [[ -f "$RUNTIME_ENV_SCRIPT" ]]; then
  # Source runtime secrets/env before starting any daemon so child processes inherit them.
  # shellcheck source=/dev/null
  . "$RUNTIME_ENV_SCRIPT"
fi

# Enable colored log tags on interactive terminals unless overridden.
if [[ -t 1 && -z "${RUSTCLAW_LOG_COLOR:-}" ]]; then
  export RUSTCLAW_LOG_COLOR=1
fi

LOG_DIR="$SCRIPT_DIR/logs"
PID_DIR="$SCRIPT_DIR/.pids"
mkdir -p "$LOG_DIR" "$PID_DIR"

# Usage:
#   ./start-all-bin.sh [release]
PROFILE="${1:-release}"
case "$PROFILE" in
  release)
    ;;
  *)
    echo "Usage: ./start-all-bin.sh [release]  # default: release" # zh: 用法：./start-all-bin.sh [release]，默认 release
    exit 1
    ;;
esac

CLAWD_BIN="$SCRIPT_DIR/target/$PROFILE/clawd"
TELEGRAMD_BIN="$SCRIPT_DIR/target/$PROFILE/telegramd"
WHATSAPPD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsappd"
WHATSAPP_WEBD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsapp_webd"
WECHATD_BIN="$SCRIPT_DIR/target/$PROFILE/wechatd"
FEISHUD_BIN="$SCRIPT_DIR/target/$PROFILE/feishud"
WEBD_BIN="$SCRIPT_DIR/target/$PROFILE/webd"
SKILL_RUNNER_ABS="$SCRIPT_DIR/target/$PROFILE/skill-runner"

read_enabled() {
  local config_path="$1"
  local section="$2"
  local default_value="$3"
  python3 - "$config_path" "$section" "$default_value" <<'PY'
import sys
import tomllib
from pathlib import Path

path = Path(sys.argv[1])
section = sys.argv[2]
default = sys.argv[3] == "1"
if not path.exists():
    print("1" if default else "0")
    raise SystemExit(0)
cfg = tomllib.loads(path.read_text(encoding="utf-8"))
value = cfg.get(section, {})
enabled = value.get("enabled", default) if isinstance(value, dict) else default
print("1" if bool(enabled) else "0")
PY
}

require_binary() {
  local binary_path="$1"
  local component="$2"
  if [[ -x "$binary_path" ]]; then
    return 0
  fi
  echo "Required binary is missing or not executable: ${binary_path} (${component})" >&2
  return 1
}

WEBD_ENABLED="$(read_enabled "$SCRIPT_DIR/configs/channels/webd.toml" "webd" "0")"
TELEGRAM_ENABLED="$(read_enabled "$SCRIPT_DIR/configs/channels/telegram.toml" "telegram_bot" "1")"
WHATSAPP_ENABLED="$(read_enabled "$SCRIPT_DIR/configs/channels/whatsapp.toml" "whatsapp" "0")"
WHATSAPP_WEB_ENABLED="$(read_enabled "$SCRIPT_DIR/configs/channels/whatsapp.toml" "whatsapp_web" "0")"
WECHAT_ENABLED="$(read_enabled "$SCRIPT_DIR/configs/channels/wechat.toml" "wechat" "0")"
FEISHU_ENABLED="$(read_enabled "$SCRIPT_DIR/configs/channels/feishu.toml" "feishu" "0")"

preflight_failed=0
require_binary "$CLAWD_BIN" "clawd" || preflight_failed=1
require_binary "$SKILL_RUNNER_ABS" "skill-runner" || preflight_failed=1
if [[ "$WEBD_ENABLED" == "1" ]]; then
  require_binary "$WEBD_BIN" "webd" || preflight_failed=1
fi
if [[ "${RUSTCLAW_SKIP_TELEGRAMD:-0}" != "1" && "$TELEGRAM_ENABLED" == "1" ]]; then
  require_binary "$TELEGRAMD_BIN" "telegramd" || preflight_failed=1
fi
if [[ "$WHATSAPP_ENABLED" == "1" ]]; then
  require_binary "$WHATSAPPD_BIN" "whatsappd" || preflight_failed=1
fi
if [[ "$WHATSAPP_WEB_ENABLED" == "1" ]]; then
  require_binary "$WHATSAPP_WEBD_BIN" "whatsapp_webd" || preflight_failed=1
fi
if [[ "$WECHAT_ENABLED" == "1" ]]; then
  require_binary "$WECHATD_BIN" "wechatd" || preflight_failed=1
fi
if [[ "$FEISHU_ENABLED" == "1" ]]; then
  require_binary "$FEISHUD_BIN" "feishud" || preflight_failed=1
fi
if [[ "$preflight_failed" -ne 0 ]]; then
  echo "Startup preflight failed; existing RustClaw processes were left unchanged." >&2
  echo "Run: ./build-all.sh $PROFILE" >&2
  exit 1
fi

# Stop managed RustClaw processes only after every enabled component passes
# preflight. A partial build must never take a healthy deployment offline.
if [[ -f "$SCRIPT_DIR/stop-rustclaw.sh" ]]; then
  "$SCRIPT_DIR/stop-rustclaw.sh" || true
fi

start_clawd() {
  if pgrep -f 'target/release/clawd|cargo run -p clawd' >/dev/null 2>&1; then
    echo "clawd is already running, skipping startup." # zh: clawd 已在运行，跳过启动。
    return 0
  fi
  nohup "$CLAWD_BIN" >"$LOG_DIR/clawd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/clawd.pid"
  echo "Starting clawd binary, PID=$pid, log: $LOG_DIR/clawd.log" # zh: clawd 二进制启动中，PID=$pid, 日志: $LOG_DIR/clawd.log
  sleep 2
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "Failed to start clawd binary. Check log: $LOG_DIR/clawd.log" # zh: clawd 二进制启动失败，请检查日志: $LOG_DIR/clawd.log
    return 1
  fi
}

start_webd() {
  if [[ "$WEBD_ENABLED" != "1" ]]; then
    echo "webd.enabled=false, skipping webd startup." # zh: webd.enabled=false，跳过 webd 启动。
    return 0
  fi
  if [[ ! -x "$WEBD_BIN" ]]; then
    echo "Binary not found or not executable: $WEBD_BIN" # zh: 二进制不存在或不可执行：$WEBD_BIN
    echo "Please run: ./build-all.sh $PROFILE" # zh: 请先执行：./build-all.sh $PROFILE
    return 1
  fi
  if pgrep -f 'target/release/webd|cargo run -p webd' >/dev/null 2>&1; then
    echo "webd is already running, skipping startup." # zh: webd 已在运行，跳过启动。
    return 0
  fi
  nohup "$WEBD_BIN" >"$LOG_DIR/webd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/webd.pid"
  echo "Starting webd, PID=$pid, log: $LOG_DIR/webd.log" # zh: webd 启动中，PID=$pid, 日志: $LOG_DIR/webd.log
  sleep 2
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "Failed to start webd. Check log: $LOG_DIR/webd.log" # zh: webd 启动失败，请检查日志: $LOG_DIR/webd.log
    return 1
  fi
}

start_telegramd() {
  if [[ "$TELEGRAM_ENABLED" != "1" ]]; then
    echo "telegram_bot.enabled=false, skipping telegramd startup." # zh: telegram_bot.enabled=false，跳过 telegramd 启动。
    return 0
  fi
  if pgrep -f 'target/release/telegramd|cargo run -p telegramd' >/dev/null 2>&1; then
    echo "telegramd is already running, skipping startup." # zh: telegramd 已在运行，跳过启动。
    return 0
  fi
  nohup "$TELEGRAMD_BIN" >"$LOG_DIR/telegramd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/telegramd.pid"
  echo "Starting telegramd binary, PID=$pid, log: $LOG_DIR/telegramd.log" # zh: telegramd 二进制启动中，PID=$pid, 日志: $LOG_DIR/telegramd.log
  sleep 2
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "Failed to start telegramd binary. Check log: $LOG_DIR/telegramd.log" # zh: telegramd 二进制启动失败，请检查日志: $LOG_DIR/telegramd.log
    return 1
  fi
}

start_whatsapp_webd() {
  if [[ "$WHATSAPP_WEB_ENABLED" != "1" ]]; then
    echo "whatsapp_web.enabled=false, skipping whatsapp_webd startup." # zh: whatsapp_web.enabled=false，跳过 whatsapp_webd 启动。
    return 0
  fi
  if [[ ! -x "$WHATSAPP_WEBD_BIN" ]]; then
    echo "Binary not found or not executable: $WHATSAPP_WEBD_BIN" # zh: 二进制不存在或不可执行：$WHATSAPP_WEBD_BIN
    echo "Please run: ./build-all.sh $PROFILE" # zh: 请先执行：./build-all.sh $PROFILE
    return 1
  fi
  if pgrep -f 'target/release/whatsapp_webd|cargo run -p whatsapp_webd' >/dev/null 2>&1; then
    echo "whatsapp_webd is already running, skipping startup." # zh: whatsapp_webd 已在运行，跳过启动。
    return 0
  fi
  nohup "$WHATSAPP_WEBD_BIN" >"$LOG_DIR/whatsapp_webd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/whatsapp_webd.pid"
  echo "Starting whatsapp_webd, PID=$pid, log: $LOG_DIR/whatsapp_webd.log" # zh: whatsapp_webd 启动中，PID=$pid, 日志: $LOG_DIR/whatsapp_webd.log
  sleep 2
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "Failed to start whatsapp_webd. Check log: $LOG_DIR/whatsapp_webd.log" # zh: whatsapp_webd 启动失败，请检查日志: $LOG_DIR/whatsapp_webd.log
    return 1
  fi
}

start_whatsappd() {
  if [[ "$WHATSAPP_ENABLED" != "1" ]]; then
    echo "whatsapp.enabled=false, skipping whatsappd startup." # zh: whatsapp.enabled=false，跳过 whatsappd 启动。
    return 0
  fi
  if [[ ! -x "$WHATSAPPD_BIN" ]]; then
    echo "Binary not found or not executable: $WHATSAPPD_BIN" # zh: 二进制不存在或不可执行：$WHATSAPPD_BIN
    echo "Please run: ./build-all.sh $PROFILE" # zh: 请先执行：./build-all.sh $PROFILE
    return 1
  fi
  if pgrep -f 'target/release/whatsappd|cargo run -p whatsappd' >/dev/null 2>&1; then
    echo "whatsappd is already running, skipping startup." # zh: whatsappd 已在运行，跳过启动。
    return 0
  fi
  nohup "$WHATSAPPD_BIN" >"$LOG_DIR/whatsappd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/whatsappd.pid"
  echo "Starting whatsappd binary, PID=$pid, log: $LOG_DIR/whatsappd.log" # zh: whatsappd 二进制启动中，PID=$pid, 日志: $LOG_DIR/whatsappd.log
  sleep 2
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "Failed to start whatsappd binary. Check log: $LOG_DIR/whatsappd.log" # zh: whatsappd 二进制启动失败，请检查日志: $LOG_DIR/whatsappd.log
    return 1
  fi
}

start_feishud() {
  if [[ "$FEISHU_ENABLED" != "1" ]]; then
    echo "feishu.enabled=false, skipping feishud startup." # zh: feishu.enabled=false，跳过 feishud 启动。
    return 0
  fi
  if [[ ! -x "$FEISHUD_BIN" ]]; then
    echo "Binary not found or not executable: $FEISHUD_BIN" # zh: 二进制不存在或不可执行：$FEISHUD_BIN
    return 0
  fi
  if pgrep -f 'target/release/feishud|cargo run -p feishud' >/dev/null 2>&1; then
    echo "feishud is already running, skipping startup." # zh: feishud 已在运行，跳过启动。
    return 0
  fi
  export FEISHU_CONFIG_PATH="${FEISHU_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/feishu.toml}"
  nohup "$FEISHUD_BIN" >"$LOG_DIR/feishud.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/feishud.pid"
  echo "Starting feishud binary, PID=$pid, log: $LOG_DIR/feishud.log" # zh: feishud 二进制启动中，PID=$pid, 日志: $LOG_DIR/feishud.log
  sleep 2
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "Failed to start feishud binary. Check log: $LOG_DIR/feishud.log" # zh: feishud 二进制启动失败，请检查日志: $LOG_DIR/feishud.log
    return 1
  fi
}

start_wechatd() {
  if [[ "$WECHAT_ENABLED" != "1" ]]; then
    echo "wechat.enabled=false, skipping wechatd startup."
    return 0
  fi
  if [[ ! -x "$WECHATD_BIN" ]]; then
    echo "Binary not found or not executable: $WECHATD_BIN"
    return 0
  fi
  if pgrep -f 'target/release/wechatd|cargo run -p wechatd' >/dev/null 2>&1; then
    echo "wechatd is already running, skipping startup."
    return 0
  fi
  export WECHAT_CONFIG_PATH="${WECHAT_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/wechat.toml}"
  nohup "$WECHATD_BIN" >"$LOG_DIR/wechatd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/wechatd.pid"
  echo "Starting wechatd binary, PID=$pid, log: $LOG_DIR/wechatd.log"
  sleep 2
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "Failed to start wechatd binary. Check log: $LOG_DIR/wechatd.log"
    return 1
  fi
}

start_future_adapters_placeholder() {
  "$SCRIPT_DIR/component_start/start-future-adapters.sh" || true
}

start_clawd
start_webd
if [[ "${RUSTCLAW_SKIP_TELEGRAMD:-0}" == "1" ]]; then
  echo "RUSTCLAW_SKIP_TELEGRAMD=1, skipping telegramd startup."
else
  start_telegramd
fi
start_future_adapters_placeholder
start_whatsapp_webd
start_whatsappd
start_wechatd
start_feishud

echo "One-click binary startup command executed (profile: $PROFILE)." # zh: 一键启动已编译二进制命令已执行（profile: $PROFILE）。

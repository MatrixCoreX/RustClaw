#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

LOG_DIR="$SCRIPT_DIR/logs"
PID_DIR="$SCRIPT_DIR/.pids"
mkdir -p "$LOG_DIR" "$PID_DIR"

# Usage:
#   ./start-all-bin.sh [release|debug]
PROFILE="${1:-debug}"
case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-all-bin.sh [release|debug]" # zh: 用法：./start-all-bin.sh [release|debug]
    exit 1
    ;;
esac

CLAWD_BIN="$SCRIPT_DIR/target/$PROFILE/clawd"
TELEGRAMD_BIN="$SCRIPT_DIR/target/$PROFILE/telegramd"
WHATSAPPD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsappd"
WHATSAPP_WEBD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsapp_webd"

if [[ ! -x "$CLAWD_BIN" ]]; then
  echo "Binary not found or not executable: $CLAWD_BIN" # zh: 二进制不存在或不可执行：$CLAWD_BIN
  echo "Please run: ./build-all.sh $PROFILE" # zh: 请先执行：./build-all.sh $PROFILE
  exit 1
fi

if [[ ! -x "$TELEGRAMD_BIN" ]]; then
  echo "Binary not found or not executable: $TELEGRAMD_BIN" # zh: 二进制不存在或不可执行：$TELEGRAMD_BIN
  echo "Please run: ./build-all.sh $PROFILE" # zh: 请先执行：./build-all.sh $PROFILE
  exit 1
fi

# Ensure skill-runner binary exists for run_skill tasks.
SKILL_RUNNER_ABS="$SCRIPT_DIR/target/$PROFILE/skill-runner"
if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  ALT_PROFILE="debug"
  if [[ "$PROFILE" == "debug" ]]; then
    ALT_PROFILE="release"
  fi
  ALT_RUNNER="$SCRIPT_DIR/target/$ALT_PROFILE/skill-runner"
  if [[ -x "$ALT_RUNNER" ]]; then
    echo "skill-runner missing in $PROFILE, fallback: $ALT_RUNNER" # zh: 当前 profile 未找到 skill-runner，回退到另一 profile。
    SKILL_RUNNER_ABS="$ALT_RUNNER"
  fi
fi

if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  echo "skill-runner missing, trying to build: $SKILL_RUNNER_ABS" # zh: 未找到 skill-runner，尝试自动编译。
  if [[ "$PROFILE" == "release" ]]; then
    cargo build -p skill-runner --release
  else
    cargo build -p skill-runner
  fi
fi

if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  echo "skill-runner still missing after build: $SKILL_RUNNER_ABS" # zh: 自动编译后仍未找到 skill-runner。
  echo "Please run: ./build-all.sh $PROFILE" # zh: 请执行：./build-all.sh $PROFILE
  exit 1
fi

start_clawd() {
  if pgrep -f 'target/(debug|release)/clawd|cargo run -p clawd' >/dev/null 2>&1; then
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

start_telegramd() {
  local tg_enabled
  tg_enabled="$(
python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
tg_cfg = Path("configs/channels/telegram.toml")
if tg_cfg.exists():
    cfg.update(tomllib.loads(tg_cfg.read_text(encoding="utf-8")))
print("1" if bool(cfg.get("telegram_bot", {}).get("enabled", True)) else "0")
PY
  )"
  if [[ "$tg_enabled" != "1" ]]; then
    echo "telegram_bot.enabled=false, skipping telegramd startup." # zh: telegram_bot.enabled=false，跳过 telegramd 启动。
    return 0
  fi
  if pgrep -f 'target/(debug|release)/telegramd|cargo run -p telegramd' >/dev/null 2>&1; then
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
  local wa_web_enabled
  wa_web_enabled="$(
python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
wa_cfg = Path("configs/channels/whatsapp.toml")
if wa_cfg.exists():
    cfg.update(tomllib.loads(wa_cfg.read_text(encoding="utf-8")))
print("1" if bool(cfg.get("whatsapp_web", {}).get("enabled", False)) else "0")
PY
  )"
  if [[ "$wa_web_enabled" != "1" ]]; then
    echo "whatsapp_web.enabled=false, skipping whatsapp_webd startup." # zh: whatsapp_web.enabled=false，跳过 whatsapp_webd 启动。
    return 0
  fi
  if [[ ! -x "$WHATSAPP_WEBD_BIN" ]]; then
    echo "Binary not found or not executable: $WHATSAPP_WEBD_BIN" # zh: 二进制不存在或不可执行：$WHATSAPP_WEBD_BIN
    echo "Please run: ./build-all.sh $PROFILE" # zh: 请先执行：./build-all.sh $PROFILE
    return 1
  fi
  if pgrep -f 'target/(debug|release)/whatsapp_webd|cargo run -p whatsapp_webd' >/dev/null 2>&1; then
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
  local wa_enabled
  wa_enabled="$(
python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
wa_cfg = Path("configs/channels/whatsapp.toml")
if wa_cfg.exists():
    cfg.update(tomllib.loads(wa_cfg.read_text(encoding="utf-8")))
print("1" if bool(cfg.get("whatsapp", {}).get("enabled", False)) else "0")
PY
  )"
  if [[ "$wa_enabled" != "1" ]]; then
    echo "whatsapp.enabled=false, skipping whatsappd startup." # zh: whatsapp.enabled=false，跳过 whatsappd 启动。
    return 0
  fi
  if [[ ! -x "$WHATSAPPD_BIN" ]]; then
    echo "Binary not found or not executable: $WHATSAPPD_BIN" # zh: 二进制不存在或不可执行：$WHATSAPPD_BIN
    echo "Please run: ./build-all.sh $PROFILE" # zh: 请先执行：./build-all.sh $PROFILE
    return 1
  fi
  if pgrep -f 'target/(debug|release)/whatsappd|cargo run -p whatsappd' >/dev/null 2>&1; then
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

start_future_adapters_placeholder() {
  "$SCRIPT_DIR/start-future-adapters.sh" || true
}

start_clawd
start_telegramd
start_future_adapters_placeholder
start_whatsapp_webd
start_whatsappd

echo "One-click binary startup command executed (profile: $PROFILE)." # zh: 一键启动已编译二进制命令已执行（profile: $PROFILE）。

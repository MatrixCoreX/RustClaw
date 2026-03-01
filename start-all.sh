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

# Optional args:
#   ./start-all.sh <vendor(openai|google|anthropic|grok)> [model_override] [release|debug]
PROVIDER_OVERRIDE="${1:-${RUSTCLAW_PROVIDER_OVERRIDE:-}}"
MODEL_OVERRIDE="${2:-${RUSTCLAW_MODEL_OVERRIDE:-}}"
PROFILE="${3:-${RUSTCLAW_START_PROFILE:-release}}"

case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-all.sh <vendor> [model_override] [release|debug]" # zh: 用法：./start-all.sh <vendor> [model_override] [release|debug]
    exit 1
    ;;
esac
export RUSTCLAW_START_PROFILE="$PROFILE"

if [[ -n "$PROVIDER_OVERRIDE" ]]; then
  export RUSTCLAW_PROVIDER_OVERRIDE="$PROVIDER_OVERRIDE"
  echo "Using preset provider: $RUSTCLAW_PROVIDER_OVERRIDE" # zh: 使用预设 provider: $RUSTCLAW_PROVIDER_OVERRIDE
fi
if [[ -n "$MODEL_OVERRIDE" ]]; then
  export RUSTCLAW_MODEL_OVERRIDE="$MODEL_OVERRIDE"
  echo "Using model override: $RUSTCLAW_MODEL_OVERRIDE" # zh: 使用模型覆盖: $RUSTCLAW_MODEL_OVERRIDE
fi

# Batch start should be non-interactive.
export RUSTCLAW_MODEL_SELECT=0

# Prefer prebuilt binaries to avoid duplicate compile checks in cargo run.
CLAWD_BIN="$SCRIPT_DIR/target/$PROFILE/clawd"
TELEGRAMD_BIN="$SCRIPT_DIR/target/$PROFILE/telegramd"
if [[ -x "$CLAWD_BIN" && -x "$TELEGRAMD_BIN" ]]; then
  echo "Detected prebuilt binaries under target/$PROFILE; using binary startup path." # zh: 检测到 target/$PROFILE 下已有二进制，改用二进制启动路径。
  exec "$SCRIPT_DIR/start-all-bin.sh" "$PROFILE"
fi
echo "Prebuilt binaries not found for profile=$PROFILE; falling back to cargo-run startup path." # zh: 未找到对应 profile 的二进制，回退到 cargo run 启动路径。

start_clawd() {
  if pgrep -f 'target/(debug|release)/clawd|cargo run -p clawd' >/dev/null 2>&1; then
    echo "clawd is already running, skipping startup." # zh: clawd 已在运行，跳过启动。
    return 0
  fi
  nohup "$SCRIPT_DIR/start-clawd.sh" "$PROFILE" >"$LOG_DIR/clawd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/clawd.pid"
  echo "Starting clawd, PID=$pid, log: $LOG_DIR/clawd.log" # zh: clawd 启动中，PID=$pid, 日志: $LOG_DIR/clawd.log
  sleep 2
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "Failed to start clawd. Check log: $LOG_DIR/clawd.log" # zh: clawd 启动失败，请检查日志: $LOG_DIR/clawd.log
    return 1
  fi
}

start_telegramd() {
  if pgrep -f 'target/(debug|release)/telegramd|cargo run -p telegramd' >/dev/null 2>&1; then
    echo "telegramd is already running, skipping startup." # zh: telegramd 已在运行，跳过启动。
    return 0
  fi
  nohup "$SCRIPT_DIR/start-telegramd.sh" "$PROFILE" >"$LOG_DIR/telegramd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/telegramd.pid"
  echo "Starting telegramd, PID=$pid, log: $LOG_DIR/telegramd.log" # zh: telegramd 启动中，PID=$pid, 日志: $LOG_DIR/telegramd.log
  sleep 2
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "Failed to start telegramd. Check log: $LOG_DIR/telegramd.log" # zh: telegramd 启动失败，请检查日志: $LOG_DIR/telegramd.log
    return 1
  fi
}

start_clawd
start_telegramd

echo "One-click startup command executed." # zh: 一键启动命令已执行。

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
#   ./start-all.sh <vendor(openai|google|anthropic|grok)> [model_override]
PROVIDER_OVERRIDE="${1:-${RUSTCLAW_PROVIDER_OVERRIDE:-}}"
MODEL_OVERRIDE="${2:-${RUSTCLAW_MODEL_OVERRIDE:-}}"
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

start_clawd() {
  if pgrep -f 'target/(debug|release)/clawd|cargo run -p clawd' >/dev/null 2>&1; then
    echo "clawd is already running, skipping startup." # zh: clawd 已在运行，跳过启动。
    return 0
  fi
  nohup "$SCRIPT_DIR/start-clawd.sh" >"$LOG_DIR/clawd.log" 2>&1 &
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
  nohup "$SCRIPT_DIR/start-telegramd.sh" >"$LOG_DIR/telegramd.log" 2>&1 &
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

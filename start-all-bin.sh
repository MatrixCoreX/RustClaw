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
PROFILE="${1:-release}"
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
SKILL_RUNNER_PATH="$(
python3 - <<'PY'
import tomllib
from pathlib import Path

cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
skills = cfg.get("skills", {})
print(str(skills.get("skill_runner_path", "target/debug/skill-runner") or "target/debug/skill-runner"))
PY
)"

if [[ "$SKILL_RUNNER_PATH" = /* ]]; then
  SKILL_RUNNER_ABS="$SKILL_RUNNER_PATH"
else
  SKILL_RUNNER_ABS="$SCRIPT_DIR/$SKILL_RUNNER_PATH"
fi

if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  echo "skill-runner missing, trying to build: $SKILL_RUNNER_ABS" # zh: 未找到 skill-runner，尝试自动编译。
  BUILD_SKILL_RELEASE=0
  if [[ "$SKILL_RUNNER_PATH" == *"/release/"* || "$SKILL_RUNNER_PATH" == *"target/release/"* ]]; then
    BUILD_SKILL_RELEASE=1
  fi
  if [[ "$BUILD_SKILL_RELEASE" == "1" ]]; then
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

start_clawd
start_telegramd

echo "One-click binary startup command executed (profile: $PROFILE)." # zh: 一键启动已编译二进制命令已执行（profile: $PROFILE）。

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PID_DIR="$SCRIPT_DIR/.pids"

stop_by_pid_file() {
  local pid_file="$1"
  local name="$2"
  if [[ ! -f "$pid_file" ]]; then
    return 1
  fi

  local pid
  pid="$(<"$pid_file")"
  if [[ -z "${pid}" || ! "$pid" =~ ^[0-9]+$ ]]; then
    rm -f "$pid_file"
    return 1
  fi

  if kill -0 "$pid" >/dev/null 2>&1; then
    kill "$pid" >/dev/null 2>&1 || true
    sleep 1
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill -9 "$pid" >/dev/null 2>&1 || true
    fi
    echo "Stopped by pid: ${name} (PID=${pid})" # zh: 已按 pid 停止: ${name} (PID=${pid})
  else
    echo "PID not running for ${name}: ${pid}" # zh: ${name} 的 pid 不在运行: ${pid}
  fi
  rm -f "$pid_file"
  return 0
}

stop_by_name() {
  local pattern="$1"
  if pgrep -f "$pattern" >/dev/null 2>&1; then
    pkill -f "$pattern" || true
    echo "Stopped: $pattern" # zh: 已停止: $pattern
  else
    echo "Not running: $pattern" # zh: 未运行: $pattern
  fi
} 

if ! stop_by_pid_file "$PID_DIR/clawd.pid" "clawd"; then
  stop_by_name 'target/(debug|release)/clawd|cargo run -p clawd'
fi
if ! stop_by_pid_file "$PID_DIR/telegramd.pid" "telegramd"; then
  stop_by_name 'target/(debug|release)/telegramd|cargo run -p telegramd'
fi

if [[ -d "$PID_DIR" ]]; then
  rm -f "$PID_DIR/clawd.pid" "$PID_DIR/telegramd.pid"
fi

echo "RustClaw has been stopped." # zh: RustClaw 已停止。

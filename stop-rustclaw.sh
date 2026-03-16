#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PID_DIR="$SCRIPT_DIR/.pids"

pid_cmdline_matches() {
  local pid="$1"
  local pattern="$2"
  local cmdline_file="/proc/$pid/cmdline"
  if [[ ! -r "$cmdline_file" ]]; then
    return 1
  fi
  local cmdline
  cmdline="$(tr '\0' ' ' < "$cmdline_file" 2>/dev/null || true)"
  [[ -n "$cmdline" && "$cmdline" =~ $pattern ]]
}

wait_pid_exit() {
  local pid="$1"
  local tries="${2:-20}"
  local delay_secs="${3:-0.2}"
  local i
  for ((i = 0; i < tries; i++)); do
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      return 0
    fi
    sleep "$delay_secs"
  done
  ! kill -0 "$pid" >/dev/null 2>&1
}

stop_by_pid_file() {
  local pid_file="$1"
  local name="$2"
  local pattern="$3"
  if [[ ! -f "$pid_file" ]]; then
    return 1
  fi

  local pid
  pid="$(<"$pid_file")"
  if [[ -z "${pid}" || ! "$pid" =~ ^[0-9]+$ ]]; then
    rm -f "$pid_file"
    return 1
  fi

  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "PID not running for ${name}: ${pid}" # zh: ${name} 的 pid 不在运行: ${pid}
    rm -f "$pid_file"
    return 1
  fi

  if ! pid_cmdline_matches "$pid" "$pattern"; then
    echo "PID file mismatch for ${name}: ${pid}" # zh: ${name} 的 pid 文件与实际进程不匹配: ${pid}
    rm -f "$pid_file"
    return 1
  fi

  if kill -0 "$pid" >/dev/null 2>&1; then
    kill "$pid" >/dev/null 2>&1 || true
    if ! wait_pid_exit "$pid" 10 0.2; then
      kill -9 "$pid" >/dev/null 2>&1 || true
      if ! wait_pid_exit "$pid" 10 0.2; then
        echo "Failed to stop by pid: ${name} (PID=${pid})" # zh: 按 pid 停止失败: ${name} (PID=${pid})
        rm -f "$pid_file"
        return 1
      fi
    fi
    echo "Stopped by pid: ${name} (PID=${pid})" # zh: 已按 pid 停止: ${name} (PID=${pid})
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

if ! stop_by_pid_file "$PID_DIR/clawd.pid" "clawd" 'target/(debug|release)/clawd|cargo run -p clawd'; then
  stop_by_name 'target/(debug|release)/clawd|cargo run -p clawd'
fi
if ! stop_by_pid_file "$PID_DIR/telegramd.pid" "telegramd" 'target/(debug|release)/telegramd|cargo run -p telegramd'; then
  stop_by_name 'target/(debug|release)/telegramd|cargo run -p telegramd'
fi
if ! stop_by_pid_file "$PID_DIR/whatsappd.pid" "whatsappd" 'target/(debug|release)/whatsappd|cargo run -p whatsappd'; then
  stop_by_name 'target/(debug|release)/whatsappd|cargo run -p whatsappd'
fi
if ! stop_by_pid_file "$PID_DIR/whatsapp_webd.pid" "whatsapp_webd" 'target/(debug|release)/whatsapp_webd|cargo run -p whatsapp_webd|services/wa-web-bridge/index.js|start-whatsapp-webd.sh|start-wa-web-bridge.sh'; then
  stop_by_name 'target/(debug|release)/whatsapp_webd|cargo run -p whatsapp_webd|services/wa-web-bridge/index.js|start-whatsapp-webd.sh|start-wa-web-bridge.sh'
fi
# Always cleanup bridge process even when whatsapp_webd was stopped via pid file.
stop_by_name 'services/wa-web-bridge/index.js'
if ! stop_by_pid_file "$PID_DIR/feishud.pid" "feishud" 'target/(debug|release)/feishud|cargo run -p feishud'; then
  stop_by_name 'target/(debug|release)/feishud|cargo run -p feishud'
fi

if [[ -d "$PID_DIR" ]]; then
  rm -f "$PID_DIR/clawd.pid" "$PID_DIR/telegramd.pid" "$PID_DIR/whatsappd.pid" "$PID_DIR/whatsapp_webd.pid" "$PID_DIR/feishud.pid"
fi

echo "RustClaw has been stopped." # zh: RustClaw 已停止。

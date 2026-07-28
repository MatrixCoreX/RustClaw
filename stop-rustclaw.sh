#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PID_DIR="$SCRIPT_DIR/.pids"
STOP_GRACE_SECONDS="${RUSTCLAW_STOP_GRACE_SECONDS:-8}"

case "$STOP_GRACE_SECONDS" in
  ""|*[!0-9]*)
    echo "RUSTCLAW_STOP_GRACE_SECONDS must be a non-negative integer." >&2
    exit 2
    ;;
esac

process_command() {
  local pid="$1"
  local cmdline_file="/proc/$pid/cmdline"
  if [[ -r "$cmdline_file" ]]; then
    tr '\0' ' ' < "$cmdline_file" 2>/dev/null || true
    return
  fi
  ps -p "$pid" -o command= 2>/dev/null || true
}

process_is_running() {
  local pid="$1"
  local state=""
  state="$(ps -p "$pid" -o stat= 2>/dev/null | tr -d '[:space:]' || true)"
  [[ -n "$state" && "$state" != Z* ]]
}

process_executable_matches() {
  local pid="$1"
  local expected_command="$2"
  local executable_path=""

  # Linux exposes the executable independently from argv. Prefer it because
  # older launchers may have used a workspace-relative argv[0].
  if [[ "$(uname -s)" == "Linux" && -L "/proc/$pid/exe" ]]; then
    executable_path="$(readlink "/proc/$pid/exe" 2>/dev/null || true)"
    executable_path="${executable_path% (deleted)}"
    [[ -n "$executable_path" && "$executable_path" == "$expected_command" ]]
    return
  fi

  # macOS has no /proc executable link. lsof exposes the mapped main binary,
  # which remains reliable when argv[0] was workspace-relative.
  if [[ "$(uname -s)" == "Darwin" ]] && command -v lsof >/dev/null 2>&1; then
    while IFS= read -r executable_path; do
      if [[ -n "$executable_path" && "$executable_path" == "$expected_command" ]]; then
        return 0
      fi
    done < <(lsof -a -p "$pid" -d txt -Fn 2>/dev/null | sed -n 's/^n//p')
  fi

  return 1
}

process_matches() {
  local pid="$1"
  local expected_command="$2"
  local cmdline_file="/proc/$pid/cmdline"
  local argument=""
  if process_executable_matches "$pid" "$expected_command"; then
    return 0
  fi
  if [[ -r "$cmdline_file" ]]; then
    while IFS= read -r -d '' argument; do
      if [[ "$argument" == "$expected_command" ]]; then
        return 0
      fi
    done < "$cmdline_file"
    return 1
  fi
  local command=""
  command="$(process_command "$pid")"
  [[ -n "$command" && "$command" == *"$expected_command"* ]]
}

wait_pid_exit() {
  local pid="$1"
  local tries="$((STOP_GRACE_SECONDS * 5))"
  local i
  for ((i = 0; i < tries; i++)); do
    if ! process_is_running "$pid"; then
      return 0
    fi
    sleep 0.2
  done
  ! process_is_running "$pid"
}

stop_owned_pid() {
  local pid="$1"
  local name="$2"
  if ! kill "$pid" >/dev/null 2>&1; then
    echo "Unable to send TERM to $name (PID=$pid)." >&2
    return 1
  fi
  if ! wait_pid_exit "$pid"; then
    if ! kill -9 "$pid" >/dev/null 2>&1; then
      echo "Unable to send KILL to $name (PID=$pid)." >&2
      return 1
    fi
    if ! wait_pid_exit "$pid"; then
      echo "Failed to stop $name (PID=$pid)." >&2
      return 1
    fi
  fi
  echo "Stopped: $name (PID=$pid)"
}

find_owned_pids() {
  local expected_command="$1"
  local search_token=""
  local candidates=""
  search_token="$(basename "$expected_command")"

  if command -v pgrep >/dev/null 2>&1; then
    candidates="$(pgrep -f "$search_token" 2>/dev/null || true)"
  else
    candidates="$(
      while read -r pid command; do
        case "$pid" in
          ""|*[!0-9]*) continue ;;
        esac
        if [[ "$command" == *"$expected_command"* ]]; then
          printf '%s\n' "$pid"
        fi
      done < <(ps -ax -o pid= -o command= 2>/dev/null || true)
    )"
  fi

  local pid=""
  for pid in $candidates; do
    case "$pid" in
      ""|*[!0-9]*) continue ;;
    esac
    if [[ "$pid" != "$$" ]] && process_is_running "$pid" &&
      process_matches "$pid" "$expected_command"; then
      printf '%s\n' "$pid"
    fi
  done
}

stop_component() {
  local component="$1"
  local name="$2"
  local expected_command="$3"
  local pid_file="$PID_DIR/$component.pid"
  local pid=""
  local stopped=0
  local failed=0

  if [[ -f "$pid_file" ]]; then
    pid="$(tr -d '[:space:]' < "$pid_file" 2>/dev/null || true)"
    case "$pid" in
      ""|*[!0-9]*)
        echo "Removed invalid PID file for $name."
        ;;
      *)
        if ! process_is_running "$pid"; then
          echo "Removed stale PID file for $name (PID=$pid)."
        elif process_matches "$pid" "$expected_command"; then
          if stop_owned_pid "$pid" "$name"; then
            stopped=1
          else
            failed=1
          fi
        else
          echo "Ignored mismatched PID file for $name (PID=$pid); the process was not touched."
        fi
        ;;
    esac
    rm -f "$pid_file"
  fi

  local discovered=""
  discovered="$(find_owned_pids "$expected_command")"
  for pid in $discovered; do
    if stop_owned_pid "$pid" "$name"; then
      stopped=1
    else
      failed=1
    fi
  done

  if [[ "$stopped" == "0" && "$failed" == "0" ]]; then
    echo "Not running: $name"
  fi
  [[ "$failed" == "0" ]]
}

mkdir -p "$PID_DIR"
failures=0

# Stop ingress/channel processes first. Keep clawd alive until adapters have
# stopped so in-flight channel shutdown does not lose its core dependency.
stop_component "telegramd" "telegramd" "$SCRIPT_DIR/target/release/telegramd" || failures=1
stop_component "whatsappd" "whatsappd" "$SCRIPT_DIR/target/release/whatsappd" || failures=1
stop_component "whatsapp_webd" "whatsapp_webd" "$SCRIPT_DIR/target/release/whatsapp_webd" || failures=1
stop_component "wa-web-bridge" "wa-web-bridge" "$SCRIPT_DIR/services/wa-web-bridge/index.js" || failures=1
stop_component "wechatd" "wechatd" "$SCRIPT_DIR/target/release/wechatd" || failures=1
stop_component "feishud" "feishud" "$SCRIPT_DIR/target/release/feishud" || failures=1
stop_component "larkd" "larkd" "$SCRIPT_DIR/target/release/larkd" || failures=1
stop_component "webd" "webd" "$SCRIPT_DIR/target/release/webd" || failures=1
stop_component "clawd" "clawd" "$SCRIPT_DIR/target/release/clawd" || failures=1
stop_component "whisper-server" "whisper-server" "${WHISPER_SERVER_BIN:-$SCRIPT_DIR/data/vendor/whisper.cpp/build/bin/whisper-server}" || failures=1

if [[ "$failures" != "0" ]]; then
  echo "RustClaw stop completed with errors." >&2
  exit 1
fi

echo "RustClaw has been stopped."

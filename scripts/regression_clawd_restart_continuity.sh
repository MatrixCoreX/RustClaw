#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

CLAWD_BIN="${CLAWD_BIN:-${ROOT_DIR}/target/debug/clawd}"
AUTO_BUILD="${AUTO_BUILD:-1}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-1}"
WAIT_SECONDS="${WAIT_SECONDS:-60}"
COMMAND_SLEEP_SECONDS="${COMMAND_SLEEP_SECONDS:-8}"
LOG_DIR="${LOG_DIR:-${ROOT_DIR}/target/clawd_restart_continuity_$(date +%Y%m%d_%H%M%S)}"

TEMP_WORKSPACE=""
CLAWD_PID=""
PORT=""
BASE_URL=""
ADMIN_KEY=""
TASK_ID=""
APPROVAL_REQUEST_ID=""

cleanup() {
  local exit_code=$?
  if [[ -n "$CLAWD_PID" ]] && kill -0 "$CLAWD_PID" >/dev/null 2>&1; then
    kill "$CLAWD_PID" >/dev/null 2>&1 || true
    wait "$CLAWD_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$TEMP_WORKSPACE" && -d "$TEMP_WORKSPACE" ]]; then
    rm -rf "$TEMP_WORKSPACE"
  fi
  exit "$exit_code"
}
trap cleanup EXIT

pick_free_port() {
  python3 - <<'PY'
import socket

sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

ensure_binary() {
  if [[ ! -x "$CLAWD_BIN" && "$AUTO_BUILD" == "1" ]]; then
    (cd "$ROOT_DIR" && cargo build -p clawd)
  fi
  [[ -x "$CLAWD_BIN" ]] || {
    echo "clawd binary not found: ${CLAWD_BIN}" >&2
    return 1
  }
}

prepare_workspace() {
  TEMP_WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/rustclaw-restart-continuity-XXXXXX")"
  cp "$ROOT_DIR/Cargo.toml" "$TEMP_WORKSPACE/Cargo.toml"
  [[ ! -f "$ROOT_DIR/Cargo.lock" ]] || cp "$ROOT_DIR/Cargo.lock" "$TEMP_WORKSPACE/Cargo.lock"
  cp -R "$ROOT_DIR/configs" "$TEMP_WORKSPACE/configs"
  cp -R "$ROOT_DIR/prompts" "$TEMP_WORKSPACE/prompts"
  mkdir -p "$TEMP_WORKSPACE/data" "$TEMP_WORKSPACE/document" "$TEMP_WORKSPACE/logs"
  ln -s "$ROOT_DIR/crates" "$TEMP_WORKSPACE/crates"
  ln -s "$ROOT_DIR/scripts" "$TEMP_WORKSPACE/scripts"
  ln -s "$ROOT_DIR/target" "$TEMP_WORKSPACE/target"

  python3 - "$TEMP_WORKSPACE/configs/config.toml" "$PORT" "$TEMP_WORKSPACE/data/tasks.sqlite" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
port = sys.argv[2]
sqlite_path = sys.argv[3]
text = path.read_text(encoding="utf-8")

def replace_once(pattern: str, replacement: str, raw: str) -> str:
    updated, count = re.subn(pattern, replacement, raw, count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"failed to patch config pattern: {pattern}")
    return updated

text = replace_once(r'^listen\s*=\s*".*"$', f'listen = "127.0.0.1:{port}"', text)
text = replace_once(r'^sqlite_path\s*=\s*".*"$', f'sqlite_path = "{sqlite_path}"', text)
text = replace_once(r'^access_profile\s*=\s*".*"$', 'access_profile = "full"', text)
text = replace_once(r'^poll_interval_ms\s*=\s*\d+$', 'poll_interval_ms = 200', text)
text = replace_once(r'^task_timeout_seconds\s*=\s*\d+$', 'task_timeout_seconds = 120', text)
path.write_text(text, encoding="utf-8")
PY

  ADMIN_KEY="$(
    RUSTCLAW_CONFIG_PATH="$TEMP_WORKSPACE/configs/config.toml" \
      bash "$ROOT_DIR/scripts/auth-key.sh" generate admin | awk '{print $1; exit}'
  )"
  [[ -n "$ADMIN_KEY" ]] || {
    echo "failed to generate isolated admin key" >&2
    return 1
  }
}

start_clawd() {
  local log_path="$1"
  (
    cd "$TEMP_WORKSPACE"
    WORKSPACE_ROOT="$TEMP_WORKSPACE" "$CLAWD_BIN"
  ) >"$log_path" 2>&1 &
  CLAWD_PID=$!
}

stop_clawd() {
  [[ -n "$CLAWD_PID" ]] || return 0
  if kill -0 "$CLAWD_PID" >/dev/null 2>&1; then
    kill "$CLAWD_PID"
    wait "$CLAWD_PID" >/dev/null 2>&1 || true
  fi
  CLAWD_PID=""
}

wait_for_health() {
  local waited=0
  while (( waited <= WAIT_SECONDS )); do
    if curl -fsS -H "X-RustClaw-Key: ${ADMIN_KEY}" "${BASE_URL}/v1/health" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -n "$CLAWD_PID" ]] && ! kill -0 "$CLAWD_PID" >/dev/null 2>&1; then
      echo "clawd exited before becoming healthy" >&2
      return 1
    fi
    sleep "$POLL_INTERVAL_SECONDS"
    waited=$((waited + POLL_INTERVAL_SECONDS))
  done
  echo "health timeout for ${BASE_URL}" >&2
  return 1
}

submit_long_command() {
  local request_path="$LOG_DIR/submit_request.json"
  local response_path="$LOG_DIR/submit_response.json"
  python3 - "$request_path" "$COMMAND_SLEEP_SECONDS" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
sleep_seconds = int(sys.argv[2])
command = (
    "printf 'mutation-once\\n' >> document/restart-continuity-counter.txt; "
    f"sleep {sleep_seconds}; "
    "printf 'RUSTCLAW_RESTART_LONG_COMMAND_DONE\\n'"
)
path.write_text(json.dumps({
    "user_id": 2_147_300_001,
    "chat_id": 2_147_300_002,
    "channel": "ui",
    "kind": "run_skill",
    "payload": {
        "skill_name": "run_cmd",
        "args": {
            "command": command,
            "async_start": True,
            "poll_after_seconds": 1,
            "expires_in_seconds": 60,
        },
    },
}, ensure_ascii=False), encoding="utf-8")
PY
  curl -fsS -X POST "${BASE_URL}/v1/tasks" \
    -H "Content-Type: application/json" \
    -H "X-RustClaw-Key: ${ADMIN_KEY}" \
    --data-binary "@${request_path}" >"$response_path"
  TASK_ID="$(
    python3 - "$response_path" <<'PY'
from pathlib import Path
import json
import sys

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
if not obj.get("ok"):
    raise SystemExit(f"submit failed: {obj.get('error')}")
print((obj.get("data") or {}).get("task_id") or "")
PY
  )"
  [[ -n "$TASK_ID" ]] || {
    echo "submit response missing task id" >&2
    return 1
  }
}

query_task() {
  local output_path="$1"
  curl -fsS -H "X-RustClaw-Key: ${ADMIN_KEY}" \
    "${BASE_URL}/v1/tasks/${TASK_ID}" >"$output_path"
}

wait_for_approval_request() {
  local waited=0
  local query_path="$LOG_DIR/approval_task.json"
  while (( waited <= WAIT_SECONDS )); do
    query_task "$query_path"
    APPROVAL_REQUEST_ID="$(
      python3 - "$query_path" <<'PY'
from pathlib import Path
import json
import sys

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
result = ((obj.get("data") or {}).get("result_json") or {})
request = ((result.get("resume_context") or {}).get("approval_request") or {})
if request.get("status") == "pending":
    print(str(request.get("request_id") or ""))
PY
    )"
    if [[ -n "$APPROVAL_REQUEST_ID" ]]; then
      return 0
    fi
    sleep "$POLL_INTERVAL_SECONDS"
    waited=$((waited + POLL_INTERVAL_SECONDS))
  done
  echo "task did not publish an approval request" >&2
  return 1
}

approve_task_once() {
  local request_path="$LOG_DIR/approval_request.json"
  local response_path="$LOG_DIR/approval_response.json"
  python3 - "$request_path" "$TASK_ID" "$APPROVAL_REQUEST_ID" <<'PY'
from pathlib import Path
import json
import sys

Path(sys.argv[1]).write_text(json.dumps({
    "task_id": sys.argv[2],
    "approval_request_id": sys.argv[3],
    "approval_decision": "approve_once",
}, ensure_ascii=False), encoding="utf-8")
PY
  curl -fsS -X POST "${BASE_URL}/v1/tasks/resume-by-task-id" \
    -H "Content-Type: application/json" \
    -H "X-RustClaw-Key: ${ADMIN_KEY}" \
    --data-binary "@${request_path}" >"$response_path"
  python3 - "$response_path" <<'PY'
from pathlib import Path
import json
import sys

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
assert obj.get("ok"), obj
assert data.get("status") == "approval_grant_approved", data
PY
}

wait_for_checkpoint() {
  local waited=0
  local query_path="$LOG_DIR/pre_restart_task.json"
  while (( waited <= WAIT_SECONDS )); do
    query_task "$query_path"
    if python3 - "$query_path" <<'PY'
from pathlib import Path
import json
import sys

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
result = ((obj.get("data") or {}).get("result_json") or {})
checkpoint = result.get("task_checkpoint") or {}
job = checkpoint.get("pending_async_job") or {}
valid = (
    bool(checkpoint.get("checkpoint_id"))
    and bool(job.get("job_id"))
    and bool(job.get("cancel_ref"))
    and int(job.get("poll_after_seconds") or 0) > 0
)
raise SystemExit(0 if valid else 1)
PY
    then
      return 0
    fi
    sleep "$POLL_INTERVAL_SECONDS"
    waited=$((waited + POLL_INTERVAL_SECONDS))
  done
  echo "task did not publish an async checkpoint" >&2
  return 1
}

wait_for_terminal_success() {
  local waited=0
  local query_path="$LOG_DIR/post_restart_task.json"
  while (( waited <= WAIT_SECONDS )); do
    query_task "$query_path"
    local status
    status="$(
      python3 - "$query_path" <<'PY'
from pathlib import Path
import json
import sys

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(str((obj.get("data") or {}).get("status") or ""))
PY
    )"
    case "$status" in
      succeeded) return 0 ;;
      failed|timeout|canceled)
        echo "task reached unexpected terminal status: ${status}" >&2
        return 1
        ;;
    esac
    sleep "$POLL_INTERVAL_SECONDS"
    waited=$((waited + POLL_INTERVAL_SECONDS))
  done
  echo "task did not complete after clawd restart" >&2
  return 1
}

assert_restart_result() {
  python3 - \
    "$LOG_DIR/pre_restart_task.json" \
    "$LOG_DIR/post_restart_task.json" \
    "$TEMP_WORKSPACE/document/restart-continuity-counter.txt" <<'PY'
from pathlib import Path
import json
import sys

before = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
after = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
counter_path = Path(sys.argv[3])
before_result = ((before.get("data") or {}).get("result_json") or {})
after_data = after.get("data") or {}
after_result = after_data.get("result_json") or {}
before_checkpoint = before_result.get("task_checkpoint") or {}
after_lifecycle = after_result.get("task_lifecycle") or {}

assert after_data.get("status") == "succeeded", after_data.get("status")
assert before_checkpoint.get("checkpoint_id"), "missing pre-restart checkpoint"
assert "RUSTCLAW_RESTART_LONG_COMMAND_DONE" in json.dumps(after_result), "missing command output"
assert after_lifecycle.get("state") == "succeeded", after_lifecycle
lines = counter_path.read_text(encoding="utf-8").splitlines()
assert lines == ["mutation-once"], lines

summary = {
    "status": "pass",
    "task_id": after_data.get("task_id"),
    "checkpoint_id": before_checkpoint.get("checkpoint_id"),
    "async_job_id": (before_checkpoint.get("pending_async_job") or {}).get("job_id"),
    "mutation_count": len(lines),
    "terminal_state": after_lifecycle.get("state"),
}
print(json.dumps(summary, ensure_ascii=False))
PY
}

mkdir -p "$LOG_DIR"
ensure_binary
PORT="$(pick_free_port)"
BASE_URL="http://127.0.0.1:${PORT}"
prepare_workspace

echo "[CASE] actual_clawd_restart_continues_async_command"
echo "  base_url=${BASE_URL}"
echo "  log_dir=${LOG_DIR}"

start_clawd "$LOG_DIR/clawd_before_restart.log"
wait_for_health
submit_long_command
echo "  task_id=${TASK_ID}"
wait_for_approval_request
approve_task_once
echo "  approval=approved_once"
wait_for_checkpoint
echo "  checkpoint=observed"

stop_clawd
echo "  clawd=stopped"
sleep 2

start_clawd "$LOG_DIR/clawd_after_restart.log"
wait_for_health
wait_for_terminal_success
assert_restart_result | tee "$LOG_DIR/summary.json"
echo "[PASS] actual_clawd_restart_continues_async_command"

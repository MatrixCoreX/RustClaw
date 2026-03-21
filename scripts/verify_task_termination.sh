#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
source "${SCRIPT_DIR}/lib.sh"

BASE_URL="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID="${USER_ID:-1985996990}"
CHAT_ID="${CHAT_ID:-1985996990}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-1}"
RUN_ROOT_DEFAULT="${SCRIPT_DIR}/task_termination_logs"
RUN_ROOT="${RUN_ROOT_DEFAULT}"
TIMEOUT_MARGIN_SECONDS="${TIMEOUT_MARGIN_SECONDS:-20}"
CANCEL_SLEEP_SECONDS="${CANCEL_SLEEP_SECONDS:-20}"
CANCEL_AFTER_SECONDS="${CANCEL_AFTER_SECONDS:-2}"
MAX_PRACTICAL_TIMEOUT_SECONDS="${MAX_PRACTICAL_TIMEOUT_SECONDS:-600}"
VERIFY_CANCEL=1
VERIFY_TIMEOUT=1
DB_PATH=""

usage() {
  cat <<'EOF'
Usage:
  bash scripts/verify_task_termination.sh [options]

Options:
  --base-url URL               clawd base url, default http://127.0.0.1:8787
  --user-key KEY               explicit user/admin key; default auto-detect enabled admin key
  --user-id N                  default 1985996990
  --chat-id N                  default 1985996990
  --log-root DIR               default scripts/task_termination_logs
  --cancel-sleep-seconds N     long sleep for cancel case, default 20
  --cancel-after-seconds N     wait before sending cancel, default 2
  --timeout-margin-seconds N   worker timeout extra margin, default 20
  --max-practical-timeout N    skip timeout case when worker timeout exceeds this, default 600
  --skip-cancel                do not run cancel verification
  --skip-timeout               do not run timeout verification
  -h, --help                   show help

What it verifies:
  1) cancel after running -> final status must remain canceled
  2) long running task -> final status must become timeout

Logs:
  scripts/task_termination_logs/<timestamp>/
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base-url)
      BASE_URL="$2"
      shift 2
      ;;
    --user-key)
      USER_KEY="$2"
      shift 2
      ;;
    --user-id)
      USER_ID="$2"
      shift 2
      ;;
    --chat-id)
      CHAT_ID="$2"
      shift 2
      ;;
    --log-root)
      RUN_ROOT="$2"
      shift 2
      ;;
    --cancel-sleep-seconds)
      CANCEL_SLEEP_SECONDS="$2"
      shift 2
      ;;
    --cancel-after-seconds)
      CANCEL_AFTER_SECONDS="$2"
      shift 2
      ;;
    --timeout-margin-seconds)
      TIMEOUT_MARGIN_SECONDS="$2"
      shift 2
      ;;
    --max-practical-timeout)
      MAX_PRACTICAL_TIMEOUT_SECONDS="$2"
      shift 2
      ;;
    --skip-cancel)
      VERIFY_CANCEL=0
      shift
      ;;
    --skip-timeout)
      VERIFY_TIMEOUT=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

normalized_user_key() {
  printf '%s' "${USER_KEY:-${RUSTCLAW_USER_KEY:-}}" | xargs
}

auto_admin_key() {
  bash "${SCRIPT_DIR}/auth-key.sh" list \
    | awk -F'\t' '$2=="admin" && $3=="enabled" {print $1; exit}'
}

ensure_user_key() {
  local key
  key="$(normalized_user_key)"
  if [[ -n "$key" ]]; then
    USER_KEY="$key"
    return 0
  fi
  key="$(auto_admin_key)"
  if [[ -z "$key" ]]; then
    echo "no enabled admin key found" >&2
    exit 2
  fi
  USER_KEY="$key"
}

json_field() {
  local raw="$1"
  local expr="$2"
  python3 - "$raw" "$expr" <<'PY'
import json
import sys

raw = sys.argv[1]
expr = sys.argv[2]
obj = json.loads(raw)
value = obj
for part in expr.split('.'):
    if not part:
        continue
    if isinstance(value, dict):
        value = value.get(part)
    else:
        value = None
        break
if value is None:
    print("")
elif isinstance(value, (dict, list)):
    print(json.dumps(value, ensure_ascii=False))
else:
    print(value)
PY
}

get_health_json() {
  local -a auth_args=()
  mapfile -t auth_args < <(curl_auth_args)
  curl -sS "${auth_args[@]}" "${BASE_URL}/v1/health"
}

get_worker_timeout_seconds() {
  local raw
  raw="$(get_health_json)"
  json_field "$raw" "data.task_timeout_seconds"
}

resolve_db_path() {
  python3 - <<'PY'
from pathlib import Path
import tomllib

root = Path.cwd()
config_path = root / "configs" / "config.toml"
with config_path.open("rb") as f:
    data = tomllib.load(f)
db_path = (((data.get("database") or {}).get("sqlite_path")) or "").strip()
if not db_path:
    print(root / "data" / "rustclaw.db")
else:
    p = Path(db_path)
    print(p if p.is_absolute() else root / p)
PY
}

effective_ids_by_task() {
  local task_id="$1"
  python3 - "$DB_PATH" "$task_id" <<'PY'
import sqlite3
import sys

db_path = sys.argv[1]
task_id = sys.argv[2]
conn = sqlite3.connect(db_path)
row = conn.execute(
    "select user_id, chat_id from tasks where task_id=? limit 1",
    (task_id,),
).fetchone()
if not row:
    raise SystemExit(1)
print(f"{row[0]}\t{row[1]}")
PY
}

build_cancel_body() {
  local cancel_user_id="$1"
  local cancel_chat_id="$2"
  python3 - "$cancel_user_id" "$cancel_chat_id" <<'PY'
import json
import sys
print(json.dumps({
    "user_id": int(sys.argv[1]),
    "chat_id": int(sys.argv[2]),
}, ensure_ascii=False))
PY
}

cancel_tasks_now() {
  local cancel_user_id="$1"
  local cancel_chat_id="$2"
  local body
  local -a auth_args=()
  body="$(build_cancel_body "$cancel_user_id" "$cancel_chat_id")"
  mapfile -t auth_args < <(curl_auth_args)
  curl -sS -X POST "${BASE_URL}/v1/tasks/cancel" \
    -H "Content-Type: application/json" \
    "${auth_args[@]}" \
    -d "$body"
}

task_status() {
  local raw="$1"
  json_field "$raw" "data.status"
}

wait_task_status() {
  local task_id="$1"
  local expected_status="$2"
  local limit_seconds="$3"
  local waited=0
  while (( waited <= limit_seconds )); do
    local raw status
    raw="$(query_task "$task_id")"
    status="$(task_status "$raw")"
    if [[ "$status" == "$expected_status" ]]; then
      printf '%s\n' "$raw"
      return 0
    fi
    sleep "$POLL_INTERVAL_SECONDS"
    waited=$((waited + POLL_INTERVAL_SECONDS))
  done
  return 1
}

run_case_cancel() {
  local case_dir="$1"
  local submit_raw task_id cancel_raw canceled_raw final_raw final_status
  local effective_ids cancel_user_id cancel_chat_id
  local args_json

  args_json="$(python3 - "$CANCEL_SLEEP_SECONDS" <<'PY'
import json
import sys
seconds = int(sys.argv[1])
print(json.dumps({"command": f"sleep {seconds}"}))
PY
)"

  submit_raw="$(submit_run_skill_task "run_cmd" "$args_json")"
  printf '%s\n' "$submit_raw" > "${case_dir}/submit.json"
  task_id="$(extract_submit_task_id "$submit_raw")"
  printf '%s\n' "$task_id" > "${case_dir}/task_id.txt"
  effective_ids="$(effective_ids_by_task "$task_id")"
  cancel_user_id="$(printf '%s' "$effective_ids" | awk -F'\t' '{print $1}')"
  cancel_chat_id="$(printf '%s' "$effective_ids" | awk -F'\t' '{print $2}')"
  {
    echo "effective_user_id=${cancel_user_id}"
    echo "effective_chat_id=${cancel_chat_id}"
  } > "${case_dir}/effective_ids.txt"

  sleep "$CANCEL_AFTER_SECONDS"
  cancel_raw="$(cancel_tasks_now "$cancel_user_id" "$cancel_chat_id")"
  printf '%s\n' "$cancel_raw" > "${case_dir}/cancel.json"

  canceled_raw="$(wait_task_status "$task_id" "canceled" 30 || true)"
  if [[ -z "${canceled_raw:-}" ]]; then
    echo "cancel case did not reach canceled in time" > "${case_dir}/assertion.txt"
    return 1
  fi
  printf '%s\n' "$canceled_raw" > "${case_dir}/canceled.json"

  sleep 3
  final_raw="$(query_task "$task_id")"
  final_status="$(task_status "$final_raw")"
  printf '%s\n' "$final_raw" > "${case_dir}/final.json"

  {
    echo "case=cancel"
    echo "task_id=${task_id}"
    echo "effective_user_id=${cancel_user_id}"
    echo "effective_chat_id=${cancel_chat_id}"
    echo "final_status=${final_status}"
    echo "expected_status=canceled"
  } > "${case_dir}/meta.txt"

  [[ "$final_status" == "canceled" ]]
}

run_case_timeout() {
  local case_dir="$1"
  local worker_timeout timeout_sleep submit_raw task_id final_raw final_status
  local args_json

  worker_timeout="$(get_worker_timeout_seconds)"
  if [[ -z "$worker_timeout" || ! "$worker_timeout" =~ ^[0-9]+$ ]]; then
    echo "failed to read worker timeout seconds" > "${case_dir}/assertion.txt"
    return 1
  fi
  if (( worker_timeout > MAX_PRACTICAL_TIMEOUT_SECONDS )); then
    {
      echo "skip_reason=worker.task_timeout_seconds too large for practical verification"
      echo "worker_timeout_seconds=${worker_timeout}"
      echo "max_practical_timeout_seconds=${MAX_PRACTICAL_TIMEOUT_SECONDS}"
    } > "${case_dir}/meta.txt"
    return 2
  fi
  timeout_sleep=$((worker_timeout + TIMEOUT_MARGIN_SECONDS))

  args_json="$(python3 - "$timeout_sleep" <<'PY'
import json
import sys
seconds = int(sys.argv[1])
print(json.dumps({"command": f"sleep {seconds}"}))
PY
)"

  submit_raw="$(submit_run_skill_task "run_cmd" "$args_json")"
  printf '%s\n' "$submit_raw" > "${case_dir}/submit.json"
  task_id="$(extract_submit_task_id "$submit_raw")"
  printf '%s\n' "$task_id" > "${case_dir}/task_id.txt"

  final_raw="$(wait_task_until_terminal_with_limit "$task_id" "$((timeout_sleep + 60))")"
  final_status="$(task_status "$final_raw")"
  printf '%s\n' "$final_raw" > "${case_dir}/final.json"

  {
    echo "case=timeout"
    echo "task_id=${task_id}"
    echo "worker_timeout_seconds=${worker_timeout}"
    echo "sleep_seconds=${timeout_sleep}"
    echo "final_status=${final_status}"
    echo "expected_status=timeout"
  } > "${case_dir}/meta.txt"

  [[ "$final_status" == "timeout" ]]
}

ensure_user_key
DB_PATH="$(resolve_db_path)"
export BASE_URL USER_ID CHAT_ID USER_KEY POLL_INTERVAL_SECONDS
health_check

timestamp="$(date +%Y%m%d_%H%M%S)"
RUN_DIR="${RUN_ROOT}/${timestamp}"
mkdir -p "${RUN_DIR}"
RUN_LOG="${RUN_DIR}/run.log"
SUMMARY_JSONL="${RUN_DIR}/summary.jsonl"

exec > >(tee -a "${RUN_LOG}") 2>&1

echo "run_dir=${RUN_DIR}"
echo "base_url=${BASE_URL}"
echo "user_id=${USER_ID}"
echo "chat_id=${CHAT_ID}"
echo "using_user_key=$(printf '%s' "$USER_KEY" | sed 's/^\(.\{6\}\).*/\1.../')"
echo "db_path=${DB_PATH}"

pass_count=0
fail_count=0

record_case() {
  local case_name="$1"
  local status="$2"
  local case_dir="$3"
  python3 - "$case_name" "$status" "$case_dir" >> "${SUMMARY_JSONL}" <<'PY'
import json
import sys
print(json.dumps({
    "case": sys.argv[1],
    "status": sys.argv[2],
    "dir": sys.argv[3],
}, ensure_ascii=False))
PY
}

if (( VERIFY_CANCEL )); then
  case_dir="${RUN_DIR}/case_cancel"
  mkdir -p "${case_dir}"
  echo "[case_cancel] verifying canceled status is not overwritten"
  if run_case_cancel "${case_dir}"; then
    echo "[case_cancel] PASS"
    record_case "case_cancel" "PASS" "${case_dir}"
    pass_count=$((pass_count + 1))
  else
    echo "[case_cancel] FAIL"
    record_case "case_cancel" "FAIL" "${case_dir}"
    fail_count=$((fail_count + 1))
  fi
fi

if (( VERIFY_TIMEOUT )); then
  case_dir="${RUN_DIR}/case_timeout"
  mkdir -p "${case_dir}"
  echo "[case_timeout] verifying long task ends as timeout"
  if run_case_timeout "${case_dir}"; then
    echo "[case_timeout] PASS"
    record_case "case_timeout" "PASS" "${case_dir}"
    pass_count=$((pass_count + 1))
  else
    rc=$?
    if (( rc == 2 )); then
      echo "[case_timeout] SKIP"
      record_case "case_timeout" "SKIP" "${case_dir}"
    else
      echo "[case_timeout] FAIL"
      record_case "case_timeout" "FAIL" "${case_dir}"
      fail_count=$((fail_count + 1))
    fi
  fi
fi

echo "PASS=${pass_count} FAIL=${fail_count}"
if (( fail_count > 0 )); then
  exit 1
fi

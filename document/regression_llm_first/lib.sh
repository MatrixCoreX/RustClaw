#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID="${USER_ID:-1985996990}"
CHAT_ID="${CHAT_ID:-1985996990}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-1}"
MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-240}"
EXTRA_GRACE_SECONDS="${EXTRA_GRACE_SECONDS:-180}"

health_check() {
  curl -sS "${BASE_URL}/v1/health" >/dev/null
}

build_submit_body() {
  local prompt="$1"
  python3 - "$USER_ID" "$CHAT_ID" "$prompt" <<'PY'
import json
import sys

user_id = int(sys.argv[1])
chat_id = int(sys.argv[2])
prompt = sys.argv[3]
body = {
    "user_id": user_id,
    "chat_id": chat_id,
    "kind": "ask",
    "payload": {
        "text": prompt,
        "agent_mode": True,
    },
}
print(json.dumps(body, ensure_ascii=False))
PY
}

submit_task() {
  local prompt="$1"
  local body
  body="$(build_submit_body "$prompt")"
  curl -sS -X POST "${BASE_URL}/v1/tasks" \
    -H "Content-Type: application/json" \
    -d "$body"
}

build_submit_body_with_options() {
  local prompt="$1"
  local agent_mode="$2"
  local source="${3:-}"
  python3 - "$USER_ID" "$CHAT_ID" "$prompt" "$agent_mode" "$source" <<'PY'
import json
import sys

user_id = int(sys.argv[1])
chat_id = int(sys.argv[2])
prompt = sys.argv[3]
agent_mode_raw = (sys.argv[4] or "").strip().lower()
source = (sys.argv[5] or "").strip()
agent_mode = False if agent_mode_raw in ("0", "false", "no") else True
payload = {
    "text": prompt,
    "agent_mode": agent_mode,
}
if source:
    payload["source"] = source
body = {
    "user_id": user_id,
    "chat_id": chat_id,
    "kind": "ask",
    "payload": payload,
}
print(json.dumps(body, ensure_ascii=False))
PY
}

submit_task_with_options() {
  local prompt="$1"
  local agent_mode="$2"
  local source="${3:-}"
  local body
  body="$(build_submit_body_with_options "$prompt" "$agent_mode" "$source")"
  curl -sS -X POST "${BASE_URL}/v1/tasks" \
    -H "Content-Type: application/json" \
    -d "$body"
}

build_submit_run_skill_body() {
  local skill_name="$1"
  local args_json="${2:-}"
  if [ -z "$args_json" ]; then
    args_json="{}"
  fi
  python3 - "$USER_ID" "$CHAT_ID" "$skill_name" "$args_json" <<'PY'
import json
import sys

user_id = int(sys.argv[1])
chat_id = int(sys.argv[2])
skill_name = (sys.argv[3] or "").strip()
args_raw = (sys.argv[4] or "").strip()
if not skill_name:
    raise SystemExit("skill_name is required")
if not args_raw:
    args_raw = "{}"
args = json.loads(args_raw)
if not isinstance(args, dict):
    raise SystemExit("run_skill args must be json object")
body = {
    "user_id": user_id,
    "chat_id": chat_id,
    "kind": "run_skill",
    "payload": {
        "skill_name": skill_name,
        "args": args,
    },
}
print(json.dumps(body, ensure_ascii=False))
PY
}

submit_run_skill_task() {
  local skill_name="$1"
  local args_json="${2:-}"
  if [ -z "$args_json" ]; then
    args_json="{}"
  fi
  local body
  body="$(build_submit_run_skill_body "$skill_name" "$args_json")"
  curl -sS -X POST "${BASE_URL}/v1/tasks" \
    -H "Content-Type: application/json" \
    -d "$body"
}

extract_submit_task_id() {
  python3 - "${1:-}" <<'PY'
import json
import sys

raw = (sys.argv[1] if len(sys.argv) > 1 else "").strip()
if not raw:
    print("empty submit response", file=sys.stderr)
    sys.exit(2)
obj = json.loads(raw)
if not obj.get("ok"):
    print(f"submit failed: {obj.get('error')}", file=sys.stderr)
    sys.exit(3)
task_id = ((obj.get("data") or {}).get("task_id") or "").strip()
if not task_id:
    print("submit response missing task_id", file=sys.stderr)
    sys.exit(4)
print(task_id)
PY
}

query_task() {
  local task_id="$1"
  curl -sS "${BASE_URL}/v1/tasks/${task_id}"
}

extract_task_triplet() {
  python3 - "${1:-}" <<'PY'
import json
import sys

raw = (sys.argv[1] if len(sys.argv) > 1 else "").strip()
obj = json.loads(raw)
if not obj.get("ok"):
    print(f"query failed\t\t{obj.get('error')}")
    sys.exit(0)
data = obj.get("data") or {}
status = str(data.get("status", "")).strip()
result = data.get("result_json") or {}
text = str(result.get("text", "") or "")
error = str(data.get("error_text", "") or "")
# Keep output in a single line for shell-side tab parsing.
text = text.replace("\r", " ").replace("\n", "\\n").replace("\t", " ")
error = error.replace("\r", " ").replace("\n", "\\n").replace("\t", " ")
print(f"{status}\t{text}\t{error}")
PY
}

wait_task_until_terminal_with_limit() {
  local task_id="$1"
  local wait_limit_seconds="$2"
  local waited=0
  while [ "$waited" -le "$wait_limit_seconds" ]; do
    local raw row
    raw="$(query_task "$task_id")"
    row="$(extract_task_triplet "$raw")"
    local status text error
    status="$(printf '%s' "$row" | awk -F'\t' '{print $1}')"
    text="$(printf '%s' "$row" | awk -F'\t' '{print $2}')"
    error="$(printf '%s' "$row" | awk -F'\t' '{print $3}')"
    case "$status" in
      succeeded|failed|canceled|timeout)
        printf '%s\t%s\t%s\n' "$status" "$text" "$error"
        return 0
        ;;
      *)
        sleep "$POLL_INTERVAL_SECONDS"
        waited=$((waited + POLL_INTERVAL_SECONDS))
        ;;
    esac
  done
  printf 'timeout_wait\t\tpoll timeout after %ss\n' "$wait_limit_seconds"
}

wait_task_until_terminal() {
  local task_id="$1"
  wait_task_until_terminal_with_limit "$task_id" "$MAX_WAIT_SECONDS"
}

is_expected_status() {
  local status="$1"
  local expected_csv="$2"
  IFS=',' read -r -a arr <<<"$expected_csv"
  for s in "${arr[@]}"; do
    if [ "$status" = "$(echo "$s" | xargs)" ]; then
      return 0
    fi
  done
  return 1
}

run_case_expect() {
  local case_name="$1"
  local prompt="$2"
  local expected_status_csv="${3:-succeeded}"
  local expect_substring="${4:-}"
  local expect_field="${5:-text}"
  local reject_substring="${6:-}"
  local reject_field="${7:-either}"

  echo "[CASE] ${case_name}"
  echo "prompt: ${prompt}"
  local submit_resp task_id
  submit_resp="$(submit_task "$prompt")"
  task_id="$(extract_submit_task_id "$submit_resp")"
  echo "task_id: ${task_id}"

  local row status text error
  row="$(wait_task_until_terminal "$task_id")"
  status="$(printf '%s' "$row" | awk -F'\t' '{print $1}')"
  text="$(printf '%s' "$row" | awk -F'\t' '{print $2}')"
  error="$(printf '%s' "$row" | awk -F'\t' '{print $3}')"

  if [ "$status" = "timeout_wait" ] && [ "$EXTRA_GRACE_SECONDS" -gt 0 ]; then
    echo "INFO: initial wait timeout, enter grace wait ${EXTRA_GRACE_SECONDS}s ..."
    row="$(wait_task_until_terminal_with_limit "$task_id" "$EXTRA_GRACE_SECONDS")"
    status="$(printf '%s' "$row" | awk -F'\t' '{print $1}')"
    text="$(printf '%s' "$row" | awk -F'\t' '{print $2}')"
    error="$(printf '%s' "$row" | awk -F'\t' '{print $3}')"
  fi

  if ! is_expected_status "$status" "$expected_status_csv"; then
    echo "FAIL: status=${status} expected=${expected_status_csv} error=${error}"
    return 1
  fi

  if [ -n "$expect_substring" ]; then
    local target=""
    case "$expect_field" in
      text) target="$text" ;;
      error) target="$error" ;;
      either) target="${text}"$'\n'"${error}" ;;
      *)
        echo "FAIL: invalid expect_field=${expect_field}"
        return 1
        ;;
    esac
    if ! printf '%s' "$target" | grep -Fq "$expect_substring"; then
      echo "FAIL: expected token missing: ${expect_substring}"
      echo "status=${status}"
      echo "text=${text}"
      echo "error=${error}"
      return 1
    fi
  fi

  if [ -n "$reject_substring" ]; then
    local target=""
    case "$reject_field" in
      text) target="$text" ;;
      error) target="$error" ;;
      either) target="${text}"$'\n'"${error}" ;;
      *)
        echo "FAIL: invalid reject_field=${reject_field}"
        return 1
        ;;
    esac
    if printf '%s' "$target" | grep -Fq "$reject_substring"; then
      echo "FAIL: forbidden token detected: ${reject_substring}"
      echo "status=${status}"
      echo "text=${text}"
      echo "error=${error}"
      return 1
    fi
  fi

  echo "PASS: status=${status}"
  return 0
}

run_case() {
  local case_name="$1"
  local prompt="$2"
  local expect_substring="${3:-}"
  run_case_expect "$case_name" "$prompt" "succeeded" "$expect_substring" "text"
}

run_skill_case_expect() {
  local case_name="$1"
  local skill_name="$2"
  local args_json="$3"
  local expected_status_csv="${4:-succeeded}"
  local expect_substring="${5:-}"
  local expect_field="${6:-text}"

  echo "[CASE] ${case_name}"
  echo "skill=${skill_name} args=${args_json}"
  local submit_resp task_id
  submit_resp="$(submit_run_skill_task "$skill_name" "$args_json")"
  task_id="$(extract_submit_task_id "$submit_resp")"
  echo "task_id: ${task_id}"

  local row status text error
  row="$(wait_task_until_terminal "$task_id")"
  status="$(printf '%s' "$row" | awk -F'\t' '{print $1}')"
  text="$(printf '%s' "$row" | awk -F'\t' '{print $2}')"
  error="$(printf '%s' "$row" | awk -F'\t' '{print $3}')"

  if ! is_expected_status "$status" "$expected_status_csv"; then
    echo "FAIL: status=${status} expected=${expected_status_csv} error=${error}"
    return 1
  fi

  if [ -n "$expect_substring" ]; then
    local target=""
    case "$expect_field" in
      text) target="$text" ;;
      error) target="$error" ;;
      either) target="${text}"$'\n'"${error}" ;;
      *)
        echo "FAIL: invalid expect_field=${expect_field}"
        return 1
        ;;
    esac
    if ! printf '%s' "$target" | grep -Fq "$expect_substring"; then
      echo "FAIL: expected token missing: ${expect_substring}"
      echo "status=${status}"
      echo "text=${text}"
      echo "error=${error}"
      return 1
    fi
  fi

  echo "PASS: status=${status}"
  return 0
}

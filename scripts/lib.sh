#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID="${USER_ID:-1985996990}"
CHAT_ID="${CHAT_ID:-1985996990}"
USER_KEY="${USER_KEY:-${RUSTCLAW_USER_KEY:-}}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-1}"
MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-240}"
EXTRA_GRACE_SECONDS="${EXTRA_GRACE_SECONDS:-180}"

normalized_user_key() {
  printf '%s' "${USER_KEY}" | xargs
}

curl_auth_args() {
  local key
  key="$(normalized_user_key)"
  if [[ -n "$key" ]]; then
    printf '%s\n' "-H" "X-RustClaw-Key: ${key}"
  fi
}

health_check() {
  local -a auth_args=()
  mapfile -t auth_args < <(curl_auth_args)
  curl -sS "${auth_args[@]}" "${BASE_URL}/v1/health" >/dev/null
}

build_submit_body() {
  local prompt="$1"
  python3 - "$USER_ID" "$CHAT_ID" "$prompt" "$(normalized_user_key)" <<'PY'
import json
import sys

user_id = int(sys.argv[1])
chat_id = int(sys.argv[2])
prompt = sys.argv[3]
user_key = (sys.argv[4] or '').strip()
body = {
    "user_id": user_id,
    "chat_id": chat_id,
    "channel": "ui",
    "external_user_id": str(user_id),
    "external_chat_id": str(chat_id),
    "kind": "ask",
    "payload": {
        "text": prompt,
        "agent_mode": True,
    },
}
if user_key:
    body["user_key"] = user_key
print(json.dumps(body, ensure_ascii=False))
PY
}

submit_task() {
  local prompt="$1"
  local body
  local -a auth_args=()
  body="$(build_submit_body "$prompt")"
  mapfile -t auth_args < <(curl_auth_args)
  curl -sS -X POST "${BASE_URL}/v1/tasks" \
    -H "Content-Type: application/json" \
    "${auth_args[@]}" \
    -d "$body"
}

build_submit_body_with_options() {
  local prompt="$1"
  local agent_mode="$2"
  local source="${3:-}"
  python3 - "$USER_ID" "$CHAT_ID" "$prompt" "$agent_mode" "$source" "$(normalized_user_key)" <<'PY'
import json
import sys

user_id = int(sys.argv[1])
chat_id = int(sys.argv[2])
prompt = sys.argv[3]
agent_mode_raw = (sys.argv[4] or "").strip().lower()
source = (sys.argv[5] or "").strip()
user_key = (sys.argv[6] or '').strip()
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
    "channel": "ui",
    "external_user_id": str(user_id),
    "external_chat_id": str(chat_id),
    "kind": "ask",
    "payload": payload,
}
if user_key:
    body["user_key"] = user_key
print(json.dumps(body, ensure_ascii=False))
PY
}

submit_task_with_options() {
  local prompt="$1"
  local agent_mode="$2"
  local source="${3:-}"
  local body
  local -a auth_args=()
  body="$(build_submit_body_with_options "$prompt" "$agent_mode" "$source")"
  mapfile -t auth_args < <(curl_auth_args)
  curl -sS -X POST "${BASE_URL}/v1/tasks" \
    -H "Content-Type: application/json" \
    "${auth_args[@]}" \
    -d "$body"
}

build_submit_run_skill_body() {
  local skill_name="$1"
  local args_json="${2:-}"
  if [ -z "$args_json" ]; then
    args_json="{}"
  fi
  python3 - "$USER_ID" "$CHAT_ID" "$skill_name" "$args_json" "$(normalized_user_key)" <<'PY'
import json
import sys

user_id = int(sys.argv[1])
chat_id = int(sys.argv[2])
skill_name = (sys.argv[3] or "").strip()
args_raw = (sys.argv[4] or "").strip()
user_key = (sys.argv[5] or '').strip()
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
    "channel": "ui",
    "external_user_id": str(user_id),
    "external_chat_id": str(chat_id),
    "kind": "run_skill",
    "payload": {
        "skill_name": skill_name,
        "args": args,
    },
}
if user_key:
    body["user_key"] = user_key
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
  local -a auth_args=()
  body="$(build_submit_run_skill_body "$skill_name" "$args_json")"
  mapfile -t auth_args < <(curl_auth_args)
  curl -sS -X POST "${BASE_URL}/v1/tasks" \
    -H "Content-Type: application/json" \
    "${auth_args[@]}" \
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
  local -a auth_args=()
  mapfile -t auth_args < <(curl_auth_args)
  curl -sS "${auth_args[@]}" "${BASE_URL}/v1/tasks/${task_id}"
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
text = text.replace("\r", " ").replace("\n", "\\n").replace("\t", " ")
error = error.replace("\r", " ").replace("\n", "\\n").replace("\t", " ")
print(f"{status}\t{text}\t{error}")
PY
}

wait_task_until_terminal_with_limit() {
  local task_id="$1"
  local wait_limit_seconds="$2"
  local waited=0

  while (( waited <= wait_limit_seconds )); do
    local raw triplet status
    raw="$(query_task "$task_id")"
    triplet="$(extract_task_triplet "$raw")"
    status="$(printf '%s\n' "$triplet" | awk -F'\t' '{print $1}')"
    case "$status" in
      succeeded|failed|canceled|timeout)
        printf '%s\n' "$raw"
        return 0
        ;;
    esac
    sleep "$POLL_INTERVAL_SECONDS"
    waited=$((waited + POLL_INTERVAL_SECONDS))
  done

  echo "poll timeout for task_id=${task_id}" >&2
  return 1
}

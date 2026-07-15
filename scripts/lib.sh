#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/shell_compat.sh"

BASE_URL="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID="${USER_ID:-1985996990}"
CHAT_ID="${CHAT_ID:-1985996990}"
USER_KEY="${USER_KEY:-${RUSTCLAW_USER_KEY:-}}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-1}"
MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-600}"
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
  array_from_command_lines auth_args curl_auth_args
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
  array_from_command_lines auth_args curl_auth_args
  curl -sS -X POST "${BASE_URL}/v1/tasks" \
    -H "Content-Type: application/json" \
    "${auth_args[@]}" \
    -d "$body"
}

build_submit_body_with_options() {
  local prompt="$1"
  local _legacy_loop_switch="${2:-}"
  local source="${3:-}"
  python3 - "$USER_ID" "$CHAT_ID" "$prompt" "$source" "$(normalized_user_key)" <<'PY'
import json
import sys

user_id = int(sys.argv[1])
chat_id = int(sys.argv[2])
prompt = sys.argv[3]
source = (sys.argv[4] or "").strip()
user_key = (sys.argv[5] or '').strip()
payload = {
    "text": prompt,
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
  local legacy_loop_switch="${2:-}"
  local source="${3:-}"
  local body
  local -a auth_args=()
  body="$(build_submit_body_with_options "$prompt" "$legacy_loop_switch" "$source")"
  array_from_command_lines auth_args curl_auth_args
  curl -sS -X POST "${BASE_URL}/v1/tasks" \
    -H "Content-Type: application/json" \
    "${auth_args[@]}" \
    -d "$body"
}

build_client_like_telegram_submit_body() {
  local prompt="$1"
  local _legacy_loop_switch="${2:-}"
  local source="${3:-}"
  local external_user_id="${4:-$USER_ID}"
  local external_chat_id="${5:-$CHAT_ID}"
  python3 - "$USER_ID" "$CHAT_ID" "$prompt" "$source" "$(normalized_user_key)" "$external_user_id" "$external_chat_id" <<'PY'
import json
import sys

user_id = int(sys.argv[1])
chat_id = int(sys.argv[2])
prompt = sys.argv[3]
source = (sys.argv[4] or "").strip()
user_key = (sys.argv[5] or "").strip()
external_user_id = (sys.argv[6] or str(user_id)).strip()
external_chat_id = (sys.argv[7] or str(chat_id)).strip()
payload = {
    "text": prompt,
}
if source:
    payload["source"] = source
body = {
    "user_id": user_id,
    "chat_id": chat_id,
    "channel": "telegram",
    "external_user_id": external_user_id,
    "external_chat_id": external_chat_id,
    "kind": "ask",
    "payload": payload,
}
if user_key:
    body["user_key"] = user_key
print(json.dumps(body, ensure_ascii=False))
PY
}

submit_client_like_telegram_task() {
  local prompt="$1"
  local legacy_loop_switch="${2:-}"
  local source="${3:-}"
  local external_user_id="${4:-$USER_ID}"
  local external_chat_id="${5:-$CHAT_ID}"
  local body
  local -a auth_args=()
  body="$(build_client_like_telegram_submit_body "$prompt" "$legacy_loop_switch" "$source" "$external_user_id" "$external_chat_id")"
  array_from_command_lines auth_args curl_auth_args
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
  array_from_command_lines auth_args curl_auth_args
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
  array_from_command_lines auth_args curl_auth_args
  curl -sS "${auth_args[@]}" "${BASE_URL}/v1/tasks/${task_id}"
}

query_task_to_file() {
  local task_id="$1"
  local out_file="$2"
  local -a auth_args=()
  array_from_command_lines auth_args curl_auth_args
  curl -sS "${auth_args[@]}" "${BASE_URL}/v1/tasks/${task_id}" > "$out_file"
}

extract_task_triplet() {
  local raw_file
  raw_file="$(mktemp /tmp/rustclaw-task-triplet.XXXXXX)"
  cat > "$raw_file"
  python3 - "$raw_file" <<'PY'
import json
import sys
from pathlib import Path

raw = Path(sys.argv[1]).read_text().strip()
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
  local status=$?
  rm -f "$raw_file"
  return "$status"
}

wait_task_until_terminal_with_limit() {
  local task_id="$1"
  local wait_limit_seconds="$2"
  local waited=0

  while (( waited <= wait_limit_seconds )); do
    local raw triplet status
    raw="$(query_task "$task_id")"
    triplet="$(printf '%s' "$raw" | extract_task_triplet)"
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

print_skill_call_details_from_final_json_file() {
  local final_json_file="$1"
  local task_id="${2:-}"
  local clawd_log_file="${3:-logs/clawd.log}"
  python3 - "$final_json_file" "$task_id" "$clawd_log_file" <<'PY'
import json
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
task_id = (sys.argv[2] or "").strip()
clawd_log_file = (sys.argv[3] or "").strip()

obj = {}
parse_error = None
try:
    if path.exists():
        obj = json.loads(path.read_text(encoding="utf-8"))
except Exception as err:
    parse_error = str(err)

data = obj.get("data") or {}
result = data.get("result_json") or {}
resume = result.get("resume_context") or {}

blobs = []

def push_blob(value):
    if value is None:
        return
    if isinstance(value, str):
        if value.strip():
            blobs.append(value)
        return
    if isinstance(value, list):
        for item in value:
            push_blob(item)
        return
    if isinstance(value, dict):
        fields = []
        for key in ("index", "type", "skill", "tool", "action", "error"):
            if key in value and value[key] not in (None, ""):
                fields.append(f"{key}={value[key]}")
        if fields:
            blobs.append(" ".join(fields))
        return

def compact(text, limit=360):
    merged = " ".join(str(text).split())
    if len(merged) > limit:
        return merged[:limit] + "...(truncated)"
    return merged

def collect_lines(text):
    out = []
    for match in re.finditer(r"\[TOOL_CALL\](.*?)\[/TOOL_CALL\]", text, flags=re.S | re.I):
        block = compact(match.group(1))
        out.append(f"[TOOL_CALL] {block} [/TOOL_CALL]")

    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        low = line.lower()
        if (
            "skill(" in line
            or "skill:" in low
            or "tool =>" in low
            or "tool=" in low
            or ("i18n:" in low and "skill" in low)
        ):
            out.append(line)
    return out

def parse_clawd_log_calls(task_id_text, explicit_log_file):
    out = []
    if not task_id_text:
        return out
    candidates = []
    if explicit_log_file:
        candidates.append(Path(explicit_log_file))
    candidates.append(Path("logs/clawd.log"))
    seen_files = set()
    for log_path in candidates:
        key = str(log_path.resolve()) if log_path.exists() else str(log_path)
        if key in seen_files:
            continue
        seen_files.add(key)
        if not log_path.exists():
            continue
        try:
            with log_path.open("r", encoding="utf-8", errors="ignore") as fh:
                for raw in fh:
                    line = raw.strip()
                    if not line or task_id_text not in line:
                        continue
                    if "executor_step_execute" in line and "type=call_" in line:
                        m = re.search(
                            r"type=(call_skill|call_tool)\s+(?:skill|tool)=([^\s]+)\s+args=(\{.*?\})\s+call_id=",
                            line,
                        )
                        if m:
                            out.append(
                                compact(f"[LOG_EXEC] type={m.group(1)} name={m.group(2)} args={m.group(3)}")
                            )
                            continue
                    if "plan_llm_response" in line and "raw=" in line and "\"call_" in line:
                        raw_json = line.split("raw=", 1)[1].split(" call_id=", 1)[0]
                        out.append(compact(f"[LOG_PLAN] {raw_json}"))
                        continue
                    if "[LLM_CALL]" in line and "stage=response" in line and "response=" in line and "\"call_" in line:
                        raw_json = line.split("response=", 1)[1].split(" call_id=", 1)[0]
                        out.append(compact(f"[LOG_LLM] {raw_json}"))
        except Exception:
            continue
    return out

push_blob(result.get("progress_messages"))
push_blob(result.get("messages"))
push_blob(result.get("text"))
push_blob(result.get("tool_calls"))
push_blob(result.get("skill_calls"))
push_blob(resume.get("completed_messages"))
push_blob(resume.get("completed_steps"))
push_blob(resume.get("failed_step"))
push_blob(resume.get("plan_steps"))
push_blob(resume.get("remaining_steps"))
push_blob(resume.get("remaining_actions"))

calls = []
seen = set()
for blob in blobs:
    for line in collect_lines(str(blob)):
        normalized = compact(line)
        key = normalized.lower()
        if key in seen:
            continue
        seen.add(key)
        calls.append(normalized)

if not calls:
    for item in parse_clawd_log_calls(task_id, clawd_log_file):
        key = item.lower()
        if key in seen:
            continue
        seen.add(key)
        calls.append(item)

print(f"  [SKILL_CALLS] count={len(calls)}")
for idx, item in enumerate(calls, start=1):
    print(f"  [SKILL_CALL {idx}] {item}")
if parse_error and not calls:
    print(f"  [SKILL_CALLS_NOTE] parse_error={compact(parse_error, 200)}")
PY
}

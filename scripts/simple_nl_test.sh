#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/lib.sh"

DEFAULT_LOG_ROOT="${SCRIPT_DIR}/nl_test_logs"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-1}"
MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-180}"

PROMPTS=()
LOG_ROOT="$DEFAULT_LOG_ROOT"
CASE_FILE=""
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
RUN_DIR=""
RUN_LOG=""
SUMMARY_JSONL=""

usage() {
  cat <<'EOF'
Usage:
  bash scripts/simple_nl_test.sh [options]

Options:
  --prompt TEXT         Add one natural-language test prompt (can repeat)
  --case-file PATH      Read prompts from file; one prompt per line, lines starting with # are ignored
  --base-url URL        clawd base url (default: http://127.0.0.1:8787)
  --user-id ID          user id for submitted tasks
  --chat-id ID          base chat id; script auto-increments per case
  --user-key KEY        RustClaw user key; if omitted, auto-detect first enabled admin key
  --wait-seconds N      max wait per prompt (default: 180)
  --poll-seconds N      poll interval seconds (default: 1)
  --log-root PATH       log root dir (default: scripts/nl_test_logs)
  -h, --help            show this help

Examples:
  bash scripts/simple_nl_test.sh --prompt "只输出当前工作目录的绝对路径，不要解释"
  bash scripts/simple_nl_test.sh --case-file scripts/nl_manual_cases.txt

Outputs:
  scripts/nl_test_logs/<timestamp>/
    run.log
    summary.jsonl
    case_XX/
      submit.json
      final.json
      meta.txt
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1" >&2
    exit 2
  }
}

resolve_admin_key() {
  if [[ -n "${USER_KEY:-}" ]]; then
    return 0
  fi
  USER_KEY="$("${SCRIPT_DIR}/auth-key.sh" list | awk '$2 == "admin" && $3 == "enabled" { print $1; exit }')"
  if [[ -z "${USER_KEY:-}" ]]; then
    echo "No enabled admin key found. Pass --user-key explicitly." >&2
    exit 2
  fi
}

load_case_file() {
  local path="$1"
  python3 - "$path" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
for raw in path.read_text(encoding="utf-8").splitlines():
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    if "|" in line:
        parts = [part.strip() for part in line.split("|", 3)]
        prompt = parts[-1]
    else:
        prompt = line
    if prompt:
        print(prompt)
PY
}

extract_submit_task_id() {
  python3 - "${1:-}" <<'PY'
import json
import sys

raw = (sys.argv[1] if len(sys.argv) > 1 else "").strip()
obj = json.loads(raw)
if not obj.get("ok"):
    raise SystemExit(f"submit failed: {obj.get('error')}")
task_id = ((obj.get("data") or {}).get("task_id") or "").strip()
if not task_id:
    raise SystemExit("submit response missing task_id")
print(task_id)
PY
}

extract_status() {
  python3 - "$1" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
print(str(data.get("status") or ""))
PY
}

extract_result_summary() {
  python3 - "$1" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
result = data.get("result_json") or {}
text = str(result.get("text") or "").replace("\n", "\\n")
error = str(data.get("error_text") or "").replace("\n", "\\n")
print(f"status={data.get('status') or ''}")
if text:
    print(f"text={text}")
if error:
    print(f"error={error}")
PY
}

poll_until_terminal() {
  local task_id="$1"
  local out_file="$2"
  local waited=0
  local last_status=""

  while [[ "$waited" -le "$MAX_WAIT_SECONDS" ]]; do
    local raw status
    raw="$(query_task "$task_id")"
    printf '%s\n' "$raw" > "$out_file"
    status="$(extract_status "$out_file")"
    if [[ "$status" != "$last_status" ]]; then
      echo "  [status] ${last_status:-<none>} -> ${status:-<empty>}"
      last_status="$status"
    fi
    case "$status" in
      succeeded|failed|canceled|timeout)
        return 0
        ;;
    esac
    sleep "$POLL_INTERVAL_SECONDS"
    waited=$((waited + POLL_INTERVAL_SECONDS))
  done

  echo "poll timeout for task_id=${task_id}" >&2
  return 1
}

append_summary_jsonl() {
  local index="$1"
  local prompt="$2"
  local task_id="$3"
  local final_json="$4"
  python3 - "$index" "$prompt" "$task_id" "$final_json" >> "$SUMMARY_JSONL" <<'PY'
import json
import sys
from pathlib import Path

index = int(sys.argv[1])
prompt = sys.argv[2]
task_id = sys.argv[3]
obj = json.loads(Path(sys.argv[4]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
result = data.get("result_json") or {}
row = {
    "index": index,
    "prompt": prompt,
    "task_id": task_id,
    "status": data.get("status"),
    "text": result.get("text"),
    "messages": result.get("messages"),
    "error_text": data.get("error_text"),
}
print(json.dumps(row, ensure_ascii=False))
PY
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prompt)
      PROMPTS+=("${2:-}")
      shift 2
      ;;
    --case-file)
      CASE_FILE="${2:-}"
      shift 2
      ;;
    --base-url)
      BASE_URL="${2:-}"
      shift 2
      ;;
    --user-id)
      USER_ID="${2:-}"
      shift 2
      ;;
    --chat-id)
      CHAT_ID="${2:-}"
      shift 2
      ;;
    --user-key)
      USER_KEY="${2:-}"
      shift 2
      ;;
    --wait-seconds)
      MAX_WAIT_SECONDS="${2:-}"
      shift 2
      ;;
    --poll-seconds)
      POLL_INTERVAL_SECONDS="${2:-}"
      shift 2
      ;;
    --log-root)
      LOG_ROOT="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

need_cmd curl
need_cmd python3
need_cmd awk

if [[ -n "$CASE_FILE" ]]; then
  while IFS= read -r prompt; do
    [[ -n "$prompt" ]] || continue
    PROMPTS+=("$prompt")
  done < <(load_case_file "$CASE_FILE")
fi

if [[ "${#PROMPTS[@]}" -eq 0 ]]; then
  echo "No prompts provided. Use --prompt or --case-file." >&2
  exit 2
fi

resolve_admin_key

RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
RUN_LOG="${RUN_DIR}/run.log"
SUMMARY_JSONL="${RUN_DIR}/summary.jsonl"
mkdir -p "$RUN_DIR"
touch "$SUMMARY_JSONL"
exec > >(tee -a "$RUN_LOG") 2>&1

echo "Simple NL test run"
echo "  run_dir:  $RUN_DIR"
echo "  base_url: ${BASE_URL}"
echo "  user_id:  ${USER_ID}"
echo "  chat_id:  ${CHAT_ID}"
echo "  prompts:  ${#PROMPTS[@]}"

health_check

BASE_CHAT_ID="$CHAT_ID"
index=0
for prompt in "${PROMPTS[@]}"; do
  index=$((index + 1))
  CHAT_ID=$((BASE_CHAT_ID + index))
  case_dir="$(printf '%s/case_%02d' "$RUN_DIR" "$index")"
  mkdir -p "$case_dir"
  submit_file="${case_dir}/submit.json"
  final_file="${case_dir}/final.json"
  meta_file="${case_dir}/meta.txt"

  echo
  echo "============================================================"
  echo "[CASE] $index"
  echo "[CHAT] $CHAT_ID"
  echo "[PROMPT] $prompt"

  submit_raw="$(submit_task "$prompt")"
  printf '%s\n' "$submit_raw" > "$submit_file"
  task_id="$(extract_submit_task_id "$submit_raw")"
  echo "[TASK] $task_id"
  printf 'index=%s\nchat_id=%s\ntask_id=%s\nprompt=%s\n' \
    "$index" "$CHAT_ID" "$task_id" "$prompt" > "$meta_file"

  poll_until_terminal "$task_id" "$final_file"
  echo "[FINAL]"
  extract_result_summary "$final_file"
  append_summary_jsonl "$index" "$prompt" "$task_id" "$final_file"
done

echo
echo "Artifacts:"
echo "  - $RUN_LOG"
echo "  - $SUMMARY_JSONL"

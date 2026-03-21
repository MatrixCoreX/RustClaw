#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/lib.sh"

DEFAULT_CASE_FILE="${SCRIPT_DIR}/nl_cases_clarify.txt"
DEFAULT_LOG_ROOT="${SCRIPT_DIR}/nl_suite_logs/clarify"

CASE_FILE="$DEFAULT_CASE_FILE"
LOG_ROOT="$DEFAULT_LOG_ROOT"
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
RUN_DIR=""
RUN_LOG=""
SUMMARY_JSONL=""

usage() {
  cat <<'EOF'
Usage:
  bash scripts/run_nl_suite_clarify.sh [options]

Options:
  --case-file PATH      clarify case file (default: scripts/nl_cases_clarify.txt)
  --base-url URL        clawd base url (default: http://127.0.0.1:8787)
  --user-id ID          user id for submitted tasks
  --chat-id ID          base chat id; script auto-increments per case
  --user-key KEY        RustClaw user key; if omitted, auto-detect first enabled admin key
  --wait-seconds N      max wait per turn (default: 240)
  --poll-seconds N      poll interval seconds (default: 1)
  --log-root PATH       log root dir (default: scripts/nl_suite_logs/clarify)
  -h, --help            show this help

Case file format:
  case_name|turn1|turn2
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

extract_result_text() {
  python3 - "$1" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
result = data.get("result_json") or {}
print(str(result.get("text") or "").replace("\n", "\\n"))
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
  local case_name="$2"
  local chat_id="$3"
  local turn1_prompt="$4"
  local turn1_task_id="$5"
  local turn1_final="$6"
  local turn2_prompt="$7"
  local turn2_task_id="$8"
  local turn2_final="$9"
  python3 - "$index" "$case_name" "$chat_id" "$turn1_prompt" "$turn1_task_id" "$turn1_final" "$turn2_prompt" "$turn2_task_id" "$turn2_final" >> "$SUMMARY_JSONL" <<'PY'
import json
import sys
from pathlib import Path

index = int(sys.argv[1])
case_name = sys.argv[2]
chat_id = int(sys.argv[3])
turn1_prompt = sys.argv[4]
turn1_task_id = sys.argv[5]
turn1_obj = json.loads(Path(sys.argv[6]).read_text(encoding="utf-8"))
turn2_prompt = sys.argv[7]
turn2_task_id = sys.argv[8]
turn2_obj = json.loads(Path(sys.argv[9]).read_text(encoding="utf-8"))

def shape(obj):
    data = obj.get("data") or {}
    result = data.get("result_json") or {}
    return {
        "status": data.get("status"),
        "text": result.get("text"),
        "messages": result.get("messages"),
        "error_text": data.get("error_text"),
    }

row = {
    "index": index,
    "case_name": case_name,
    "chat_id": chat_id,
    "turn1": {
        "prompt": turn1_prompt,
        "task_id": turn1_task_id,
        **shape(turn1_obj),
    },
    "turn2": {
        "prompt": turn2_prompt,
        "task_id": turn2_task_id,
        **shape(turn2_obj),
    },
}
print(json.dumps(row, ensure_ascii=False))
PY
}

load_cases() {
  python3 - "$1" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
for raw in path.read_text(encoding="utf-8").splitlines():
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = [part.strip() for part in line.split("|", 2)]
    if len(parts) != 3 or not all(parts):
        raise SystemExit(f"invalid clarify case line: {raw}")
    print("\t".join(parts))
PY
}

while [[ $# -gt 0 ]]; do
  case "$1" in
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

resolve_admin_key

RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
RUN_LOG="${RUN_DIR}/run.log"
SUMMARY_JSONL="${RUN_DIR}/summary.jsonl"
mkdir -p "$RUN_DIR"
touch "$SUMMARY_JSONL"
exec > >(tee -a "$RUN_LOG") 2>&1

echo "NL suite: clarify"
echo "  run_dir:   $RUN_DIR"
echo "  run_log:   $RUN_LOG"
echo "  case_file: $CASE_FILE"
echo "  base_url:  ${BASE_URL}"
echo "  user_id:   ${USER_ID}"
echo "  chat_id:   ${CHAT_ID}"
echo

health_check

BASE_CHAT_ID="$CHAT_ID"
index=0
while IFS=$'\t' read -r case_name turn1_prompt turn2_prompt; do
  [[ -n "${case_name:-}" ]] || continue
  index=$((index + 1))
  CHAT_ID=$((BASE_CHAT_ID + index))
  case_dir="$(printf '%s/case_%02d_%s' "$RUN_DIR" "$index" "$case_name")"
  mkdir -p "$case_dir"

  echo "============================================================"
  echo "[CASE]   $index"
  echo "[NAME]   $case_name"
  echo "[CHAT]   $CHAT_ID"
  echo "[TURN1]  $turn1_prompt"

  turn1_submit="${case_dir}/turn1_submit.json"
  turn1_final="${case_dir}/turn1_final.json"
  turn2_submit="${case_dir}/turn2_submit.json"
  turn2_final="${case_dir}/turn2_final.json"
  meta_file="${case_dir}/meta.txt"

  turn1_raw="$(submit_task "$turn1_prompt")"
  printf '%s\n' "$turn1_raw" > "$turn1_submit"
  turn1_task_id="$(extract_submit_task_id "$turn1_raw")"
  echo "[TASK1]  $turn1_task_id"
  poll_until_terminal "$turn1_task_id" "$turn1_final"
  echo "[TEXT1]  $(extract_result_text "$turn1_final")"

  echo "[TURN2]  $turn2_prompt"
  turn2_raw="$(submit_task "$turn2_prompt")"
  printf '%s\n' "$turn2_raw" > "$turn2_submit"
  turn2_task_id="$(extract_submit_task_id "$turn2_raw")"
  echo "[TASK2]  $turn2_task_id"
  poll_until_terminal "$turn2_task_id" "$turn2_final"
  echo "[TEXT2]  $(extract_result_text "$turn2_final")"

  printf 'index=%s\ncase_name=%s\nchat_id=%s\nturn1_task_id=%s\nturn1_prompt=%s\nturn2_task_id=%s\nturn2_prompt=%s\n' \
    "$index" "$case_name" "$CHAT_ID" "$turn1_task_id" "$turn1_prompt" "$turn2_task_id" "$turn2_prompt" > "$meta_file"

  append_summary_jsonl \
    "$index" "$case_name" "$CHAT_ID" \
    "$turn1_prompt" "$turn1_task_id" "$turn1_final" \
    "$turn2_prompt" "$turn2_task_id" "$turn2_final"

  echo
done < <(load_cases "$CASE_FILE")

echo "Artifacts:"
echo "  - $RUN_DIR"
echo "  - $RUN_LOG"
echo "  - $SUMMARY_JSONL"

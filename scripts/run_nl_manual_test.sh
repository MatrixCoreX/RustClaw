#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/lib.sh"

DEFAULT_CASE_FILE="${SCRIPT_DIR}/nl_manual_cases.txt"
DEFAULT_LOG_ROOT="${ROOT_DIR}/logs/run_nl_manual_test"

CASE_FILE="$DEFAULT_CASE_FILE"
LOG_ROOT="$DEFAULT_LOG_ROOT"
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
RUN_DIR=""
RUN_LOG=""
SUMMARY_JSONL=""

BASE_URL_VALUE="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID_VALUE="${USER_ID:-1985996990}"
CHAT_ID_VALUE="${CHAT_ID:-1985996990}"
USER_KEY_VALUE="${RUSTCLAW_USER_KEY:-${USER_KEY:-}}"
WAIT_SECONDS_VALUE="${MAX_WAIT_SECONDS:-180}"
POLL_SECONDS_VALUE="${POLL_INTERVAL_SECONDS:-1}"
FULL_TEXT=0

RESUME_DIR=""
RESUME_LINE=0
CURRENT_SOURCE_LINE=0
LAST_COMPLETED_LINE=0

usage() {
  cat <<'EOF'
Usage:
  bash scripts/run_nl_manual_test.sh [options]

Options:
  --case-file PATH      Case file to run. Default: scripts/nl_manual_cases.txt
  --log-root PATH       Root log dir. Default: logs/run_nl_manual_test
  --resume-dir PATH     Existing run dir to append logs/results into
  --resume-line N       Continue after this tested source line number
  --base-url URL        clawd base url. Default: http://127.0.0.1:8787
  --user-id ID          User id for submit
  --chat-id ID          Base chat id for submit
  --user-key KEY        RustClaw user key
  --wait-seconds N      Max wait seconds per case
  --poll-seconds N      Poll interval seconds
  --full-text           Print full response text
  -h, --help            Show this help

Case format:
  suite|name|tags|prompt
  prompt-only lines are also allowed

Resume behavior:
  If you already tested through line 12, rerun with:
    bash scripts/run_nl_manual_test.sh --resume-dir <run_dir> --resume-line 12

Artifacts:
  logs/run_nl_manual_test/<timestamp>/
    run.log
    summary.jsonl
    case_*/
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
  if [[ -n "${USER_KEY_VALUE:-}" ]]; then
    return 0
  fi
  USER_KEY_VALUE="$("${SCRIPT_DIR}/auth-key.sh" list | awk '$2 == "admin" && $3 == "enabled" { print $1; exit }')"
  if [[ -z "${USER_KEY_VALUE:-}" ]]; then
    echo "No enabled admin key found. Pass --user-key explicitly." >&2
    exit 2
  fi
}

load_case_rows() {
  local case_file="$1"
  python3 - "$case_file" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
for idx, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    prompt = line
    case_name = f"line_{idx:03d}"
    if "|" in line:
        parts = [part.strip() for part in line.split("|", 3)]
        if len(parts) == 4:
            _, name, _, prompt = parts
            if name:
                case_name = name
        else:
            prompt = parts[-1]
    prompt = prompt.strip()
    if not prompt:
        continue
    print(f"{idx}\x1f{case_name}\x1f{prompt}")
PY
}

sanitize_name() {
  local raw="${1:-}"
  python3 - "$raw" <<'PY'
import re
import sys

name = (sys.argv[1] or "").strip().lower()
name = re.sub(r"[^a-z0-9._-]+", "_", name)
name = re.sub(r"_+", "_", name).strip("._-")
print(name or "case")
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
  python3 - "$1" "$2" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
mode = sys.argv[2]
data = obj.get("data") or {}
result = data.get("result_json") or {}
status = data.get("status") or ""
text = str(result.get("text") or "")
error_text = str(data.get("error_text") or "")

print(f"  [final] status={status}")
if error_text:
    print(f"  [error] {error_text.replace(chr(10), '\\n')}")
if text:
    if mode == "full":
        print("  [text]")
        print(text)
    else:
        compact = " ".join(text.split())
        if len(compact) > 220:
            compact = compact[:220] + "...(truncated)"
        print(f"  [text] {compact}")
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
  local source_line="$1"
  local case_name="$2"
  local prompt="$3"
  local task_id="$4"
  local final_json="$5"
  python3 - "$source_line" "$case_name" "$prompt" "$task_id" "$final_json" >> "$SUMMARY_JSONL" <<'PY'
import json
import sys
from pathlib import Path

source_line = int(sys.argv[1])
case_name = sys.argv[2]
prompt = sys.argv[3]
task_id = sys.argv[4]
obj = json.loads(Path(sys.argv[5]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
result = data.get("result_json") or {}
row = {
    "source_line": source_line,
    "case_name": case_name,
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

prepare_run_dir() {
  if [[ -n "$RESUME_DIR" ]]; then
    RUN_DIR="$RESUME_DIR"
  else
    RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
  fi

  RUN_LOG="${RUN_DIR}/run.log"
  SUMMARY_JSONL="${RUN_DIR}/summary.jsonl"

  mkdir -p "$RUN_DIR"
  touch "$SUMMARY_JSONL"
  exec > >(tee -a "$RUN_LOG") 2>&1
}

handle_interrupt() {
  local code=130
  echo
  echo "[INTERRUPTED]"
  echo "  run_dir:              ${RUN_DIR:-<not-created>}"
  echo "  current_source_line:  ${CURRENT_SOURCE_LINE:-0}"
  echo "  last_completed_line:  ${LAST_COMPLETED_LINE:-0}"
  echo "Resume by reusing the same args and adding:"
  echo "  --resume-dir ${RUN_DIR:-<run_dir>} --resume-line ${LAST_COMPLETED_LINE:-0}"
  exit "$code"
}

run_one_case() {
  local ordinal="$1"
  local source_line="$2"
  local case_name="$3"
  local prompt="$4"
  local chat_id="$5"
  local safe_name case_dir submit_file final_file meta_file task_id raw

  CURRENT_SOURCE_LINE="$source_line"
  safe_name="$(sanitize_name "$case_name")"
  case_dir="$(printf '%s/case_%03d_line_%03d_%s' "$RUN_DIR" "$ordinal" "$source_line" "$safe_name")"
  submit_file="${case_dir}/submit.json"
  final_file="${case_dir}/final.json"
  meta_file="${case_dir}/meta.txt"
  mkdir -p "$case_dir"

  CHAT_ID="$chat_id"

  echo
  echo "============================================================"
  echo "[CASE]        $ordinal"
  echo "[SOURCE_LINE] $source_line"
  echo "[NAME]        $case_name"
  echo "[CHAT]        $CHAT_ID"
  echo "[PROMPT]      $prompt"

  raw="$(submit_task "$prompt")"
  printf '%s\n' "$raw" > "$submit_file"
  task_id="$(extract_submit_task_id "$raw")"
  echo "[TASK]        $task_id"
  poll_until_terminal "$task_id" "$final_file"

  echo "[RESULT]"
  if [[ "$FULL_TEXT" -eq 1 ]]; then
    extract_result_summary "$final_file" "full"
  else
    extract_result_summary "$final_file" "summary"
  fi

  printf 'ordinal=%s\nsource_line=%s\ncase_name=%s\nchat_id=%s\ntask_id=%s\nprompt=%s\n' \
    "$ordinal" "$source_line" "$case_name" "$CHAT_ID" "$task_id" "$prompt" > "$meta_file"

  append_summary_jsonl "$source_line" "$case_name" "$prompt" "$task_id" "$final_file"
  LAST_COMPLETED_LINE="$source_line"
  CURRENT_SOURCE_LINE=0
}

print_final_summary() {
  python3 - "$SUMMARY_JSONL" <<'PY'
import json
import sys
from collections import Counter
from pathlib import Path

path = Path(sys.argv[1])
rows = []
for raw in path.read_text(encoding="utf-8").splitlines():
    raw = raw.strip()
    if not raw:
        continue
    try:
        rows.append(json.loads(raw))
    except Exception:
        continue

counter = Counter(str(row.get("status") or "unknown") for row in rows)
print()
print("================ Final Summary ================")
print(
    "TOTAL_CASES={} SUCCEEDED={} FAILED={} CANCELED={} TIMEOUT={}".format(
        len(rows),
        counter.get("succeeded", 0),
        counter.get("failed", 0),
        counter.get("canceled", 0),
        counter.get("timeout", 0),
    )
)
PY
}

trap handle_interrupt INT TERM

while [[ $# -gt 0 ]]; do
  case "$1" in
    --case-file)
      CASE_FILE="$2"
      shift 2
      ;;
    --log-root)
      LOG_ROOT="$2"
      shift 2
      ;;
    --resume-dir)
      RESUME_DIR="$2"
      shift 2
      ;;
    --resume-line)
      RESUME_LINE="$2"
      shift 2
      ;;
    --base-url)
      BASE_URL_VALUE="$2"
      shift 2
      ;;
    --user-id)
      USER_ID_VALUE="$2"
      shift 2
      ;;
    --chat-id)
      CHAT_ID_VALUE="$2"
      shift 2
      ;;
    --user-key)
      USER_KEY_VALUE="$2"
      shift 2
      ;;
    --wait-seconds)
      WAIT_SECONDS_VALUE="$2"
      shift 2
      ;;
    --poll-seconds)
      POLL_SECONDS_VALUE="$2"
      shift 2
      ;;
    --full-text)
      FULL_TEXT=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

need_cmd curl
need_cmd python3
need_cmd awk
need_cmd tee

if [[ ! -f "$CASE_FILE" ]]; then
  echo "Case file not found: $CASE_FILE" >&2
  exit 2
fi

if ! [[ "$RESUME_LINE" =~ ^[0-9]+$ ]]; then
  echo "--resume-line must be a non-negative integer" >&2
  exit 2
fi

if [[ -z "$RESUME_DIR" && "$RESUME_LINE" != "0" ]]; then
  echo "--resume-line requires --resume-dir" >&2
  exit 2
fi

if [[ -n "$RESUME_DIR" && "$RESUME_LINE" == "0" ]]; then
  if [[ -t 0 ]]; then
    printf 'Tested through source line number: ' >&2
    read -r RESUME_LINE
  else
    echo "--resume-dir requires --resume-line in non-interactive mode" >&2
    exit 2
  fi
  if ! [[ "$RESUME_LINE" =~ ^[0-9]+$ ]]; then
    echo "--resume-line must be a non-negative integer" >&2
    exit 2
  fi
fi

BASE_URL="$BASE_URL_VALUE"
USER_ID="$USER_ID_VALUE"
CHAT_ID="$CHAT_ID_VALUE"
USER_KEY="$USER_KEY_VALUE"
MAX_WAIT_SECONDS="$WAIT_SECONDS_VALUE"
POLL_INTERVAL_SECONDS="$POLL_SECONDS_VALUE"

resolve_admin_key
USER_KEY="$USER_KEY_VALUE"

prepare_run_dir

echo "Natural-language manual regression"
echo "  case_file:     $CASE_FILE"
echo "  run_dir:       $RUN_DIR"
echo "  run_log:       $RUN_LOG"
echo "  summary_jsonl: $SUMMARY_JSONL"
echo "  base_url:      $BASE_URL"
echo "  user_id:       $USER_ID"
echo "  chat_id:       $CHAT_ID"
echo "  user_key:      ${USER_KEY:+<set>}"
echo "  wait:          ${MAX_WAIT_SECONDS}s"
echo "  poll:          ${POLL_INTERVAL_SECONDS}s"
if [[ -n "$RESUME_DIR" ]]; then
  echo "  resume_dir:    $RESUME_DIR"
  echo "  resume_line:   $RESUME_LINE"
fi
echo

health_check

mapfile -t CASE_ROWS < <(load_case_rows "$CASE_FILE")
if [[ "${#CASE_ROWS[@]}" -eq 0 ]]; then
  echo "No runnable cases found in $CASE_FILE" >&2
  exit 2
fi

BASE_CHAT_ID="$CHAT_ID"
ordinal=0
run_count=0
for row in "${CASE_ROWS[@]}"; do
  IFS=$'\x1f' read -r source_line case_name prompt <<< "$row"
  ordinal=$((ordinal + 1))
  chat_id_for_case=$((BASE_CHAT_ID + ordinal))

  if (( source_line <= RESUME_LINE )); then
    LAST_COMPLETED_LINE="$source_line"
    echo "[SKIP] source_line=${source_line} name=${case_name} already covered by --resume-line ${RESUME_LINE}"
    continue
  fi

  run_count=$((run_count + 1))
  run_one_case "$ordinal" "$source_line" "$case_name" "$prompt" "$chat_id_for_case"
done

if (( run_count == 0 )); then
  echo "No remaining cases after --resume-line ${RESUME_LINE}."
fi

print_final_summary

echo
echo "Artifacts:"
echo "  - $RUN_DIR"
echo "  - $RUN_LOG"
echo "  - $SUMMARY_JSONL"

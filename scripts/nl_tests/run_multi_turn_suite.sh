#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/lib.sh"

SUITE="clarify"
CASE_FILE=""
LOG_ROOT=""
TURN_COUNT=0

RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
RUN_DIR=""
RUN_LOG=""
SUMMARY_JSONL=""

BASE_URL_VALUE="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID_VALUE="${USER_ID:-1985996990}"
CHAT_ID_VALUE="${CHAT_ID:-1985996990}"
USER_KEY_VALUE="${RUSTCLAW_USER_KEY:-${USER_KEY:-}}"
WAIT_SECONDS_VALUE="${MAX_WAIT_SECONDS:-240}"
POLL_SECONDS_VALUE="${POLL_INTERVAL_SECONDS:-1}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_multi_turn_suite.sh [options]
  Preferred unified entry:
    bash scripts/nl_tests/run_suite.sh clarify [options]
    bash scripts/nl_tests/run_suite.sh context_chain [options]

Options:
  --suite NAME          clarify | context_chain (default: clarify)
  --case-file PATH      case file path
  --log-root PATH       root log dir
  --base-url URL        clawd base url (default: http://127.0.0.1:8787)
  --user-id ID          user id for submitted tasks
  --chat-id ID          base chat id; auto-increments per case
  --user-key KEY        RustClaw user key; omitted => auto-detect first enabled admin key
  --wait-seconds N      max wait per turn (default: 240)
  --poll-seconds N      poll interval seconds (default: 1)
  -h, --help            show this help

Built-in suites:
  clarify:       case format case_name|turn1|turn2
  context_chain: case format case_name|turn1|turn2|turn3
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
  USER_KEY_VALUE="$("${ROOT_DIR}/scripts/auth-key.sh" list | awk '$2 == "admin" && $3 == "enabled" { print $1; exit }')"
  if [[ -z "${USER_KEY_VALUE:-}" ]]; then
    echo "No enabled admin key found. Pass --user-key explicitly." >&2
    exit 2
  fi
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
    local status
    query_task_to_file "$task_id" "$out_file"
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
  python3 - "$@" >> "$SUMMARY_JSONL" <<'PY'
import json
import sys
from pathlib import Path

index = int(sys.argv[1])
case_name = sys.argv[2]
chat_id = int(sys.argv[3])
turn_count = int(sys.argv[4])

expected = 5 + turn_count * 3
if len(sys.argv) != expected:
    raise SystemExit(f"invalid summary args: got={len(sys.argv)} expected={expected}")

row = {
    "index": index,
    "case_name": case_name,
    "chat_id": chat_id,
}

for i in range(turn_count):
    base = 5 + i * 3
    prompt = sys.argv[base]
    task_id = sys.argv[base + 1]
    final_path = Path(sys.argv[base + 2])
    obj = json.loads(final_path.read_text(encoding="utf-8"))
    data = obj.get("data") or {}
    result = data.get("result_json") or {}
    row[f"turn{i + 1}"] = {
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

load_cases() {
  local case_file="$1"
  local turn_count="$2"
  python3 - "$case_file" "$turn_count" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
turn_count = int(sys.argv[2])
expected_parts = turn_count + 1

for raw in path.read_text(encoding="utf-8").splitlines():
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = [part.strip() for part in line.split("|", turn_count)]
    if len(parts) != expected_parts or not all(parts):
        raise SystemExit(f"invalid case line: {raw}")
    print("\t".join(parts))
PY
}

select_suite_defaults() {
  case "$SUITE" in
    clarify)
      TURN_COUNT=2
      [[ -n "$CASE_FILE" ]] || CASE_FILE="${SCRIPT_DIR}/cases/nl_cases_clarify.txt"
      [[ -n "$LOG_ROOT" ]] || LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/clarify"
      ;;
    context_chain)
      TURN_COUNT=3
      [[ -n "$CASE_FILE" ]] || CASE_FILE="${SCRIPT_DIR}/cases/nl_cases_context_chain.txt"
      [[ -n "$LOG_ROOT" ]] || LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/context_chain"
      ;;
    *)
      echo "Unsupported --suite: $SUITE (expected: clarify|context_chain)" >&2
      exit 2
      ;;
  esac
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --suite)
      SUITE="${2:-}"
      shift 2
      ;;
    --case-file)
      CASE_FILE="${2:-}"
      shift 2
      ;;
    --log-root)
      LOG_ROOT="${2:-}"
      shift 2
      ;;
    --base-url)
      BASE_URL_VALUE="${2:-}"
      shift 2
      ;;
    --user-id)
      USER_ID_VALUE="${2:-}"
      shift 2
      ;;
    --chat-id)
      CHAT_ID_VALUE="${2:-}"
      shift 2
      ;;
    --user-key)
      USER_KEY_VALUE="${2:-}"
      shift 2
      ;;
    --wait-seconds)
      WAIT_SECONDS_VALUE="${2:-}"
      shift 2
      ;;
    --poll-seconds)
      POLL_SECONDS_VALUE="${2:-}"
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

select_suite_defaults

need_cmd curl
need_cmd python3
need_cmd awk

if [[ ! -f "$CASE_FILE" ]]; then
  echo "Case file not found: $CASE_FILE" >&2
  exit 2
fi

BASE_URL="$BASE_URL_VALUE"
USER_ID="$USER_ID_VALUE"
CHAT_ID="$CHAT_ID_VALUE"
USER_KEY="$USER_KEY_VALUE"
MAX_WAIT_SECONDS="$WAIT_SECONDS_VALUE"
POLL_INTERVAL_SECONDS="$POLL_SECONDS_VALUE"

resolve_admin_key
USER_KEY="$USER_KEY_VALUE"

RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
RUN_LOG="${RUN_DIR}/run.log"
SUMMARY_JSONL="${RUN_DIR}/summary.jsonl"
mkdir -p "$RUN_DIR"
touch "$SUMMARY_JSONL"
exec > >(tee -a "$RUN_LOG") 2>&1

echo "NL multi-turn suite: ${SUITE}"
echo "  run_dir:    ${RUN_DIR}"
echo "  run_log:    ${RUN_LOG}"
echo "  case_file:  ${CASE_FILE}"
echo "  turn_count: ${TURN_COUNT}"
echo "  base_url:   ${BASE_URL}"
echo "  user_id:    ${USER_ID}"
echo "  chat_id:    ${CHAT_ID}"
echo

health_check

BASE_CHAT_ID="$CHAT_ID"
index=0
while IFS=$'\t' read -r -a parts; do
  [[ "${#parts[@]}" -eq $((TURN_COUNT + 1)) ]] || continue
  case_name="${parts[0]}"
  index=$((index + 1))
  CHAT_ID=$((BASE_CHAT_ID + index))
  safe_case_name="$(sanitize_name "$case_name")"
  case_dir="$(printf '%s/case_%02d_%s' "$RUN_DIR" "$index" "$safe_case_name")"
  mkdir -p "$case_dir"

  declare -a prompts=()
  declare -a task_ids=()
  declare -a finals=()

  for ((turn = 1; turn <= TURN_COUNT; turn++)); do
    prompts+=("${parts[$turn]}")
  done

  echo "============================================================"
  echo "[CASE]   ${index}"
  echo "[NAME]   ${case_name}"
  echo "[CHAT]   ${CHAT_ID}"

  for ((turn = 1; turn <= TURN_COUNT; turn++)); do
    prompt="${prompts[$((turn - 1))]}"
    submit_file="${case_dir}/turn${turn}_submit.json"
    final_file="${case_dir}/turn${turn}_final.json"

    echo "[TURN${turn}]  ${prompt}"
    raw="$(submit_task "$prompt")"
    printf '%s\n' "$raw" > "$submit_file"
    task_id="$(extract_submit_task_id "$raw")"
    echo "[TASK${turn}]  ${task_id}"
    poll_until_terminal "$task_id" "$final_file"
    echo "[TEXT${turn}]  $(extract_result_text "$final_file")"

    task_ids+=("$task_id")
    finals+=("$final_file")
  done

  meta_file="${case_dir}/meta.txt"
  {
    echo "index=${index}"
    echo "case_name=${case_name}"
    echo "chat_id=${CHAT_ID}"
    for ((turn = 1; turn <= TURN_COUNT; turn++)); do
      echo "turn${turn}_task_id=${task_ids[$((turn - 1))]}"
      echo "turn${turn}_prompt=${prompts[$((turn - 1))]}"
    done
  } > "$meta_file"

  summary_args=("$index" "$case_name" "$CHAT_ID" "$TURN_COUNT")
  for ((turn = 1; turn <= TURN_COUNT; turn++)); do
    summary_args+=(
      "${prompts[$((turn - 1))]}"
      "${task_ids[$((turn - 1))]}"
      "${finals[$((turn - 1))]}"
    )
  done
  append_summary_jsonl "${summary_args[@]}"
  echo
done < <(load_cases "$CASE_FILE" "$TURN_COUNT")

echo "Artifacts:"
echo "  - $RUN_DIR"
echo "  - $RUN_LOG"
echo "  - $SUMMARY_JSONL"

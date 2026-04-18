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
PROVIDER_RETRIES_VALUE="${PROVIDER_RETRIES:-2}"
PROVIDER_RETRY_SLEEP_VALUE="${PROVIDER_RETRY_SLEEP_SECONDS:-3}"
PRINT_LLM_TRACE_VALUE="${PRINT_LLM_TRACE:-1}"
ISOLATE_CHAT_ID_BASE_VALUE="${ISOLATE_CHAT_ID_BASE:-1}"

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
  --reuse-chat-id-base  do not add a run-scoped offset to the base chat id
  --user-key KEY        RustClaw user key; omitted => auto-detect first enabled admin key
  --wait-seconds N      max wait per turn (default: 240)
  --poll-seconds N      poll interval seconds (default: 1)
  --provider-retries N  retry count when provider is unavailable/capacity-limited (default: 2)
  --provider-retry-sleep N
                        sleep seconds before provider retry (default: 3)
  --no-llm-trace        Do not print per-turn LLM request/response trace
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

compute_effective_chat_id_base() {
  local requested_base="$1"
  local isolate="${2:-1}"
  if [[ "$isolate" != "1" ]]; then
    printf '%s\n' "$requested_base"
    return 0
  fi
  local epoch
  epoch="$(date +%s)"
  printf '%s\n' "$((requested_base + (epoch % 100000000)))"
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
import re
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

print_turn_dialog() {
  python3 - "$1" "$2" "$3" <<'PY'
import json
import sys
from pathlib import Path

turn = sys.argv[2]
prompt = sys.argv[3]
obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
result = data.get("result_json") or {}
messages = result.get("messages") or []
text = str(result.get("text") or "").strip()
if not text:
    for item in messages:
        if isinstance(item, dict):
            candidate = str(item.get("text") or "").strip()
            if candidate:
                text = candidate
                break
if not text:
    text = str(data.get("error_text") or "").strip()
if not text:
    text = "<empty>"

print(f"  [USER{turn}]")
for line in prompt.splitlines() or [""]:
    print(f"    {line}")
print(f"  [ASSISTANT{turn}]")
for line in text.splitlines() or [""]:
    print(f"    {line}")
PY
}

final_result_provider_unavailable() {
  python3 - "$1" <<'PY'
import json
import re
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
result = data.get("result_json") or {}
messages = result.get("messages") or []
error_text = str(data.get("error_text") or "").strip().lower()
result_text = str(result.get("text") or "").strip().lower()
message_texts = []
for item in messages:
    if isinstance(item, dict):
        message_texts.append(str(item.get("text") or "").strip().lower())

strong_markers = [
    "当前大模型服务暂时不可用",
    "模型暂不可用",
    "selected model is at capacity",
    "usage limit exceeded",
    "rate limit",
    "rate_limit",
    "too many requests",
    "http 429",
    "http 529",
    "529 overloaded",
    "missing choices[0].message.content",
    "timeout: error sending request for url",
    "error sending request for url",
    "operation timed out",
]

def provider_like_blob(text: str) -> bool:
    text = (text or "").strip().lower()
    if not text:
        return False
    if any(marker in text for marker in strong_markers):
        return True
    return (
        "provider=vendor-" in text
        and (
            re.search(r"http 5\d\d", text) is not None
            or '"type":"server_error"' in text
            or "unknown error, 520" in text
        )
    )

def provider_like_final_text(text: str) -> bool:
    text = (text or "").strip().lower()
    if not text:
        return False
    if len(text) > 400:
        return False
    anchored_markers = [
        "当前大模型服务暂时不可用",
        "模型暂不可用",
        "selected model is at capacity",
        "usage limit exceeded",
        "rate limit",
        "rate_limit",
        "too many requests",
        "http 429",
        "http 529",
        "529 overloaded",
        "missing choices[0].message.content",
        "timeout: error sending request for url",
        "error sending request for url",
        "operation timed out",
    ]
    if any(text.startswith(marker) for marker in anchored_markers):
        return True
    return (
        "provider=vendor-" in text
        and (
            re.search(r"http 5\d\d", text) is not None
            or '"type":"server_error"' in text
            or "unknown error, 520" in text
        )
    )

if provider_like_blob(error_text):
    raise SystemExit(0)
if provider_like_final_text(result_text):
    raise SystemExit(0)
if any(provider_like_final_text(text) for text in message_texts):
    raise SystemExit(0)
raise SystemExit(1)
PY
}

init_llm_trace_offset() {
  local offset_file="$1"
  python3 - "$ROOT_DIR/logs/model_io.log" "$offset_file" <<'PY'
import sys
from pathlib import Path

log_path = Path(sys.argv[1])
offset_file = Path(sys.argv[2])
size = log_path.stat().st_size if log_path.exists() else 0
offset_file.write_text(str(size), encoding="utf-8")
PY
}

print_new_llm_trace() {
  local task_id="$1"
  local offset_file="$2"
  [[ "${PRINT_LLM_TRACE:-1}" == "1" ]] || return 0
  python3 - "$ROOT_DIR/logs/model_io.log" "$task_id" "$offset_file" <<'PY'
import json
import sys
from pathlib import Path

log_path = Path(sys.argv[1])
task_id = sys.argv[2]
offset_file = Path(sys.argv[3])

def indent_block(text: str) -> str:
    return "\n".join(f"    {line}" for line in text.splitlines()) if text else ""

offset = 0
if offset_file.exists():
    raw = offset_file.read_text(encoding="utf-8").strip()
    if raw:
        try:
            offset = int(raw)
        except ValueError:
            offset = 0

if not log_path.exists():
    offset_file.write_text(str(offset), encoding="utf-8")
    raise SystemExit(0)

with log_path.open("rb") as fh:
    fh.seek(offset)
    chunk = fh.read()
    new_offset = fh.tell()

offset_file.write_text(str(new_offset), encoding="utf-8")
if not chunk:
    raise SystemExit(0)

for raw_line in chunk.decode("utf-8", errors="replace").splitlines():
    line = raw_line.strip()
    if not line:
        continue
    try:
        obj = json.loads(line)
    except Exception:
        continue
    if str(obj.get("task_id") or "") != task_id:
        continue

    vendor = str(obj.get("vendor") or "").strip()
    model = str(obj.get("model") or "").strip()
    status = str(obj.get("status") or "").strip()
    prompt = str(obj.get("prompt") or "").strip()
    response = str(obj.get("clean_response") or obj.get("response") or "").strip()
    error = str(obj.get("error") or "").strip()

    print(f"  [llm] vendor={vendor or '<unknown>'} model={model or '<unknown>'} status={status or '<unknown>'}")
    if prompt:
        print("  [llm-request]")
        print(indent_block(prompt))
    if response:
        print("  [llm-response]")
        print(indent_block(response))
    if error:
        print("  [llm-error]")
        print(indent_block(error))
PY
}

poll_until_terminal() {
  local task_id="$1"
  local out_file="$2"
  local llm_offset_file="${3:-}"
  local waited=0
  local last_status=""

  while [[ "$waited" -le "$MAX_WAIT_SECONDS" ]]; do
    local status
    query_task_to_file "$task_id" "$out_file"
    if [[ -n "$llm_offset_file" ]]; then
      print_new_llm_trace "$task_id" "$llm_offset_file"
    fi
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

expected = 5 + turn_count * 4
if len(sys.argv) != expected:
    raise SystemExit(f"invalid summary args: got={len(sys.argv)} expected={expected}")

row = {
    "index": index,
    "case_name": case_name,
    "chat_id": chat_id,
}

for i in range(turn_count):
    base = 5 + i * 4
    prompt = sys.argv[base]
    task_id = sys.argv[base + 1]
    final_path = Path(sys.argv[base + 2])
    effective_status = (sys.argv[base + 3] or "").strip()
    obj = json.loads(final_path.read_text(encoding="utf-8"))
    data = obj.get("data") or {}
    result = data.get("result_json") or {}
    row[f"turn{i + 1}"] = {
        "prompt": prompt,
        "task_id": task_id,
        "status": effective_status or data.get("status"),
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
    --reuse-chat-id-base)
      ISOLATE_CHAT_ID_BASE_VALUE=0
      shift
      ;;
    --wait-seconds)
      WAIT_SECONDS_VALUE="${2:-}"
      shift 2
      ;;
    --poll-seconds)
      POLL_SECONDS_VALUE="${2:-}"
      shift 2
      ;;
    --provider-retries)
      PROVIDER_RETRIES_VALUE="${2:-}"
      shift 2
      ;;
    --provider-retry-sleep)
      PROVIDER_RETRY_SLEEP_VALUE="${2:-}"
      shift 2
      ;;
    --no-llm-trace)
      PRINT_LLM_TRACE_VALUE=0
      shift
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
PROVIDER_RETRIES="$PROVIDER_RETRIES_VALUE"
PROVIDER_RETRY_SLEEP_SECONDS="$PROVIDER_RETRY_SLEEP_VALUE"
PRINT_LLM_TRACE="$PRINT_LLM_TRACE_VALUE"
ISOLATE_CHAT_ID_BASE="$ISOLATE_CHAT_ID_BASE_VALUE"

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
BASE_CHAT_ID="$(compute_effective_chat_id_base "$CHAT_ID" "$ISOLATE_CHAT_ID_BASE")"
echo "  run_chat_id_base: ${BASE_CHAT_ID}"
echo "  provider_retry: ${PROVIDER_RETRIES} x ${PROVIDER_RETRY_SLEEP_SECONDS}s"
echo

health_check

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
  declare -a effective_statuses=()

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
    echo "[USER${turn}]"
    printf '  %s\n' "$prompt"
    attempt=0
    effective_status=""
    while :; do
      attempt=$((attempt + 1))
      llm_offset_file="${case_dir}/turn${turn}_attempt_${attempt}_llm.offset"
      init_llm_trace_offset "$llm_offset_file"
      raw="$(submit_task "$prompt")"
      printf '%s\n' "$raw" > "$submit_file"
      task_id="$(extract_submit_task_id "$raw")"
      echo "[TASK${turn}]  ${task_id}"
      # poll_until_terminal returns 1 on poll timeout; under `set -e` that would
      # tear down the whole multi-turn suite (and the parent run_suite). Wrap
      # with `if !` and synthesize a timeout final so the case is recorded as
      # `timeout` and the suite continues. Mirrors run_manual_test.sh:744.
      poll_failed=0
      if ! poll_until_terminal "$task_id" "$final_file" "$llm_offset_file"; then
        poll_failed=1
      fi
      if (( poll_failed != 0 )); then
        echo "  [poll] timed out waiting for terminal status on turn ${turn}; marking turn as timeout"
        if [[ ! -s "$final_file" ]]; then
          printf '%s\n' '{"data":{"status":"timeout","result_json":{"text":""},"error_text":"poll timeout"}}' > "$final_file"
        fi
        effective_status="timeout"
      fi
      echo "[TEXT${turn}]  $(extract_result_text "$final_file")"
      print_turn_dialog "$final_file" "$turn" "$prompt"
      if final_result_provider_unavailable "$final_file"; then
        if [[ "$attempt" -le "$PROVIDER_RETRIES" ]]; then
          echo "  [model] provider unavailable; retry turn ${turn} (${attempt}/${PROVIDER_RETRIES}) after ${PROVIDER_RETRY_SLEEP_SECONDS}s"
          sleep "$PROVIDER_RETRY_SLEEP_SECONDS"
          continue
        fi
        echo "  [model] provider unavailable after retries; mark turn ${turn} as inconclusive"
        effective_status="provider_unavailable"
      fi
      break
    done

    task_ids+=("$task_id")
    finals+=("$final_file")
    effective_statuses+=("$effective_status")
    if [[ "$effective_status" == "provider_unavailable" ]]; then
      for ((remaining = turn + 1; remaining <= TURN_COUNT; remaining++)); do
        remaining_final="${case_dir}/turn${remaining}_final.json"
        printf '%s\n' '{"data":{"status":"not_run_after_provider_unavailable","result_json":{"text":"","messages":[]}}}' > "$remaining_final"
        task_ids+=("")
        finals+=("$remaining_final")
        effective_statuses+=("not_run_after_provider_unavailable")
      done
      break
    fi
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
      "${effective_statuses[$((turn - 1))]}"
    )
  done
  append_summary_jsonl "${summary_args[@]}"
  echo
done < <(load_cases "$CASE_FILE" "$TURN_COUNT")

echo "Artifacts:"
echo "  - $RUN_DIR"
echo "  - $RUN_LOG"
echo "  - $SUMMARY_JSONL"

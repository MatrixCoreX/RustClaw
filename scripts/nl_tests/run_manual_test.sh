#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/shell_compat.sh"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/lib.sh"

DEFAULT_CASE_FILE="${SCRIPT_DIR}/cases/nl_cases_manual.txt"
DEFAULT_LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/manual"

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
PROVIDER_RETRIES_VALUE="${PROVIDER_RETRIES:-2}"
PROVIDER_RETRY_SLEEP_VALUE="${PROVIDER_RETRY_SLEEP_SECONDS:-3}"
PRINT_LLM_TRACE_VALUE="${PRINT_LLM_TRACE:-1}"
ISOLATE_CHAT_ID_BASE_VALUE="${ISOLATE_CHAT_ID_BASE:-1}"
FULL_TEXT=0

RESUME_DIR=""
RESUME_LINE=0
CURRENT_SOURCE_LINE=0
LAST_COMPLETED_LINE=0

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_manual_test.sh [options]
  Preferred unified entry:
    bash scripts/nl_tests/run_suite.sh manual [options]

Options:
  --case-file PATH      Case file to run. Default: scripts/nl_tests/cases/nl_cases_manual.txt
  --log-root PATH       Root log dir. Default: scripts/nl_suite_logs/manual
  --resume-dir PATH     Existing run dir to append logs/results into
  --resume-line N       Continue after this tested source line number
  --base-url URL        clawd base url. Default: http://127.0.0.1:8787
  --user-id ID          User id for submit
  --chat-id ID          Base chat id for submit
  --reuse-chat-id-base  Do not add a run-scoped offset to the base chat id
  --user-key KEY        RustClaw user key
  --wait-seconds N      Max wait seconds per case
  --poll-seconds N      Poll interval seconds
  --provider-retries N  Retry count when provider is unavailable/capacity-limited (default: 2)
  --provider-retry-sleep N
                        Sleep seconds before provider retry (default: 3)
  --no-llm-trace        Do not print per-task LLM request/response trace
  --full-text           Print full response text
  -h, --help            Show this help

Case format:
  suite|name|tags|prompt
  prompt-only lines are also allowed

Resume behavior:
  If you already tested through line 12, rerun with:
    bash scripts/nl_tests/run_manual_test.sh --resume-dir <run_dir> --resume-line 12

Artifacts:
  scripts/nl_suite_logs/manual/<timestamp>/
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
import re
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

print_user_visible_dialog() {
  python3 - "$1" "$2" <<'PY'
import json
import sys
from pathlib import Path

prompt = sys.argv[2]
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

print("  [user]")
for line in prompt.splitlines() or [""]:
    print(f"    {line}")
print("  [assistant]")
for line in text.splitlines() or [""]:
    print(f"    {line}")
PY
}

log_terminal_result_flags() {
  python3 - "$1" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
result = data.get("result_json") or {}
messages = result.get("messages") or []
parts = [
    str(data.get("error_text") or ""),
    str(result.get("text") or ""),
]
for item in messages:
    if isinstance(item, dict):
        parts.append(str(item.get("text") or ""))
text = "\n".join(parts).lower()
markers = [
    "当前大模型服务暂时不可用",
    "selected model is at capacity",
    "usage limit exceeded",
    "rate limit",
    "rate_limit",
    "too many requests",
    "http 429",
]
if any(m in text for m in markers):
    print("[model] unavailable/capacity/rate-limit message observed in final result")
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

    header = f"  [llm] vendor={vendor or '<unknown>'} model={model or '<unknown>'} status={status or '<unknown>'}"
    print(header)
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
    local status err_file rc
    err_file="$(mktemp)"
    set +e
    query_task_to_file "$task_id" "$out_file" 2>"$err_file"
    rc=$?
    set -e
    if [[ "$rc" -ne 0 ]]; then
      local err_text
      err_text="$(cat "$err_file")"
      rm -f "$err_file"
      echo "  [network] query failed for task_id=${task_id}: ${err_text}" >&2
      return 1
    fi
    rm -f "$err_file"
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
        log_terminal_result_flags "$out_file"
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
  local effective_status="${6:-}"
  python3 - "$source_line" "$case_name" "$prompt" "$task_id" "$final_json" "$effective_status" >> "$SUMMARY_JSONL" <<'PY'
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
effective_status = (sys.argv[6] or "").strip()
row = {
    "source_line": source_line,
    "case_name": case_name,
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
  local safe_name case_dir submit_file final_file meta_file task_id raw rc llm_offset_file

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
  echo "[USER]"
  printf '  %s\n' "$prompt"

  local attempt=0
  local effective_status=""
  while :; do
    attempt=$((attempt + 1))
    local err_file
    llm_offset_file="${case_dir}/attempt_${attempt}_llm.offset"
    init_llm_trace_offset "$llm_offset_file"
    err_file="$(mktemp)"
    set +e
    raw="$(submit_task "$prompt" 2>"$err_file")"
    rc=$?
    set -e
    if [[ "$rc" -ne 0 ]]; then
      echo "  [network] submit failed: $(cat "$err_file")" >&2
      rm -f "$err_file"
      return "$rc"
    fi
    rm -f "$err_file"
    printf '%s\n' "$raw" > "$submit_file"
    task_id="$(extract_submit_task_id "$raw")"
    echo "[TASK]        $task_id"
    poll_until_terminal "$task_id" "$final_file" "$llm_offset_file"

    echo "[RESULT]"
    if [[ "$FULL_TEXT" -eq 1 ]]; then
      extract_result_summary "$final_file" "full"
    else
      extract_result_summary "$final_file" "summary"
    fi
    print_user_visible_dialog "$final_file" "$prompt"

    if final_result_provider_unavailable "$final_file"; then
      if [[ "$attempt" -le "$PROVIDER_RETRIES" ]]; then
        echo "  [model] provider unavailable; retrying (${attempt}/${PROVIDER_RETRIES}) after ${PROVIDER_RETRY_SLEEP_SECONDS}s"
        sleep "$PROVIDER_RETRY_SLEEP_SECONDS"
        continue
      fi
      echo "  [model] provider unavailable after retries; mark as inconclusive"
      effective_status="provider_unavailable"
    fi
    break
  done

  printf 'ordinal=%s\nsource_line=%s\ncase_name=%s\nchat_id=%s\ntask_id=%s\nprompt=%s\n' \
    "$ordinal" "$source_line" "$case_name" "$CHAT_ID" "$task_id" "$prompt" > "$meta_file"

  append_summary_jsonl "$source_line" "$case_name" "$prompt" "$task_id" "$final_file" "$effective_status"
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
    "TOTAL_CASES={} SUCCEEDED={} FAILED={} CANCELED={} TIMEOUT={} PROVIDER_UNAVAILABLE={}".format(
        len(rows),
        counter.get("succeeded", 0),
        counter.get("failed", 0),
        counter.get("canceled", 0),
        counter.get("timeout", 0),
        counter.get("provider_unavailable", 0),
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
    --reuse-chat-id-base)
      ISOLATE_CHAT_ID_BASE_VALUE=0
      shift
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
    --provider-retries)
      PROVIDER_RETRIES_VALUE="$2"
      shift 2
      ;;
    --provider-retry-sleep)
      PROVIDER_RETRY_SLEEP_VALUE="$2"
      shift 2
      ;;
    --no-llm-trace)
      PRINT_LLM_TRACE_VALUE=0
      shift
      ;;
    --network-retries|--network-sleep|--model-sleep|--model-retries)
      echo "[deprecated] $1 is ignored; retry/sleep logic has been disabled" >&2
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
PROVIDER_RETRIES="$PROVIDER_RETRIES_VALUE"
PROVIDER_RETRY_SLEEP_SECONDS="$PROVIDER_RETRY_SLEEP_VALUE"
PRINT_LLM_TRACE="$PRINT_LLM_TRACE_VALUE"
ISOLATE_CHAT_ID_BASE="$ISOLATE_CHAT_ID_BASE_VALUE"

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
BASE_CHAT_ID="$(compute_effective_chat_id_base "$CHAT_ID" "$ISOLATE_CHAT_ID_BASE")"
echo "  run_chat_id_base: $BASE_CHAT_ID"
echo "  user_key:      ${USER_KEY:+<set>}"
echo "  wait:          ${MAX_WAIT_SECONDS}s"
echo "  poll:          ${POLL_INTERVAL_SECONDS}s"
echo "  provider_retry:${PROVIDER_RETRIES} x ${PROVIDER_RETRY_SLEEP_SECONDS}s"
if [[ -n "$RESUME_DIR" ]]; then
  echo "  resume_dir:    $RESUME_DIR"
  echo "  resume_line:   $RESUME_LINE"
fi
echo

health_check

array_from_command_lines CASE_ROWS load_case_rows "$CASE_FILE"
if [[ "${#CASE_ROWS[@]}" -eq 0 ]]; then
  echo "No runnable cases found in $CASE_FILE" >&2
  exit 2
fi

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

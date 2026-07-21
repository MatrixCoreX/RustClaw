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
CASE_NAME_FILTER=""
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
# Default 1 (was 2): 之前默认 2 意味着上游一抖手就把同一个 case ×3 调用，浪费
# LLM 预算 + 翻倍跑 intent_router。N=1 只在 provider_unavailable 强标记时再
# 尝试一次。要恢复旧行为传 --provider-retries 2 即可。
PROVIDER_RETRIES_VALUE="${PROVIDER_RETRIES:-1}"
PROVIDER_RETRY_SLEEP_VALUE="${PROVIDER_RETRY_SLEEP_SECONDS:-3}"
PRINT_LLM_TRACE_VALUE="${PRINT_LLM_TRACE:-1}"
ISOLATE_CHAT_ID_BASE_VALUE="${ISOLATE_CHAT_ID_BASE:-1}"
FULL_TEXT=0
PROMPT_REPLY_ONLY=0
# 0 = 不启用；>0 = 连续这么多个 case 走到 timeout / provider_unavailable / network
# 失败后中断剩余测试，避免上游真挂时还把 60 个 case 都跑完。
FAIL_FAST_VALUE="${NL_TEST_FAIL_FAST:-0}"
# 运行期累计的连续坏 case 数，run_one_case 内部维护。
CONSECUTIVE_BAD=0
ABORTED_FAIL_FAST=0
SKIPPED_AFTER_ABORT=0

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
  --case-name NAME      Run only the matching case_name from the case file
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
  --provider-retries N  Retry count when provider is unavailable/capacity-limited (default: 1)
  --provider-retry-sleep N
                        Sleep seconds before provider retry (default: 3)
  --fail-fast N         Abort remaining cases after N consecutive bad cases
                        (timeout / provider_unavailable / network). Default: 0 = disabled.
                        Also reads NL_TEST_FAIL_FAST env var.
  --no-llm-trace        Do not print per-task LLM request/response trace
  --full-text           Print full response text
  --prompt-reply-only   Print only prompt and assistant reply for each case
  -h, --help            Show this help

Case file format:
  suite|name|tags|prompt
  suite|name|tags|prompt|expect=<substring>     # 5th field optional, asserts response contains substring
  suite|name|tags|prompt|expect=contains:<substring>;json_exists:/data/result_json/machine_reply
  suite|name|tags|prompt|expect=json_eq:/data/status=succeeded
  suite|name|tags|prompt|expect=contains:<substring>;confirm:确认执行

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

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" --anchor "$1" "$2"
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1" >&2
    exit 2
  }
}

# A1: 启动时打印当前 clawd binary 的位置 + mtime，并和源码比对。
#
# 历史教训：Cursor 沙箱 / 远端开发容器会把 CARGO_TARGET_DIR 重定向到 /tmp，
# 结果 cargo build 后的新 binary 落在沙箱缓存里，target/release/clawd 还是
# 几天前的旧二进制 —— 但 NL 测试照常跑通，让人误以为新代码已经在跑。
#
# 这个函数只警告不阻断：拿到 /v1/health 里的进程信息（如果有），并比对
# crates/clawd/src 下最新源码 mtime；如果 src 比 binary 新，就喷红字。
check_binary_freshness() {
  local clawd_bin="${ROOT_DIR}/target/release/clawd"
  if [[ ! -x "$clawd_bin" ]]; then
    echo "[binary] WARN: ${clawd_bin} not found (server may have been started from a different path)"
  else
    local bin_mtime
    bin_mtime="$(file_mtime_epoch "$clawd_bin")"
    local bin_mtime_str
    bin_mtime_str="$(format_epoch_local "$bin_mtime")"
    echo "[binary] target/release/clawd mtime=${bin_mtime_str} size=$(file_size_bytes "$clawd_bin")"

    local src_dir="${ROOT_DIR}/crates/clawd/src"
    if [[ -d "$src_dir" ]]; then
      local src_latest
      src_latest="$(latest_tree_mtime_epoch "$src_dir" ".rs")"
      if [[ -n "$src_latest" && "$src_latest" -gt "$bin_mtime" ]]; then
        local src_str
        src_str="$(format_epoch_local "$src_latest")"
        echo "[binary] WARN: source under crates/clawd/src has files newer than binary"
        echo "[binary]       latest src mtime: ${src_str}"
        echo "[binary]       binary  mtime:   ${bin_mtime_str}"
        echo "[binary]       => the running clawd may NOT contain your latest changes."
      fi
    fi
  fi
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    echo "[binary] note: CARGO_TARGET_DIR=${CARGO_TARGET_DIR} is set;"
    echo "[binary]       'cargo build' artifacts will land there, not in target/release/."
    echo "[binary]       After building, copy with:"
    echo "[binary]         cp \"\${CARGO_TARGET_DIR}/release/clawd\" target/release/clawd"
  fi
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

case_group_key() {
  local tags="$1"
  local token
  local -a tag_tokens=()
  IFS=',;' read -r -a tag_tokens <<< "$tags"
  for token in "${tag_tokens[@]}"; do
    token="${token#"${token%%[![:space:]]*}"}"
    token="${token%"${token##*[![:space:]]}"}"
    if [[ "$token" == group:* && -n "${token#group:}" ]]; then
      printf '%s\n' "${token#group:}"
      return 0
    fi
  done
  return 1
}

resolve_run_chat_id_base() {
  local requested_chat_id="$1"
  local isolate_chat_id_base="$2"
  local state_file="${RUN_DIR}/chat_id_base.txt"
  local persisted=""
  if [[ -f "$state_file" ]]; then
    persisted="$(tr -d '[:space:]' < "$state_file")"
    if [[ "$persisted" =~ ^-?[0-9]+$ ]]; then
      printf '%s\n' "$persisted"
      return 0
    fi
  fi
  persisted="$(compute_effective_chat_id_base "$requested_chat_id" "$isolate_chat_id_base")"
  printf '%s\n' "$persisted" > "$state_file"
  printf '%s\n' "$persisted"
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
  # Emit one row per case as: source_line\x1f case_name \x1f tags \x1f prompt \x1f expect \x1f confirm
  # Backwards compatible: 4-field rows (suite|name|tags|prompt) still parse.
  # New 5th field is optional and looks like `expect=<substring>` (literal substring assertion).
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
    tags = ""
    expect = ""
    confirm = ""
    if "|" in line:
        # split with limit=4 so we can have at most 5 fields (last field may itself contain '|')
        parts = [part.strip() for part in line.split("|", 4)]
        if len(parts) >= 4:
            _, name, tags_field, prompt = parts[:4]
            if name:
                case_name = name
            tags = tags_field
            if len(parts) == 5:
                tail = parts[4].strip()
                if tail.startswith("expect="):
                    directives = tail[len("expect="):]
                    kept = []
                    for directive in [part.strip() for part in directives.split(";") if part.strip()]:
                        if directive.startswith("confirm:"):
                            confirm = directive[len("confirm:"):].strip()
                        else:
                            kept.append(directive)
                    expect = ";".join(kept)
                # else: silently ignore unknown 5th field for forward-compat
        else:
            prompt = parts[-1]
    prompt = prompt.strip()
    if not prompt:
        continue
    # Use \x1f as delimiter — never appears in user prompts.
    print(f"{idx}\x1f{case_name}\x1f{tags}\x1f{prompt}\x1f{expect}\x1f{confirm}")
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
  python3 - "$1" "$2" "${3:-0}" <<'PY'
import json
import sys
from pathlib import Path

prompt = sys.argv[2]
prompt_reply_only = sys.argv[3] == "1"
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

if prompt_reply_only:
    print("[PROMPT]")
    for line in prompt.splitlines() or [""]:
        print(line)
    print("[REPLY]")
    for line in text.splitlines() or [""]:
        print(line)
else:
    print("  [PROMPT]")
    for line in prompt.splitlines() or [""]:
        print(f"    {line}")
    print("  [REPLY]")
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
    "模型暂时不可用",
    "selected model is at capacity",
    "usage limit exceeded",
    "rate limit",
    "rate_limit",
    "too many requests",
    "http 429",
    "http 401",
    "authorized_error",
    "login fail",
    "鉴权失败",
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
    "模型暂时不可用",
    "模型暂不可用",
    "selected model is at capacity",
    "usage limit exceeded",
    "rate limit",
    "rate_limit",
    "too many requests",
    "http 401",
    "http 429",
    "http 529",
    "529 overloaded",
    "authorized_error",
    "login fail",
    "鉴权失败",
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
        "模型暂时不可用",
        "模型暂不可用",
        "selected model is at capacity",
        "usage limit exceeded",
        "rate limit",
        "rate_limit",
        "too many requests",
        "http 401",
        "http 429",
        "http 529",
        "529 overloaded",
        "authorized_error",
        "login fail",
        "鉴权失败",
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

final_result_needs_confirmation() {
  python3 - "$1" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))

def walk(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)

for item in walk(obj):
    if item.get("needs_confirmation") is True:
        raise SystemExit(0)
    if str(item.get("terminal_intent") or "") == "needs_confirmation":
        raise SystemExit(0)
    if str(item.get("reason_code") or "") == "confirmation_required":
        raise SystemExit(0)
    if str(item.get("status_code") or "") == "confirmation_required":
        raise SystemExit(0)
    if str(item.get("resume_reason") or "") == "confirmation_required":
        raise SystemExit(0)

raise SystemExit(1)
PY
}

init_llm_trace_offset() {
  local offset_file="$1"
  python3 "${SCRIPT_DIR}/print_llm_raw_trace.py" \
    --log "$ROOT_DIR/logs/model_io.log" \
    --state-file "$offset_file" \
    --init-state
}

print_new_llm_trace() {
  local task_id="$1"
  local offset_file="$2"
  [[ "${PRINT_LLM_TRACE:-1}" == "1" ]] || return 0
  python3 "${SCRIPT_DIR}/print_llm_raw_trace.py" \
    --log "$ROOT_DIR/logs/model_io.log" \
    --task-id "$task_id" \
    --state-file "$offset_file"
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
      if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
        echo "  [status] ${last_status:-<none>} -> ${status:-<empty>}"
      fi
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
  local tags="$3"
  local prompt="$4"
  local task_id="$5"
  local final_json="$6"
  local effective_status="${7:-}"
  local started_at="${8:-0}"
  local ended_at="${9:-0}"
  local expect_substr="${10:-}"
  local mode="${11:-ask}"
  python3 "${SCRIPT_DIR}/manual_case_assertions.py" \
    "$source_line" "$case_name" "$tags" "$prompt" "$task_id" \
    "$final_json" "$effective_status" "$started_at" "$ended_at" \
    "$expect_substr" "$mode" \
    >> "$SUMMARY_JSONL"
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
  RUN_STARTED_AT_EPOCH="$(date +%s)"
  exec > >(tee -a "$RUN_LOG") 2>&1
}

handle_interrupt() {
  local code=130
  echo
  echo "[INTERRUPTED]"
  if [[ -n "${RUN_DIR:-}" ]]; then
    echo "  run_dir_ref:          $(path_ref "$RUN_DIR" "$RUN_DIR")"
  else
    echo "  run_dir_ref:          not_created"
  fi
  echo "  current_source_line:  ${CURRENT_SOURCE_LINE:-0}"
  echo "  last_completed_line:  ${LAST_COMPLETED_LINE:-0}"
  echo "Resume by reusing the same args and adding:"
  echo "  --resume-dir <run_dir_ref> --resume-line ${LAST_COMPLETED_LINE:-0}"
  exit "$code"
}

run_one_case() {
  local ordinal="$1"
  local source_line="$2"
  local case_name="$3"
  local tags="$4"
  local prompt="$5"
  local expect_substr="$6"
  local chat_id="$7"
  local confirm_reply="${8:-}"
  local safe_name case_dir submit_file final_file meta_file task_id raw rc llm_offset_file
  local started_at ended_at mode submit_payload skill_args

  CURRENT_SOURCE_LINE="$source_line"
  safe_name="$(sanitize_name "$case_name")"
  case_dir="$(printf '%s/case_%03d_line_%03d_%s' "$RUN_DIR" "$ordinal" "$source_line" "$safe_name")"
  submit_file="${case_dir}/submit.json"
  final_file="${case_dir}/final.json"
  meta_file="${case_dir}/meta.txt"
  mkdir -p "$case_dir"

  CHAT_ID="$chat_id"
  started_at="$(date +%s)"

  mode="ask"

  echo
  if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
    echo "============================================================"
    echo "[CASE]        $ordinal"
    echo "[SOURCE_LINE] $source_line"
    echo "[NAME]        $case_name"
    if [[ -n "$tags" ]]; then
      echo "[TAGS]        $tags"
    fi
    echo "[MODE]        $mode"
    echo "[CHAT]        $CHAT_ID"
    if [[ -n "$expect_substr" ]]; then
      echo "[EXPECT]      ${expect_substr}"
    fi
    echo "[PROMPT]      $prompt"
    echo "[PROMPT]"
    printf '  %s\n' "$prompt"
  fi

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
      ended_at="$(date +%s)"
      append_summary_jsonl "$source_line" "$case_name" "$tags" "$prompt" "" "" "network_error" "$started_at" "$ended_at" "$expect_substr" "$mode"
      CONSECUTIVE_BAD=$((CONSECUTIVE_BAD + 1))
      LAST_COMPLETED_LINE="$source_line"
      CURRENT_SOURCE_LINE=0
      return 0
    fi
    rm -f "$err_file"
    printf '%s\n' "$raw" > "$submit_file"
    task_id="$(extract_submit_task_id "$raw")"
    if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
      echo "[TASK]        $task_id"
    fi

    # poll_until_terminal returns non-zero on poll timeout; under `set -e`
    # that would tear down the whole run mid-suite. (poll_until_terminal
    # internally re-enables `set -e`, so a plain `set +e ... set -e`
    # wrapper would not work — that's why we use `if ! ...`.)
    local poll_failed=0
    if ! poll_until_terminal "$task_id" "$final_file" "$llm_offset_file"; then
      poll_failed=1
    fi
    if (( poll_failed != 0 )); then
      if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
        echo "  [poll] timed out waiting for terminal status; marking case as timeout"
      fi
      effective_status="timeout"
      if [[ ! -s "$final_file" ]]; then
        printf '%s\n' '{"data":{"status":"timeout","result_json":{"text":""},"error_text":"poll timeout"}}' > "$final_file"
      fi
    fi

    if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
      echo "[RESULT]"
      if [[ "$FULL_TEXT" -eq 1 ]]; then
        extract_result_summary "$final_file" "full"
      else
        extract_result_summary "$final_file" "summary"
      fi
    fi
    print_user_visible_dialog "$final_file" "$prompt" "$PROMPT_REPLY_ONLY"

    if [[ -n "$confirm_reply" ]]; then
      if ! final_result_needs_confirmation "$final_file"; then
        if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
          echo "[CONFIRM_SKIPPED] final result does not require confirmation"
        else
          echo "[CONFIRM_SKIPPED]"
          echo "final result does not require confirmation"
        fi
      else
      local confirm_submit_file confirm_final_file confirm_llm_offset_file confirm_raw confirm_task_id confirm_err_file
      confirm_submit_file="${case_dir}/submit_confirm.json"
      confirm_final_file="${case_dir}/final_confirm.json"
      confirm_llm_offset_file="${case_dir}/attempt_${attempt}_confirm_llm.offset"
      init_llm_trace_offset "$confirm_llm_offset_file"
      if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
        echo "[CONFIRM]     $confirm_reply"
      else
        echo "[CONFIRM]"
        echo "$confirm_reply"
      fi
      confirm_err_file="$(mktemp)"
      set +e
      confirm_raw="$(submit_task "$confirm_reply" 2>"$confirm_err_file")"
      rc=$?
      set -e
      if [[ "$rc" -ne 0 ]]; then
        echo "  [network] confirm submit failed: $(cat "$confirm_err_file")" >&2
        rm -f "$confirm_err_file"
        effective_status="network_error"
        break
      fi
      rm -f "$confirm_err_file"
      printf '%s\n' "$confirm_raw" > "$confirm_submit_file"
      confirm_task_id="$(extract_submit_task_id "$confirm_raw")"
      if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
        echo "[CONFIRM_TASK] $confirm_task_id"
      fi
      if ! poll_until_terminal "$confirm_task_id" "$confirm_final_file" "$confirm_llm_offset_file"; then
        effective_status="timeout"
        if [[ ! -s "$confirm_final_file" ]]; then
          printf '%s\n' '{"data":{"status":"timeout","result_json":{"text":""},"error_text":"confirm poll timeout"}}' > "$confirm_final_file"
        fi
      fi
      task_id="$confirm_task_id"
      final_file="$confirm_final_file"
      print_user_visible_dialog "$final_file" "$confirm_reply" "$PROMPT_REPLY_ONLY"
      fi
    fi

    if final_result_provider_unavailable "$final_file"; then
      if [[ "$attempt" -le "$PROVIDER_RETRIES" ]]; then
        if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
          echo "  [model] provider unavailable; retrying (${attempt}/${PROVIDER_RETRIES}) after ${PROVIDER_RETRY_SLEEP_SECONDS}s"
        fi
        sleep "$PROVIDER_RETRY_SLEEP_SECONDS"
        continue
      fi
      if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
        echo "  [model] provider unavailable after retries; mark as inconclusive"
      fi
      effective_status="provider_unavailable"
    fi
    break
  done

  ended_at="$(date +%s)"

  printf 'ordinal=%s\nsource_line=%s\ncase_name=%s\ntags=%s\nmode=%s\nchat_id=%s\ntask_id=%s\nprompt=%s\nconfirm=%s\nexpect=%s\nstarted_at=%s\nended_at=%s\n' \
    "$ordinal" "$source_line" "$case_name" "$tags" "$mode" "$CHAT_ID" "$task_id" "$prompt" "$confirm_reply" "$expect_substr" "$started_at" "$ended_at" > "$meta_file"

  append_summary_jsonl "$source_line" "$case_name" "$tags" "$prompt" "$task_id" "$final_file" "$effective_status" "$started_at" "$ended_at" "$expect_substr" "$mode"

  # A3: 维护连续坏 case 计数（用于 --fail-fast）
  local final_status assertion_status
  final_status="$(extract_status "$final_file" 2>/dev/null || echo "")"
  assertion_status="$(
    tail -n 1 "$SUMMARY_JSONL" | python3 -c '
import json
import sys

try:
    row = json.load(sys.stdin)
except (json.JSONDecodeError, TypeError):
    print("-")
else:
    print(str(row.get("assertion") or "-"))
'
  )"
  if [[ -n "$effective_status" ]]; then
    final_status="$effective_status"
  fi
  case "${assertion_status}:${final_status}" in
    fail:*)
      CONSECUTIVE_BAD=$((CONSECUTIVE_BAD + 1))
      ;;
    *:succeeded)
      CONSECUTIVE_BAD=0
      ;;
    *:timeout|*:provider_unavailable|*:network_error|*:)
      CONSECUTIVE_BAD=$((CONSECUTIVE_BAD + 1))
      ;;
    *)
      # failed / canceled etc — don't count toward fail-fast (typically real product issues we want to see all of)
      CONSECUTIVE_BAD=0
      ;;
  esac
  if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
    echo "  [stats] wall=$((ended_at - started_at))s consecutive_bad=${CONSECUTIVE_BAD}"
  fi

  LAST_COMPLETED_LINE="$source_line"
  CURRENT_SOURCE_LINE=0
}

print_final_summary() {
  python3 - "$SUMMARY_JSONL" "${RUN_STARTED_AT_EPOCH:-0}" "${ABORTED_FAIL_FAST:-0}" "${SKIPPED_AFTER_ABORT:-0}" <<'PY'
import json
import sys
import time
from collections import Counter
from pathlib import Path

path = Path(sys.argv[1])
run_started_at = int(sys.argv[2] or 0)
aborted = int(sys.argv[3] or 0)
skipped_after_abort = int(sys.argv[4] or 0)

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
assertion_counter = Counter(str(row.get("assertion") or "-") for row in rows)
asserted_rows = [r for r in rows if (r.get("expect_substr") or "")]
walls = [int(r.get("wall_seconds") or 0) for r in rows if r.get("wall_seconds") is not None]
total_wall = (int(time.time()) - run_started_at) if run_started_at else (sum(walls))

def fmt_dur(secs):
    if secs <= 0:
        return "0s"
    h, rem = divmod(secs, 3600)
    m, s = divmod(rem, 60)
    if h:
        return f"{h}h{m:02d}m{s:02d}s"
    if m:
        return f"{m}m{s:02d}s"
    return f"{s}s"

print()
print("================ Final Summary ================")
print(
    "TOTAL_CASES={} SUCCEEDED={} FAILED={} CANCELED={} TIMEOUT={} PROVIDER_UNAVAILABLE={} NETWORK_ERROR={}".format(
        len(rows),
        counter.get("succeeded", 0),
        counter.get("failed", 0),
        counter.get("canceled", 0),
        counter.get("timeout", 0),
        counter.get("provider_unavailable", 0),
        counter.get("network_error", 0),
    )
)
print(
    "WALL_TIME_TOTAL={} CASE_AVG={}s CASE_P95={}s".format(
        fmt_dur(total_wall),
        (sum(walls) // len(walls)) if walls else 0,
        sorted(walls)[int(0.95 * (len(walls) - 1))] if walls else 0,
    )
)
if asserted_rows:
    print(
        "ASSERTIONS  PASS={} FAIL={} (out of {} cases with `expect=`)".format(
            assertion_counter.get("pass", 0),
            assertion_counter.get("fail", 0),
            len(asserted_rows),
        )
    )

# Per-status case-name lists for triage.
def list_for(status_or_assertion, lookup):
    matches = [r for r in rows if str(r.get(lookup) or "") == status_or_assertion]
    return [str(r.get("case_name") or "?") for r in matches]

for s in ("failed", "timeout", "provider_unavailable", "network_error"):
    names = list_for(s, "status")
    if names:
        print(f"  [{s:>22}] {', '.join(names)}")

failed_assertions = [r for r in rows if r.get("assertion") == "fail"]
if failed_assertions:
    print("  [    failed assertions] " + ", ".join(str(r.get("case_name") or "?") for r in failed_assertions))

if aborted:
    print(f"FAIL_FAST: aborted after consecutive bad cases; skipped {skipped_after_abort} remaining cases.")
PY
}

summary_exit_code() {
  python3 - "$SUMMARY_JSONL" "${ABORTED_FAIL_FAST:-0}" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
aborted = int(sys.argv[2] or 0)
bad_statuses = {
    "failed",
    "canceled",
    "timeout",
    "provider_unavailable",
    "network_error",
    "unknown",
    "",
}

bad = aborted != 0
for raw in path.read_text(encoding="utf-8").splitlines():
    raw = raw.strip()
    if not raw:
        continue
    row = json.loads(raw)
    if str(row.get("status") or "") in bad_statuses:
        bad = True
    if row.get("assertion") == "fail":
        bad = True

raise SystemExit(1 if bad else 0)
PY
}

trap handle_interrupt INT TERM

while [[ $# -gt 0 ]]; do
  case "$1" in
    --case-file)
      CASE_FILE="$2"
      shift 2
      ;;
    --case-name)
      CASE_NAME_FILTER="$2"
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
    --fail-fast)
      FAIL_FAST_VALUE="$2"
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
    --prompt-reply-only)
      PROMPT_REPLY_ONLY=1
      PRINT_LLM_TRACE_VALUE=0
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
FAIL_FAST_THRESHOLD="$FAIL_FAST_VALUE"
if ! [[ "$FAIL_FAST_THRESHOLD" =~ ^[0-9]+$ ]]; then
  echo "--fail-fast must be a non-negative integer" >&2
  exit 2
fi

resolve_admin_key
USER_KEY="$USER_KEY_VALUE"

prepare_run_dir

if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
  echo "Natural-language manual regression"
  echo "  case_file_ref:     $(path_ref "$RUN_DIR" "$CASE_FILE")"
  echo "  run_dir_ref:       $(path_ref "$RUN_DIR" "$RUN_DIR")"
  echo "  run_log_ref:       $(path_ref "$RUN_DIR" "$RUN_LOG")"
  echo "  summary_jsonl_ref: $(path_ref "$RUN_DIR" "$SUMMARY_JSONL")"
  echo "  base_url:      $BASE_URL"
  echo "  user_id:       $USER_ID"
  echo "  chat_id:       $CHAT_ID"
fi
BASE_CHAT_ID="$(resolve_run_chat_id_base "$CHAT_ID" "$ISOLATE_CHAT_ID_BASE")"
if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
  echo "  run_chat_id_base: $BASE_CHAT_ID"
  echo "  user_key:      ${USER_KEY:+<set>}"
  echo "  wait:          ${MAX_WAIT_SECONDS}s"
  echo "  poll:          ${POLL_INTERVAL_SECONDS}s"
  echo "  provider_retry:${PROVIDER_RETRIES} x ${PROVIDER_RETRY_SLEEP_SECONDS}s"
  if (( FAIL_FAST_THRESHOLD > 0 )); then
    echo "  fail_fast:     abort after ${FAIL_FAST_THRESHOLD} consecutive bad cases"
  else
    echo "  fail_fast:     disabled"
  fi
  if [[ -n "$RESUME_DIR" ]]; then
    echo "  resume_dir:    $RESUME_DIR"
    echo "  resume_line:   $RESUME_LINE"
  fi
  echo
fi

if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
  health_check
  check_binary_freshness
  echo
else
  health_check >/dev/null
fi

array_from_command_lines CASE_ROWS load_case_rows "$CASE_FILE"
if [[ "${#CASE_ROWS[@]}" -eq 0 ]]; then
  echo "No runnable cases found in $CASE_FILE" >&2
  exit 2
fi

ordinal=0
run_count=0
GROUP_CHAT_KEYS=()
GROUP_CHAT_VALUES=()

group_chat_id_for_key() {
  local requested="$1"
  local index
  for ((index = 0; index < ${#GROUP_CHAT_KEYS[@]}; index++)); do
    if [[ "${GROUP_CHAT_KEYS[$index]}" == "$requested" ]]; then
      printf '%s\n' "${GROUP_CHAT_VALUES[$index]}"
      return 0
    fi
  done
  return 1
}

remember_group_chat_id() {
  GROUP_CHAT_KEYS+=("$1")
  GROUP_CHAT_VALUES+=("$2")
}

for row in "${CASE_ROWS[@]}"; do
  IFS=$'\x1f' read -r source_line case_name tags prompt expect_substr confirm_reply <<< "$row"
  ordinal=$((ordinal + 1))
  chat_id_for_case=$((BASE_CHAT_ID + ordinal))
  group_key="$(case_group_key "$tags" || true)"
  if [[ -n "$group_key" ]]; then
    if existing_group_chat_id="$(group_chat_id_for_key "$group_key")"; then
      chat_id_for_case="$existing_group_chat_id"
    else
      remember_group_chat_id "$group_key" "$chat_id_for_case"
    fi
  fi

  if (( source_line <= RESUME_LINE )); then
    LAST_COMPLETED_LINE="$source_line"
    if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
      echo "[SKIP] source_line=${source_line} name=${case_name} already covered by --resume-line ${RESUME_LINE}"
    fi
    continue
  fi

  if [[ -n "$CASE_NAME_FILTER" && "$case_name" != "$CASE_NAME_FILTER" ]]; then
    continue
  fi

  if (( ABORTED_FAIL_FAST == 1 )); then
    SKIPPED_AFTER_ABORT=$((SKIPPED_AFTER_ABORT + 1))
    continue
  fi

  run_count=$((run_count + 1))
  run_one_case "$ordinal" "$source_line" "$case_name" "$tags" "$prompt" "$expect_substr" "$chat_id_for_case" "$confirm_reply"

  if (( FAIL_FAST_THRESHOLD > 0 )) && (( CONSECUTIVE_BAD >= FAIL_FAST_THRESHOLD )); then
    if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
      echo
      echo "[FAIL_FAST] reached ${CONSECUTIVE_BAD} consecutive bad cases >= threshold ${FAIL_FAST_THRESHOLD}; aborting remaining cases."
    fi
    ABORTED_FAIL_FAST=1
  fi
done

if (( run_count == 0 )); then
  if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
    if [[ -n "$CASE_NAME_FILTER" ]]; then
      echo "No matching remaining cases for --case-name ${CASE_NAME_FILTER} after --resume-line ${RESUME_LINE}."
    else
      echo "No remaining cases after --resume-line ${RESUME_LINE}."
    fi
  fi
fi

if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
  print_final_summary
  echo
  echo "Artifacts:"
  echo "  - run_dir_ref=$(path_ref "$RUN_DIR" "$RUN_DIR")"
  echo "  - run_log_ref=$(path_ref "$RUN_DIR" "$RUN_LOG")"
  echo "  - summary_jsonl_ref=$(path_ref "$RUN_DIR" "$SUMMARY_JSONL")"
fi

summary_exit_code

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NL_TEST_SCRIPT_DIR="$SCRIPT_DIR"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/lib.sh"

BASE_URL_VALUE="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID_VALUE="${USER_ID:-2403753217836067397}"
CHAT_ID_VALUE="${CHAT_ID:--1002403753217}"
EXTERNAL_CHAT_ID_VALUE="${EXTERNAL_CHAT_ID:-}"
EXTERNAL_USER_ID_VALUE="${EXTERNAL_USER_ID:-}"
USER_KEY_VALUE="${RUSTCLAW_USER_KEY:-${USER_KEY:-}}"
CHANNEL_VALUE="${CLIENT_LIKE_CHANNEL:-telegram}"
CONFIG_PATH_VALUE="${RUSTCLAW_CONFIG_PATH:-${ROOT_DIR}/configs/config.toml}"
DB_PATH_VALUE="${RUSTCLAW_DB_PATH:-}"
WAIT_SECONDS_VALUE="${MAX_WAIT_SECONDS:-1200}"
POLL_SECONDS_VALUE="${POLL_INTERVAL_SECONDS:-1}"
LLM_INFRA_TURN_RETRIES_VALUE="${LLM_INFRA_TURN_RETRIES:-3}"
PRINT_LLM_TRACE_VALUE="${PRINT_LLM_TRACE:-1}"
PRINT_LLM_TRACE_MAX_CHARS_VALUE="${PRINT_LLM_TRACE_MAX_CHARS:-1200}"
LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/client_like_continuous"
PROMPT_REPLY_ONLY=1
QUALITY_GUARD=0
CASE_FILE_VALUE=""
CASE_FILE_LOADER_VALUE=""
CASE_FILE_VALUES=()
CASE_JSONL_VALUE=""
CASE_LIMIT_VALUE=""
CASE_GROUP_LIMIT_VALUE="${CASE_GROUP_LIMIT:-}"
CASE_START_VALUE="${CASE_START:-1}"
CASE_INCLUDE_TAGS_VALUE="${CASE_INCLUDE_TAGS:-}"
CASE_INCLUDE_ANY_TAGS_VALUE="${CASE_INCLUDE_ANY_TAGS:-}"
CASE_EXCLUDE_TAGS_VALUE="${CASE_EXCLUDE_TAGS:-}"
CASE_INCLUDE_GROUPS_VALUE="${CASE_INCLUDE_GROUPS:-0}"
CASE_INCLUDE_GROUP_CONTEXT_VALUE="${CASE_INCLUDE_GROUP_CONTEXT:-0}"
RUN_BUILTIN_SMOKE=1
CASE_GROUP_ISOLATION="${CASE_GROUP_ISOLATION:-1}"
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
TEST_ID="${CLIENT_LIKE_TEST_ID:-client-like-continuous-${RUN_STAMP}}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_client_like_continuous_suite.sh [options]

What it tests:
  Directly POSTs /v1/tasks to clawd with stable user_id/chat_id, external ids,
  user_key, and text payload. The default channel is telegram for historical
  client parity; isolated server wrappers set the non-delivering ui channel.
  Multiple turns reuse one client identity so clawd accumulates tasks, conversation state, and memory.

Options:
  --base-url URL             clawd base URL. Default: http://127.0.0.1:8787
  --user-id ID               RustClaw/Telegram-side user id. Default: deterministic large id
  --chat-id ID               Telegram raw chat id. Default: deterministic negative group id
  --external-user-id ID      Telegramd-compatible external_user_id. Default: user-id
  --external-chat-id ID      Telegramd-compatible external_chat_id. Default: chat-id
  --channel CHANNEL          Submission channel: ui or telegram. Default:
                             CLIENT_LIKE_CHANNEL or telegram
  --user-key KEY             RustClaw user key. Default: RUSTCLAW_USER_KEY/USER_KEY or first enabled admin key
  --config PATH              config.toml used to resolve DB path for assertions
  --db-path PATH             main SQLite DB path for assertions
  --wait-seconds N           max wait per turn. Default: 1200
  --poll-seconds N           poll interval seconds. Default: 1
  --llm-infra-turn-retries N retry a turn when trace shows model infra failure.
                             Default: LLM_INFRA_TURN_RETRIES or 3
  --log-root PATH            log output root
  --case-file PATH           append prompts from a case file into the same client-like conversation
  --case-jsonl PATH          append prompts from JSONL rows with name/tags/prompt/expect fields
  --full-nl                  shorthand for --case-file scripts/nl_tests/cases/nl_cases_full.txt
  --case-limit N             max appended cases from --case-file/--full-nl
  --case-group-limit N       max appended case groups; preserves all rows in each emitted group.
                             When set, this is preferred over --case-limit for group-preserving runs.
  --case-start N             start from the Nth appended case. Use with --skip-smoke and the same
                             --external-chat-id/--external-user-id to resume after provider failure.
  --include-case-tag TAG     run only appended cases whose tag string contains TAG. May be repeated;
                             repeated values are all required.
  --include-case-tag-any TAG run appended cases whose tag string contains any of these tags.
                             May be repeated; combines with --include-case-tag as an additional OR filter.
  --include-case-groups      when include tags match any row in a case group, keep the whole group.
                             This preserves setup turns for filtered continuous cases.
  --include-case-group-context
                             when include tags match any row in a case group, keep matched rows plus
                             tagged context/setup rows from that group instead of the whole group.
  --exclude-case-tag TAG     skip appended cases whose tag string contains TAG. May be repeated.
  --skip-smoke               run only the case file prompts, without the built-in 5-turn memory smoke
  --shared-case-chat         append all case-file prompts into one external_chat_id. By default,
                             independent case groups are isolated while turns in the same group share context.
  --prompt-reply-only        print only prompt and reply snippets. Default: on
  --verbose-turn-output      print compact turn status/reply fields instead of prompt/reply blocks
  --no-llm-trace             do not print numbered raw LLM return fields after each turn
  --llm-trace-max-chars N    max chars per long raw LLM field excerpt. Default: 1200
  --quality-guard            fail on common soft failures, not only terminal task status
  -h, --help                 show this help
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1" >&2
    exit 2
  }
}

join_case_files() {
  local sep="$1"
  shift
  local out=""
  local path
  for path in "$@"; do
    if [[ -z "$out" ]]; then
      out="$path"
    else
      out="${out}${sep}${path}"
    fi
  done
  printf "%s" "$out"
}

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" "$1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
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
    --external-user-id)
      EXTERNAL_USER_ID_VALUE="${2:-}"
      shift 2
      ;;
    --external-chat-id)
      EXTERNAL_CHAT_ID_VALUE="${2:-}"
      shift 2
      ;;
    --channel)
      CHANNEL_VALUE="${2:-}"
      shift 2
      ;;
    --user-key)
      USER_KEY_VALUE="${2:-}"
      shift 2
      ;;
    --config)
      CONFIG_PATH_VALUE="${2:-}"
      shift 2
      ;;
    --db-path)
      DB_PATH_VALUE="${2:-}"
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
    --llm-infra-turn-retries)
      LLM_INFRA_TURN_RETRIES_VALUE="${2:-}"
      shift 2
      ;;
    --log-root)
      LOG_ROOT="${2:-}"
      shift 2
      ;;
    --case-file)
      CASE_FILE_VALUES+=("${2:-}")
      shift 2
      ;;
    --case-jsonl)
      CASE_JSONL_VALUE="${2:-}"
      shift 2
      ;;
    --full-nl)
      CASE_FILE_VALUES=("${ROOT_DIR}/scripts/nl_tests/cases/nl_cases_full.txt")
      shift
      ;;
    --case-limit)
      CASE_LIMIT_VALUE="${2:-}"
      shift 2
      ;;
    --case-group-limit)
      CASE_GROUP_LIMIT_VALUE="${2:-}"
      shift 2
      ;;
    --case-start)
      CASE_START_VALUE="${2:-}"
      shift 2
      ;;
    --include-case-tag)
      if [[ -z "${2:-}" ]]; then
        echo "--include-case-tag requires a value" >&2
        exit 2
      fi
      if [[ -n "${CASE_INCLUDE_TAGS_VALUE:-}" ]]; then
        CASE_INCLUDE_TAGS_VALUE="${CASE_INCLUDE_TAGS_VALUE},${2}"
      else
        CASE_INCLUDE_TAGS_VALUE="${2}"
      fi
      shift 2
      ;;
    --include-case-tag-any)
      if [[ -z "${2:-}" ]]; then
        echo "--include-case-tag-any requires a value" >&2
        exit 2
      fi
      if [[ -n "${CASE_INCLUDE_ANY_TAGS_VALUE:-}" ]]; then
        CASE_INCLUDE_ANY_TAGS_VALUE="${CASE_INCLUDE_ANY_TAGS_VALUE},${2}"
      else
        CASE_INCLUDE_ANY_TAGS_VALUE="${2}"
      fi
      shift 2
      ;;
    --include-case-groups)
      CASE_INCLUDE_GROUPS_VALUE=1
      shift
      ;;
    --include-case-group-context)
      CASE_INCLUDE_GROUP_CONTEXT_VALUE=1
      shift
      ;;
    --exclude-case-tag)
      if [[ -z "${2:-}" ]]; then
        echo "--exclude-case-tag requires a value" >&2
        exit 2
      fi
      if [[ -n "${CASE_EXCLUDE_TAGS_VALUE:-}" ]]; then
        CASE_EXCLUDE_TAGS_VALUE="${CASE_EXCLUDE_TAGS_VALUE},${2}"
      else
        CASE_EXCLUDE_TAGS_VALUE="${2}"
      fi
      shift 2
      ;;
    --skip-smoke)
      RUN_BUILTIN_SMOKE=0
      shift
      ;;
    --shared-case-chat)
      CASE_GROUP_ISOLATION=0
      shift
      ;;
    --prompt-reply-only)
      PROMPT_REPLY_ONLY=1
      shift
      ;;
    --verbose-turn-output)
      PROMPT_REPLY_ONLY=0
      shift
      ;;
    --no-llm-trace)
      PRINT_LLM_TRACE_VALUE=0
      shift
      ;;
    --llm-trace-max-chars)
      PRINT_LLM_TRACE_MAX_CHARS_VALUE="${2:-}"
      shift 2
      ;;
    --quality-guard)
      QUALITY_GUARD=1
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

if [[ "${CHANNEL_VALUE}" != "ui" && "${CHANNEL_VALUE}" != "telegram" ]]; then
  echo "--channel must be ui or telegram" >&2
  exit 2
fi

need_cmd curl
need_cmd python3

cd "${ROOT_DIR}"

resolve_admin_key() {
  if [[ -n "${USER_KEY_VALUE:-}" ]]; then
    return 0
  fi
  USER_KEY_VALUE="$("${ROOT_DIR}/scripts/auth-key.sh" list | awk '$2 == "admin" && $3 == "enabled" { print $1; exit }')"
  if [[ -z "${USER_KEY_VALUE:-}" ]]; then
    echo "No enabled admin key found. Pass --user-key or set RUSTCLAW_USER_KEY." >&2
    exit 2
  fi
}

resolve_db_path() {
  if [[ -n "${DB_PATH_VALUE:-}" ]]; then
    python3 - "${DB_PATH_VALUE}" <<'PY'
from pathlib import Path
import sys
print(Path(sys.argv[1]).resolve())
PY
    return 0
  fi
  python3 - "${ROOT_DIR}" "${CONFIG_PATH_VALUE}" <<'PY'
from pathlib import Path
import sys
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

root = Path(sys.argv[1]).resolve()
config_path = Path(sys.argv[2]).resolve()
cfg = tomllib.loads(config_path.read_text(encoding="utf-8"))
raw = cfg.get("database", {}).get("sqlite_path", "data/rustclaw.db")
path = Path(raw)
if not path.is_absolute():
    path = root / path
print(path.resolve())
PY
}

normalize_llm_trace_max_chars() {
  local value="${1:-1200}"
  if [[ ! "$value" =~ ^[0-9]+$ ]]; then
    value=1200
  fi
  if (( value < 240 )); then
    value=240
  fi
  printf "%s" "$value"
}

extract_json_field() {
  local file="$1"
  local field="$2"
  python3 - "$file" "$field" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
field = sys.argv[2]
data = obj.get("data") or {}
result = data.get("result_json") or {}
def message_texts():
    texts = []
    for item in result.get("messages") or []:
        if isinstance(item, str) and item.strip():
            texts.append(item.strip())
        elif isinstance(item, dict) and str(item.get("text") or "").strip():
            texts.append(str(item.get("text") or "").strip())
    return texts

if field == "status":
    print(str(data.get("status") or ""))
elif field == "text":
    text = str(result.get("text") or "").strip()
    if not text:
        messages = message_texts()
        if messages:
            text = messages[-1]
    print(text.replace("\n", "\\n"))
elif field == "text_raw":
    text = str(result.get("text") or "").strip()
    if not text:
        messages = message_texts()
        if messages:
            text = messages[-1]
    print(text)
elif field == "messages":
    print("\\n---\\n".join(message.replace("\n", "\\n") for message in message_texts()))
elif field == "messages_raw":
    print("\n---\n".join(message_texts()))
elif field == "error":
    print(str(data.get("error_text") or "").strip().replace("\n", "\\n"))
elif field == "error_raw":
    print(str(data.get("error_text") or "").strip())
else:
    raise SystemExit(f"unknown field: {field}")
PY
}

result_text_contains() {
  local file="$1"
  local expected="$2"
  python3 - "$file" "$expected" <<'PY'
import json
import sys
import unicodedata
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
expected = sys.argv[2]
data = obj.get("data") or {}
result = data.get("result_json") or {}
journal = result.get("task_journal") or {}
summary = journal.get("summary") or {}
route_result = summary.get("route_result") or {}
needs_clarify = bool(route_result.get("needs_clarify"))
texts = [str(data.get("error_text") or ""), str(result.get("text") or "")]
visible_items = []
if str(result.get("text") or "").strip():
    visible_items.append(str(result.get("text") or "").strip())
for item in result.get("messages") or []:
    if isinstance(item, str):
        texts.append(item)
        if item.strip():
            visible_items.append(item.strip())
    elif isinstance(item, dict):
        text_item = str(item.get("text") or "")
        texts.append(text_item)
        if text_item.strip():
            visible_items.append(text_item.strip())
joined = "\n".join(texts)
visible_joined = "\n".join(visible_items)
def normalize_text(value: str) -> str:
    return unicodedata.normalize("NFKC", value).replace("\u00a0", " ").replace("\u202f", " ")

_MISSING = object()

def json_pointer_get(value, pointer):
    if not pointer.startswith("/"):
        return _MISSING
    current = value
    for raw_part in pointer.split("/")[1:]:
        part = raw_part.replace("~1", "/").replace("~0", "~")
        if isinstance(current, dict):
            if part not in current:
                return _MISSING
            current = current[part]
        elif isinstance(current, list):
            try:
                index = int(part)
            except ValueError:
                return _MISSING
            if index < 0 or index >= len(current):
                return _MISSING
            current = current[index]
        else:
            return _MISSING
    return current

def compare_text(value):
    if value is _MISSING:
        return None
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return "null"
    if isinstance(value, (int, float, str)):
        return str(value)
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))

ok = True
for raw in [part.strip() for part in expected.split(";") if part.strip()]:
    if raw.startswith("contains:"):
        needle = raw[len("contains:"):]
        part_ok = normalize_text(needle) in normalize_text(joined)
    elif raw.startswith("json_exists:"):
        pointer = raw[len("json_exists:"):]
        part_ok = json_pointer_get(obj, pointer) is not _MISSING
    elif raw.startswith("json_eq:"):
        expr = raw[len("json_eq:"):]
        pointer, sep, wanted = expr.partition("=")
        actual = compare_text(json_pointer_get(obj, pointer))
        part_ok = bool(sep) and actual == wanted
    elif raw.startswith("visible_json_fields:"):
        wanted_fields = [
            field.strip()
            for field in raw[len("visible_json_fields:"):].split(",")
            if field.strip()
        ]
        candidate_texts = [item for item in visible_items if item]
        if visible_joined.strip():
            candidate_texts.append(visible_joined.strip())
        part_ok = False
        for candidate_text in candidate_texts:
            try:
                visible_obj = json.loads(candidate_text)
            except Exception:
                continue
            if (
                isinstance(visible_obj, dict)
                and bool(wanted_fields)
                and all(field in visible_obj for field in wanted_fields)
            ):
                part_ok = True
                break
    else:
        part_ok = normalize_text(raw) in normalize_text(joined)
    ok = ok and part_ok
raise SystemExit(0 if ok else 1)
PY
}

result_has_bad_fallback() {
  local file="$1"
  local prompt="${2:-}"
  python3 - "$file" "$prompt" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
prompt = sys.argv[2].lower()
data = obj.get("data") or {}
result = data.get("result_json") or {}
texts = [
    str(data.get("error_text") or ""),
    str(result.get("text") or ""),
]
for item in result.get("messages") or []:
    if isinstance(item, str):
        texts.append(item)
    elif isinstance(item, dict):
        texts.append(str(item.get("text") or ""))
joined = "\n".join(texts).lower()
hard_markers = [
    "intent_unresolved",
    "context window exceeds limit",
    "invalid params",
]
soft_markers = [
    "模型暂时不可用",
    "当前大模型服务暂时不可用",
    "model is temporarily unavailable",
    "temporarily unavailable (auth/network/circuit",
    "auth/network/circuit",
    "could not reach the model service",
    "please retry later or switch to an available model",
    "我没看出这条消息要做什么",
    "没有足够的上下文",
    "没有足够上下文",
    "无法确定这个连续会话测试",
]
if any(marker in joined for marker in hard_markers):
    raise SystemExit(0)
for marker in soft_markers:
    if marker in joined and marker not in prompt:
        raise SystemExit(0)
raise SystemExit(1)
PY
}

result_is_retryable_llm_infra_failure() {
  local file="$1"
  python3 - "$file" <<'PY'
import json
import sys
from pathlib import Path

try:
    obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
except Exception:
    raise SystemExit(1)

data = obj.get("data") or {}
result = data.get("result_json") or {}
journal = result.get("task_journal") or {}
summary = journal.get("summary") or {}
trace = journal.get("trace") or {}

status = str(data.get("status") or "").strip().lower()
error_text = str(data.get("error_text") or result.get("error_text") or "").strip().lower()
if status == "timeout":
    raise SystemExit(0)
if error_text.startswith("provider=") and "timeout:" in error_text:
    raise SystemExit(0)
if "poll timeout waiting for terminal task status" in error_text:
    raise SystemExit(0)

values = []
for parent in (summary, trace):
    route = (parent or {}).get("route_result") or {}
    values.append(str(route.get("route_reason") or ""))
    values.append(str(route.get("route_gate_kind") or ""))
values.append(str(summary.get("final_stop_signal") or ""))
values.append(str(trace.get("final_stop_signal") or ""))
joined = ";".join(values)

retryable_codes = {
    "llm_failed_safe_clarify",
    "normalizer_llm_failed",
    "fallback_router_llm_failed",
}
if any(code in joined for code in retryable_codes):
    raise SystemExit(0)
raise SystemExit(1)
PY
}

quality_violation_reason() {
  local file="$1"
  local prompt="${2:-}"
  local expected="${3:-}"
  local case_tags="${4:-}"
  python3 - "$file" "$prompt" "$expected" "$case_tags" "$NL_TEST_SCRIPT_DIR" <<'PY'
import json
import re
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
prompt = sys.argv[2]
expected = sys.argv[3] if len(sys.argv) > 3 else ""
case_tags = sys.argv[4] if len(sys.argv) > 4 else ""
sys.path.insert(0, sys.argv[5])
from manual_case_assertions import structural_assertions

prompt_l = prompt.lower()
tagset = {part.strip().lower() for part in re.split(r"[,;]", case_tags) if part.strip()}
for segment in case_tags.split(";"):
    segment = segment.strip()
    if not segment.lower().startswith("covers:"):
        continue
    tagset.update(
        part.strip().lower()
        for part in segment.split(":", 1)[1].split(",")
        if part.strip()
    )
data = obj.get("data") or {}
result = data.get("result_json") or {}
journal = result.get("task_journal") or {}
summary = journal.get("summary") or {}
route_result = summary.get("route_result") or {}
needs_clarify = bool(route_result.get("needs_clarify"))
final_status = str(summary.get("final_status") or "").strip().lower()
final_visible = str(result.get("text") or "").strip()
if not final_visible:
    for item in result.get("messages") or []:
        if isinstance(item, str) and item.strip():
            final_visible = item.strip()
            break
        if isinstance(item, dict) and str(item.get("text") or "").strip():
            final_visible = str(item.get("text") or "").strip()
            break
texts = [str(data.get("error_text") or ""), str(result.get("text") or "")]
for item in result.get("messages") or []:
    if isinstance(item, str):
        texts.append(item)
    elif isinstance(item, dict):
        texts.append(str(item.get("text") or ""))
text = "\n".join(part for part in texts if part).strip()
text_l = text.lower()

failed_structural_assertions = [
    detail
    for detail in structural_assertions(case_tags, final_visible, result)
    if not detail.get("ok")
]
if failed_structural_assertions:
    print(
        "case_structural_assertion_failed:"
        + json.dumps(failed_structural_assertions[0], ensure_ascii=False, sort_keys=True)
    )
    raise SystemExit(0)

internal_context_markers = [
    "### ACTIVE_EXECUTION_ANCHOR",
    "### ACTIVE_TASK",
    "### ALIASES",
    "### ANCHOR",
    "### AUTH",
    "### CAPABILITIES",
    "### HINTS",
    "### MEMORY",
    "### RECENT",
    "### RECENT_EXECUTION_CONTEXT",
    "### RECENT_TURNS_FULL",
    "### SESSION_ALIAS_BINDINGS",
]
if any(marker in text for marker in internal_context_markers):
    print("internal_context_marker_visible")
    raise SystemExit(0)

try:
    visible_json = json.loads(text)
except Exception:
    visible_json = None
if isinstance(visible_json, dict) and {
    "message_key",
    "status_source",
    "field_value",
    "error_code",
    "reason_code",
} & set(visible_json):
    print("runtime_structured_json_visible")
    raise SystemExit(0)

if "structured_failure" in tagset:
    trace = journal.get("trace") or {}
    step_results = trace.get("step_results") or []
    failed_steps = [
        item for item in step_results
        if str((item or {}).get("status") or "").strip().lower() in {"error", "failed"}
        and str((item or {}).get("skill") or (item or {}).get("executed_skill") or "").strip()
        not in {"respond", "think", "synthesize_answer"}
    ]
    if not failed_steps:
        print("structured_failure_without_failed_evidence")
        raise SystemExit(0)
    ok_outputs = []
    for item in step_results:
        item = item or {}
        if str(item.get("status") or "").strip().lower() != "ok":
            continue
        skill = str(item.get("skill") or item.get("executed_skill") or "").strip()
        if skill in {"respond", "think", "synthesize_answer"}:
            continue
        output = str(item.get("output_excerpt") or "").strip()
        if output:
            ok_outputs.append(output)
    if final_visible and final_visible in ok_outputs:
        print("structured_failure_final_is_success_output")
        raise SystemExit(0)

if "scalar" in tagset and "allow_multiline_scalar" not in tagset:
    scalar_lines = [line.strip() for line in final_visible.splitlines() if line.strip()]
    if len(scalar_lines) > 1:
        print("scalar_case_multiline_reply")
        raise SystemExit(0)

for token in sorted(tagset):
    if not token.startswith("expect_max_lines:"):
        continue
    raw_limit = token.split(":", 1)[1].strip()
    try:
        max_lines = int(raw_limit)
    except ValueError:
        print(f"invalid_expect_max_lines_tag:{token}")
        raise SystemExit(0)
    visible_lines = [line.strip() for line in final_visible.splitlines() if line.strip()]
    if len(visible_lines) > max_lines:
        print(f"reply_exceeds_expected_max_lines expected<={max_lines} got={len(visible_lines)}")
        raise SystemExit(0)

if (
    ("missing_file_graceful" in tagset or "failure" in tagset)
    and final_visible
    and re.match(r"^(?:FILE|IMAGE_FILE):", final_visible.strip(), flags=re.IGNORECASE)
):
    print("failure_case_returned_delivery_token")
    raise SystemExit(0)

language_render_tags = {
    "bound_path_summary",
    "chat",
    "content_grounded_summary",
    "content_grounded_excerpt_and_summary",
    "summary",
    "workspace_project_summary",
}
token_only_reply = bool(re.fullmatch(r"[\w./:@+-]+", final_visible.strip()))
if (
    "ko" in tagset
    and tagset & language_render_tags
    and final_visible
    and not token_only_reply
    and not re.search(r"[\uac00-\ud7a3]", final_visible)
):
    print("ko_reply_missing_hangul")
    raise SystemExit(0)

clarify_allowed_tags = {
    "allow_clarify",
    "clarify",
    "missing_file_graceful",
    "suite:ask",
    "suite:failure",
}
clarify_allowed = bool(tagset & clarify_allowed_tags) or bool(
    re.search(r"(?:^|[,;:])clarify(?:$|[,;])", case_tags.lower())
)
if final_status == "clarify" and not clarify_allowed:
    print("unexpected_clarify_final_status")
    raise SystemExit(0)

if (
    "client-like-continuous-" in text
    and "client-like-continuous-" not in prompt
    and not (expected and expected in text)
):
    print("reply_leaked_unrequested_test_id")
    raise SystemExit(0)

if "<missing>" in text and "<missing>" not in prompt:
    print("reply_contains_internal_missing_sentinel")
    raise SystemExit(0)

strict_prompt_markers = [
    "只输出",
    "只回答",
    "不要解释",
    "不要总结",
    "不要贴正文",
    "不要贴内容",
    "只告诉我",
    "恰好",
    "不要多也不要少",
    "output only",
    "answer only",
    "return only",
    "exactly",
    "no more no less",
    "no explanation",
    "do not paste",
    "do not summarize",
    "with no summary",
]
output_contract = route_result.get("output_contract") or {}
delivery_intent = str(output_contract.get("delivery_intent") or "").strip().lower()
delivery_requested = bool(route_result.get("wants_file_delivery")) or bool(
    output_contract.get("delivery_required")
) or delivery_intent not in ("", "none")
missing_or_clarify = any(
    marker in text_l
    for marker in [
        "not found",
        "不存在",
        "没有找到",
        "找不到",
        "未找到",
        "文件未找到",
        "无法完成发送",
        "请提供完整路径",
        "provide the full path",
    ]
)
if delivery_requested and not needs_clarify and not missing_or_clarify and "file:" not in text_l and "image_file:" not in text_l:
    print("delivery_request_without_file_token")
    raise SystemExit(0)

filesystem_refusal_markers = [
    "chat-only 模式无法直接访问文件系统",
    "chat-only mode cannot perform filesystem checks",
    "chat-only mode",
    "cannot perform filesystem checks",
    "无法直接访问文件系统",
    "没有执行文件系统命令的能力",
    "无法检查当前目录",
    "suggested_command",
    "cannot directly access the file system",
    "do not have the ability to execute filesystem commands",
]
filesystem_prompt_markers = [
    "目录",
    "文件",
    "当前目录",
    "仓库",
    "logs",
    "document",
    "read",
    "list",
    "check",
    "file",
    "directory",
    "pwd",
]
if any(marker in text_l or marker in text for marker in filesystem_refusal_markers):
    if any(marker in prompt_l or marker in prompt for marker in filesystem_prompt_markers):
        print("filesystem_request_answered_as_chat_only_refusal")
        raise SystemExit(0)

path_clarify_markers = [
    "请提供完整的仓库路径",
    "请提供完整路径",
    "请提供文件所在目录",
    "provide the full repository path",
    "provide the full path",
    "provide the directory",
]
current_scope_markers = [
    "当前",
    "仓库",
    "当前目录",
    "当前仓库",
    "current",
    "repo",
    "repository",
    "workspace",
]
observable_markers = [
    "有没有",
    "是否存在",
    "列出",
    "读取",
    "读一下",
    "看一下",
    "exists",
    "exist",
    "list",
    "read",
    "check",
]
if any(marker in text_l or marker in text for marker in path_clarify_markers):
    if any(marker in prompt_l or marker in prompt for marker in current_scope_markers) and any(
        marker in prompt_l or marker in prompt for marker in observable_markers
    ):
        print("current_workspace_request_asked_for_path")
        raise SystemExit(0)

if "{{last_output" in text_l or re.search(r"\{\{[^}]+\}\}", text):
    print("unresolved_template_visible_or_executed")
    raise SystemExit(0)

if "调用技能 `schedule`（action=compile" in text and re.search(r"已成功(创建|设置)", text):
    print("schedule_compile_overclaimed_created")
    raise SystemExit(0)

print("__NO_QUALITY_VIOLATION__")
raise SystemExit(0)
PY
}

assert_expected_skill_from_tags() {
  local file="$1"
  local tags="${2:-}"
  python3 - "$file" "$tags" <<'PY'
import json
import re
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
tags = sys.argv[2]

def normalize_expected_name(raw: str) -> str:
    suffixes = [
        "_local_side_effect_client_like_aggregate",
        "_local_conditional_client_like_aggregate",
        "_network_side_effect_client_like_aggregate",
        "_network_client_like_aggregate",
        "_local_client_like_aggregate",
        "_client_like_aggregate",
    ]
    raw = (raw or "").strip()
    for suffix in suffixes:
        if raw.endswith(suffix):
            return raw[: -len(suffix)]
    return raw

def split_tag_tokens(raw_tags: str) -> list[str]:
    return [token for token in re.split(r"[\s,;]+", raw_tags.strip()) if token]

def expected_contract_from_tags(raw_tags: str) -> dict:
    expected = {
        "skill": "",
        "capabilities": [],
        "any_skills": [],
        "requires_tool_call": False,
        "direct_allowed": False,
        "evidence_fields": [],
    }
    for token in split_tag_tokens(raw_tags):
        if token.startswith("builtin_skill:"):
            raw = normalize_expected_name(token[len("builtin_skill:"):])
        elif token.startswith("skill:"):
            raw = normalize_expected_name(token[len("skill:"):])
        else:
            continue
        if raw and raw != "chat":
            expected["skill"] = raw
            break
    for token in split_tag_tokens(raw_tags):
        if token == "direct_allowed":
            expected["direct_allowed"] = True
            continue
        if token == "requires_tool_call=true":
            expected["requires_tool_call"] = True
            continue
        if token.startswith("capability:"):
            raw = normalize_expected_name(token[len("capability:"):])
        elif token.startswith("tool:"):
            raw = normalize_expected_name(token[len("tool:"):])
        else:
            continue
        if raw and raw != "chat":
            expected["capabilities"].append(raw)
    for match in re.finditer(r"(?:^|[\s,;])any_skill:([^\s;]+)", raw_tags):
        for raw in re.split(r"[,;+]+", match.group(1).strip()):
            name = normalize_expected_name(raw)
            if name and name != "chat":
                expected["any_skills"].append(name)
    for match in re.finditer(r"(?:^|[\s,;])evidence:([^\s;]+)", raw_tags):
        for raw in re.split(r"[,;+]+", match.group(1).strip()):
            field = raw.strip()
            if field:
                expected["evidence_fields"].append(field)
    return expected

expected = expected_contract_from_tags(tags)
if (
    not expected["skill"]
    and not expected["capabilities"]
    and not expected["any_skills"]
    and not expected["requires_tool_call"]
    and not expected["evidence_fields"]
):
    raise SystemExit(0)

data = obj.get("data") or {}
result = data.get("result_json") or {}
journal = result.get("task_journal") or {}
trace = journal.get("trace") or {}
executed = []
requested_capabilities = []
tool_steps = []
for item in trace.get("step_results") or []:
    item = item or {}
    skill = str(item.get("executed_skill") or item.get("skill") or "").strip()
    if skill:
        executed.append(skill)
    if skill and skill not in {"think", "respond", "synthesize_answer"}:
        tool_steps.append(item)
    requested = str(item.get("requested_capability") or "").strip()
    if requested:
        requested_capabilities.append(requested)
for round_item in trace.get("rounds") or []:
    plan = (round_item or {}).get("plan_result") or {}
    for step in plan.get("steps") or []:
        step = step or {}
        skill = str(step.get("skill") or "").strip()
        if skill:
            executed.append(skill)
        if str(step.get("action_type") or "").strip() == "call_tool" and skill:
            requested_capabilities.append(skill)
            tool_steps.append(step)

executed_set = set(executed)
capability_set = set(requested_capabilities) | executed_set

def capability_family_names(name: str) -> set[str]:
    """Return runtime-equivalent capability names for legacy test expectations."""
    families = {
        # Structured field/range reads and document parsing are now often planned
        # through system_basic/doc_parse rather than the old read_file wrapper.
        "read_file": {"read_file", "system_basic", "doc_parse", "config_basic", "fs_basic"},
        # File writes may be served by the structured filesystem tool.
        "write_file": {"write_file", "fs_basic"},
        # Directory creation may be served by the structured filesystem tool.
        "make_dir": {"make_dir", "fs_basic"},
        # Directory inventory/listing may be served by the structured system tool.
        "list_dir": {"list_dir", "system_basic", "fs_basic"},
        # Legacy system_basic filesystem/runtime probes may now be planned through
        # fs_basic for filesystem facts or health_check for host/system snapshots.
        "system_basic": {"system_basic", "fs_basic", "health_check"},
        # Repository/file search now commonly uses fs_basic.find_entries.
        "fs_search": {"fs_search", "fs_basic"},
        # Read-only service status may be answered through process inventory.
        "service_control": {"service_control", "process_basic", "health_check"},
        # RustClaw config guard is now exposed through config_edit.guard_config;
        # config_guard remains a compatibility label in older NL case tags.
        "config_guard": {"config_guard", "config_edit", "config_basic"},
        # Native child delegation is traced by exact planner capability while
        # source-controlled case tags retain the owning builtin skill token.
        "subagent": {"subagent", "agent.subagent"},
    }
    return families.get(name, {name})

def expected_name_observed(name: str, observed: set[str]) -> bool:
    return bool(capability_family_names(name).intersection(observed))

def json_contains_key(value, key: str) -> bool:
    if isinstance(value, dict):
        if key in value:
            return True
        return any(json_contains_key(child, key) for child in value.values())
    if isinstance(value, list):
        return any(json_contains_key(child, key) for child in value)
    if isinstance(value, str) and key in value:
        try:
            parsed = json.loads(value)
        except Exception:
            return False
        return json_contains_key(parsed, key)
    return False

if (
    expected["skill"]
    and not expected_name_observed(expected["skill"], executed_set)
    and not (expected["direct_allowed"] and not tool_steps)
):
    actual = ",".join(dict.fromkeys(executed)) or "<none>"
    print(f"expected_skill_not_executed expected={expected['skill']} actual={actual}")
    raise SystemExit(1)
for capability in expected["capabilities"]:
    if not expected_name_observed(capability, capability_set):
        actual = ",".join(dict.fromkeys(requested_capabilities + executed)) or "<none>"
        print(f"expected_capability_not_observed expected={capability} actual={actual}")
        raise SystemExit(1)
if expected["any_skills"] and not any(
    expected_name_observed(name, executed_set) for name in expected["any_skills"]
):
    actual = ",".join(dict.fromkeys(executed)) or "<none>"
    allowed = ",".join(dict.fromkeys(expected["any_skills"])) or "<none>"
    print(f"expected_any_skill_not_executed allowed={allowed} actual={actual}")
    raise SystemExit(1)
if expected["requires_tool_call"] and not tool_steps:
    print("expected_tool_call_not_observed actual=<none>")
    raise SystemExit(1)
for field in expected["evidence_fields"]:
    if not json_contains_key(journal, field):
        print(f"expected_evidence_field_missing field={field}")
        raise SystemExit(1)
raise SystemExit(0)
PY
}

assert_reply_scalar_equals() {
  local file="$1"
  local expected="$2"
  python3 - "$file" "$expected" <<'PY'
import json
import re
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
expected = sys.argv[2].strip()
data = obj.get("data") or {}
result = data.get("result_json") or {}
texts = []
if str(result.get("text") or "").strip():
    texts.append(str(result.get("text") or "").strip())
for item in result.get("messages") or []:
    if isinstance(item, str) and item.strip():
        texts.append(item.strip())
    elif isinstance(item, dict) and str(item.get("text") or "").strip():
        texts.append(str(item.get("text") or "").strip())
text = (texts[-1] if texts else str(data.get("error_text") or "")).strip()
normalized = re.sub(r"^[`'\"\s]+|[`'\"\s]+$", "", text)
if normalized != expected:
    raise SystemExit(f"expected scalar reply {expected!r}, got {text!r}")
PY
}

assert_reply_concise_summary() {
  local file="$1"
  python3 - "$file" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
result = data.get("result_json") or {}
texts = []
if str(result.get("text") or "").strip():
    texts.append(str(result.get("text") or "").strip())
for item in result.get("messages") or []:
    if isinstance(item, str) and item.strip():
        texts.append(item.strip())
    elif isinstance(item, dict) and str(item.get("text") or "").strip():
        texts.append(str(item.get("text") or "").strip())
text = (texts[-1] if texts else str(data.get("error_text") or "")).strip()
bad_markers = [
    "缺少的验证信息",
    "下一步建议",
    "请问",
    "?",
    "？",
    "没有足够的上下文",
    "没有足够上下文",
    "无法确定",
]
if any(marker in text for marker in bad_markers):
    raise SystemExit(f"summary reply looks like clarification or advice: {text!r}")
if "\n" in text or len(text) > 180:
    raise SystemExit(f"expected one concise summary sentence, got {text!r}")
continuity_markers = [
    "连续",
    "多轮",
    "同一会话",
    "同一个会话",
    "上下文",
    "真实客户端",
    "recent_turns",
    "memory_context",
    "conversation",
    "context",
]
if not any(marker in text for marker in continuity_markers):
    raise SystemExit(f"summary reply lost the continuous-session topic: {text!r}")
PY
}

print_log_hints() {
  local task_id="$1"
  local log_path="${ROOT_DIR}/logs/clawd.log"
  [[ -f "$log_path" ]] || return 0
  python3 - "$log_path" "$task_id" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
task_id = sys.argv[2]
lines = []
for raw in path.read_text(encoding="utf-8", errors="ignore").splitlines():
    if task_id in raw and (
        "LLM_CALL" in raw
        or "clarify_fallback_emitted" in raw
        or "route_reason" in raw
        or "context window" in raw
        or "intent_normalizer" in raw
    ):
        lines.append(raw)
for line in lines[-20:]:
    print("  [clawd-log] " + line[:1200])
PY
}

init_llm_trace_offset() {
  local offset_file="$1"
  python3 "${NL_TEST_SCRIPT_DIR}/print_llm_raw_trace.py" \
    --log "${ROOT_DIR}/logs/model_io.log" \
    --state-file "$offset_file" \
    --init-state
}

print_new_llm_trace() {
  local turn_number="$1"
  local task_id="$2"
  [[ "${PRINT_LLM_TRACE:-1}" == "1" ]] || return 0
  [[ -n "${LLM_TRACE_STATE_FILE:-}" ]] || return 0
  echo "[TURN ${turn_number}] llm_trace task_id=${task_id}"
  local trace_args=(
    --log "${ROOT_DIR}/logs/model_io.log"
    --task-id "$task_id"
    --state-file "$LLM_TRACE_STATE_FILE"
    --max-field-chars "${PRINT_LLM_TRACE_MAX_CHARS:-1200}"
  )
  if [[ -n "${CURRENT_LLM_TRACE_RESULT_FILE:-}" ]]; then
    trace_args+=(--result-file "$CURRENT_LLM_TRACE_RESULT_FILE")
  fi
  python3 "${NL_TEST_SCRIPT_DIR}/print_llm_raw_trace.py" "${trace_args[@]}"
}

annotate_turn_harness_metrics() {
  local out_file="$1"
  local wall_time_ms="$2"
  python3 - "$out_file" "$wall_time_ms" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
wall_time_ms = max(0, int(sys.argv[2]))
payload = json.loads(path.read_text(encoding="utf-8"))
payload["harness_metrics"] = {
    "schema_version": 1,
    "wall_time_ms": wall_time_ms,
}
path.write_text(json.dumps(payload, ensure_ascii=False) + "\n", encoding="utf-8")
PY
}

submit_turn() {
  local turn="$1"
  local prompt="$2"
  local out_file="$3"
  local expected_marker="${4:-}"
  local case_tags="${5:-}"
  local turn_external_chat_id="${6:-$EXTERNAL_CHAT_ID_VALUE}"
  local submit_raw task_id status text messages error
  local submit_attempt submit_status submit_extract
  local max_submit_attempts="${SUBMIT_RETRIES:-5}"
  local submit_retry_sleep_seconds="${SUBMIT_RETRY_SLEEP_SECONDS:-30}"
  local infra_retry_count="${7:-0}"
  local max_infra_retries="${LLM_INFRA_TURN_RETRIES_VALUE:-0}"
  local overall_started_ms="${8:-$(date +%s%3N)}"

  [[ "$max_submit_attempts" =~ ^[0-9]+$ ]] || max_submit_attempts=1
  [[ "$submit_retry_sleep_seconds" =~ ^[0-9]+$ ]] || submit_retry_sleep_seconds=30
  [[ "$infra_retry_count" =~ ^[0-9]+$ ]] || infra_retry_count=0
  [[ "$max_infra_retries" =~ ^[0-9]+$ ]] || max_infra_retries=0
  if [[ "$max_submit_attempts" -lt 1 ]]; then
    max_submit_attempts=1
  fi

  for ((submit_attempt = 1; submit_attempt <= max_submit_attempts; submit_attempt++)); do
    submit_raw="$(submit_client_like_task "$prompt" "" "" "$EXTERNAL_USER_ID_VALUE" "$turn_external_chat_id" "$CHANNEL_VALUE")"
    submit_status=0
    submit_extract="$(extract_submit_task_id "$submit_raw" 2>&1)" || submit_status=$?
    if [[ "$submit_status" -eq 0 ]]; then
      task_id="$submit_extract"
      break
    fi
    echo "[TURN ${turn}] submit_failed attempt=${submit_attempt}/${max_submit_attempts} status=${submit_status} error=${submit_extract}" >&2
    if [[ "$submit_attempt" -lt "$max_submit_attempts" && "${submit_raw} ${submit_extract}" == *"Rate limit"* ]]; then
      echo "[TURN ${turn}] submit_retry sleep_seconds=${submit_retry_sleep_seconds}" >&2
      sleep "$submit_retry_sleep_seconds"
      continue
    fi
    return 1
  done
  TASK_IDS+=("$task_id")
  echo "[TURN ${turn}] task_id=${task_id}"
  if [[ "$PROMPT_REPLY_ONLY" -eq 1 ]]; then
    echo "[PROMPT]"
    printf '%s\n' "$prompt"
  fi

  if ! wait_task_until_terminal_with_limit "$task_id" "$MAX_WAIT_SECONDS" > "$out_file"; then
    local final_raw final_status
    final_raw="$(query_task "$task_id" || true)"
    final_status="$(python3 - "${final_raw:-}" <<'PY'
import json
import sys

raw = (sys.argv[1] if len(sys.argv) > 1 else "").strip()
try:
    obj = json.loads(raw)
except Exception:
    print("")
    raise SystemExit(0)
data = obj.get("data") or {}
print(str(data.get("status") or "").strip())
PY
)"
    case "$final_status" in
      succeeded|failed|canceled|timeout)
        printf '%s\n' "$final_raw" > "$out_file"
        ;;
      *)
        python3 - "$task_id" > "$out_file" <<'PY'
import json
import sys

task_id = sys.argv[1]
print(json.dumps({
    "ok": True,
    "data": {
        "task_id": task_id,
        "status": "timeout",
        "result_json": {"text": "", "messages": []},
        "error_text": "poll timeout waiting for terminal task status",
    },
    "error": None,
}, ensure_ascii=False))
PY
        ;;
    esac
  fi
  local turn_finished_ms
  turn_finished_ms="$(date +%s%3N)"
  annotate_turn_harness_metrics "$out_file" "$((turn_finished_ms - overall_started_ms))"
  status="$(extract_json_field "$out_file" status)"
  text="$(extract_json_field "$out_file" text)"
  messages="$(extract_json_field "$out_file" messages)"
  error="$(extract_json_field "$out_file" error)"
  echo "[TURN ${turn}] status=${status}"
  if [[ "$PROMPT_REPLY_ONLY" -eq 1 ]]; then
    echo "[REPLY]"
    local raw_messages raw_text raw_error
    raw_messages="$(extract_json_field "$out_file" messages_raw)"
    raw_text="$(extract_json_field "$out_file" text_raw)"
    raw_error="$(extract_json_field "$out_file" error_raw)"
    if [[ -n "${raw_messages:-}" ]]; then
      printf '%s\n' "$raw_messages"
    else
      printf '%s\n' "${raw_text:-${raw_error:-<empty>}}"
    fi
  else
    echo "[TURN ${turn}] reply=${text:-${error:-<empty>}}"
    if [[ -n "${messages:-}" ]]; then
      echo "[TURN ${turn}] messages=${messages}"
    fi
  fi
  print_turn_metrics "$out_file" "$turn"
  CURRENT_LLM_TRACE_RESULT_FILE="$out_file"
  print_new_llm_trace "$turn" "$task_id"
  unset CURRENT_LLM_TRACE_RESULT_FILE

  if [[ "$status" != "succeeded" ]]; then
    echo "Turn ${turn} did not succeed: status=${status} error=${error}" >&2
    print_log_hints "$task_id" >&2
    if result_is_retryable_llm_infra_failure "$out_file" && [[ "$infra_retry_count" -lt "$max_infra_retries" ]]; then
      echo "[TURN ${turn}] retry_llm_infra retry=$((infra_retry_count + 1))/${max_infra_retries}" >&2
      submit_turn "$turn" "$prompt" "$out_file" "$expected_marker" "$case_tags" "$turn_external_chat_id" "$((infra_retry_count + 1))" "$overall_started_ms"
      return $?
    fi
    return 1
  fi
  if result_has_bad_fallback "$out_file" "$prompt"; then
    echo "Turn ${turn} returned bad fallback/unavailable text." >&2
    print_log_hints "$task_id" >&2
    if result_is_retryable_llm_infra_failure "$out_file" && [[ "$infra_retry_count" -lt "$max_infra_retries" ]]; then
      echo "[TURN ${turn}] retry_llm_infra retry=$((infra_retry_count + 1))/${max_infra_retries}" >&2
      submit_turn "$turn" "$prompt" "$out_file" "$expected_marker" "$case_tags" "$turn_external_chat_id" "$((infra_retry_count + 1))" "$overall_started_ms"
      return $?
    fi
    return 1
  fi
  if [[ "$QUALITY_GUARD" -eq 1 ]]; then
    local quality_reason quality_status
    quality_status=0
    quality_reason="$(quality_violation_reason "$out_file" "$prompt" "$expected_marker" "$case_tags" 2>&1)" || quality_status=$?
    if [[ "$quality_status" -ne 0 ]]; then
      echo "Turn ${turn} quality guard crashed." >&2
      echo "  checker_output=${quality_reason:-<empty>}" >&2
      print_log_hints "$task_id" >&2
      return 1
    fi
    if [[ "$quality_reason" != "__NO_QUALITY_VIOLATION__" ]]; then
      echo "Turn ${turn} failed quality guard: ${quality_reason}" >&2
      echo "  reply=${text:-${error:-<empty>}}" >&2
      print_log_hints "$task_id" >&2
      if result_is_retryable_llm_infra_failure "$out_file" && [[ "$infra_retry_count" -lt "$max_infra_retries" ]]; then
        echo "[TURN ${turn}] retry_llm_infra retry=$((infra_retry_count + 1))/${max_infra_retries}" >&2
        submit_turn "$turn" "$prompt" "$out_file" "$expected_marker" "$case_tags" "$turn_external_chat_id" "$((infra_retry_count + 1))" "$overall_started_ms"
        return $?
      fi
      return 1
    fi
    local skill_guard_reason
    if ! skill_guard_reason="$(assert_expected_skill_from_tags "$out_file" "$case_tags" 2>&1)"; then
      echo "Turn ${turn} failed quality guard: ${skill_guard_reason}" >&2
      echo "  reply=${text:-${error:-<empty>}}" >&2
      print_log_hints "$task_id" >&2
      return 1
    fi
  fi
  local case_tags_l
  case_tags_l=",$(printf '%s' "$case_tags" | tr '[:upper:]' '[:lower:]'),"
  if [[ -n "$expected_marker" && "$case_tags_l" == *",expect_exact_scalar,"* ]]; then
    local scalar_reason
    if ! scalar_reason="$(assert_reply_scalar_equals "$out_file" "$expected_marker" 2>&1)"; then
      echo "Turn ${turn} failed exact scalar expectation: ${scalar_reason}" >&2
      echo "  reply=${text:-${error:-<empty>}}" >&2
      print_log_hints "$task_id" >&2
      return 1
    fi
  fi
  if [[ -n "$expected_marker" ]] && ! result_text_contains "$out_file" "$expected_marker"; then
    echo "Turn ${turn} did not include expected marker: ${expected_marker}" >&2
    echo "  reply=${text:-${error:-<empty>}}" >&2
    print_log_hints "$task_id" >&2
    if result_is_retryable_llm_infra_failure "$out_file" && [[ "$infra_retry_count" -lt "$max_infra_retries" ]]; then
      echo "[TURN ${turn}] retry_llm_infra retry=$((infra_retry_count + 1))/${max_infra_retries}" >&2
      submit_turn "$turn" "$prompt" "$out_file" "$expected_marker" "$case_tags" "$turn_external_chat_id" "$((infra_retry_count + 1))" "$overall_started_ms"
      return $?
    fi
    return 1
  fi
}

load_case_rows() {
  local case_file="$1"
  local case_limit="$2"
  local case_start="$3"
  local exclude_tags="$4"
  local include_tags="$5"
  local include_groups="$6"
  local group_limit="$7"
  local include_group_context="$8"
  local include_any_tags="$9"
  python3 - "$case_file" "$case_limit" "$case_start" "$exclude_tags" "$include_tags" "$include_groups" "$group_limit" "$include_group_context" "$include_any_tags" <<'PY'
import hashlib
import re
import sys
from pathlib import Path

case_files = [Path(part) for part in sys.argv[1].split("\x1e") if part]
limit_raw = sys.argv[2].strip()
start_raw = sys.argv[3].strip()
exclude_tags = [token.strip() for token in sys.argv[4].split(",") if token.strip()]
include_tags = [token.strip() for token in sys.argv[5].split(",") if token.strip()]
include_groups = sys.argv[6].strip() in {"1", "true", "yes"}
group_limit_raw = sys.argv[7].strip()
include_group_context = sys.argv[8].strip() in {"1", "true", "yes"}
include_any_tags = [token.strip() for token in sys.argv[9].split(",") if token.strip()]
limit = int(limit_raw) if limit_raw else 0
group_limit = int(group_limit_raw) if group_limit_raw else 0
start = int(start_raw) if start_raw else 1
if start < 1:
    raise SystemExit(f"case_start must be >= 1, got {start}")
if group_limit < 0:
    raise SystemExit(f"case_group_limit must be >= 0, got {group_limit}")
seen = 0
emitted = 0
emitted_groups = set()
row_sep = "\x1f"

def split_tag_tokens(tags: str) -> list[str]:
    return [token.strip() for token in re.split(r"[,;\s]+", tags) if token.strip()]

def explicit_group_from_tags(tags: str) -> str:
    for raw in split_tag_tokens(tags):
        token = raw.strip()
        if token.startswith("group:"):
            return token[len("group:") :].strip()
        if token.startswith("group="):
            return token[len("group=") :].strip()
    return ""

def tag_tokens(tags: str) -> set[str]:
    return set(split_tag_tokens(tags))

def row_is_context_setup(tags: str) -> bool:
    tokens = tag_tokens(tags)
    return bool(tokens & {"alias", "correction", "context_setup"})

def row_matches_include_filter(tags: str) -> bool:
    tokens = tag_tokens(tags)
    if include_tags and not all(token in tokens for token in include_tags):
        return False
    if include_any_tags and not any(token in tokens for token in include_any_tags):
        return False
    return True

def row_matches_exclude_filter(tags: str) -> bool:
    tokens = tag_tokens(tags)
    return bool(exclude_tags and any(token in tokens for token in exclude_tags))

def case_group_for_name(name: str, tags: str) -> str:
    explicit = explicit_group_from_tags(tags)
    if explicit:
        return explicit
    base = name.strip() or "unnamed_case"
    stripped = re.sub(r"_turn[0-9]+$", "", base)
    return stripped or base

def group_key_for_name(name: str, tags: str) -> str:
    group = case_group_for_name(name, tags)
    safe = re.sub(r"[^A-Za-z0-9_.-]+", "-", group).strip("-")[:72]
    digest = hashlib.sha1(group.encode("utf-8")).hexdigest()[:12]
    return f"{safe or 'case'}-{digest}"

rows = []
for case_file in case_files:
    for raw in case_file.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("|", 3)
        if len(parts) < 4:
            continue
        suite, name, tags, prompt = parts
        expect = ""
        expect_marker = "|expect="
        if expect_marker in prompt:
            prompt, expect = prompt.rsplit(expect_marker, 1)
            expect = expect.strip()
        seen += 1
        group_key = group_key_for_name(name, tags)
        rows.append({
            "seen": seen,
            "suite": suite,
            "name": name,
            "tags": tags,
            "prompt": prompt,
            "expect": expect,
            "group_key": group_key,
        })

selected_groups = set()
matched_group_max_seen = {}
if (include_tags or include_any_tags) and (include_groups or include_group_context):
    for row in rows:
        if row["seen"] < start:
            continue
        tags = row["tags"]
        if row_matches_exclude_filter(tags):
            continue
        if row_matches_include_filter(tags):
            group_key = row["group_key"]
            selected_groups.add(group_key)
            matched_group_max_seen[group_key] = max(
                row["seen"], matched_group_max_seen.get(group_key, 0)
            )

for row in rows:
    if row["seen"] < start:
        continue
    suite = row["suite"]
    name = row["name"]
    tags = row["tags"]
    prompt = row["prompt"]
    expect = row["expect"]
    group_key = row["group_key"]
    if row_matches_exclude_filter(tags):
        continue
    if include_tags or include_any_tags:
        if include_groups:
            if group_key not in selected_groups:
                continue
        elif include_group_context:
            if group_key not in selected_groups:
                continue
            row_matches = row_matches_include_filter(tags)
            context_setup = row_is_context_setup(tags) and row["seen"] <= matched_group_max_seen.get(group_key, 0)
            if not row_matches and not context_setup:
                continue
        elif not row_matches_include_filter(tags):
            continue
    if group_limit and group_key not in emitted_groups:
        if len(emitted_groups) >= group_limit:
            break
        emitted_groups.add(group_key)
    emitted += 1
    emitted_tags = ",".join(part for part in [f"suite:{suite.strip()}", tags] if part)
    print(row_sep.join([
        str(row["seen"]),
        group_key,
        name.replace("\t", " ").replace(row_sep, " "),
        emitted_tags.replace("\t", " ").replace(row_sep, " "),
        prompt.replace("\t", " ").replace(row_sep, " "),
        expect.replace("\t", " ").replace(row_sep, " "),
    ]))
    if not group_limit and limit and emitted >= limit:
        break
PY
}

load_case_rows_jsonl() {
  local case_jsonl="$1"
  local case_limit="$2"
  local case_start="$3"
  local exclude_tags="$4"
  local include_tags="$5"
  local include_groups="$6"
  local group_limit="$7"
  local include_group_context="$8"
  local include_any_tags="$9"
  python3 - "$case_jsonl" "$case_limit" "$case_start" "$exclude_tags" "$include_tags" "$include_groups" "$group_limit" "$include_group_context" "$include_any_tags" <<'PY'
import hashlib
import json
import re
import sys
from pathlib import Path

case_file = Path(sys.argv[1])
limit_raw = sys.argv[2].strip()
start_raw = sys.argv[3].strip()
exclude_tags = [token.strip() for token in sys.argv[4].split(",") if token.strip()]
include_tags = [token.strip() for token in sys.argv[5].split(",") if token.strip()]
include_groups = sys.argv[6].strip() in {"1", "true", "yes"}
group_limit_raw = sys.argv[7].strip()
include_group_context = sys.argv[8].strip() in {"1", "true", "yes"}
include_any_tags = [token.strip() for token in sys.argv[9].split(",") if token.strip()]
limit = int(limit_raw) if limit_raw else 0
group_limit = int(group_limit_raw) if group_limit_raw else 0
start = int(start_raw) if start_raw else 1
if start < 1:
    raise SystemExit(f"case_start must be >= 1, got {start}")
if group_limit < 0:
    raise SystemExit(f"case_group_limit must be >= 0, got {group_limit}")
seen = 0
emitted = 0
emitted_groups = set()
row_sep = "\x1f"

def split_tag_tokens(tags: str) -> list[str]:
    return [token.strip() for token in re.split(r"[,;\s]+", tags) if token.strip()]

def explicit_group_from_tags(tags: str) -> str:
    for raw in split_tag_tokens(tags):
        token = raw.strip()
        if token.startswith("group:"):
            return token[len("group:") :].strip()
        if token.startswith("group="):
            return token[len("group=") :].strip()
    return ""

def tag_tokens(tags: str) -> set[str]:
    return set(split_tag_tokens(tags))

def row_is_context_setup(tags: str) -> bool:
    tokens = tag_tokens(tags)
    return bool(tokens & {"alias", "correction", "context_setup"})

def row_matches_include_filter(tags: str) -> bool:
    tokens = tag_tokens(tags)
    if include_tags and not all(token in tokens for token in include_tags):
        return False
    if include_any_tags and not any(token in tokens for token in include_any_tags):
        return False
    return True

def row_matches_exclude_filter(tags: str) -> bool:
    tokens = tag_tokens(tags)
    return bool(exclude_tags and any(token in tokens for token in exclude_tags))

def group_key_for_name(name: str, tags: str) -> str:
    explicit = explicit_group_from_tags(tags)
    base = name.strip() or "unnamed_case"
    stripped = explicit or re.sub(r"_turn[0-9]+$", "", base) or base
    safe = re.sub(r"[^A-Za-z0-9_.-]+", "-", stripped).strip("-")[:72]
    digest = hashlib.sha1(stripped.encode("utf-8")).hexdigest()[:12]
    return f"{safe or 'case'}-{digest}"

rows = []
for lineno, raw in enumerate(case_file.read_text(encoding="utf-8").splitlines(), 1):
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    row = json.loads(line)
    prompt = str(row.get("prompt") or "").strip()
    if not prompt:
        raise SystemExit(f"{case_file}:{lineno}: JSONL row missing prompt")
    suite = str(row.get("suite") or "jsonl")
    name = str(row.get("name") or f"case_{seen + 1}")
    tags = row.get("tags") or ""
    if isinstance(tags, list):
        tags = ",".join(str(item) for item in tags)
    else:
        tags = str(tags)
    expect = row.get("expect") or ""
    seen += 1
    group_key = group_key_for_name(name, tags)
    rows.append({
        "seen": seen,
        "suite": suite,
        "name": name,
        "tags": tags,
        "prompt": prompt,
        "expect": str(expect),
        "group_key": group_key,
    })

selected_groups = set()
matched_group_max_seen = {}
if (include_tags or include_any_tags) and (include_groups or include_group_context):
    for row in rows:
        if row["seen"] < start:
            continue
        tags = row["tags"]
        if row_matches_exclude_filter(tags):
            continue
        if row_matches_include_filter(tags):
            group_key = row["group_key"]
            selected_groups.add(group_key)
            matched_group_max_seen[group_key] = max(
                row["seen"], matched_group_max_seen.get(group_key, 0)
            )

for row in rows:
    if row["seen"] < start:
        continue
    tags = row["tags"]
    if row_matches_exclude_filter(tags):
        continue
    if include_tags or include_any_tags:
        if include_groups:
            if row["group_key"] not in selected_groups:
                continue
        elif include_group_context:
            group_key = row["group_key"]
            if group_key not in selected_groups:
                continue
            row_matches = row_matches_include_filter(tags)
            context_setup = row_is_context_setup(tags) and row["seen"] <= matched_group_max_seen.get(group_key, 0)
            if not row_matches and not context_setup:
                continue
        elif not row_matches_include_filter(tags):
            continue
    if group_limit and row["group_key"] not in emitted_groups:
        if len(emitted_groups) >= group_limit:
            break
        emitted_groups.add(row["group_key"])
    emitted += 1
    emitted_tags = ",".join(part for part in [f"suite:{row['suite'].strip()}", tags] if part)
    print(row_sep.join([
        str(row["seen"]),
        row["group_key"],
        row["name"].replace("\t", " ").replace(row_sep, " "),
        emitted_tags.replace("\t", " ").replace(row_sep, " "),
        json.dumps(row["prompt"], ensure_ascii=False),
        json.dumps(row["expect"], ensure_ascii=False),
    ]))
    if not group_limit and limit and emitted >= limit:
        break
PY
}

json_decode_arg() {
  python3 - "$1" <<'PY'
import json
import sys
print(json.loads(sys.argv[1]), end="")
PY
}

verify_db_state() {
  local require_test_id_memory="${1:-1}"
  python3 - "$DB_PATH_VALUE" "$TEST_ID" "$require_test_id_memory" "$CHANNEL_VALUE" "${TASK_IDS[@]}" <<'PY'
import sqlite3
import sys
from pathlib import Path

db_path = Path(sys.argv[1])
test_id = sys.argv[2]
require_test_id_memory = sys.argv[3] == "1"
expected_channel = sys.argv[4]
task_ids = sys.argv[5:]
if not db_path.exists():
    raise SystemExit(f"database not found: {db_path}")
if not task_ids:
    raise SystemExit("no task ids to verify")

conn = sqlite3.connect(db_path)
conn.row_factory = sqlite3.Row
placeholders = ",".join("?" for _ in task_ids)
rows = conn.execute(
    f"SELECT task_id, user_id, chat_id, user_key, channel, external_chat_id, status FROM tasks WHERE task_id IN ({placeholders})",
    task_ids,
).fetchall()
if len(rows) != len(task_ids):
    found = {row["task_id"] for row in rows}
    missing = [tid for tid in task_ids if tid not in found]
    raise SystemExit(f"missing task rows: {missing}")

first = rows[0]
user_id = first["user_id"]
user_key = str(first["user_key"] or "")
if any(row["channel"] != expected_channel for row in rows):
    channels = sorted({str(row["channel"]) for row in rows})
    raise SystemExit(f"expected {expected_channel} channel, got {channels}")

conversation_keys = sorted({(row["user_id"], row["chat_id"]) for row in rows})
for row in rows:
    if row["user_id"] != user_id:
        raise SystemExit("turns did not land under the same effective user")

def count(sql, params=()):
    return int(conn.execute(sql, params).fetchone()[0])

total_tasks_count = 0
total_memories_count = 0
total_conversation_states_count = 0
total_retrieval_count = 0
total_long_term_count = 0
total_preference_count = 0
effective_chat_ids = []
for group_user_id, group_chat_id in conversation_keys:
    tasks_count = count(
        "SELECT COUNT(*) FROM tasks WHERE chat_id = ? AND user_id = ?",
        (group_chat_id, group_user_id),
    )
    memories_count = count(
        "SELECT COUNT(*) FROM memories WHERE chat_id = ? AND user_id = ?",
        (group_chat_id, group_user_id),
    )
    conversation_states_count = count(
        "SELECT COUNT(*) FROM conversation_states WHERE chat_id = ? AND user_id = ?",
        (group_chat_id, group_user_id),
    )
    retrieval_count = count(
        "SELECT COUNT(*) FROM memory_retrieval_index WHERE chat_id = ? AND user_id = ?",
        (group_chat_id, group_user_id),
    )
    long_term_count = count(
        "SELECT COUNT(*) FROM long_term_memories WHERE chat_id = ? AND user_id = ?",
        (group_chat_id, group_user_id),
    )
    preference_count = count(
        "SELECT COUNT(*) FROM user_preferences WHERE chat_id = ? AND user_id = ?",
        (group_chat_id, group_user_id),
    )
    require_group_memory = require_test_id_memory or tasks_count > 1
    if require_group_memory and memories_count <= 0:
        raise SystemExit(f"expected memories for effective_chat_id={group_chat_id}")
    if require_group_memory and conversation_states_count <= 0:
        raise SystemExit(f"expected conversation_states for effective_chat_id={group_chat_id}")
    total_tasks_count += tasks_count
    total_memories_count += memories_count
    total_conversation_states_count += conversation_states_count
    total_retrieval_count += retrieval_count
    total_long_term_count += long_term_count
    total_preference_count += preference_count
    effective_chat_ids.append(str(group_chat_id))

test_id_memory_count = 0
if require_test_id_memory:
    test_id_memory_count = sum(
        count(
            "SELECT COUNT(*) FROM memories WHERE chat_id = ? AND user_id = ? AND content LIKE ?",
            (group_chat_id, group_user_id, f"%{test_id}%"),
        )
        for group_user_id, group_chat_id in conversation_keys
    )
    if test_id_memory_count <= 0:
        raise SystemExit("expected short-term memory to contain the suite test id")
execution_summary_leak_count = sum(
    count(
        "SELECT COUNT(*) FROM memories WHERE chat_id = ? AND user_id = ? AND content LIKE ?",
        (group_chat_id, group_user_id, "%**执行过程**%"),
    )
    for group_user_id, group_chat_id in conversation_keys
)
if execution_summary_leak_count > 0:
    raise SystemExit("execution summary leaked into short-term memory")

print(
    "DB_VERIFY_OK "
    f"effective_user_id={user_id} effective_chat_groups={len(conversation_keys)} "
    f"effective_chat_ids={','.join(effective_chat_ids[:8])}{'...' if len(effective_chat_ids) > 8 else ''} "
    f"user_key_present={bool(user_key)} "
    f"tasks={total_tasks_count} memories={total_memories_count} "
    f"conversation_states={total_conversation_states_count} retrieval_index={total_retrieval_count} "
    f"long_term={total_long_term_count} preferences={total_preference_count} "
    f"test_id_memory_checked={require_test_id_memory} test_id_memory_count={test_id_memory_count}"
)
PY
}

print_prompt_budget_report() {
  local run_dir="$1"
  python3 - "$run_dir" <<'PY'
import json
import sys
from pathlib import Path

run_dir = Path(sys.argv[1])
labels = {}
files = []

def result_json_from_response(obj):
    data = obj.get("data") or {}
    result = data.get("result_json") or {}
    if isinstance(result, str):
        try:
            result = json.loads(result)
        except Exception:
            result = {}
    return result if isinstance(result, dict) else {}

for path in sorted(run_dir.glob("turn*.json")):
    try:
        obj = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        continue
    result = result_json_from_response(obj)
    journal = result.get("task_journal") or {}
    summary = journal.get("summary") or {}
    metrics = summary.get("task_metrics") or {}
    by_prompt = metrics.get("by_prompt") or {}
    file_count = 0
    if isinstance(by_prompt, dict):
        for label, bucket in by_prompt.items():
            if not isinstance(bucket, dict):
                continue
            count = int(bucket.get("prompt_truncation_count") or 0)
            if count <= 0:
                continue
            labels[label] = labels.get(label, 0) + count
            file_count += count
    if file_count:
        files.append(f"{path.name}:{file_count}")

total = sum(labels.values())
if total:
    label_text = ",".join(f"{label}:{count}" for label, count in sorted(labels.items()))
    file_text = ",".join(files[:12])
    if len(files) > 12:
        file_text += f",...(+{len(files) - 12})"
    print(f"PROMPT_BUDGET_RISK prompt_truncations={total} labels={label_text} files={file_text}")
else:
    print("PROMPT_BUDGET_OK prompt_truncations=0")
PY
}

print_turn_metrics() {
  local out_file="$1"
  local turn="$2"
  python3 - "$out_file" "$turn" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
turn = sys.argv[2]

def result_json_from_response(obj):
    data = obj.get("data") or {}
    result = data.get("result_json") or {}
    if isinstance(result, str):
        try:
            result = json.loads(result)
        except Exception:
            result = {}
    return result if isinstance(result, dict) else {}

try:
    obj = json.loads(path.read_text(encoding="utf-8"))
except Exception:
    print(f"[TURN {turn}] metrics unavailable=parse_error")
    raise SystemExit(0)

result = result_json_from_response(obj)
journal = result.get("task_journal") or {}
summary = journal.get("summary") or {}
metrics = summary.get("task_metrics") or {}
by_prompt = metrics.get("by_prompt") or {}
prompt_parts = []
if isinstance(by_prompt, dict):
    for label, bucket in sorted(by_prompt.items()):
        if not isinstance(bucket, dict):
            continue
        count = int(bucket.get("count") or 0)
        elapsed = int(bucket.get("elapsed_ms") or 0)
        trunc = int(bucket.get("prompt_truncation_count") or 0)
        prompt_parts.append(f"{label}:{count}/{elapsed}ms/trunc={trunc}")

llm_calls = metrics.get("llm_calls_per_task")
llm_elapsed = metrics.get("llm_elapsed_ms_per_task")
prompt_truncations = metrics.get("prompt_truncation_count")
round_count = summary.get("round_count")
step_count = summary.get("step_count")
final_status = summary.get("final_status") or ""
prompt_text = ",".join(prompt_parts) if prompt_parts else "none"
print(
    f"[TURN {turn}] metrics "
    f"llm_calls={llm_calls if llm_calls is not None else 'n/a'} "
    f"llm_elapsed_ms={llm_elapsed if llm_elapsed is not None else 'n/a'} "
    f"rounds={round_count if round_count is not None else 'n/a'} "
    f"steps={step_count if step_count is not None else 'n/a'} "
    f"prompt_truncations={prompt_truncations if prompt_truncations is not None else 'n/a'} "
    f"final_status={final_status or 'n/a'} "
    f"by_prompt={prompt_text}"
)
PY
}

print_llm_metrics_report() {
  local run_dir="$1"
  python3 - "$run_dir" <<'PY'
import json
import sys
from pathlib import Path

run_dir = Path(sys.argv[1])
total_calls = 0
total_elapsed = 0
total_rounds = 0
total_steps = 0
total_truncations = 0
turns_with_metrics = 0
max_calls = None
max_elapsed = None
slow_files = []

def result_json_from_response(obj):
    data = obj.get("data") or {}
    result = data.get("result_json") or {}
    if isinstance(result, str):
        try:
            result = json.loads(result)
        except Exception:
            result = {}
    return result if isinstance(result, dict) else {}

for path in sorted(run_dir.glob("turn*.json")):
    try:
        obj = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        continue
    result = result_json_from_response(obj)
    journal = result.get("task_journal") or {}
    summary = journal.get("summary") or {}
    metrics = summary.get("task_metrics") or {}
    if not isinstance(metrics, dict) or not metrics:
        continue
    calls = metrics.get("llm_calls_per_task")
    elapsed = metrics.get("llm_elapsed_ms_per_task")
    rounds = summary.get("round_count")
    steps = summary.get("step_count")
    truncations = metrics.get("prompt_truncation_count")
    if calls is None and elapsed is None and rounds is None and steps is None:
        continue
    turns_with_metrics += 1
    calls_i = int(calls or 0)
    elapsed_i = int(elapsed or 0)
    rounds_i = int(rounds or 0)
    steps_i = int(steps or 0)
    trunc_i = int(truncations or 0)
    total_calls += calls_i
    total_elapsed += elapsed_i
    total_rounds += rounds_i
    total_steps += steps_i
    total_truncations += trunc_i
    if max_calls is None or calls_i > max_calls[1]:
        max_calls = (path.name, calls_i)
    if max_elapsed is None or elapsed_i > max_elapsed[1]:
        max_elapsed = (path.name, elapsed_i)
    if calls_i >= 5 or elapsed_i >= 60000 or rounds_i >= 2:
        slow_files.append(f"{path.name}:calls={calls_i},elapsed_ms={elapsed_i},rounds={rounds_i}")

if turns_with_metrics <= 0:
    print("LLM_METRICS unavailable")
else:
    slow_text = ",".join(slow_files[:10])
    if len(slow_files) > 10:
        slow_text += f",...(+{len(slow_files) - 10})"
    print(
        "LLM_METRICS "
        f"turns={turns_with_metrics} total_calls={total_calls} total_elapsed_ms={total_elapsed} "
        f"avg_calls={total_calls / turns_with_metrics:.2f} avg_elapsed_ms={total_elapsed / turns_with_metrics:.0f} "
        f"total_rounds={total_rounds} total_steps={total_steps} prompt_truncations={total_truncations} "
        f"max_calls={max_calls[0]}:{max_calls[1]} "
        f"max_elapsed={max_elapsed[0]}:{max_elapsed[1]} "
        f"heavy_turns={slow_text or 'none'}"
    )
PY
}

resolve_admin_key

if [[ -z "${EXTERNAL_USER_ID_VALUE:-}" ]]; then
  EXTERNAL_USER_ID_VALUE="$USER_ID_VALUE"
fi
if [[ -z "${EXTERNAL_CHAT_ID_VALUE:-}" ]]; then
  # Each suite run gets a fresh synthetic Telegram chat scope, while all turns within the run share it.
  EXTERNAL_CHAT_ID_VALUE="${CHAT_ID_VALUE}-${RUN_STAMP}"
fi

BASE_URL="$BASE_URL_VALUE"
USER_ID="$USER_ID_VALUE"
CHAT_ID="$CHAT_ID_VALUE"
USER_KEY="$USER_KEY_VALUE"
MAX_WAIT_SECONDS="$WAIT_SECONDS_VALUE"
POLL_INTERVAL_SECONDS="$POLL_SECONDS_VALUE"
PRINT_LLM_TRACE="$PRINT_LLM_TRACE_VALUE"
PRINT_LLM_TRACE_MAX_CHARS="$(normalize_llm_trace_max_chars "$PRINT_LLM_TRACE_MAX_CHARS_VALUE")"
DB_PATH_VALUE="$(resolve_db_path)"
if [[ "${#CASE_FILE_VALUES[@]}" -gt 0 && -n "${CASE_JSONL_VALUE:-}" ]]; then
  echo "Use only one of --case-file or --case-jsonl." >&2
  exit 2
fi

if [[ "${#CASE_FILE_VALUES[@]}" -gt 0 ]]; then
  resolved_case_files=()
  for case_file_path in "${CASE_FILE_VALUES[@]}"; do
    if [[ ! -f "$case_file_path" ]]; then
      echo "Case file not found: $case_file_path" >&2
      exit 2
    fi
    resolved_case_files+=("$(python3 - "$case_file_path" <<'PY'
from pathlib import Path
import sys
print(Path(sys.argv[1]).resolve())
PY
    )")
  done
  CASE_FILE_VALUES=("${resolved_case_files[@]}")
  CASE_FILE_VALUE="$(join_case_files "," "${CASE_FILE_VALUES[@]}")"
  CASE_FILE_LOADER_VALUE="$(join_case_files $'\x1e' "${CASE_FILE_VALUES[@]}")"
fi
if [[ -n "${CASE_JSONL_VALUE:-}" ]]; then
  if [[ ! -f "$CASE_JSONL_VALUE" ]]; then
    echo "Case JSONL not found: $CASE_JSONL_VALUE" >&2
    exit 2
  fi
  CASE_JSONL_VALUE="$(python3 - "$CASE_JSONL_VALUE" <<'PY'
from pathlib import Path
import sys
print(Path(sys.argv[1]).resolve())
PY
)"
fi

health_check || {
  echo "clawd is not healthy at ${BASE_URL}. Start clawd first or pass --base-url." >&2
  exit 2
}

mkdir -p "$LOG_ROOT"
RUN_DIR="${LOG_ROOT%/}/run_${RUN_STAMP}"
mkdir -p "$RUN_DIR"
TASK_IDS=()

echo "CLIENT_LIKE_CONTINUOUS_SUITE"
echo "base_url=${BASE_URL}"
echo "channel=${CHANNEL_VALUE}"
echo "db_path_ref=$(path_ref "$DB_PATH_VALUE")"
echo "raw_user_id=${USER_ID}"
echo "raw_chat_id=${CHAT_ID}"
echo "external_user_id=${EXTERNAL_USER_ID_VALUE}"
echo "external_chat_id=${EXTERNAL_CHAT_ID_VALUE}"
echo "test_id=${TEST_ID}"
echo "log_dir=$(path_ref "$RUN_DIR")"
if [[ -n "${CASE_FILE_VALUE:-}" ]]; then
  echo "case_file_ref=$(path_ref "$CASE_FILE_VALUE")"
else
  echo "case_file_ref=none"
fi
if [[ -n "${CASE_JSONL_VALUE:-}" ]]; then
  echo "case_jsonl_ref=$(path_ref "$CASE_JSONL_VALUE")"
else
  echo "case_jsonl_ref=none"
fi
echo "case_limit=${CASE_LIMIT_VALUE:-<none>}"
echo "case_group_limit=${CASE_GROUP_LIMIT_VALUE:-<none>}"
echo "case_start=${CASE_START_VALUE:-1}"
echo "include_case_tags=${CASE_INCLUDE_TAGS_VALUE:-<none>}"
echo "include_case_any_tags=${CASE_INCLUDE_ANY_TAGS_VALUE:-<none>}"
echo "include_case_groups=${CASE_INCLUDE_GROUPS_VALUE}"
echo "include_case_group_context=${CASE_INCLUDE_GROUP_CONTEXT_VALUE}"
echo "exclude_case_tags=${CASE_EXCLUDE_TAGS_VALUE:-<none>}"
echo "case_group_isolation=${CASE_GROUP_ISOLATION}"
echo "quality_guard=${QUALITY_GUARD}"
echo "llm_infra_turn_retries=${LLM_INFRA_TURN_RETRIES_VALUE}"
echo "print_llm_trace=${PRINT_LLM_TRACE}"
echo "llm_trace_max_chars=${PRINT_LLM_TRACE_MAX_CHARS}"

LLM_TRACE_STATE_FILE="${RUN_DIR}/llm_trace_state.json"
if [[ "${PRINT_LLM_TRACE}" == "1" ]]; then
  init_llm_trace_offset "$LLM_TRACE_STATE_FILE"
fi

read -r -d '' HEAVY_CONTEXT_PROMPT <<'EOF' || true
请记住下面这段较长的上下文，后续我会基于它继续问问题。不要执行外部工具，只需要用中文确认已收到。

项目背景：RustClaw 是一个多渠道 agent 控制台，非技术用户会通过 Telegram、网页、微信、飞书等渠道连续交互。真实客户端不会每条消息都换 chat，而是在同一个会话里不断累积任务、短期记忆、长期摘要、最近执行记录和澄清状态。测试必须模拟这种连续会话，否则空库短句测试无法发现 intent normalizer 在长上下文下超过模型窗口的问题。

验证目标：连续消息应落入同一个 effective_chat_id；后续 ask 应能读取前序消息形成的 recent_turns_full、last_turn_full、memory_context；即使上下文变长，也不应返回“模型暂时不可用”或“我没看出这条消息要做什么”。
EOF

PROMPTS=(
  "你好，我正在做 RustClaw 的真实客户端连续会话测试，请用一句中文回复确认。"
  "请记住测试编号 ${TEST_ID}，后续我会问你这个编号。"
  "$HEAVY_CONTEXT_PROMPT"
  "刚才我让你记住的测试编号是什么？只回答编号。"
  "请用一句话总结这个连续会话测试主要验证什么。"
)
EXPECTED_MARKERS=(
  ""
  ""
  ""
  "$TEST_ID"
  ""
)

turn=0
if [[ "$RUN_BUILTIN_SMOKE" -eq 1 ]]; then
  for idx in "${!PROMPTS[@]}"; do
    turn=$((turn + 1))
    submit_turn "$turn" "${PROMPTS[$idx]}" "${RUN_DIR}/turn_${turn}.json" "${EXPECTED_MARKERS[$idx]}"
    if [[ "$turn" -eq 4 ]]; then
      assert_reply_scalar_equals "${RUN_DIR}/turn_${turn}.json" "$TEST_ID"
    elif [[ "$turn" -eq 5 ]]; then
      assert_reply_concise_summary "${RUN_DIR}/turn_${turn}.json"
    fi
  done
fi

if [[ "${#CASE_FILE_VALUES[@]}" -gt 0 || -n "${CASE_JSONL_VALUE:-}" ]]; then
  if [[ -n "${CASE_JSONL_VALUE:-}" ]]; then
    case_row_loader=(load_case_rows_jsonl "$CASE_JSONL_VALUE" "$CASE_LIMIT_VALUE" "$CASE_START_VALUE" "$CASE_EXCLUDE_TAGS_VALUE" "$CASE_INCLUDE_TAGS_VALUE" "$CASE_INCLUDE_GROUPS_VALUE" "$CASE_GROUP_LIMIT_VALUE" "$CASE_INCLUDE_GROUP_CONTEXT_VALUE" "$CASE_INCLUDE_ANY_TAGS_VALUE")
    resume_case_arg="--case-jsonl ${CASE_JSONL_VALUE}"
  else
    case_row_loader=(load_case_rows "$CASE_FILE_LOADER_VALUE" "$CASE_LIMIT_VALUE" "$CASE_START_VALUE" "$CASE_EXCLUDE_TAGS_VALUE" "$CASE_INCLUDE_TAGS_VALUE" "$CASE_INCLUDE_GROUPS_VALUE" "$CASE_GROUP_LIMIT_VALUE" "$CASE_INCLUDE_GROUP_CONTEXT_VALUE" "$CASE_INCLUDE_ANY_TAGS_VALUE")
    resume_case_arg=""
    for case_file_path in "${CASE_FILE_VALUES[@]}"; do
      resume_case_arg="${resume_case_arg} --case-file ${case_file_path}"
    done
    resume_case_arg="${resume_case_arg# }"
  fi
  while IFS=$'\x1f' read -r case_index case_group_key case_name case_tags case_prompt case_expect; do
    [[ -n "${case_index:-}" ]] || continue
    if [[ -n "${CASE_JSONL_VALUE:-}" ]]; then
      case_prompt="$(json_decode_arg "$case_prompt")"
      case_expect="$(json_decode_arg "$case_expect")"
    fi
    turn=$((turn + 1))
    case_external_chat_id="$EXTERNAL_CHAT_ID_VALUE"
    if [[ "$CASE_GROUP_ISOLATION" -eq 1 ]]; then
      case_external_chat_id="${EXTERNAL_CHAT_ID_VALUE}--${case_group_key}"
    fi
    if [[ "$case_tags" == *"skill:make_dir"* && "$case_name" == *"builtin_make_dir_smoke"* ]]; then
      rmdir "${ROOT_DIR}/document/nl_skill_tmp" 2>/dev/null || true
    fi
    echo "[CASE ${case_index}] name=${case_name} group=${case_group_key} external_chat_id=${case_external_chat_id}"
    if ! submit_turn "$turn" "$case_prompt" "${RUN_DIR}/turn_${turn}_case_${case_index}.json" "${case_expect:-}" "${case_tags:-}" "$case_external_chat_id"; then
      quality_guard_arg=""
      if [[ "$QUALITY_GUARD" -eq 1 ]]; then
        quality_guard_arg=" --quality-guard"
      fi
      shared_chat_arg=""
      if [[ "$CASE_GROUP_ISOLATION" -eq 0 ]]; then
        shared_chat_arg=" --shared-case-chat"
      fi
      include_tag_args=""
      if [[ -n "${CASE_INCLUDE_TAGS_VALUE:-}" ]]; then
        IFS=',' read -r -a include_tag_parts <<<"${CASE_INCLUDE_TAGS_VALUE}"
        for tag in "${include_tag_parts[@]}"; do
          [[ -n "${tag:-}" ]] && include_tag_args="${include_tag_args} --include-case-tag ${tag}"
        done
      fi
      include_any_tag_args=""
      if [[ -n "${CASE_INCLUDE_ANY_TAGS_VALUE:-}" ]]; then
        IFS=',' read -r -a include_any_tag_parts <<<"${CASE_INCLUDE_ANY_TAGS_VALUE}"
        for tag in "${include_any_tag_parts[@]}"; do
          [[ -n "${tag:-}" ]] && include_any_tag_args="${include_any_tag_args} --include-case-tag-any ${tag}"
        done
      fi
      group_filter_args=""
      if [[ "$CASE_INCLUDE_GROUPS_VALUE" -eq 1 ]]; then
        group_filter_args="${group_filter_args} --include-case-groups"
      fi
      if [[ "$CASE_INCLUDE_GROUP_CONTEXT_VALUE" -eq 1 ]]; then
        group_filter_args="${group_filter_args} --include-case-group-context"
      fi
      if [[ -n "${CASE_GROUP_LIMIT_VALUE:-}" ]]; then
        group_filter_args="${group_filter_args} --case-group-limit ${CASE_GROUP_LIMIT_VALUE}"
      fi
      echo "RESUME_HINT bash scripts/nl_tests/run_client_like_continuous_suite.sh ${resume_case_arg} --case-start ${case_index} --skip-smoke --external-user-id ${EXTERNAL_USER_ID_VALUE} --external-chat-id ${EXTERNAL_CHAT_ID_VALUE} --prompt-reply-only${quality_guard_arg}${shared_chat_arg}${include_tag_args}${include_any_tag_args}${group_filter_args}" >&2
      exit 1
    fi
  done < <("${case_row_loader[@]}")
fi

if [[ "$turn" -eq 0 ]]; then
  echo "No turns were run. Remove --skip-smoke or pass --case-file/--case-jsonl/--full-nl." >&2
  exit 2
fi

if [[ "$RUN_BUILTIN_SMOKE" -eq 1 ]]; then
  verify_db_state 1
else
  verify_db_state 0
fi

python3 "${ROOT_DIR}/scripts/nl_tests/tag_run_outcomes.py" "$RUN_DIR"
if [[ "$QUALITY_GUARD" -eq 1 ]]; then
  python3 - "$RUN_DIR/attribution.jsonl" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
bad = []
if path.exists():
    with path.open("r", encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            if row.get("attribution") == "verifier_should_retry_not_applied":
                bad.append(row)
if bad:
    print(
        "QUALITY_GUARD_FAIL verifier_should_retry_not_applied="
        f"{len(bad)} first={bad[0].get('file')} reason={bad[0].get('reason')}",
        file=sys.stderr,
    )
    sys.exit(1)
PY
fi
print_prompt_budget_report "$RUN_DIR"
print_llm_metrics_report "$RUN_DIR"

echo "CLIENT_LIKE_CONTINUOUS_SUITE_OK turns=${turn} log_dir=$(path_ref "$RUN_DIR")"

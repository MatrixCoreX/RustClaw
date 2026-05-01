#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/lib.sh"

BASE_URL_VALUE="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID_VALUE="${USER_ID:-2403753217836067397}"
CHAT_ID_VALUE="${CHAT_ID:--1002403753217}"
EXTERNAL_CHAT_ID_VALUE="${EXTERNAL_CHAT_ID:-}"
EXTERNAL_USER_ID_VALUE="${EXTERNAL_USER_ID:-}"
USER_KEY_VALUE="${RUSTCLAW_USER_KEY:-${USER_KEY:-}}"
CONFIG_PATH_VALUE="${RUSTCLAW_CONFIG_PATH:-${ROOT_DIR}/configs/config.toml}"
DB_PATH_VALUE="${RUSTCLAW_DB_PATH:-}"
WAIT_SECONDS_VALUE="${MAX_WAIT_SECONDS:-240}"
POLL_SECONDS_VALUE="${POLL_INTERVAL_SECONDS:-1}"
LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/client_like_continuous"
PROMPT_REPLY_ONLY=1
QUALITY_GUARD=0
CASE_FILE_VALUE=""
CASE_LIMIT_VALUE=""
CASE_START_VALUE="${CASE_START:-1}"
RUN_BUILTIN_SMOKE=1
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
TEST_ID="${CLIENT_LIKE_TEST_ID:-client-like-continuous-${RUN_STAMP}}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_client_like_continuous_suite.sh [options]

What it tests:
  Directly POSTs /v1/tasks to clawd, but uses the same request shape as telegramd:
  channel=telegram, stable user_id/chat_id, external ids, user_key, and payload.agent_mode=true.
  Multiple turns reuse one client identity so clawd accumulates tasks, conversation state, and memory.

Options:
  --base-url URL             clawd base URL. Default: http://127.0.0.1:8787
  --user-id ID               RustClaw/Telegram-side user id. Default: deterministic large id
  --chat-id ID               Telegram raw chat id. Default: deterministic negative group id
  --external-user-id ID      Telegramd-compatible external_user_id. Default: user-id
  --external-chat-id ID      Telegramd-compatible external_chat_id. Default: chat-id
  --user-key KEY             RustClaw user key. Default: RUSTCLAW_USER_KEY/USER_KEY or first enabled admin key
  --config PATH              config.toml used to resolve DB path for assertions
  --db-path PATH             main SQLite DB path for assertions
  --wait-seconds N           max wait per turn. Default: 240
  --poll-seconds N           poll interval seconds. Default: 1
  --log-root PATH            log output root
  --case-file PATH           append prompts from a case file into the same client-like conversation
  --full-nl                  shorthand for --case-file scripts/nl_tests/cases/nl_cases_full.txt
  --case-limit N             max appended cases from --case-file/--full-nl
  --case-start N             start from the Nth appended case. Use with --skip-smoke and the same
                             --external-chat-id/--external-user-id to resume after provider failure.
  --skip-smoke               run only the case file prompts, without the built-in 5-turn memory smoke
  --prompt-reply-only        print only prompt and reply snippets. Default: on
  --verbose-turn-output      print compact turn status/reply fields instead of prompt/reply blocks
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
    --log-root)
      LOG_ROOT="${2:-}"
      shift 2
      ;;
    --case-file)
      CASE_FILE_VALUE="${2:-}"
      shift 2
      ;;
    --full-nl)
      CASE_FILE_VALUE="${ROOT_DIR}/scripts/nl_tests/cases/nl_cases_full.txt"
      shift
      ;;
    --case-limit)
      CASE_LIMIT_VALUE="${2:-}"
      shift 2
      ;;
    --case-start)
      CASE_START_VALUE="${2:-}"
      shift 2
      ;;
    --skip-smoke)
      RUN_BUILTIN_SMOKE=0
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
for item in result.get("messages") or []:
    if isinstance(item, str):
        texts.append(item)
    elif isinstance(item, dict):
        texts.append(str(item.get("text") or ""))
joined = "\n".join(texts)
raise SystemExit(0 if expected in joined else 1)
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
    "http 400",
]
soft_markers = [
    "模型暂时不可用",
    "当前大模型服务暂时不可用",
    "model is temporarily unavailable",
    "temporarily unavailable (auth/network/circuit",
    "auth/network/circuit",
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

quality_violation_reason() {
  local file="$1"
  local prompt="${2:-}"
  local expected="${3:-}"
  python3 - "$file" "$prompt" "$expected" <<'PY'
import json
import re
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
prompt = sys.argv[2]
expected = sys.argv[3] if len(sys.argv) > 3 else ""
prompt_l = prompt.lower()
data = obj.get("data") or {}
result = data.get("result_json") or {}
texts = [str(data.get("error_text") or ""), str(result.get("text") or "")]
for item in result.get("messages") or []:
    if isinstance(item, str):
        texts.append(item)
    elif isinstance(item, dict):
        texts.append(str(item.get("text") or ""))
text = "\n".join(part for part in texts if part).strip()
text_l = text.lower()

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
execution_trace_markers = [
    "**执行过程**",
    "调用技能 `",
    "调用命令 `",
    "called skill",
    "called command",
]
if any(marker in prompt_l or marker in prompt for marker in strict_prompt_markers):
    if any(marker.lower() in text_l for marker in execution_trace_markers):
        print("strict_output_contains_execution_trace")
        raise SystemExit(0)

delivery_markers = [
    "发给我",
    "发送给我",
    "把文件发",
    "直接发",
    "send me",
    "deliver",
]
delivery_requested = any(marker in prompt_l or marker in prompt for marker in delivery_markers)
missing_or_clarify = any(
    marker in text_l
    for marker in [
        "not found",
        "不存在",
        "未找到",
        "文件未找到",
        "请提供完整路径",
        "provide the full path",
    ]
)
if delivery_requested and not needs_clarify and not missing_or_clarify and "file:" not in text_l and "image_file:" not in text_l:
    print("delivery_request_without_file_token")
    raise SystemExit(0)

filesystem_refusal_markers = [
    "chat-only 模式无法直接访问文件系统",
    "无法直接访问文件系统",
    "没有执行文件系统命令的能力",
    "无法检查当前目录",
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

raise SystemExit(1)
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

submit_turn() {
  local turn="$1"
  local prompt="$2"
  local out_file="$3"
  local expected_marker="${4:-}"
  local submit_raw task_id status text messages error

  submit_raw="$(submit_client_like_telegram_task "$prompt" "true" "" "$EXTERNAL_USER_ID_VALUE" "$EXTERNAL_CHAT_ID_VALUE")"
  task_id="$(extract_submit_task_id "$submit_raw")"
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

  if [[ "$status" != "succeeded" ]]; then
    echo "Turn ${turn} did not succeed: status=${status} error=${error}" >&2
    print_log_hints "$task_id" >&2
    return 1
  fi
  if result_has_bad_fallback "$out_file" "$prompt"; then
    echo "Turn ${turn} returned bad fallback/unavailable text." >&2
    print_log_hints "$task_id" >&2
    return 1
  fi
  if [[ "$QUALITY_GUARD" -eq 1 ]]; then
    local quality_reason
    quality_reason="$(quality_violation_reason "$out_file" "$prompt" "$expected_marker" || true)"
    if [[ -n "$quality_reason" ]]; then
      echo "Turn ${turn} failed quality guard: ${quality_reason}" >&2
      echo "  reply=${text:-${error:-<empty>}}" >&2
      print_log_hints "$task_id" >&2
      return 1
    fi
  fi
  if [[ -n "$expected_marker" ]] && ! result_text_contains "$out_file" "$expected_marker"; then
    echo "Turn ${turn} did not include expected marker: ${expected_marker}" >&2
    echo "  reply=${text:-${error:-<empty>}}" >&2
    print_log_hints "$task_id" >&2
    return 1
  fi
}

load_case_rows() {
  local case_file="$1"
  local case_limit="$2"
  local case_start="$3"
  python3 - "$case_file" "$case_limit" "$case_start" <<'PY'
import sys
from pathlib import Path

case_file = Path(sys.argv[1])
limit_raw = sys.argv[2].strip()
start_raw = sys.argv[3].strip()
limit = int(limit_raw) if limit_raw else 0
start = int(start_raw) if start_raw else 1
if start < 1:
    raise SystemExit(f"case_start must be >= 1, got {start}")
seen = 0
emitted = 0
for raw in case_file.read_text(encoding="utf-8").splitlines():
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = line.split("|", 4)
    if len(parts) < 4:
        continue
    suite, name, tags, prompt = parts[:4]
    expect = ""
    if len(parts) >= 5:
        extra = parts[4].strip()
        if extra.startswith("expect="):
            expect = extra[len("expect="):]
    seen += 1
    if seen < start:
        continue
    emitted += 1
    print("\t".join([
        str(seen),
        name.replace("\t", " "),
        prompt.replace("\t", " "),
        expect.replace("\t", " "),
    ]))
    if limit and emitted >= limit:
        break
PY
}

verify_db_state() {
  local require_test_id_memory="${1:-1}"
  python3 - "$DB_PATH_VALUE" "$TEST_ID" "$require_test_id_memory" "${TASK_IDS[@]}" <<'PY'
import sqlite3
import sys
from pathlib import Path

db_path = Path(sys.argv[1])
test_id = sys.argv[2]
require_test_id_memory = sys.argv[3] == "1"
task_ids = sys.argv[4:]
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
chat_id = first["chat_id"]
user_key = str(first["user_key"] or "")
channel = first["channel"]
if channel != "telegram":
    raise SystemExit(f"expected telegram channel, got {channel}")
for row in rows:
    if row["user_id"] != user_id or row["chat_id"] != chat_id:
        raise SystemExit("turns did not land in the same effective conversation")

def count(sql, params=()):
    return int(conn.execute(sql, params).fetchone()[0])

tasks_count = count("SELECT COUNT(*) FROM tasks WHERE chat_id = ? AND user_id = ?", (chat_id, user_id))
memories_count = count("SELECT COUNT(*) FROM memories WHERE chat_id = ? AND user_id = ?", (chat_id, user_id))
conversation_states_count = count(
    "SELECT COUNT(*) FROM conversation_states WHERE chat_id = ? AND user_id = ?",
    (chat_id, user_id),
)
retrieval_count = count(
    "SELECT COUNT(*) FROM memory_retrieval_index WHERE chat_id = ? AND user_id = ?",
    (chat_id, user_id),
)
long_term_count = count("SELECT COUNT(*) FROM long_term_memories WHERE chat_id = ? AND user_id = ?", (chat_id, user_id))
preference_count = count("SELECT COUNT(*) FROM user_preferences WHERE chat_id = ? AND user_id = ?", (chat_id, user_id))

if memories_count <= 0:
    raise SystemExit("expected memories to be written for client-like continuous conversation")
if conversation_states_count <= 0:
    raise SystemExit("expected conversation_states to be written for client-like continuous conversation")

test_id_memory_count = 0
if require_test_id_memory:
    test_id_memory_count = count(
        "SELECT COUNT(*) FROM memories WHERE chat_id = ? AND user_id = ? AND content LIKE ?",
        (chat_id, user_id, f"%{test_id}%"),
    )
    if test_id_memory_count <= 0:
        raise SystemExit("expected short-term memory to contain the suite test id")
execution_summary_leak_count = count(
    "SELECT COUNT(*) FROM memories WHERE chat_id = ? AND user_id = ? AND content LIKE ?",
    (chat_id, user_id, "%**执行过程**%"),
)
if execution_summary_leak_count > 0:
    raise SystemExit("execution summary leaked into short-term memory")

print(
    "DB_VERIFY_OK "
    f"effective_user_id={user_id} effective_chat_id={chat_id} user_key_present={bool(user_key)} "
    f"tasks={tasks_count} memories={memories_count} conversation_states={conversation_states_count} "
    f"retrieval_index={retrieval_count} long_term={long_term_count} preferences={preference_count} "
    f"test_id_memory_checked={require_test_id_memory} test_id_memory_count={test_id_memory_count}"
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
DB_PATH_VALUE="$(resolve_db_path)"
if [[ -n "${CASE_FILE_VALUE:-}" ]]; then
  if [[ ! -f "$CASE_FILE_VALUE" ]]; then
    echo "Case file not found: $CASE_FILE_VALUE" >&2
    exit 2
  fi
  CASE_FILE_VALUE="$(python3 - "$CASE_FILE_VALUE" <<'PY'
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
echo "db_path=${DB_PATH_VALUE}"
echo "raw_user_id=${USER_ID}"
echo "raw_chat_id=${CHAT_ID}"
echo "external_user_id=${EXTERNAL_USER_ID_VALUE}"
echo "external_chat_id=${EXTERNAL_CHAT_ID_VALUE}"
echo "test_id=${TEST_ID}"
echo "log_dir=${RUN_DIR}"
echo "case_file=${CASE_FILE_VALUE:-<none>}"
echo "case_limit=${CASE_LIMIT_VALUE:-<none>}"
echo "case_start=${CASE_START_VALUE:-1}"
echo "quality_guard=${QUALITY_GUARD}"

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

if [[ -n "${CASE_FILE_VALUE:-}" ]]; then
  while IFS=$'\t' read -r case_index case_name case_prompt case_expect; do
    [[ -n "${case_index:-}" ]] || continue
    turn=$((turn + 1))
    echo "[CASE ${case_index}] name=${case_name}"
    if ! submit_turn "$turn" "$case_prompt" "${RUN_DIR}/turn_${turn}_case_${case_index}.json" "${case_expect:-}"; then
      quality_guard_arg=""
      if [[ "$QUALITY_GUARD" -eq 1 ]]; then
        quality_guard_arg=" --quality-guard"
      fi
      echo "RESUME_HINT bash scripts/nl_tests/run_client_like_continuous_suite.sh --case-file ${CASE_FILE_VALUE} --case-start ${case_index} --skip-smoke --external-user-id ${EXTERNAL_USER_ID_VALUE} --external-chat-id ${EXTERNAL_CHAT_ID_VALUE} --prompt-reply-only${quality_guard_arg}" >&2
      exit 1
    fi
  done < <(load_case_rows "$CASE_FILE_VALUE" "$CASE_LIMIT_VALUE" "$CASE_START_VALUE")
fi

if [[ "$turn" -eq 0 ]]; then
  echo "No turns were run. Remove --skip-smoke or pass --case-file/--full-nl." >&2
  exit 2
fi

if [[ "$RUN_BUILTIN_SMOKE" -eq 1 ]]; then
  verify_db_state 1
else
  verify_db_state 0
fi

echo "CLIENT_LIKE_CONTINUOUS_SUITE_OK turns=${turn} log_dir=${RUN_DIR}"

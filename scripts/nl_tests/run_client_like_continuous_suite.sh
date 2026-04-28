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
PROMPT_REPLY_ONLY=0
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
  --prompt-reply-only        print only prompt and reply snippets
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
    --prompt-reply-only)
      PROMPT_REPLY_ONLY=1
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
elif field == "messages":
    print("\\n---\\n".join(message.replace("\n", "\\n") for message in message_texts()))
elif field == "error":
    print(str(data.get("error_text") or "").strip().replace("\n", "\\n"))
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
  python3 - "$file" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
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
markers = [
    "模型暂时不可用",
    "当前大模型服务暂时不可用",
    "我没看出这条消息要做什么",
    "intent_unresolved",
    "context window exceeds limit",
    "invalid params",
    "http 400",
]
raise SystemExit(0 if any(marker in joined for marker in markers) else 1)
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

  wait_task_until_terminal_with_limit "$task_id" "$MAX_WAIT_SECONDS" > "$out_file"
  status="$(extract_json_field "$out_file" status)"
  text="$(extract_json_field "$out_file" text)"
  messages="$(extract_json_field "$out_file" messages)"
  error="$(extract_json_field "$out_file" error)"
  echo "[TURN ${turn}] status=${status}"
  if [[ "$PROMPT_REPLY_ONLY" -eq 1 ]]; then
    echo "[REPLY]"
    if [[ -n "${messages:-}" ]]; then
      printf '%s\n' "$messages"
    else
      printf '%s\n' "${text:-${error:-<empty>}}"
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
  if result_has_bad_fallback "$out_file"; then
    echo "Turn ${turn} returned bad fallback/unavailable text." >&2
    print_log_hints "$task_id" >&2
    return 1
  fi
  if [[ -n "$expected_marker" ]] && ! result_text_contains "$out_file" "$expected_marker"; then
    echo "Turn ${turn} did not include expected marker: ${expected_marker}" >&2
    echo "  reply=${text:-${error:-<empty>}}" >&2
    print_log_hints "$task_id" >&2
    return 1
  fi
}

verify_db_state() {
  python3 - "$DB_PATH_VALUE" "${TASK_IDS[@]}" <<'PY'
import sqlite3
import sys
from pathlib import Path

db_path = Path(sys.argv[1])
task_ids = sys.argv[2:]
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

print(
    "DB_VERIFY_OK "
    f"effective_user_id={user_id} effective_chat_id={chat_id} user_key_present={bool(user_key)} "
    f"tasks={tasks_count} memories={memories_count} conversation_states={conversation_states_count} "
    f"retrieval_index={retrieval_count} long_term={long_term_count} preferences={preference_count}"
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
  "连续"
)

for idx in "${!PROMPTS[@]}"; do
  turn=$((idx + 1))
  submit_turn "$turn" "${PROMPTS[$idx]}" "${RUN_DIR}/turn_${turn}.json" "${EXPECTED_MARKERS[$idx]}"
done

verify_db_state

echo "CLIENT_LIKE_CONTINUOUS_SUITE_OK turns=${#PROMPTS[@]} log_dir=${RUN_DIR}"

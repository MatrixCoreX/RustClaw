#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Regression: failed multi-step ask should emit resume_context,
# and follow-up "continue" message should trigger LLM-based resume path.
#
# Usage:
#   bash scripts/regression_resume_continue.sh [--base-url URL] [--user-id ID] [--chat-id ID] [--user-key KEY] [--wait-seconds N]

BASE_URL="${BASE_URL:-}"
USER_ID="${USER_ID:-}"
CHAT_ID="${CHAT_ID:-}"
USER_KEY="${RUSTCLAW_USER_KEY:-${USER_KEY:-}}"
WAIT_SECONDS="${WAIT_SECONDS:-120}"
POLL_INTERVAL="${POLL_INTERVAL:-1}"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1"
    exit 2
  }
}

resolve_admin_key() {
  if [[ -n "${USER_KEY:-}" ]]; then
    return 0
  fi
  USER_KEY="$("${ROOT_DIR}/scripts/auth-key.sh" list | awk '$2 == "admin" && $3 == "enabled" { print $1; exit }')"
  if [[ -z "${USER_KEY:-}" ]]; then
    echo "No enabled admin key found. Pass --user-key explicitly." >&2
    exit 2
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
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
      WAIT_SECONDS="${2:-}"
      shift 2
      ;;
    -h|--help)
      sed -n '1,20p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      exit 2
      ;;
  esac
done

need_cmd curl
need_cmd jq
need_cmd python3

resolve_admin_key

if [[ -z "$USER_ID" || -z "$CHAT_ID" ]]; then
  read -r default_user_id default_chat_id < <(
    python3 - <<'PY'
import time

seed = time.time_ns() % 9_000_000_000
base = 1_000_000_000 + seed
print(base, base + 1)
PY
  )
  USER_ID="${USER_ID:-$default_user_id}"
  CHAT_ID="${CHAT_ID:-$default_chat_id}"
fi

if [[ -z "$BASE_URL" ]]; then
  BASE_URL="$(
python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
listen = str(cfg.get("server", {}).get("listen", "127.0.0.1:8787"))
print(f"http://{listen}")
PY
  )"
fi
BASE_URL="${BASE_URL%/}"

submit_ask() {
  local text="$1"
  jq -nc \
    --argjson user_id "$USER_ID" \
    --argjson chat_id "$CHAT_ID" \
    --arg text "$text" \
    --arg user_key "$USER_KEY" \
    '{
      user_id: $user_id,
      chat_id: $chat_id,
      kind: "ask",
      payload: { text: $text }
    } + (if ($user_key | length) > 0 then { user_key: $user_key } else {} end)' \
  | curl -sS -X POST "${BASE_URL}/v1/tasks" \
      -H "Content-Type: application/json" \
      -H "X-RustClaw-Key: ${USER_KEY}" \
      -d @-
}

poll_terminal() {
  local task_id="$1"
  local waited=0
  while [[ "$waited" -le "$WAIT_SECONDS" ]]; do
    local row status
    row="$(curl -sS -H "X-RustClaw-Key: ${USER_KEY}" "${BASE_URL}/v1/tasks/${task_id}")"
    status="$(echo "$row" | jq -r '.data.status // ""')"
    case "$status" in
      succeeded|failed|timeout|canceled)
        printf '%s\n' "$row"
        return 0
        ;;
      *)
        sleep "$POLL_INTERVAL"
        waited=$((waited + POLL_INTERVAL))
        ;;
    esac
  done
  echo "poll timeout for task_id=${task_id}" >&2
  return 1
}

result_provider_unavailable() {
  python3 - "$1" <<'PY'
import json
import re
import sys

obj = json.loads(sys.argv[1])
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
    elif isinstance(item, str):
        parts.append(item)
text = "\n".join(part.strip().lower() for part in parts if str(part).strip())
markers = [
    "当前大模型服务暂时不可用",
    "selected model is at capacity",
    "usage limit exceeded",
    "rate limit",
    "rate_limit",
    "too many requests",
    "http 429",
    "http 529",
    "529 overloaded",
    "missing choices[0].message.content",
]
provider_like = any(marker in text for marker in markers)
provider_like = provider_like or (
    "provider=vendor-" in text
    and (
        re.search(r"http 5\d\d", text) is not None
        or '"type":"server_error"' in text
        or "unknown error, 520" in text
    )
)
raise SystemExit(0 if provider_like else 1)
PY
}

echo "[1/4] submit failing multi-step ask"
first_submit="$(submit_ask '先查BTC价格，再执行一个不存在的命令 resume_fail_cmd_001_xyz，再查ETH价格')"
first_task_id="$(echo "$first_submit" | jq -r '.data.task_id // empty')"
[[ -n "$first_task_id" ]] || { echo "submit failed: $first_submit"; exit 1; }

echo "[2/4] wait first task terminal"
first_row="$(poll_terminal "$first_task_id")"
first_status="$(echo "$first_row" | jq -r '.data.status // ""')"
if [[ "$first_status" != "failed" && "$first_status" != "timeout" ]]; then
  if result_provider_unavailable "$first_row"; then
    echo "provider unavailable during first task; skip resume regression"
    exit 2
  fi
  echo "expected first task failed/timeout, got: $first_status"
  exit 1
fi

echo "[3/4] verify resume_context exists"
has_resume_ctx="$(echo "$first_row" | jq -r '.data.result_json.resume_context != null')"
if [[ "$has_resume_ctx" != "true" ]]; then
  echo "missing resume_context in failed task result_json"
  echo "$first_row"
  exit 1
fi

echo "[4/4] submit continue ask and verify terminal"
second_submit="$(submit_ask '继续，把后面的执行完')"
second_task_id="$(echo "$second_submit" | jq -r '.data.task_id // empty')"
[[ -n "$second_task_id" ]] || { echo "submit failed: $second_submit"; exit 1; }
second_row="$(poll_terminal "$second_task_id")"
second_status="$(echo "$second_row" | jq -r '.data.status // ""')"
if [[ "$second_status" != "succeeded" && "$second_status" != "failed" && "$second_status" != "timeout" ]]; then
  echo "unexpected second task status: $second_status"
  exit 1
fi

echo "PASS: resume flow regression basic checks finished"

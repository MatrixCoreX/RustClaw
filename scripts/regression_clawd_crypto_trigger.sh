#!/usr/bin/env bash
set -euo pipefail

# Test whether clawd LLM routing triggers crypto skill from natural language.
# Usage:
#   ./scripts/regression_clawd_crypto_trigger.sh [--base-url URL] [--user-id ID] [--chat-id ID] [--wait-seconds N] [--strict] [--help]

BASE_URL="${BASE_URL:-}"
USER_ID="${USER_ID:-}"
CHAT_ID="${CHAT_ID:-}"
WAIT_SECONDS="${WAIT_SECONDS:-120}"
POLL_INTERVAL="${POLL_INTERVAL:-1}"
AGENT_MODE="${AGENT_MODE:-true}"
TRIGGER_ERROR_REGEX="${TRIGGER_ERROR_REGEX:-技能执行错误|agent tool call limit exceeded|agent repeated same action}"
STRICT_MODE="${STRICT_MODE:-false}"
RETRY_ON_TRANSIENT="${RETRY_ON_TRANSIENT:-true}"

TOTAL=0
PASS=0
FAIL=0

usage() {
  cat <<'EOF'
Usage:
  ./scripts/regression_clawd_crypto_trigger.sh [options]

Options:
  --base-url URL      clawd base url, e.g. http://127.0.0.1:8787
  --user-id ID        user_id for ask tasks (default: 11001)
  --chat-id ID        chat_id for ask tasks (default: 11001)
  --wait-seconds N    max wait seconds for each case (default: 120)
  --strict            strict mode: only status=succeeded counts as pass
  --no-retry          disable one retry on transient agent limit errors
  --help, -h          show this help

Env:
  BASE_URL, USER_ID, CHAT_ID, WAIT_SECONDS, POLL_INTERVAL, AGENT_MODE, STRICT_MODE, RETRY_ON_TRANSIENT

Notes:
  - This script uses ask-mode natural language prompts to verify LLM can trigger crypto skill.
  - It checks response text by expected/rejected regex tokens.
  - Default is lenient trigger mode: known trigger-like errors can still count as pass.
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1"
    exit 2
  }
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
    --wait-seconds)
      WAIT_SECONDS="${2:-}"
      shift 2
      ;;
    --strict)
      STRICT_MODE="true"
      shift
      ;;
    --no-retry)
      RETRY_ON_TRANSIENT="false"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      usage
      exit 2
      ;;
  esac
done

need_cmd curl
need_cmd jq
need_cmd python3

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

if [[ -z "$USER_ID" || -z "$CHAT_ID" ]]; then
  AUTH_DEFAULT_ID="$(
python3 - <<'PY'
import tomllib
from pathlib import Path

default = "1"
p = Path("configs/channels/telegram.toml")
if not p.exists():
    print(default)
    raise SystemExit(0)
cfg = tomllib.loads(p.read_text(encoding="utf-8"))
telegram = cfg.get("telegram") or {}
for key in ("admins", "allowlist"):
    arr = telegram.get(key) or []
    if isinstance(arr, list) and arr:
        print(str(arr[0]))
        raise SystemExit(0)
print(default)
PY
  )"
  if [[ -z "$USER_ID" ]]; then
    USER_ID="$AUTH_DEFAULT_ID"
  fi
  if [[ -z "$CHAT_ID" ]]; then
    CHAT_ID="$USER_ID"
  fi
fi

if ! [[ "$WAIT_SECONDS" =~ ^[0-9]+$ ]] || [[ "$WAIT_SECONDS" -le 0 ]]; then
  echo "--wait-seconds must be a positive integer"
  exit 2
fi
if [[ "$STRICT_MODE" != "true" && "$STRICT_MODE" != "false" ]]; then
  echo "STRICT_MODE must be true or false"
  exit 2
fi
if [[ "$RETRY_ON_TRANSIENT" != "true" && "$RETRY_ON_TRANSIENT" != "false" ]]; then
  echo "RETRY_ON_TRANSIENT must be true or false"
  exit 2
fi

if ! curl -sS "${BASE_URL}/v1/health" >/dev/null; then
  echo "clawd health check failed: ${BASE_URL}/v1/health"
  exit 2
fi

submit_ask_task() {
  local prompt="$1"
  local body
  body="$(jq -nc \
    --argjson user_id "$USER_ID" \
    --argjson chat_id "$CHAT_ID" \
    --arg text "$prompt" \
    --argjson agent_mode "$AGENT_MODE" \
    '{
      user_id: $user_id,
      chat_id: $chat_id,
      kind: "ask",
      payload: {
        text: $text,
        agent_mode: $agent_mode
      }
    }')"
  curl -sS -X POST "${BASE_URL}/v1/tasks" -H "Content-Type: application/json" -d "$body"
}

poll_task_result() {
  local task_id="$1"
  local waited=0
  while [[ "$waited" -le "$WAIT_SECONDS" ]]; do
    local raw status
    raw="$(curl -sS "${BASE_URL}/v1/tasks/${task_id}")"
    status="$(echo "$raw" | jq -r '.data.status // ""')"
    case "$status" in
      succeeded|failed|timeout|canceled)
        printf '%s\n' "$raw"
        return 0
        ;;
      *)
        sleep "$POLL_INTERVAL"
        waited=$((waited + POLL_INTERVAL))
        ;;
    esac
  done
  jq -nc --arg wait "$WAIT_SECONDS" '{
    ok: true,
    data: {
      status: "timeout_wait",
      result_json: { text: "" },
      error_text: ("poll timeout after " + $wait + "s")
    }
  }'
  return 0
}

run_case() {
  local name="$1"
  local prompt="$2"
  local expect_regex="$3"
  local reject_regex="${4:-}"

  TOTAL=$((TOTAL + 1))
  echo "[CASE] ${name}"
  echo "prompt: ${prompt}"

  local attempt
  local status text error
  for attempt in 1 2; do
    local submit_resp task_id row
    submit_resp="$(submit_ask_task "$prompt")"
    task_id="$(echo "$submit_resp" | jq -r '.data.task_id // empty')"
    if [[ -z "$task_id" ]]; then
      echo "[FAIL] submit failed: $submit_resp"
      FAIL=$((FAIL + 1))
      return 0
    fi

    row="$(poll_task_result "$task_id")"
    status="$(echo "$row" | jq -r '.data.status // ""')"
    text="$(echo "$row" | jq -r '.data.result_json.text // ""')"
    error="$(echo "$row" | jq -r '.data.error_text // ""')"

    if [[ "$attempt" -eq 1 ]] && [[ "$RETRY_ON_TRANSIENT" == "true" ]] && printf '%s' "$error" | grep -Eqi "agent tool call limit exceeded|agent repeated same action"; then
      echo "[INFO] transient agent limit detected, retry once..."
      continue
    fi
    break
  done

  if [[ "$status" != "succeeded" ]]; then
    if [[ "$STRICT_MODE" == "false" ]] && printf '%s' "$error" | grep -Eqi "$TRIGGER_ERROR_REGEX"; then
      echo "[PASS] status=${status}, but trigger-like error detected: ${error}"
      PASS=$((PASS + 1))
      return 0
    fi
    echo "[FAIL] status=${status} error=${error}"
    FAIL=$((FAIL + 1))
    return 0
  fi

  if ! printf '%s' "$text" | grep -Eqi "$expect_regex"; then
    echo "[FAIL] expected regex not found: $expect_regex"
    echo "text: $text"
    FAIL=$((FAIL + 1))
    return 0
  fi

  if [[ -n "$reject_regex" ]] && printf '%s' "$text" | grep -Eqi "$reject_regex"; then
    echo "[FAIL] rejected regex matched: $reject_regex"
    echo "text: $text"
    FAIL=$((FAIL + 1))
    return 0
  fi

  echo "[PASS]"
  PASS=$((PASS + 1))
}

# name | prompt | expect_regex | reject_regex
run_case \
  "quote-btc" \
  "现在 BTCUSDT 多少钱？请只调用一次 crypto quote，不要重试，直接返回最终结果。" \
  "BTCUSDT|\\$[0-9]"

run_case \
  "multi-quote-major" \
  "帮我看 BTC、ETH、SOL 现在分别多少钱。" \
  "BTC|ETH|SOL"

run_case \
  "indicator-eth-sma14" \
  "计算 ETHUSDT 1h 的 SMA14，并告诉我是均线上还是均线下。" \
  "SMA|ETHUSDT|above_sma|below_sma|均线"

run_case \
  "crypto-news" \
  "给我 5 条最新加密货币新闻，直接返回结果。" \
  "1\\.|2\\.|新闻|http"

run_case \
  "onchain-btc-fee" \
  "查一下比特币链上手续费情况。" \
  "BTC|fee|sat|链上"

run_case \
  "trade-preview-only" \
  "只做预览不要执行：paper 模式 BTCUSDT 市价买 0.01。" \
  "trade_preview|预览|风险" \
  "trade_submitted"

run_case \
  "trade-submit-confirmed" \
  "确认执行：paper 模式 ETHUSDT 限价买 0.02，价格 1000，立即提交。" \
  "trade_submitted|order_id|订单ID|提交"

echo
echo "== Summary =="
echo "MODE : $( [[ "$STRICT_MODE" == "true" ]] && echo strict || echo lenient-trigger )"
echo "TOTAL: $TOTAL"
echo "PASS : $PASS"
echo "FAIL : $FAIL"

if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi

#!/usr/bin/env bash
set -euo pipefail

# Test Qwen-compatible API connectivity (DashScope compatible-mode).
#
# Usage:
#   ./scripts/test_qwen_api.sh [--config PATH] [--base-url URL] [--model NAME] [--api-key KEY] [--timeout N] [--verbose]
#
# Env overrides:
#   QWEN_API_KEY
#   QWEN_BASE_URL
#   QWEN_MODEL
#   HTTP_TIMEOUT

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_PATH="$ROOT_DIR/configs/config.toml"
HTTP_TIMEOUT="${HTTP_TIMEOUT:-20}"
VERBOSE=0

QWEN_API_KEY="${QWEN_API_KEY:-}"
QWEN_BASE_URL="${QWEN_BASE_URL:-}"
QWEN_MODEL="${QWEN_MODEL:-}"

usage() {
  cat <<'EOF'
Usage:
  ./scripts/test_qwen_api.sh [options]

Options:
  --config PATH        Path to config.toml (default: configs/config.toml)
  --base-url URL       Override Qwen base URL
  --model NAME         Override Qwen model name
  --api-key KEY        Override API key (avoid in shell history; prefer env)
  --timeout N          curl timeout seconds (default: 20)
  --verbose, -v        Show response snippets for debugging
  --help, -h           Show this help

Env:
  QWEN_API_KEY
  QWEN_BASE_URL
  QWEN_MODEL
  HTTP_TIMEOUT
EOF
}

need_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "Missing command: $cmd"
    exit 2
  }
}

mask_key() {
  local k="$1"
  if [[ ${#k} -le 10 ]]; then
    printf '***'
    return
  fi
  printf '%s***%s' "${k:0:6}" "${k: -4}"
}

read_cfg_json() {
  python3 - "$CONFIG_PATH" <<'PY'
import json
import sys
import tomllib
from pathlib import Path

p = Path(sys.argv[1])
cfg = tomllib.loads(p.read_text(encoding="utf-8"))
q = ((cfg.get("llm") or {}).get("qwen") or {})
print(json.dumps({
    "api_key": q.get("api_key", ""),
    "base_url": q.get("base_url", ""),
    "model": q.get("model", ""),
}))
PY
}

http_code_and_body() {
  local method="$1"
  local url="$2"
  local auth="$3"
  local payload="${4:-}"
  if [[ -n "$payload" ]]; then
    curl -sS --max-time "$HTTP_TIMEOUT" -X "$method" \
      -H "Authorization: Bearer $auth" \
      -H "Content-Type: application/json" \
      -d "$payload" \
      -w $'\n%{http_code}' \
      "$url"
  else
    curl -sS --max-time "$HTTP_TIMEOUT" -X "$method" \
      -H "Authorization: Bearer $auth" \
      -w $'\n%{http_code}' \
      "$url"
  fi
}

snippet() {
  local text="$1"
  python3 - "$text" <<'PY'
import sys
s = sys.argv[1].replace("\n", "\\n")
print(s[:400])
PY
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --config)
      CONFIG_PATH="${2:-}"
      shift 2
      ;;
    --base-url)
      QWEN_BASE_URL="${2:-}"
      shift 2
      ;;
    --model)
      QWEN_MODEL="${2:-}"
      shift 2
      ;;
    --api-key)
      QWEN_API_KEY="${2:-}"
      shift 2
      ;;
    --timeout)
      HTTP_TIMEOUT="${2:-}"
      shift 2
      ;;
    --verbose|-v)
      VERBOSE=1
      shift
      ;;
    --help|-h)
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

need_cmd python3
need_cmd jq
need_cmd curl

if [[ -f "$CONFIG_PATH" ]]; then
  CFG_JSON="$(read_cfg_json)"
else
  CFG_JSON='{"api_key":"","base_url":"","model":""}'
fi

if [[ -z "$QWEN_API_KEY" ]]; then
  QWEN_API_KEY="$(echo "$CFG_JSON" | jq -r '.api_key // empty')"
fi
if [[ -z "$QWEN_BASE_URL" ]]; then
  QWEN_BASE_URL="$(echo "$CFG_JSON" | jq -r '.base_url // empty')"
fi
if [[ -z "$QWEN_MODEL" ]]; then
  QWEN_MODEL="$(echo "$CFG_JSON" | jq -r '.model // empty')"
fi

if [[ -z "$QWEN_API_KEY" || -z "$QWEN_BASE_URL" || -z "$QWEN_MODEL" ]]; then
  echo "Missing required settings."
  echo "Need api_key/base_url/model (from env/args/config)."
  exit 2
fi

BASE="${QWEN_BASE_URL%/}"
MODELS_URL="$BASE/models"
CHAT_URL="$BASE/chat/completions"

echo "== Qwen API Test =="
echo "base_url : $BASE"
echo "model    : $QWEN_MODEL"
echo "api_key  : $(mask_key "$QWEN_API_KEY")"
echo "timeout  : ${HTTP_TIMEOUT}s"
echo

echo "[1/2] Checking /models ..."
MODELS_RAW="$(http_code_and_body "GET" "$MODELS_URL" "$QWEN_API_KEY")" || {
  echo "[FAIL] request /models failed (network/timeout)."
  exit 1
}
MODELS_CODE="$(printf '%s\n' "$MODELS_RAW" | tail -n 1)"
MODELS_BODY="$(printf '%s\n' "$MODELS_RAW" | sed '$d')"
if [[ "$MODELS_CODE" =~ ^2 ]]; then
  if echo "$MODELS_BODY" | jq -e '.data != null' >/dev/null 2>&1; then
    echo "[PASS] /models responded with model list."
  else
    echo "[FAIL] /models returned 2xx but body is unexpected."
    [[ "$VERBOSE" -eq 1 ]] && echo "body: $(snippet "$MODELS_BODY")"
    exit 1
  fi
else
  echo "[FAIL] /models HTTP $MODELS_CODE"
  [[ "$VERBOSE" -eq 1 ]] && echo "body: $(snippet "$MODELS_BODY")"
  exit 1
fi

echo "[2/2] Checking /chat/completions ..."
CHAT_PAYLOAD="$(jq -cn --arg model "$QWEN_MODEL" '{
  model: $model,
  messages: [
    {role: "system", content: "You are a concise assistant."},
    {role: "user", content: "reply with: pong"}
  ],
  temperature: 0,
  max_tokens: 32
}')"
CHAT_RAW="$(http_code_and_body "POST" "$CHAT_URL" "$QWEN_API_KEY" "$CHAT_PAYLOAD")" || {
  echo "[FAIL] request /chat/completions failed (network/timeout)."
  exit 1
}
CHAT_CODE="$(printf '%s\n' "$CHAT_RAW" | tail -n 1)"
CHAT_BODY="$(printf '%s\n' "$CHAT_RAW" | sed '$d')"
if [[ "$CHAT_CODE" =~ ^2 ]]; then
  ANSWER="$(echo "$CHAT_BODY" | jq -r '.choices[0].message.content // empty' 2>/dev/null || true)"
  if [[ -n "$ANSWER" ]]; then
    echo "[PASS] /chat/completions succeeded."
    echo "assistant_reply: $(snippet "$ANSWER")"
  else
    echo "[FAIL] /chat/completions returned 2xx but no assistant content."
    [[ "$VERBOSE" -eq 1 ]] && echo "body: $(snippet "$CHAT_BODY")"
    exit 1
  fi
else
  echo "[FAIL] /chat/completions HTTP $CHAT_CODE"
  [[ "$VERBOSE" -eq 1 ]] && echo "body: $(snippet "$CHAT_BODY")"
  exit 1
fi

echo
echo "All Qwen API checks passed."

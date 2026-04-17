#!/usr/bin/env bash
# 使用 configs/config.toml 中 [llm.minimax]，通过 curl 调用 OpenAI 兼容 chat/completions 做联通测试。
#
# 用法:
#   ./scripts/test_minimax_curl.sh
#   CONFIG=/path/to/config.toml ./scripts/test_minimax_curl.sh
#   MODEL=MiniMax-M2.5 PROMPT='ping' ./scripts/test_minimax_curl.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG="${CONFIG:-$ROOT/configs/config.toml}"

[[ -f "$CONFIG" ]] || { echo "找不到配置: $CONFIG" >&2; exit 1; }

# 取出 [llm.minimax] 段（到下一段 [xxx] 为止）
read_minimax_block() {
  awk '
    /^\[llm\.minimax\]/ { inblk=1; next }
    inblk && /^\[/ { exit }
    inblk { print }
  ' "$CONFIG"
}

# key = "value" 或 key = number
get_toml_string() {
  local key="$1"
  grep -E "^[[:space:]]*${key}[[:space:]]*=" | head -1 | sed -E 's/^[[:space:]]*[^=]+=[[:space:]]*"([^"]*)"[[:space:]]*$/\1/'
}

get_toml_int() {
  local key="$1"
  local line
  line=$(grep -E "^[[:space:]]*${key}[[:space:]]*=" | head -1)
  line="${line#*=}"
  echo "${line// /}" | tr -d '\r'
}

BLOCK="$(read_minimax_block)"
if [[ -z "${BLOCK// /}" ]]; then
  echo "配置中未找到 [llm.minimax] 段: $CONFIG" >&2
  exit 1
fi

API_KEY="$(echo "$BLOCK" | get_toml_string api_key)"
BASE_URL="$(echo "$BLOCK" | get_toml_string base_url)"
MODEL_CFG="$(echo "$BLOCK" | get_toml_string model)"
TIMEOUT="$(echo "$BLOCK" | get_toml_int timeout_seconds)"
TIMEOUT="${TIMEOUT:-60}"

MODEL="${MODEL:-$MODEL_CFG}"
PROMPT="${PROMPT:-请只回复一个字：好}"

[[ -n "$API_KEY" && "$API_KEY" != "REPLACE_ME" ]] || { echo "api_key 为空或占位符，请配置 [llm.minimax].api_key" >&2; exit 1; }
[[ -n "$BASE_URL" ]] || { echo "base_url 为空" >&2; exit 1; }
[[ -n "$MODEL" ]] || { echo "model 为空（可用环境变量 MODEL 指定）" >&2; exit 1; }

BASE_URL="${BASE_URL%/}"
URL="${BASE_URL}/chat/completions"

build_json_body() {
  if command -v jq >/dev/null 2>&1; then
    jq -n \
      --arg m "$MODEL" \
      --arg p "$PROMPT" \
      '{model:$m, messages:[{role:"user",content:$p}], temperature:0, max_tokens:32}'
  else
    python3 -c 'import json,os,sys
print(json.dumps({"model":os.environ["M"],"messages":[{"role":"user","content":os.environ["P"]}],"temperature":0,"max_tokens":32}))'
  fi
}

BODY="$(M="$MODEL" P="$PROMPT" build_json_body)"

echo "POST $URL"
echo "model=$MODEL connect-timeout=${TIMEOUT}s"

# --max-time: 总超时；--connect-timeout: 建连超时
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

HTTP_CODE="$(
  curl -sS -o "$TMP" -w '%{http_code}' \
    --connect-timeout "$TIMEOUT" \
    --max-time "$TIMEOUT" \
    -X POST "$URL" \
    -H "Authorization: Bearer ${API_KEY}" \
    -H "Content-Type: application/json" \
    -d "$BODY"
)" || true

if [[ "$HTTP_CODE" != "200" ]]; then
  echo "HTTP $HTTP_CODE" >&2
  head -c 2000 "$TMP" >&2 || true
  echo >&2
  exit 1
fi

# 尽量打印第一条回复正文（有 jq 则解析，否则提示原始 JSON）
if command -v jq >/dev/null 2>&1; then
  CONTENT="$(jq -r '.choices[0].message.content // empty' "$TMP" 2>/dev/null || true)"
  if [[ -n "${CONTENT// /}" ]]; then
    echo "OK — 联通正常"
    echo "回复: $CONTENT"
  else
    echo "OK — HTTP 200，但未解析到 choices[0].message.content，原始响应：" >&2
    head -c 4000 "$TMP" >&2
    echo >&2
    exit 1
  fi
else
  echo "OK — HTTP 200（未安装 jq，未解析正文）。响应前 800 字节："
  head -c 800 "$TMP"
  echo
  echo "提示: 安装 jq 后可自动打印模型回复文本。"
fi

exit 0

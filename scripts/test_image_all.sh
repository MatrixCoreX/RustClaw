#!/usr/bin/env bash
set -euo pipefail

# End-to-end image skills regression:
# 1) image_generate (skill-runner)
# 2) image_vision (skill-runner)
# 3) image_edit (skill-runner, native URL flow for qwen/wanx)

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="debug"
AUTO_BUILD=0
VENDOR="qwen"
GEN_MODEL="qwen-image-plus"
VISION_MODEL="qwen-vl-max-latest"
EDIT_MODEL="wanx2.1-imageedit"
GEN_PROMPT="一只极简红色螃蟹图标，白色背景"
EDIT_INSTRUCTION="把整体色调略微偏暖，保持构图不变"
SIZE="1024x1024"
POLL_SECONDS=2
POLL_ROUNDS=10

TOTAL=0
PASS=0
FAIL=0

usage() {
  cat <<'EOF'
Usage:
  ./scripts/test_image_all.sh [options]

Options:
  --profile debug|release   Build profile (default: debug)
  --auto-build              Auto build missing binaries
  --vendor NAME             Vendor for skill calls (default: qwen)
  --gen-model NAME          image_generate model (default: qwen-image-plus)
  --vision-model NAME       image_vision model (default: qwen-vl-max-latest)
  --edit-model NAME         image_edit model (default: wanx2.1-imageedit)
  --prompt TEXT             Generate prompt
  --edit-instruction TEXT   Edit instruction
  --size WxH                Generate size (default: 1024x1024)
  --help, -h                Show help
EOF
}

need_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "Missing command: $cmd"
    exit 2
  }
}

mark_pass() {
  PASS=$((PASS + 1))
  echo "[PASS] $1"
}

mark_fail() {
  FAIL=$((FAIL + 1))
  echo "[FAIL] $1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    --auto-build)
      AUTO_BUILD=1
      shift
      ;;
    --vendor)
      VENDOR="${2:-}"
      shift 2
      ;;
    --gen-model)
      GEN_MODEL="${2:-}"
      shift 2
      ;;
    --vision-model)
      VISION_MODEL="${2:-}"
      shift 2
      ;;
    --edit-model)
      EDIT_MODEL="${2:-}"
      shift 2
      ;;
    --prompt)
      GEN_PROMPT="${2:-}"
      shift 2
      ;;
    --edit-instruction)
      EDIT_INSTRUCTION="${2:-}"
      shift 2
      ;;
    --size)
      SIZE="${2:-}"
      shift 2
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

if [[ "$PROFILE" != "debug" && "$PROFILE" != "release" ]]; then
  echo "--profile must be debug or release"
  exit 2
fi

need_cmd jq
need_cmd curl
need_cmd python3
need_cmd bash

cd "$ROOT_DIR"

echo "== Image E2E Test =="
echo "profile        : $PROFILE"
echo "vendor         : $VENDOR"
echo "gen_model      : $GEN_MODEL"
echo "vision_model   : $VISION_MODEL"
echo "edit_model     : $EDIT_MODEL"
echo

AUTO_BUILD_ARG=()
if [[ "$AUTO_BUILD" == "1" ]]; then
  AUTO_BUILD_ARG=(--auto-build)
fi

GEN_OUTPUT_PATH="image/download/e2e-gen-$(date +%Y%m%d-%H%M%S).png"
GEN_ARGS="$(
  jq -nc \
    --arg p "$GEN_PROMPT" \
    --arg v "$VENDOR" \
    --arg m "$GEN_MODEL" \
    --arg s "$SIZE" \
    --arg out "$GEN_OUTPUT_PATH" \
    '{prompt:$p,vendor:$v,model:$m,size:$s,n:1,output_path:$out}'
)"

TOTAL=$((TOTAL + 1))
GEN_RESP="$(bash scripts/skill_calls/call_image_generate.sh --profile "$PROFILE" --args "$GEN_ARGS" --raw "${AUTO_BUILD_ARG[@]}")"
if echo "$GEN_RESP" | jq -e '.status=="ok"' >/dev/null 2>&1; then
  GEN_FILE_PATH="$(echo "$GEN_RESP" | jq -r '.extra.outputs[0].path // empty')"
  if [[ -n "$GEN_FILE_PATH" && -f "$GEN_FILE_PATH" ]]; then
    mark_pass "image_generate"
  else
    mark_fail "image_generate (output file missing)"
  fi
else
  mark_fail "image_generate"
  echo "$GEN_RESP" | jq .
  echo
  echo "== Summary =="
  echo "TOTAL: $TOTAL  PASS: $PASS  FAIL: $FAIL"
  exit 1
fi

TOTAL=$((TOTAL + 1))
VISION_ARGS="$(
  jq -nc \
    --arg v "$VENDOR" \
    --arg m "$VISION_MODEL" \
    --arg img "$GEN_FILE_PATH" \
    '{action:"describe",vendor:$v,model:$m,images:[$img]}'
)"
VISION_RESP="$(bash scripts/skill_calls/call_image_vision.sh --profile "$PROFILE" --args "$VISION_ARGS" --raw "${AUTO_BUILD_ARG[@]}")"
if echo "$VISION_RESP" | jq -e '.status=="ok"' >/dev/null 2>&1; then
  mark_pass "image_vision"
else
  mark_fail "image_vision"
  echo "$VISION_RESP" | jq .
fi

QWEN_KEY="$(
python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
print(((cfg.get("llm") or {}).get("qwen") or {}).get("api_key",""))
PY
)"
if [[ -z "$QWEN_KEY" ]]; then
  echo "[WARN] llm.qwen.api_key is empty, skip image_edit native-url test."
  echo
  echo "== Summary =="
  echo "TOTAL: $TOTAL  PASS: $PASS  FAIL: $FAIL"
  [[ "$FAIL" -eq 0 ]] && exit 0 || exit 1
fi

NATIVE_CREATE_RESP="$(
  curl -sS --max-time 90 \
    -X POST 'https://dashscope.aliyuncs.com/api/v1/services/aigc/text2image/image-synthesis' \
    -H "Authorization: Bearer $QWEN_KEY" \
    -H 'Content-Type: application/json' \
    -H 'X-DashScope-Async: enable' \
    -d "{\"model\":\"$GEN_MODEL\",\"input\":{\"prompt\":\"$GEN_PROMPT\"},\"parameters\":{\"size\":\"1024*1024\",\"n\":1,\"watermark\":false}}"
)"
TASK_ID="$(echo "$NATIVE_CREATE_RESP" | jq -r '.output.task_id // empty')"
if [[ -z "$TASK_ID" ]]; then
  TOTAL=$((TOTAL + 1))
  mark_fail "image_edit (cannot create native task for base image url)"
  echo "$NATIVE_CREATE_RESP" | jq .
  echo
  echo "== Summary =="
  echo "TOTAL: $TOTAL  PASS: $PASS  FAIL: $FAIL"
  exit 1
fi

BASE_IMG_URL=""
for ((i=1; i<=POLL_ROUNDS; i++)); do
  POLL_RESP="$(
    curl -sS --max-time 60 \
      -H "Authorization: Bearer $QWEN_KEY" \
      "https://dashscope.aliyuncs.com/api/v1/tasks/$TASK_ID"
  )"
  STATUS="$(echo "$POLL_RESP" | jq -r '.output.task_status // empty')"
  BASE_IMG_URL="$(echo "$POLL_RESP" | jq -r '.output.results[0].url // empty')"
  if [[ "$STATUS" == "SUCCEEDED" && -n "$BASE_IMG_URL" ]]; then
    break
  fi
  sleep "$POLL_SECONDS"
done

TOTAL=$((TOTAL + 1))
if [[ -z "$BASE_IMG_URL" ]]; then
  mark_fail "image_edit (no base image url from native task)"
else
  EDIT_ARGS="$(
    jq -nc \
      --arg v "$VENDOR" \
      --arg m "$EDIT_MODEL" \
      --arg img "$BASE_IMG_URL" \
      --arg ins "$EDIT_INSTRUCTION" \
      --arg out "image/download/e2e-edit-$(date +%Y%m%d-%H%M%S).png" \
      '{action:"edit",vendor:$v,model:$m,image:$img,instruction:$ins,output_path:$out}'
  )"
  EDIT_RESP="$(bash scripts/skill_calls/call_image_edit.sh --profile "$PROFILE" --args "$EDIT_ARGS" --raw "${AUTO_BUILD_ARG[@]}")"
  if echo "$EDIT_RESP" | jq -e '.status=="ok"' >/dev/null 2>&1; then
    mark_pass "image_edit"
  else
    mark_fail "image_edit"
    echo "$EDIT_RESP" | jq .
  fi
fi

echo
echo "== Summary =="
echo "TOTAL: $TOTAL  PASS: $PASS  FAIL: $FAIL"
[[ "$FAIL" -eq 0 ]] && exit 0 || exit 1

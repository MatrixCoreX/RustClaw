#!/usr/bin/env bash
set -euo pipefail

# Test image_generate skill via skill-runner directly.
#
# Usage:
#   ./scripts/test_image_module.sh [--profile debug|release] [--vendor qwen] [--model MODEL] [--prompt TEXT] [--size 1024x1024] [--output-path PATH] [--auto-build] [--raw]

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="debug"
VENDOR="${VENDOR:-qwen}"
MODEL="${MODEL:-}"
PROMPT="${PROMPT:-A minimal flat icon of a crab, clean background}"
SIZE="${SIZE:-1024x1024}"
OUTPUT_PATH="${OUTPUT_PATH:-}"
AUTO_BUILD=0
RAW=0

usage() {
  cat <<'EOF'
Usage:
  ./scripts/test_image_module.sh [options]

Options:
  --profile debug|release   Build profile (default: debug)
  --vendor NAME             Vendor name (default: qwen)
  --model MODEL             Model name (default: auto-read from configs/image.toml image_generation.default_model)
  --prompt TEXT             Prompt for generation
  --size WxH                Image size (default: 1024x1024)
  --output-path PATH        Output file path (default: image/download/test-image-<ts>.png)
  --auto-build              Auto build missing binaries
  --raw                     Print raw response JSON
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

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    --vendor)
      VENDOR="${2:-}"
      shift 2
      ;;
    --model)
      MODEL="${2:-}"
      shift 2
      ;;
    --prompt)
      PROMPT="${2:-}"
      shift 2
      ;;
    --size)
      SIZE="${2:-}"
      shift 2
      ;;
    --output-path)
      OUTPUT_PATH="${2:-}"
      shift 2
      ;;
    --auto-build)
      AUTO_BUILD=1
      shift
      ;;
    --raw)
      RAW=1
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

if [[ "$PROFILE" != "debug" && "$PROFILE" != "release" ]]; then
  echo "--profile must be debug or release"
  exit 2
fi

need_cmd jq
need_cmd python3
need_cmd bash

if [[ -z "$MODEL" ]]; then
  MODEL="$(
    python3 - <<'PY'
import tomllib
from pathlib import Path
p = Path("configs/image.toml")
if not p.exists():
    print("")
    raise SystemExit(0)
cfg = tomllib.loads(p.read_text(encoding="utf-8"))
print(((cfg.get("image_generation") or {}).get("default_model") or "").strip())
PY
  )"
fi

if [[ -z "$MODEL" ]]; then
  echo "Model is empty. Set --model or configs/image.toml image_generation.default_model."
  exit 2
fi

if [[ -z "$OUTPUT_PATH" ]]; then
  OUTPUT_PATH="image/download/test-image-$(date +%Y%m%d-%H%M%S).png"
fi

ARGS_JSON="$(
  jq -nc \
    --arg prompt "$PROMPT" \
    --arg size "$SIZE" \
    --arg vendor "$VENDOR" \
    --arg model "$MODEL" \
    --arg output "$OUTPUT_PATH" \
    '{
      prompt: $prompt,
      size: $size,
      n: 1,
      vendor: $vendor,
      model: $model,
      output_path: $output
    }'
)"

echo "== image_generate module test =="
echo "profile    : $PROFILE"
echo "vendor     : $VENDOR"
echo "model      : $MODEL"
echo "size       : $SIZE"
echo "output     : $OUTPUT_PATH"
echo

call_cmd=(bash "$ROOT_DIR/scripts/skill_calls/call_image_generate.sh" --profile "$PROFILE" --args "$ARGS_JSON" --raw)
if [[ "$AUTO_BUILD" == "1" ]]; then
  call_cmd+=(--auto-build)
fi

RESP="$("${call_cmd[@]}")"

if [[ "$RAW" == "1" ]]; then
  printf '%s\n' "$RESP"
  exit 0
fi

STATUS="$(echo "$RESP" | jq -r '.status // "unknown"')"
if [[ "$STATUS" == "ok" ]]; then
  PROVIDER="$(echo "$RESP" | jq -r '.extra.provider // ""')"
  USED_MODEL="$(echo "$RESP" | jq -r '.extra.model // ""')"
  FILE_PATH="$(echo "$RESP" | jq -r '.extra.file_path // empty')"
  [[ -z "$FILE_PATH" ]] && FILE_PATH="$OUTPUT_PATH"
  echo "[PASS] image_generate ok"
  echo "provider   : ${PROVIDER:-unknown}"
  echo "used_model : ${USED_MODEL:-unknown}"
  echo "file_path  : $FILE_PATH"
  exit 0
fi

ERR="$(echo "$RESP" | jq -r '.error_text // "unknown error"')"
echo "[FAIL] image_generate error"
echo "error_text : $ERR"
echo
echo "Raw response:"
echo "$RESP" | jq .
exit 1

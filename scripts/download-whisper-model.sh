#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

MODEL_DIR="${WHISPER_MODEL_DIR:-${ROOT_DIR}/data/models/whisper.cpp}"
BASE_URL="${WHISPER_MODEL_BASE_URL:-https://huggingface.co/ggerganov/whisper.cpp/resolve/main}"
SERVER_PORT="${WHISPER_SERVER_PORT:-8178}"
SERVER_BIN="${WHISPER_SERVER_BIN:-${ROOT_DIR}/data/vendor/whisper.cpp/build/bin/whisper-server}"

MODEL=""
DRY_RUN=0
FORCE=0
PRINT_PATH_ONLY=0

usage() {
  cat <<'USAGE'
Usage: scripts/download-whisper-model.sh [options]

Download a multilingual whisper.cpp model into data/models/whisper.cpp.
By default, the script recommends a model from detected physical memory:
  < 3 GiB   -> tiny
  < 6 GiB   -> base
  < 12 GiB  -> small
  >= 12 GiB -> medium

Options:
  --model <name>       Override auto selection. Supported: tiny, base, small, medium, large-v3.
  --model-dir <path>   Override download directory.
  --base-url <url>     Override model base URL.
  --dry-run            Print the selected model and target path without downloading.
  --force              Re-download even if the target file already exists.
  --print-path-only    Print only the final model path on stdout; logs go to stderr.
  -h, --help           Show this help.

Environment overrides:
  WHISPER_MODEL_DIR, WHISPER_MODEL_BASE_URL, WHISPER_SERVER_BIN, WHISPER_SERVER_PORT
USAGE
}

log() {
  if (( PRINT_PATH_ONLY )); then
    printf '%s\n' "$*" >&2
  else
    printf '%s\n' "$*"
  fi
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

detect_total_mem_bytes() {
  if [[ -r /proc/meminfo ]]; then
    awk '/^MemTotal:/ { printf "%.0f\n", $2 * 1024; exit }' /proc/meminfo
    return
  fi

  if command -v sysctl >/dev/null 2>&1; then
    local sysctl_bytes
    sysctl_bytes="$(sysctl -n hw.memsize 2>/dev/null || true)"
    if [[ "$sysctl_bytes" =~ ^[0-9]+$ ]]; then
      printf '%s\n' "$sysctl_bytes"
      return
    fi
  fi

  local pages page_size
  pages="$(getconf _PHYS_PAGES 2>/dev/null || true)"
  page_size="$(getconf PAGE_SIZE 2>/dev/null || getconf PAGESIZE 2>/dev/null || true)"
  if [[ "$pages" =~ ^[0-9]+$ && "$page_size" =~ ^[0-9]+$ ]]; then
    printf '%s\n' "$(( pages * page_size ))"
    return
  fi

  return 1
}

normalize_model() {
  local raw="$1"
  raw="${raw#ggml-}"
  raw="${raw%.bin}"

  case "$raw" in
    tiny|base|small|medium|large-v3)
      printf '%s\n' "$raw"
      ;;
    *)
      die "unsupported model '$1' (supported: tiny, base, small, medium, large-v3)"
      ;;
  esac
}

model_filename() {
  printf 'ggml-%s.bin\n' "$1"
}

model_size_hint() {
  case "$1" in
    tiny) printf '~75 MiB' ;;
    base) printf '~142 MiB' ;;
    small) printf '~466 MiB' ;;
    medium) printf '~1.5 GiB' ;;
    large-v3) printf '~2.9 GiB' ;;
    *) printf 'unknown size' ;;
  esac
}

select_model_for_memory() {
  local bytes="$1"
  local mib=$(( bytes / 1024 / 1024 ))

  if (( mib < 3072 )); then
    printf 'tiny\n'
  elif (( mib < 6144 )); then
    printf 'base\n'
  elif (( mib < 12288 )); then
    printf 'small\n'
  else
    printf 'medium\n'
  fi
}

download_model() {
  local url="$1"
  local target="$2"
  local tmp="${target}.part.$$"

  rm -f "$tmp"
  if command -v curl >/dev/null 2>&1; then
    curl -L --fail --progress-bar --output "$tmp" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget --no-config --quiet --show-progress -O "$tmp" "$url"
  else
    die "curl or wget is required to download the model"
  fi

  if [[ ! -s "$tmp" ]]; then
    rm -f "$tmp"
    die "downloaded file is empty: $url"
  fi

  mv "$tmp" "$target"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --model)
      [[ $# -ge 2 ]] || die "--model requires a value"
      MODEL="$(normalize_model "$2")"
      shift 2
      ;;
    --model=*)
      MODEL="$(normalize_model "${1#*=}")"
      shift
      ;;
    --model-dir)
      [[ $# -ge 2 ]] || die "--model-dir requires a value"
      MODEL_DIR="$2"
      shift 2
      ;;
    --model-dir=*)
      MODEL_DIR="${1#*=}"
      shift
      ;;
    --base-url)
      [[ $# -ge 2 ]] || die "--base-url requires a value"
      BASE_URL="$2"
      shift 2
      ;;
    --base-url=*)
      BASE_URL="${1#*=}"
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --force)
      FORCE=1
      shift
      ;;
    --print-path-only)
      PRINT_PATH_ONLY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

MEM_BYTES="$(detect_total_mem_bytes || true)"
if [[ -z "${MODEL}" ]]; then
  if [[ -z "${MEM_BYTES}" ]]; then
    MODEL="small"
    MEMORY_NOTE="memory detection failed; using small as a balanced default"
  else
    MODEL="$(select_model_for_memory "$MEM_BYTES")"
    MEMORY_MIB=$(( MEM_BYTES / 1024 / 1024 ))
    MEMORY_NOTE="detected memory: ${MEMORY_MIB} MiB"
  fi
else
  MEMORY_NOTE="manual model override"
fi

FILENAME="$(model_filename "$MODEL")"
TARGET="${MODEL_DIR%/}/${FILENAME}"
URL="${BASE_URL%/}/${FILENAME}"

if (( PRINT_PATH_ONLY == 0 )); then
  log "Selected model: ${MODEL} ($(model_size_hint "$MODEL"))"
  log "${MEMORY_NOTE}"
  log "Target path: ${TARGET}"
fi

if (( DRY_RUN )); then
  log "Dry run: not downloading."
  if (( PRINT_PATH_ONLY )); then
    printf '%s\n' "$TARGET"
  fi
  exit 0
fi

mkdir -p "$MODEL_DIR"

if [[ -f "$TARGET" && "$FORCE" -eq 0 ]]; then
  log "Model already exists; skip download."
else
  log "Downloading: ${URL}"
  download_model "$URL" "$TARGET"
  log "Download complete."
fi

if (( PRINT_PATH_ONLY )); then
  printf '%s\n' "$TARGET"
else
  log "Whisper server example:"
  log "${SERVER_BIN} -m ${TARGET} --host 127.0.0.1 --port ${SERVER_PORT} --request-path /v1 --inference-path /audio/transcriptions --convert --language auto"
fi

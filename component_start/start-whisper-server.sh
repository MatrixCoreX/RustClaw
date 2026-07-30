#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT_DIR/component_start/common.sh"

PROFILE="release"
CHECK_ONLY=0
for arg in "$@"; do
  case "$arg" in
    release) PROFILE="release" ;;
    --check) CHECK_ONLY=1 ;;
    -h|--help)
      echo "Usage: component_start/start-whisper-server.sh [release] [--check]"
      exit 0
      ;;
    *)
      echo "Unknown option: $arg" >&2
      exit 2
      ;;
  esac
done

component_start_init "$ROOT_DIR" "$PROFILE" "./component_start/start-whisper-server.sh"

AUDIO_CONFIG_PATH="${APP_AUDIO_CONFIG_PATH:-$ROOT_DIR/configs/audio.toml}"
if [[ ! -f "$AUDIO_CONFIG_PATH" ]]; then
  echo "Audio config not found: $AUDIO_CONFIG_PATH" >&2
  exit 1
fi

CONFIG_VALUES="$(python3 - "$AUDIO_CONFIG_PATH" <<'PY'
import json
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
section = ""
values = {}

def strip_comment(line: str) -> str:
    quoted = False
    escaped = False
    out = []
    for char in line:
        if char == "#" and not quoted:
            break
        out.append(char)
        if escaped:
            escaped = False
        elif char == "\\" and quoted:
            escaped = True
        elif char == '"':
            quoted = not quoted
    return "".join(out).strip()

for raw in path.read_text(encoding="utf-8").splitlines():
    line = strip_comment(raw)
    if not line:
        continue
    table = re.fullmatch(r"\[\s*([^]]+?)\s*\]", line)
    if table:
        section = table.group(1).strip()
        continue
    if section != "audio_transcribe":
        continue
    pair = re.fullmatch(r"([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.+)", line)
    if not pair:
        continue
    key, raw_value = pair.groups()
    if raw_value.startswith('"'):
        try:
            values[key] = str(json.loads(raw_value))
        except json.JSONDecodeError:
            continue
    else:
        values[key] = raw_value.strip()

ordered = [
    values.get("default_vendor", ""),
    values.get("default_model", ""),
    values.get("local_server_enabled", "false"),
    values.get("local_server_binary", "data/vendor/whisper.cpp/build/bin/whisper-server"),
    values.get("local_model_path", ""),
    values.get("local_server_host", "127.0.0.1"),
    values.get("local_server_port", "8178"),
    values.get("local_server_threads", "4"),
]
print("\x1f".join(ordered))
PY
)"
IFS=$'\x1f' read -r DEFAULT_VENDOR DEFAULT_MODEL CONFIG_ENABLED CONFIG_SERVER_BIN \
  CONFIG_MODEL_PATH CONFIG_HOST CONFIG_PORT CONFIG_THREADS <<<"$CONFIG_VALUES"

ENABLED="${APP_LOCAL_WHISPER_ENABLED:-auto}"
case "$ENABLED" in
  auto)
    if [[ "$CONFIG_ENABLED" != "true" || "$DEFAULT_VENDOR" != "custom" || "$DEFAULT_MODEL" != "local-whisper" ]]; then
      echo "Local whisper.cpp is not selected; skipping component startup."
      exit 0
    fi
    ;;
  1|true|yes) ;;
  0|false|no)
    echo "Local whisper.cpp startup is disabled."
    exit 0
    ;;
  *)
    echo "APP_LOCAL_WHISPER_ENABLED must be auto, true, or false." >&2
    exit 2
    ;;
esac

resolve_path() {
  case "$1" in
    /*) printf '%s\n' "$1" ;;
    *) printf '%s\n' "$ROOT_DIR/$1" ;;
  esac
}

SERVER_BIN="$(resolve_path "${WHISPER_SERVER_BIN:-$CONFIG_SERVER_BIN}")"
MODEL_PATH="${WHISPER_MODEL_PATH:-$CONFIG_MODEL_PATH}"
if [[ -z "$MODEL_PATH" ]]; then
  MODEL_PATH="$({
    for candidate in "$ROOT_DIR"/data/models/whisper.cpp/ggml-*.bin; do
      [[ -f "$candidate" ]] || continue
      [[ "$candidate" == *.en.bin ]] && continue
      printf '%s\n' "$candidate"
    done
  } | LC_ALL=C sort | head -n 1 || true)"
else
  MODEL_PATH="$(resolve_path "$MODEL_PATH")"
fi
HOST="${WHISPER_SERVER_HOST:-$CONFIG_HOST}"
PORT="${WHISPER_SERVER_PORT:-$CONFIG_PORT}"
THREADS="${WHISPER_SERVER_THREADS:-$CONFIG_THREADS}"

[[ "$PORT" =~ ^[0-9]+$ ]] || { echo "Invalid whisper server port: $PORT" >&2; exit 2; }
[[ "$THREADS" =~ ^[0-9]+$ ]] || { echo "Invalid whisper server thread count: $THREADS" >&2; exit 2; }
[[ -x "$SERVER_BIN" ]] || { echo "whisper-server is missing: $SERVER_BIN" >&2; exit 1; }
[[ -f "$MODEL_PATH" ]] || { echo "Whisper model is missing: $MODEL_PATH" >&2; exit 1; }
command -v ffmpeg >/dev/null 2>&1 || { echo "ffmpeg is required for browser WebM transcription." >&2; exit 1; }

if (( CHECK_ONLY )); then
  echo "Local whisper.cpp is ready: model=$(basename "$MODEL_PATH"), endpoint=http://$HOST:$PORT/v1/audio/transcriptions"
  exit 0
fi

if component_pid_is_running "whisper-server" "$SERVER_BIN"; then
  echo "whisper-server is already running."
  exit 0
fi

LOG_DIR="$ROOT_DIR/logs"
TMP_DIR="$ROOT_DIR/data/tmp/whisper"
mkdir -p "$LOG_DIR" "$TMP_DIR"
component_launch_detached "$LOG_DIR/whisper-server.log" "$SERVER_BIN" \
  -m "$MODEL_PATH" \
  --host "$HOST" \
  --port "$PORT" \
  --threads "$THREADS" \
  --request-path /v1 \
  --inference-path /audio/transcriptions \
  --convert \
  --tmp-dir "$TMP_DIR" \
  --language auto
PID="$COMPONENT_LAUNCHED_PID"
component_write_pid_file "whisper-server" "$PID"

READY_WAIT="${WHISPER_SERVER_READY_WAIT_SECONDS:-5}"
[[ "$READY_WAIT" =~ ^[0-9]+$ ]] || READY_WAIT=5
for ((second = 0; second < READY_WAIT; second++)); do
  if ! component_process_is_running "$PID"; then
    rm -f "$COMPONENT_PID_DIR/whisper-server.pid"
    echo "whisper-server exited during startup; see $LOG_DIR/whisper-server.log" >&2
    exit 1
  fi
  if curl -sS --max-time 1 -o /dev/null "http://$HOST:$PORT/" 2>/dev/null; then
    echo "whisper-server is ready: PID=$PID, endpoint=http://$HOST:$PORT/v1/audio/transcriptions"
    exit 0
  fi
  sleep 1
done

echo "whisper-server started and is loading the model: PID=$PID, log=$LOG_DIR/whisper-server.log"
exit 0

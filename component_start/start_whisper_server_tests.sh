#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/component_start/start-whisper-server.sh"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
TRUE_BIN="$(type -P true)"
FALSE_BIN="$(type -P false)"

cat >"$TMP_ROOT/local.toml" <<'EOF'
[audio_transcribe]
default_vendor = "custom"
default_model = "local-whisper"
local_server_enabled = true
EOF

ready="$({
  APP_AUDIO_CONFIG_PATH="$TMP_ROOT/local.toml" \
  WHISPER_SERVER_BIN="$TRUE_BIN" \
  WHISPER_MODEL_PATH="$TRUE_BIN" \
    "$SCRIPT" --check
})"
[[ "$ready" == *"Local whisper.cpp is ready"* ]]

cat >"$TMP_ROOT/remote.toml" <<'EOF'
[audio_transcribe]
default_vendor = "qwen"
default_model = "qwen3-asr-flash"
local_server_enabled = true
EOF

skipped="$({
  APP_AUDIO_CONFIG_PATH="$TMP_ROOT/remote.toml" \
  WHISPER_SERVER_BIN="$FALSE_BIN" \
  WHISPER_MODEL_PATH=/definitely/missing \
    "$SCRIPT" --check
})"
[[ "$skipped" == *"not selected"* ]]

if APP_AUDIO_CONFIG_PATH="$TMP_ROOT/local.toml" \
  WHISPER_SERVER_BIN="$TRUE_BIN" \
  WHISPER_MODEL_PATH=/definitely/missing \
  "$SCRIPT" --check >/dev/null 2>&1; then
  echo "Missing local model unexpectedly passed validation." >&2
  exit 1
fi

echo "START_WHISPER_SERVER_TESTS ok"

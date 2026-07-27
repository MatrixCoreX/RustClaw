#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/component_start/start-whisper-server.sh"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

cat >"$TMP_ROOT/local.toml" <<'EOF'
[audio_transcribe]
default_vendor = "custom"
default_model = "local-whisper"
local_server_enabled = true
EOF

ready="$({
  RUSTCLAW_AUDIO_CONFIG_PATH="$TMP_ROOT/local.toml" \
  WHISPER_SERVER_BIN=/bin/true \
  WHISPER_MODEL_PATH=/bin/true \
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
  RUSTCLAW_AUDIO_CONFIG_PATH="$TMP_ROOT/remote.toml" \
  WHISPER_SERVER_BIN=/bin/false \
  WHISPER_MODEL_PATH=/definitely/missing \
    "$SCRIPT" --check
})"
[[ "$skipped" == *"not selected"* ]]

if RUSTCLAW_AUDIO_CONFIG_PATH="$TMP_ROOT/local.toml" \
  WHISPER_SERVER_BIN=/bin/true \
  WHISPER_MODEL_PATH=/definitely/missing \
  "$SCRIPT" --check >/dev/null 2>&1; then
  echo "Missing local model unexpectedly passed validation." >&2
  exit 1
fi

echo "START_WHISPER_SERVER_TESTS ok"

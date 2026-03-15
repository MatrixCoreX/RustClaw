#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "== [1/2] audio_synthesize =="
bash "$ROOT_DIR/scripts/skill_calls/call_audio_synthesize.sh" \
  --args '{"text":"你好，这是 RustClaw 音频联调。","voice":"Cherry"}'

echo
echo "== [2/2] audio_transcribe (fun-asr, url input) =="
bash "$ROOT_DIR/scripts/skill_calls/call_audio_transcribe.sh" \
  --args '{"vendor":"qwen","model":"fun-asr","audio_url":"https://dashscope.oss-cn-beijing.aliyuncs.com/samples/audio/paraformer/hello_world_female2.wav"}'

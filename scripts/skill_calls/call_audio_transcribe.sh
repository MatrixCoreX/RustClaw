#!/usr/bin/env bash
SKILL_NAME="audio_transcribe"
DEFAULT_ARGS='{"audio_path":"audio/sample.wav","language":"zh"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

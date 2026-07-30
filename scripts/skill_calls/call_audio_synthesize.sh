#!/usr/bin/env bash
SKILL_NAME="audio_synthesize"
DEFAULT_ARGS='{"text":"你好，这是音频联调。","voice":"nova"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

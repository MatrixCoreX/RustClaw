#!/usr/bin/env bash
SKILL_NAME="audio_synthesize"
DEFAULT_ARGS='{"text":"你好，这是 RustClaw","voice":"nova"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

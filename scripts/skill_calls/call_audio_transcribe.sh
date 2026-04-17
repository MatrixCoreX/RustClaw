#!/usr/bin/env bash
SKILL_NAME="audio_transcribe"
DEFAULT_ARGS='{"audio_url":"https://dashscope.oss-cn-beijing.aliyuncs.com/samples/audio/paraformer/hello_world_female2.wav","language":"zh"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

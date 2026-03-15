#!/usr/bin/env bash
SKILL_NAME="image_vision"
DEFAULT_ARGS='{"action":"describe","images":[]}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

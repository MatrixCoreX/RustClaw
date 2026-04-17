#!/usr/bin/env bash
SKILL_NAME="image_edit"
DEFAULT_ARGS='{"action":"edit","instruction":"increase contrast slightly"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

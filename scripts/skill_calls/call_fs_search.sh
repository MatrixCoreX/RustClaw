#!/usr/bin/env bash
SKILL_NAME="fs_search"
DEFAULT_ARGS='{"action":"find_name","path":".","name":"README","limit":20}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

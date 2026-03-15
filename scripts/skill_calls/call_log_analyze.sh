#!/usr/bin/env bash
SKILL_NAME="log_analyze"
DEFAULT_ARGS='{"action":"summary","path":"logs/clawd.log","limit":200}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

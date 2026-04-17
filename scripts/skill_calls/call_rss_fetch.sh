#!/usr/bin/env bash
SKILL_NAME="rss_fetch"
DEFAULT_ARGS='{"action":"latest","category":"general","limit":5}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

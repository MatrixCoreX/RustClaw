#!/usr/bin/env bash
SKILL_NAME="web_search_extract"
DEFAULT_ARGS='{"action":"search_extract","query":"rust async tutorial","top_k":3,"include_snippet":true}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

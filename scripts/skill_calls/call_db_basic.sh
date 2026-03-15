#!/usr/bin/env bash
SKILL_NAME="db_basic"
DEFAULT_ARGS='{"action":"query","dialect":"sqlite","sql":"select 1 as ok;"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

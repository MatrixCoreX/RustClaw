#!/usr/bin/env bash
SKILL_NAME="stock"
DEFAULT_ARGS='{"action":"quote","symbol":"600519"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

#!/usr/bin/env bash
SKILL_NAME="health_check"
DEFAULT_ARGS='{"action":"check"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

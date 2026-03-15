#!/usr/bin/env bash
SKILL_NAME="system_basic"
DEFAULT_ARGS='{"action":"uptime"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

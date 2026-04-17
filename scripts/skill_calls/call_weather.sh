#!/usr/bin/env bash
SKILL_NAME="weather"
DEFAULT_ARGS='{"action":"query","city":"Beijing"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

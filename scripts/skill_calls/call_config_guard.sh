#!/usr/bin/env bash
SKILL_NAME="config_guard"
DEFAULT_ARGS='{"action":"validate","path":"configs/config.toml"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

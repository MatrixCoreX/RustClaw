#!/usr/bin/env bash
SKILL_NAME="http_basic"
DEFAULT_ARGS='{"method":"GET","url":"https://api.github.com","timeout_seconds":15}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

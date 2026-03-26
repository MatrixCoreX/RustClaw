#!/usr/bin/env bash
SKILL_NAME="http_basic"
DEFAULT_ARGS='{"action":"get","url":"http://127.0.0.1:8787/api/health","timeout_seconds":5}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

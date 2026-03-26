#!/usr/bin/env bash
SKILL_NAME="archive_basic"
DEFAULT_ARGS='{"action":"pack","source":"scripts/skill_calls","archive":"tmp/archive-basic-smoke.zip","format":"zip"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

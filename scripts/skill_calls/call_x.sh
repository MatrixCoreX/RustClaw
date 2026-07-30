#!/usr/bin/env bash
SKILL_NAME="x"
DEFAULT_ARGS='{"text":"hello from the agent runtime","dry_run":true}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

#!/usr/bin/env bash
SKILL_NAME="install_module"
DEFAULT_ARGS='{"manager":"npm","module":"lodash","dry_run":true}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

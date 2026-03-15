#!/usr/bin/env bash
SKILL_NAME="package_manager"
DEFAULT_ARGS='{"action":"info","manager":"npm","package":"react"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

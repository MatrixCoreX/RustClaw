#!/usr/bin/env bash
SKILL_NAME="service_control"
DEFAULT_ARGS='{"action":"status","service":"clawd"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

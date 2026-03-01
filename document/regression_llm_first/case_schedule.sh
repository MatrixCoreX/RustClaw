#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
run_case "schedule" "每隔5分钟提醒我：LLM_FIRST_SCHEDULE_OK" "LLM_FIRST_SCHEDULE_OK"

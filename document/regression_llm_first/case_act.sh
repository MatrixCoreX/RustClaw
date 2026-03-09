#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
run_case "act" "请只执行这一条命令，不要做其他动作：echo LLM_FIRST_ACT_OK" "LLM_FIRST_ACT_OK"

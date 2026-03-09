#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
# 非系统命令风格请求：预期失败，但失败详情应包含 command not found
run_case_expect "act_custom_cmd" "执行 rustclaw_custom_demo_command --version" "failed" "command not found" "error"

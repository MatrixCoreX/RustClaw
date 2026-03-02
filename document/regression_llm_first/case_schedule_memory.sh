#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
run_case_expect "schedule_memory_list" "查看所有定时任务" "succeeded"
run_case_expect \
  "schedule_memory_close_these" \
  "关闭这些" \
  "succeeded" \
  "" \
  "text" \
  "请提供任务ID" \
  "either"
run_case_expect \
  "schedule_memory_all_disable" \
  "全部禁用" \
  "succeeded" \
  "" \
  "text" \
  "未找到任务ID：ALL" \
  "either"

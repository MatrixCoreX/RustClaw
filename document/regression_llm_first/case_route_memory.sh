#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
run_case_expect \
  "route_memory_seed" \
  "请记住：稍后当我说“继续”时，表示执行命令 echo ROUTE_MEMORY_OK。现在只回复“已记住”。" \
  "succeeded" \
  "已记住" \
  "text"
run_case_expect \
  "route_memory_followup" \
  "继续" \
  "succeeded" \
  "ROUTE_MEMORY_OK" \
  "text"

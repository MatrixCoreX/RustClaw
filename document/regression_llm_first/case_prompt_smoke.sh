#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

# Keep authorized USER_ID; isolate CHAT_ID only to avoid memory interference.
base_chat_id="${CHAT_ID:-1985996990}"
isolate_suffix="$(date +%s | tail -c 7)"
export CHAT_ID="$((base_chat_id + isolate_suffix))"

health_check

run_case_expect \
  "prompt_route_chat_only" \
  "请只做解释，不要执行任何操作：LLM_FIRST_PROMPT_CHAT_OK" \
  "succeeded" \
  "LLM_FIRST_PROMPT_CHAT_OK" \
  "text"

run_case_expect \
  "prompt_route_act_only" \
  "请只执行这一条命令，不要做其他动作：echo LLM_FIRST_PROMPT_ACT_OK" \
  "succeeded" \
  "LLM_FIRST_PROMPT_ACT_OK" \
  "text"

run_case_expect \
  "prompt_route_chat_act" \
  "请执行命令 echo LLM_FIRST_PROMPT_CHAT_ACT_OK 并总结结果" \
  "succeeded" \
  "LLM_FIRST_PROMPT_CHAT_ACT_OK" \
  "text"

run_case_expect \
  "prompt_schedule_bulk_delete" \
  "删除所有定时任务" \
  "succeeded"


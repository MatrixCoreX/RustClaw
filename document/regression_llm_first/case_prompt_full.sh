#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

# Keep authorized USER_ID; isolate CHAT_ID to avoid memory cross-talk.
base_chat_id="${CHAT_ID:-1985996990}"
isolate_suffix="$(date +%s | tail -c 7)"
export CHAT_ID="$((base_chat_id + isolate_suffix))"

health_check

echo "[FLOW] routing and chat/act coverage"
run_case_expect \
  "full_route_chat" \
  "请只做解释，不要执行任何操作：PROMPT_FULL_CHAT_OK" \
  "succeeded" \
  "PROMPT_FULL_CHAT_OK" \
  "text"

run_case_expect \
  "full_route_act" \
  "请只执行这一条命令，不要做其他动作：echo PROMPT_FULL_ACT_OK" \
  "succeeded" \
  "PROMPT_FULL_ACT_OK" \
  "text"

run_case_expect \
  "full_route_chat_act" \
  "请执行命令 echo PROMPT_FULL_CHAT_ACT_OK 并总结结果" \
  "succeeded" \
  "PROMPT_FULL_CHAT_ACT_OK" \
  "text"

run_case_expect \
  "full_route_ask_clarify_like" \
  "继续" \
  "succeeded"

echo "[FLOW] schedule coverage"
run_case_expect \
  "full_schedule_create" \
  "每隔23分钟提醒我：PROMPT_FULL_SCHEDULE_CREATE" \
  "succeeded"

run_case_expect \
  "full_schedule_list" \
  "查看定时任务" \
  "succeeded"

run_case_expect \
  "full_schedule_delete_bulk" \
  "删除所有定时任务" \
  "succeeded"

echo "[FLOW] language consistency spot check"
run_case_expect \
  "full_language_reply_switch" \
  "请用西班牙语简短回复：hola" \
  "succeeded"

echo "[FLOW] image skill spot check (allow provider-dependent result)"
run_skill_case_expect \
  "full_image_vision_describe_invalid_path" \
  "image_vision" \
  '{"action":"describe","images":[{"path":"image/not_found_for_regression.png"}]}' \
  "failed,succeeded"

echo "[FLOW] prompt full regression done"

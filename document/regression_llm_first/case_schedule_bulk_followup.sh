#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

# This case may involve multiple routed steps; allow longer polling by default.
export MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-360}"
export EXTRA_GRACE_SECONDS="${EXTRA_GRACE_SECONDS:-240}"

health_check

echo "[FLOW] seed scheduled jobs"
run_case_expect \
  "schedule_seed_a" \
  "每隔17分钟提醒我：BULK_FLOW_A" \
  "succeeded" \
  "已创建定时任务" \
  "text"
run_case_expect \
  "schedule_seed_b" \
  "每隔19分钟提醒我：BULK_FLOW_B" \
  "succeeded" \
  "已创建定时任务" \
  "text"

echo "[FLOW] list -> short follow-up words (no job id required)"
run_case_expect \
  "schedule_list_before_bulk_ops" \
  "帮我查看我的定时任务" \
  "succeeded" \
  "定时任务列表" \
  "text"

run_case_expect \
  "schedule_pause_all_short_close_wording" \
  "关掉" \
  "succeeded" \
  "已暂停" \
  "text" \
  "任务ID" \
  "either"

run_case_expect \
  "schedule_resume_all_between_synonyms_1" \
  "恢复所有任务" \
  "succeeded" \
  "已恢复" \
  "text"

run_case_expect \
  "schedule_list_before_short_stop_wording" \
  "帮我查看我的定时任务" \
  "succeeded" \
  "定时任务列表" \
  "text"

run_case_expect \
  "schedule_pause_all_short_stop_wording" \
  "停掉" \
  "succeeded" \
  "已暂停" \
  "text" \
  "任务ID" \
  "either"

run_case_expect \
  "schedule_resume_all_between_synonyms_2" \
  "恢复所有任务" \
  "succeeded" \
  "已恢复" \
  "text"

run_case_expect \
  "schedule_list_before_short_halt_wording" \
  "帮我查看我的定时任务" \
  "succeeded" \
  "定时任务列表" \
  "text"

run_case_expect \
  "schedule_pause_all_short_halt_wording" \
  "停止" \
  "succeeded" \
  "已暂停" \
  "text" \
  "任务ID" \
  "either"

run_case_expect \
  "schedule_delete_all" \
  "帮我删除所有任务" \
  "succeeded" \
  "已删除全部定时任务" \
  "text"

run_case_expect \
  "schedule_list_after_delete" \
  "帮我查看我的定时任务" \
  "succeeded" \
  "当前没有定时任务" \
  "text"

echo "[FLOW] PASS: bulk follow-up schedule operations"

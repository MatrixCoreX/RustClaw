#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "== LLM-first regression =="
echo "BASE_URL=${BASE_URL:-http://127.0.0.1:8787}"
echo "USER_ID=${USER_ID:-1985996990} CHAT_ID=${CHAT_ID:-1985996990}"
echo

failed=0
phase="${1:-all}"

run_case_script() {
  local script_name="$1"
  if ! bash "${SCRIPT_DIR}/${script_name}"; then
    failed=$((failed + 1))
  fi
  echo
}

case "$phase" in
  prompt)
    run_case_script "case_prompt_smoke.sh"
    ;;
  prompt_full)
    run_case_script "case_prompt_smoke.sh"
    run_case_script "case_prompt_full.sh"
    run_case_script "case_voice_mode_intent.sh"
    ;;
  phase1)
    run_case_script "case_chat.sh"
    run_case_script "case_act_perl_save.sh"
    run_case_script "case_act_delete_dir.sh"
    run_case_script "case_route_memory.sh"
    run_case_script "case_skill_memory.sh"
    ;;
  phase2)
    run_case_script "case_schedule.sh"
    run_case_script "case_schedule_bulk_followup.sh"
    run_case_script "case_schedule_memory.sh"
    run_case_script "case_image_memory.sh"
    ;;
  all)
    run_case_script "case_chat.sh"
    run_case_script "case_act.sh"
    run_case_script "case_act_perl_save.sh"
    run_case_script "case_act_delete_dir.sh"
    run_case_script "case_schedule.sh"
    run_case_script "case_act_custom_cmd.sh"
    run_case_script "case_memory_preference_persist.sh"
    run_case_script "case_memory_preference_override.sh"
    run_case_script "case_prompt_injection_no_leak.sh"
    run_case_script "case_prompt_injection_no_exec.sh"
    run_case_script "case_memory_relevance.sh"
    run_case_script "case_route_memory.sh"
    run_case_script "case_skill_memory.sh"
    run_case_script "case_schedule_bulk_followup.sh"
    run_case_script "case_schedule_memory.sh"
    run_case_script "case_image_memory.sh"
    ;;
  *)
    echo "Unknown phase: ${phase}"
    echo "Usage: $0 [all|phase1|phase2|prompt|prompt_full]"
    exit 2
    ;;
esac

if [ "$failed" -gt 0 ]; then
  echo "Regression FAILED: ${failed} case(s) failed"
  exit 1
fi

echo "Regression PASSED: all cases succeeded"

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
run_case_expect "skill_memory_seed_pref" "以后都用英文回复" "succeeded"

echo "[CASE] skill_memory_direct_health_check"
submit_resp="$(submit_run_skill_task "health_check" "{}")"
task_id="$(extract_submit_task_id "$submit_resp")"
echo "task_id: ${task_id}"
row="$(wait_task_until_terminal "$task_id")"
status="$(printf '%s' "$row" | awk -F'\t' '{print $1}')"
if ! is_expected_status "$status" "succeeded"; then
  text="$(printf '%s' "$row" | awk -F'\t' '{print $2}')"
  error="$(printf '%s' "$row" | awk -F'\t' '{print $3}')"
  echo "FAIL: status=${status} expected=succeeded"
  echo "text=${text}"
  echo "error=${error}"
  exit 1
fi
echo "PASS: status=${status}"

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SEMANTIC_EVAL_SCRIPT="${SCRIPT_DIR}/evaluate_dynamic_guard_semantic.py"

MANUAL_CASE_FILE="${SCRIPT_DIR}/cases/nl_cases_dynamic_guard_manual_20260327.txt"
MANUAL_LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/dynamic_guard_manual"

CLARIFY_CASE_FILE="${SCRIPT_DIR}/cases/nl_cases_dynamic_guard_clarify_20260327.txt"
CLARIFY_LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/dynamic_guard_clarify"

CONTEXT_CASE_FILE="${SCRIPT_DIR}/cases/nl_cases_dynamic_guard_context_20260327.txt"
CONTEXT_LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/dynamic_guard_context"

RAW_BASE_CHAT_ID="${CHAT_ID:-$(date +%s)}"
if ! [[ "${RAW_BASE_CHAT_ID}" =~ ^[0-9]+$ ]]; then
  RAW_BASE_CHAT_ID="$(date +%s)"
fi
BASE_CHAT_ID="${RAW_BASE_CHAT_ID}"
MANUAL_CHAT_ID=$((BASE_CHAT_ID * 10 + 101))
CLARIFY_CHAT_ID=$((BASE_CHAT_ID * 10 + 201))
CONTEXT_CHAT_ID=$((BASE_CHAT_ID * 10 + 301))

run_step() {
  local title="$1"
  shift
  echo
  echo "============================================================"
  echo "[RUN] ${title}"
  echo "[CMD] $*"
  "$@"
}

latest_run_dir() {
  local log_root="$1"
  ls -1dt "${log_root}"/* 2>/dev/null | head -n 1 || true
}

run_step \
  "dynamic_guard manual" \
  bash "${SCRIPT_DIR}/run_manual_test.sh" \
  --case-file "${MANUAL_CASE_FILE}" \
  --log-root "${MANUAL_LOG_ROOT}" \
  --chat-id "${MANUAL_CHAT_ID}" \
  "$@"

run_step \
  "dynamic_guard clarify" \
  bash "${SCRIPT_DIR}/run_multi_turn_suite.sh" \
  --suite clarify \
  --case-file "${CLARIFY_CASE_FILE}" \
  --log-root "${CLARIFY_LOG_ROOT}" \
  --chat-id "${CLARIFY_CHAT_ID}" \
  "$@"

run_step \
  "dynamic_guard context_chain" \
  bash "${SCRIPT_DIR}/run_multi_turn_suite.sh" \
  --suite context_chain \
  --case-file "${CONTEXT_CASE_FILE}" \
  --log-root "${CONTEXT_LOG_ROOT}" \
  --chat-id "${CONTEXT_CHAT_ID}" \
  "$@"

manual_latest="$(latest_run_dir "${MANUAL_LOG_ROOT}")"
clarify_latest="$(latest_run_dir "${CLARIFY_LOG_ROOT}")"
context_latest="$(latest_run_dir "${CONTEXT_LOG_ROOT}")"

run_semantic_eval() {
  local suite="$1"
  local case_file="$2"
  local run_dir="$3"
  if [[ -z "${run_dir}" || ! -d "${run_dir}" ]]; then
    echo "[semantic] skip ${suite}: run_dir not found"
    return 0
  fi
  local summary_jsonl="${run_dir}/summary.jsonl"
  local report_jsonl="${run_dir}/semantic_report.jsonl"
  if [[ ! -f "${summary_jsonl}" ]]; then
    echo "[semantic] skip ${suite}: summary.jsonl not found"
    return 0
  fi
  if [[ ! -f "${SEMANTIC_EVAL_SCRIPT}" ]]; then
    echo "[semantic] skip ${suite}: evaluator script not found"
    return 0
  fi
  echo
  echo "[semantic] evaluate suite=${suite}"
  python3 "${SEMANTIC_EVAL_SCRIPT}" \
    --suite "${suite}" \
    --case-file "${case_file}" \
    --summary-jsonl "${summary_jsonl}" \
    --report-jsonl "${report_jsonl}" \
    --workspace-root "${ROOT_DIR}" || true
}

run_semantic_eval "manual" "${MANUAL_CASE_FILE}" "${manual_latest}"
run_semantic_eval "clarify" "${CLARIFY_CASE_FILE}" "${clarify_latest}"
run_semantic_eval "context_chain" "${CONTEXT_CASE_FILE}" "${context_latest}"

echo
echo "All dynamic-guard suites completed."
echo
echo "Artifacts summary:"
if [[ -n "${manual_latest}" ]]; then
  echo "  manual_run_dir: ${manual_latest}"
  echo "  manual_run_log: ${manual_latest}/run.log"
  echo "  manual_summary_jsonl: ${manual_latest}/summary.jsonl"
  echo "  manual_semantic_report: ${manual_latest}/semantic_report.jsonl"
else
  echo "  manual_run_dir: <not found>"
fi
if [[ -n "${clarify_latest}" ]]; then
  echo "  clarify_run_dir: ${clarify_latest}"
  echo "  clarify_run_log: ${clarify_latest}/run.log"
  echo "  clarify_summary_jsonl: ${clarify_latest}/summary.jsonl"
  echo "  clarify_semantic_report: ${clarify_latest}/semantic_report.jsonl"
else
  echo "  clarify_run_dir: <not found>"
fi
if [[ -n "${context_latest}" ]]; then
  echo "  context_run_dir: ${context_latest}"
  echo "  context_run_log: ${context_latest}/run.log"
  echo "  context_summary_jsonl: ${context_latest}/summary.jsonl"
  echo "  context_semantic_report: ${context_latest}/semantic_report.jsonl"
else
  echo "  context_run_dir: <not found>"
fi

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CASE_DIR="${SCRIPT_DIR}/cases"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_suite.sh <mode> [shared options]

Modes:
  manual                  run manual NL cases
  text_match              run text-match NL cases
  full                    run full NL suite
  trace                   run trace ask regression (wrapped with run.log)
  resume                  run resume/continue regression (wrapped with run.log)
  clarify                 run 2-turn clarify suite
  context_chain           run 3-turn context-chain suite
  all                     run: manual, text_match, full, trace, resume, clarify, context_chain
  clarify_context_prompt  run clarify_hard + context_chain and print a ready prompt

Shared options are passed through to the selected mode scripts.
EOF
}

run_wrapped_suite() {
  local name="$1"
  shift
  local log_root="${ROOT_DIR}/scripts/nl_suite_logs/${name}"
  local run_stamp run_dir run_log
  run_stamp="$(date +%Y%m%d_%H%M%S)"
  run_dir="${log_root}/${run_stamp}"
  run_log="${run_dir}/run.log"
  mkdir -p "$run_dir"

  (
    exec > >(tee -a "$run_log") 2>&1
    echo "NL suite: ${name}"
    echo "  run_dir: ${run_dir}"
    echo "  run_log: ${run_log}"
    echo
    "$@"
    echo
    echo "Artifacts:"
    echo "  - ${run_dir}"
    echo "  - ${run_log}"
  )
}

run_mode_manual() {
  bash "${SCRIPT_DIR}/run_manual_test.sh" \
    --case-file "${CASE_DIR}/nl_cases_manual.txt" \
    --log-root "${ROOT_DIR}/scripts/nl_suite_logs/manual" \
    "$@"
}

run_mode_text_match() {
  bash "${SCRIPT_DIR}/run_manual_test.sh" \
    --case-file "${CASE_DIR}/nl_cases_text_match.txt" \
    --log-root "${ROOT_DIR}/scripts/nl_suite_logs/text_match" \
    "$@"
}

run_mode_full() {
  bash "${SCRIPT_DIR}/run_full_suite.sh" \
    --case-file "${CASE_DIR}/nl_cases_full.txt" \
    --trace-case-file "${CASE_DIR}/nl_cases_trace.txt" \
    --log-root "${ROOT_DIR}/scripts/nl_suite_logs/full" \
    "$@"
}

run_mode_trace() {
  run_wrapped_suite \
    "trace" \
    bash "${ROOT_DIR}/scripts/regression_trace_ask.sh" \
    --no-defaults \
    --case-file "${CASE_DIR}/nl_cases_trace.txt" \
    "$@"
}

run_mode_resume() {
  run_wrapped_suite \
    "resume" \
    bash "${ROOT_DIR}/scripts/regression_resume_continue.sh" \
    "$@"
}

run_mode_clarify() {
  bash "${SCRIPT_DIR}/run_multi_turn_suite.sh" \
    --suite clarify \
    --case-file "${CASE_DIR}/nl_cases_clarify.txt" \
    --log-root "${ROOT_DIR}/scripts/nl_suite_logs/clarify" \
    "$@"
}

run_mode_context_chain() {
  bash "${SCRIPT_DIR}/run_multi_turn_suite.sh" \
    --suite context_chain \
    --case-file "${CASE_DIR}/nl_cases_context_chain_20260326.txt" \
    --log-root "${ROOT_DIR}/scripts/nl_suite_logs/context_chain" \
    "$@"
}

run_mode_all() {
  local mode
  for mode in manual text_match full trace resume clarify context_chain; do
    echo "============================================================"
    echo "[MODE] ${mode}"
    bash "$0" "$mode" "$@"
  done
}

run_mode_clarify_context_prompt() {
  local clarify_log_root="${ROOT_DIR}/scripts/nl_suite_logs/clarify_hard"
  local context_log_root="${ROOT_DIR}/scripts/nl_suite_logs/context_chain"
  local latest_clarify latest_context

  bash "${SCRIPT_DIR}/run_multi_turn_suite.sh" \
    --suite clarify \
    --case-file "${CASE_DIR}/nl_cases_clarify_hard_20260326.txt" \
    --log-root "${clarify_log_root}" \
    "$@"

  bash "${SCRIPT_DIR}/run_multi_turn_suite.sh" \
    --suite context_chain \
    --case-file "${CASE_DIR}/nl_cases_context_chain_20260326.txt" \
    --log-root "${context_log_root}" \
    "$@"

  latest_clarify="$(ls -1dt "${clarify_log_root}"/* 2>/dev/null | head -n 1 || true)"
  latest_context="$(ls -1dt "${context_log_root}"/* 2>/dev/null | head -n 1 || true)"

  echo
  echo "==== Paste this to Codex ===="
  if [[ -n "${latest_clarify}" && -n "${latest_context}" ]]; then
    printf "请分析这两次测试结果：\n"
    printf "clarify_run_dir: %s\n" "$latest_clarify"
    printf "clarify_run_log: %s/run.log\n" "$latest_clarify"
    printf "clarify_summary_jsonl: %s/summary.jsonl\n" "$latest_clarify"
    printf "context_run_dir: %s\n" "$latest_context"
    printf "context_run_log: %s/run.log\n" "$latest_context"
    printf "context_summary_jsonl: %s/summary.jsonl\n" "$latest_context"
  else
    echo "Unable to locate one or both latest run directories."
  fi
}

MODE="${1:-}"
if [[ -z "$MODE" || "$MODE" == "-h" || "$MODE" == "--help" ]]; then
  usage
  exit 0
fi
shift

case "$MODE" in
  manual)
    run_mode_manual "$@"
    ;;
  text_match)
    run_mode_text_match "$@"
    ;;
  full)
    run_mode_full "$@"
    ;;
  trace)
    run_mode_trace "$@"
    ;;
  resume)
    run_mode_resume "$@"
    ;;
  clarify)
    run_mode_clarify "$@"
    ;;
  context_chain)
    run_mode_context_chain "$@"
    ;;
  all)
    run_mode_all "$@"
    ;;
  clarify_context_prompt)
    run_mode_clarify_context_prompt "$@"
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    usage >&2
    exit 2
    ;;
esac

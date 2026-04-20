#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CASE_DIR="${SCRIPT_DIR}/cases"

ALL_SUITES=(
  manual
  text_match
  full
  trace
  resume
  self_extension
  sensitive_flows
  ops_deterministic
  ops_http_repair
  long_tail_flows
  clarify
  clarify_hard
  context_chain
  dynamic_guard
  clarify_context_prompt
)

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_suite.sh <suite-or-category>... [shared options]
  bash scripts/nl_tests/run_suite.sh --category <name> [--category <name> ...] [shared options]
  bash scripts/nl_tests/run_suite.sh --list

Suites:
  manual
  text_match
  full
  trace
  resume
  self_extension
  sensitive_flows
  ops_deterministic
  ops_http_repair
  long_tail_flows
  clarify
  clarify_hard
  context_chain
  dynamic_guard
  clarify_context_prompt

Categories:
  smoke         -> manual, clarify
  single_turn   -> manual, text_match, full
  multi_turn    -> clarify, clarify_hard, context_chain
  regression    -> trace, resume
  guard         -> dynamic_guard, sensitive_flows
  ops           -> ops_deterministic, long_tail_flows
  core          -> manual, text_match, trace, resume, clarify, context_chain
  all           -> manual, text_match, full, trace, resume, clarify, clarify_hard, context_chain, dynamic_guard

Examples:
  bash scripts/nl_tests/run_suite.sh manual
  bash scripts/nl_tests/run_suite.sh manual trace clarify
  bash scripts/nl_tests/run_suite.sh sensitive_flows
  bash scripts/nl_tests/run_suite.sh ops_deterministic
  bash scripts/nl_tests/run_suite.sh ops_http_repair
  bash scripts/nl_tests/run_suite.sh long_tail_flows
  bash scripts/nl_tests/run_suite.sh --category multi_turn
  bash scripts/nl_tests/run_suite.sh --category regression --category guard --base-url http://127.0.0.1:8787
  bash scripts/nl_tests/run_suite.sh --category ops

Notes:
  - Shared options are passed through to the underlying suite runner.
  - If the first unknown flag starts with '-', it and the remaining args are treated as pass-through args.
EOF
}

print_available() {
  cat <<'EOF'
Available suites:
  - manual
  - text_match
  - full
  - trace
  - resume
  - self_extension
  - sensitive_flows
  - ops_deterministic
  - ops_http_repair
  - long_tail_flows
  - clarify
  - clarify_hard
  - context_chain
  - dynamic_guard
  - clarify_context_prompt

Available categories:
  - smoke
  - single_turn
  - multi_turn
  - regression
  - guard
  - ops
  - core
  - all
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
    NL_SUITE_RUN_DIR="${run_dir}" "$@"
    echo
    echo "Artifacts:"
    echo "  - ${run_dir}"
    echo "  - ${run_log}"
  )
}

latest_run_dir() {
  local log_root="$1"
  ls -1dt "${log_root}"/* 2>/dev/null | head -n 1 || true
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

run_mode_self_extension() {
  run_wrapped_suite \
    "self_extension" \
    bash "${ROOT_DIR}/scripts/regression_self_extension_suite.sh" \
    "$@"
}

run_mode_sensitive_flows() {
  run_wrapped_suite \
    "sensitive_flows" \
    bash "${ROOT_DIR}/scripts/regression_sensitive_nl_flows.sh" \
    "$@"
}

run_mode_ops_deterministic() {
  run_wrapped_suite \
    "ops_deterministic" \
    bash "${ROOT_DIR}/scripts/regression_ops_closed_loop_deterministic.sh" \
    "$@"
}

run_mode_ops_http_repair() {
  run_wrapped_suite \
    "ops_http_repair" \
    bash "${ROOT_DIR}/scripts/regression_ops_http_repair_nl_flows.sh" \
    "$@"
}

run_mode_long_tail_flows() {
  run_wrapped_suite \
    "long_tail_flows" \
    bash "${ROOT_DIR}/scripts/regression_long_tail_nl_flows.sh" \
    "$@"
}

run_mode_clarify() {
  bash "${SCRIPT_DIR}/run_multi_turn_suite.sh" \
    --suite clarify \
    --case-file "${CASE_DIR}/nl_cases_clarify.txt" \
    --log-root "${ROOT_DIR}/scripts/nl_suite_logs/clarify" \
    "$@"
}

run_mode_clarify_hard() {
  bash "${SCRIPT_DIR}/run_multi_turn_suite.sh" \
    --suite clarify \
    --case-file "${CASE_DIR}/nl_cases_clarify_hard.txt" \
    --log-root "${ROOT_DIR}/scripts/nl_suite_logs/clarify_hard" \
    "$@"
}

run_mode_context_chain() {
  bash "${SCRIPT_DIR}/run_multi_turn_suite.sh" \
    --suite context_chain \
    --case-file "${CASE_DIR}/nl_cases_context_chain.txt" \
    --log-root "${ROOT_DIR}/scripts/nl_suite_logs/context_chain" \
    "$@"
}

run_mode_dynamic_guard() {
  bash "${SCRIPT_DIR}/run_dynamic_guard_all.sh" "$@"
}

run_mode_clarify_context_prompt() {
  local clarify_log_root="${ROOT_DIR}/scripts/nl_suite_logs/clarify_hard"
  local context_log_root="${ROOT_DIR}/scripts/nl_suite_logs/context_chain"
  local latest_clarify latest_context

  run_mode_clarify_hard "$@"
  run_mode_context_chain "$@"

  latest_clarify="$(latest_run_dir "${clarify_log_root}")"
  latest_context="$(latest_run_dir "${context_log_root}")"

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

FILTERED_SUITE_ARGS=()

suite_accepts_value_option() {
  local suite="$1"
  local option="$2"
  case "$option" in
    --base-url|--user-id|--chat-id|--user-key)
      case "$suite" in
        manual|text_match|full|trace|resume|clarify|clarify_hard|context_chain|dynamic_guard|clarify_context_prompt)
          return 0
          ;;
      esac
      ;;
    --wait-seconds)
      case "$suite" in
        manual|text_match|full|trace|resume|self_extension|sensitive_flows|ops_http_repair|long_tail_flows|clarify|clarify_hard|context_chain|dynamic_guard|clarify_context_prompt)
          return 0
          ;;
      esac
      ;;
    --poll-seconds)
      case "$suite" in
        manual|text_match|full|trace|clarify|clarify_hard|context_chain|dynamic_guard|clarify_context_prompt)
          return 0
          ;;
      esac
      ;;
    --provider-retries|--provider-retry-sleep)
      case "$suite" in
        manual|text_match|full|clarify|clarify_hard|context_chain|dynamic_guard|clarify_context_prompt)
          return 0
          ;;
      esac
      ;;
  esac
  return 1
}

suite_accepts_flag_option() {
  local suite="$1"
  local option="$2"
  case "$option" in
    --no-llm-trace)
      case "$suite" in
        manual|text_match|full|clarify|clarify_hard|context_chain|dynamic_guard|clarify_context_prompt)
          return 0
          ;;
      esac
      ;;
    --reuse-chat-id-base)
      case "$suite" in
        manual|text_match|clarify|clarify_hard|context_chain|dynamic_guard|clarify_context_prompt)
          return 0
          ;;
      esac
      ;;
  esac
  return 1
}

filter_pass_through_for_suite() {
  local suite="$1"
  shift

  FILTERED_SUITE_ARGS=()

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --base-url|--user-id|--chat-id|--user-key|--wait-seconds|--poll-seconds|--provider-retries|--provider-retry-sleep)
        if [[ $# -lt 2 ]]; then
          echo "Missing value for $1" >&2
          exit 2
        fi
        if suite_accepts_value_option "$suite" "$1"; then
          FILTERED_SUITE_ARGS+=("$1" "$2")
        fi
        shift 2
        ;;
      --no-llm-trace|--reuse-chat-id-base)
        if suite_accepts_flag_option "$suite" "$1"; then
          FILTERED_SUITE_ARGS+=("$1")
        fi
        shift
        ;;
      *)
        FILTERED_SUITE_ARGS+=("$1")
        shift
        ;;
    esac
  done
}

run_one_suite() {
  local suite="$1"
  shift
  filter_pass_through_for_suite "$suite" "$@"
  case "$suite" in
    manual)
      run_mode_manual "${FILTERED_SUITE_ARGS[@]}"
      ;;
    text_match)
      run_mode_text_match "${FILTERED_SUITE_ARGS[@]}"
      ;;
    full)
      run_mode_full "${FILTERED_SUITE_ARGS[@]}"
      ;;
    trace)
      run_mode_trace "${FILTERED_SUITE_ARGS[@]}"
      ;;
    resume)
      run_mode_resume "${FILTERED_SUITE_ARGS[@]}"
      ;;
    self_extension)
      run_mode_self_extension "${FILTERED_SUITE_ARGS[@]}"
      ;;
    sensitive_flows)
      run_mode_sensitive_flows "${FILTERED_SUITE_ARGS[@]}"
      ;;
    ops_deterministic)
      run_mode_ops_deterministic "${FILTERED_SUITE_ARGS[@]}"
      ;;
    ops_http_repair)
      run_mode_ops_http_repair "${FILTERED_SUITE_ARGS[@]}"
      ;;
    long_tail_flows)
      run_mode_long_tail_flows "${FILTERED_SUITE_ARGS[@]}"
      ;;
    clarify)
      run_mode_clarify "${FILTERED_SUITE_ARGS[@]}"
      ;;
    clarify_hard)
      run_mode_clarify_hard "${FILTERED_SUITE_ARGS[@]}"
      ;;
    context_chain)
      run_mode_context_chain "${FILTERED_SUITE_ARGS[@]}"
      ;;
    dynamic_guard)
      run_mode_dynamic_guard "${FILTERED_SUITE_ARGS[@]}"
      ;;
    clarify_context_prompt)
      run_mode_clarify_context_prompt "${FILTERED_SUITE_ARGS[@]}"
      ;;
    *)
      echo "Unknown suite: $suite" >&2
      exit 2
      ;;
  esac
}

declare -A SEEN_SUITES=()
ORDERED_SUITES=()

add_suite() {
  local suite="$1"
  [[ -n "$suite" ]] || return 0
  if [[ -z "${SEEN_SUITES[$suite]:-}" ]]; then
    SEEN_SUITES["$suite"]=1
    ORDERED_SUITES+=("$suite")
  fi
}

expand_selector() {
  local selector="$1"
  case "$selector" in
    manual|text_match|full|trace|resume|self_extension|sensitive_flows|ops_deterministic|ops_http_repair|long_tail_flows|clarify|clarify_hard|context_chain|dynamic_guard|clarify_context_prompt)
      add_suite "$selector"
      ;;
    smoke)
      add_suite manual
      add_suite clarify
      ;;
    single_turn)
      add_suite manual
      add_suite text_match
      add_suite full
      ;;
    multi_turn)
      add_suite clarify
      add_suite clarify_hard
      add_suite context_chain
      ;;
    regression)
      add_suite trace
      add_suite resume
      ;;
    guard)
      add_suite dynamic_guard
      add_suite sensitive_flows
      ;;
    ops)
      add_suite ops_deterministic
      add_suite long_tail_flows
      ;;
    core)
      add_suite manual
      add_suite text_match
      add_suite trace
      add_suite resume
      add_suite clarify
      add_suite context_chain
      ;;
    all)
      local suite
      for suite in manual text_match full trace resume clarify clarify_hard context_chain dynamic_guard; do
        add_suite "$suite"
      done
      ;;
    *)
      echo "Unknown suite/category: $selector" >&2
      print_available >&2
      exit 2
      ;;
  esac
}

SELECTORS=()
PASS_THROUGH_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --list)
      print_available
      exit 0
      ;;
    --category|--suite)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "Missing value for $1" >&2
        exit 2
      fi
      SELECTORS+=("$2")
      shift 2
      ;;
    --)
      shift
      PASS_THROUGH_ARGS+=("$@")
      break
      ;;
    -*)
      PASS_THROUGH_ARGS+=("$@")
      break
      ;;
    *)
      SELECTORS+=("$1")
      shift
      ;;
  esac
done

if [[ "${#SELECTORS[@]}" -eq 0 ]]; then
  usage
  echo
  print_available
  exit 0
fi

for selector in "${SELECTORS[@]}"; do
  expand_selector "$selector"
done

for suite in "${ORDERED_SUITES[@]}"; do
  echo "============================================================"
  echo "[SUITE] ${suite}"
  run_one_suite "$suite" "${PASS_THROUGH_ARGS[@]}"
done

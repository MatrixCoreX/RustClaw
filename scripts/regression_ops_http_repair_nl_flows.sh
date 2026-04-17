#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CASE_FILE="${CASE_FILE:-${ROOT_DIR}/scripts/nl_tests/cases/nl_cases_ops_http_repair.txt}"
LOG_DIR_ARGS=()

if [[ -n "${NL_SUITE_RUN_DIR:-}" ]]; then
  LOG_DIR_ARGS+=(--log-dir "${NL_SUITE_RUN_DIR}/raw")
fi

exec bash "${ROOT_DIR}/scripts/regression_long_tail_nl_flows.sh" \
  --case-file "${CASE_FILE}" \
  "${LOG_DIR_ARGS[@]}" \
  "$@"

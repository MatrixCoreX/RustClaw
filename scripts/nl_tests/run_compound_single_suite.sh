#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CASE_FILE="${SCRIPT_DIR}/cases/nl_cases_compound_single_language.txt"
LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/compound_single"

exec bash "${SCRIPT_DIR}/run_manual_test.sh" \
  --case-file "${CASE_FILE}" \
  --log-root "${LOG_ROOT}" \
  "$@"

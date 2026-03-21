#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_ROOT="${SCRIPT_DIR}/nl_suite_logs/resume"
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
RUN_LOG="${RUN_DIR}/run.log"

mkdir -p "$RUN_DIR"
exec > >(tee -a "$RUN_LOG") 2>&1

echo "NL suite: resume"
echo "  run_dir:  $RUN_DIR"
echo "  run_log:  $RUN_LOG"
echo

bash "${SCRIPT_DIR}/regression_resume_continue.sh" "$@"

echo
echo "Artifacts:"
echo "  - $RUN_DIR"
echo "  - $RUN_LOG"

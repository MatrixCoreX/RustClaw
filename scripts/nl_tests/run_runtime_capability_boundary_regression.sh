#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

CASE_FILE="${ROOT_DIR}/scripts/nl_tests/cases/nl_cases_runtime_capability_boundary_regression_20260515.txt"
EXPECTATIONS="${ROOT_DIR}/scripts/nl_tests/expectations/runtime_capability_boundary_regression_20260515.jsonl"
OUTPUT_CAPTURE="$(mktemp)"
RUN_STATUS=0

cleanup() {
  rm -f "$OUTPUT_CAPTURE"
}
trap cleanup EXIT

cd "$ROOT_DIR"

set +e
bash "${ROOT_DIR}/scripts/nl_tests/run_client_like_continuous_suite.sh" \
  --skip-smoke \
  --case-file "$CASE_FILE" \
  --quality-guard \
  --prompt-reply-only \
  "$@" | tee "$OUTPUT_CAPTURE"
RUN_STATUS="${PIPESTATUS[0]}"
set -e

RUN_DIR="$(awk -F= '/^log_dir=/ {print $2; exit}' "$OUTPUT_CAPTURE")"
if [[ -z "${RUN_DIR:-}" || ! -d "$RUN_DIR" ]]; then
  echo "Runtime capability regression did not produce a usable log_dir." >&2
  exit "${RUN_STATUS:-1}"
fi

python3 "${ROOT_DIR}/scripts/nl_tests/summarize_client_like_run.py" "$RUN_DIR"

if [[ "$RUN_STATUS" -ne 0 ]]; then
  echo "Runtime capability regression run failed before expectation evaluation: ${RUN_DIR}" >&2
  exit "$RUN_STATUS"
fi

python3 "${ROOT_DIR}/scripts/nl_tests/evaluate_client_like_run.py" \
  "$RUN_DIR" \
  --expectations "$EXPECTATIONS"

echo "RUNTIME_CAPABILITY_BOUNDARY_REGRESSION_OK log_dir=${RUN_DIR}"

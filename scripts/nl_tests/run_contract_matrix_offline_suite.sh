#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CASE_COUNT="${CONTRACT_MATRIX_CASE_COUNT:-100}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_contract_matrix_offline_suite.sh [--count N]

Options:
  --count N    number of generated contract-matrix cases to validate. Default: 100
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --count)
      CASE_COUNT="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if ! [[ "${CASE_COUNT}" =~ ^[0-9]+$ ]] || [[ "${CASE_COUNT}" -le 0 ]]; then
  echo "--count must be a positive integer, got: ${CASE_COUNT}" >&2
  exit 2
fi

cd "$ROOT_DIR"

echo "Checking contract matrix generator syntax"
python3 -m py_compile \
  "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  "${ROOT_DIR}/scripts/nl_tests/evaluate_client_like_run.py"

echo "Generating deterministic contract matrix seed cases"
python3 "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  --count "${CASE_COUNT}" \
  --check \
  --report \
  > /tmp/rustclaw-contract-matrix-cases.jsonl

echo "Generating deterministic contract matrix live NL rows"
python3 "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  --count "${CASE_COUNT}" \
  --check \
  --nl \
  --expectations /tmp/rustclaw-contract-matrix-nl.expectations.jsonl \
  --report \
  > /tmp/rustclaw-contract-matrix-nl.jsonl

fixtures=(
  observed_finalizer_scalar
  verifier_issue_missing_arg
  contract_rejection_attribution
  budget_exhausted_attribution
  code_gap_attribution
  permission_denied_attribution
  schema_error_attribution
  tool_gap_attribution
  provider_error_attribution
  delivery_error_attribution
  prompt_budget_error_attribution
)

for fixture in "${fixtures[@]}"; do
  echo "Evaluating offline fixture: ${fixture}"
  python3 "${ROOT_DIR}/scripts/nl_tests/evaluate_client_like_run.py" \
    "${ROOT_DIR}/scripts/nl_tests/fixtures/client_like_runs/${fixture}" \
    --expectations "${ROOT_DIR}/scripts/nl_tests/expectations/${fixture}_fixture.jsonl"
done

echo "CONTRACT_MATRIX_OFFLINE_SUITE_OK"

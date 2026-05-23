#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "$ROOT_DIR"

echo "Checking contract matrix generator syntax"
python3 -m py_compile \
  "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  "${ROOT_DIR}/scripts/nl_tests/evaluate_client_like_run.py"

echo "Generating deterministic contract matrix seed cases"
python3 "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  --count 100 \
  --check \
  --report \
  > /tmp/rustclaw-contract-matrix-cases.jsonl

echo "Generating deterministic contract matrix live NL rows"
python3 "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  --count 100 \
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

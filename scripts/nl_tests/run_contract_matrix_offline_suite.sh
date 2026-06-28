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

echo "Checking docker contract matrix copy is current"
if ! cmp -s \
  "${ROOT_DIR}/configs/task_contract_matrix.toml" \
  "${ROOT_DIR}/docker/config/task_contract_matrix.toml"; then
  diff -u \
    "${ROOT_DIR}/configs/task_contract_matrix.toml" \
    "${ROOT_DIR}/docker/config/task_contract_matrix.toml" || true
  echo "docker/config/task_contract_matrix.toml must match configs/task_contract_matrix.toml" >&2
  exit 1
fi

echo "Checking contract matrix generator syntax"
python3 -m py_compile \
  "${ROOT_DIR}/scripts/sync_skill_docs.py" \
  "${ROOT_DIR}/scripts/check_skill_prompts.py" \
  "${ROOT_DIR}/scripts/nl_tests/build_client_like_case_aggregate.py" \
  "${ROOT_DIR}/scripts/nl_tests/build_release_gate_subset.py" \
  "${ROOT_DIR}/scripts/nl_tests/compare_contract_provider_runs.py" \
  "${ROOT_DIR}/scripts/nl_tests/compare_multilingual_contract_cells.py" \
  "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  "${ROOT_DIR}/scripts/nl_tests/evaluate_client_like_run.py" \
  "${ROOT_DIR}/scripts/nl_tests/extract_client_like_replay.py"
bash -n "${ROOT_DIR}/scripts/nl_tests/run_contract_provider_ab_suite.sh"

echo "Checking provider A/B missing-key preflight"
PROVIDER_PREFLIGHT_DIR="$(mktemp -d)"
printf '{"case_id":"provider-key-preflight","prompt":"provider key preflight"}\n' > "${PROVIDER_PREFLIGHT_DIR}/cases.jsonl"
env -u OPENAI_API_KEY \
  bash "${ROOT_DIR}/scripts/nl_tests/run_contract_provider_ab_suite.sh" \
    --run-side right \
    --provider openai \
    --case-jsonl "${PROVIDER_PREFLIGHT_DIR}/cases.jsonl" \
    --out-dir "${PROVIDER_PREFLIGHT_DIR}" \
  > "${PROVIDER_PREFLIGHT_DIR}/preflight.out"
grep -q 'PROVIDER_AB_RUN_SIDE_INCONCLUSIVE side=right provider=openai reason=missing_env:OPENAI_API_KEY' \
  "${PROVIDER_PREFLIGHT_DIR}/preflight.out"
python3 - "${PROVIDER_PREFLIGHT_DIR}/right/metadata.json" <<'PY'
import json
import sys
from pathlib import Path

metadata = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
assert metadata["status"] == "inconclusive", metadata
assert metadata["attempts"] == 0, metadata
assert metadata["reason"] == "missing_env:OPENAI_API_KEY", metadata
print("PROVIDER_AB_MISSING_KEY_PREFLIGHT_OK")
PY

echo "Checking skill doc sync self-test"
python3 "${ROOT_DIR}/scripts/sync_skill_docs.py" --self-test

echo "Checking provider comparator self-test"
python3 "${ROOT_DIR}/scripts/nl_tests/compare_contract_provider_runs.py" --self-test

echo "Checking multilingual contract cell comparator self-test"
python3 "${ROOT_DIR}/scripts/nl_tests/compare_multilingual_contract_cells.py" --self-test

echo "Checking layered skill prompt invariants"
python3 "${ROOT_DIR}/scripts/check_skill_prompts.py"

echo "Checking legacy client-like aggregate is current"
python3 "${ROOT_DIR}/scripts/nl_tests/build_client_like_case_aggregate.py" --self-test
python3 "${ROOT_DIR}/scripts/nl_tests/build_client_like_case_aggregate.py" --check

echo "Checking release-gate equivalent subset self-test"
python3 "${ROOT_DIR}/scripts/nl_tests/build_release_gate_subset.py" --self-test

echo "Checking release-gate equivalent subset is current"
python3 "${ROOT_DIR}/scripts/nl_tests/build_release_gate_subset.py" --check

echo "Checking legacy client-like aggregate coverage tokens"
python3 - <<'PY'
from pathlib import Path


aggregate = Path("scripts/nl_tests/cases/nl_cases_client_like_all_aggregate.txt")
required_groups = {
    "builtin_tools": {
        "fs_basic",
        "config_basic",
        "db_basic",
        "package_manager",
        "archive_basic",
        "process_basic",
        "health_check",
    },
    "skills": {"builtin_skill", "skill:"},
    "memory": {"memory"},
    "multi_turn_context": {"turn_chain", "clarify_chain", "context_chain"},
    "structured_transform": {"transform", "json", "table", "sqlite"},
}

seen = {name: set() for name in required_groups}
current_source = ""
for raw in aggregate.read_text(encoding="utf-8").splitlines():
    line = raw.strip()
    if not line:
        continue
    if line.startswith("# source:"):
        current_source = line.removeprefix("# source:").strip()
        continue
    if line.startswith("#"):
        continue
    parts = line.split("|", 4)
    if len(parts) < 4:
        continue
    suite, name, tags, prompt = parts[:4]
    metadata_fields = [current_source, suite, name, tags]
    if prompt.lower().startswith(("tool:", "skill:", "expect=")):
        metadata_fields.append(prompt)
    metadata = " ".join(metadata_fields).lower()
    for group, tokens in required_groups.items():
        for token in tokens:
            if token.lower() in metadata:
                seen[group].add(token)

missing = {
    group: sorted(tokens - seen[group])
    for group, tokens in required_groups.items()
    if tokens - seen[group]
}
if missing:
    raise SystemExit(f"CLIENT_LIKE_AGGREGATE_COVERAGE_INCOMPLETE missing={missing}")
print(
    "CLIENT_LIKE_AGGREGATE_COVERAGE_OK "
    + " ".join(
        f"{group}={','.join(sorted(values))}"
        for group, values in sorted(seen.items())
    )
)
PY

echo "Generating deterministic contract matrix seed cases"
python3 "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  --count "${CASE_COUNT}" \
  --check \
  --report \
  > /tmp/rustclaw-contract-matrix-cases.jsonl

echo "Generating external skill admission contract cases"
python3 "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  --external-admission-cases \
  --check \
  --report \
  > /tmp/rustclaw-external-admission-cases.jsonl

echo "Generating deterministic contract matrix live NL rows"
python3 "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  --count "${CASE_COUNT}" \
  --check \
  --nl \
  --expectations /tmp/rustclaw-contract-matrix-nl.expectations.jsonl \
  --report \
  > /tmp/rustclaw-contract-matrix-nl.jsonl

echo "Generating multilingual contract matrix live NL rows"
python3 "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
  --count "${CASE_COUNT}" \
  --check \
  --nl \
  --multilingual-variants \
  --expectations /tmp/rustclaw-contract-matrix-nl-multilingual.expectations.jsonl \
  --report \
  > /tmp/rustclaw-contract-matrix-nl-multilingual.jsonl

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

echo "Checking attribution fixture coverage"
python3 - <<'PY'
import json
from pathlib import Path


root = Path("scripts/nl_tests/expectations")
files = sorted(root.glob("*_attribution_fixture.jsonl"))
files.append(root / "verifier_issue_missing_arg_fixture.jsonl")

required_attributions = {
    "model_error",
    "schema_error",
    "code_gap",
    "contract_gap",
    "tool_gap",
    "permission_denied",
    "budget_exhausted",
    "prompt_budget_error",
    "delivery_error",
    "provider_error",
}
required_error_kinds = {
    "capability_unavailable",
    "channel_send_failed",
    "contract_action_rejected",
    "evidence_extractor_failed",
    "permission_denied",
    "provider_unavailable",
    "schema_validation_failed",
}
required_stop_signals = {
    "prompt_budget_error",
    "recipe_repair_budget_exhausted",
}


def add_values(value, target):
    if isinstance(value, str):
        target.add(value)
        return
    if isinstance(value, list):
        for item in value:
            if isinstance(item, str):
                target.add(item)


attributions = set()
error_kinds = set()
stop_signals = set()
for path in files:
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        add_values(row.get("failure_attribution_any"), attributions)
        add_values(row.get("stop_failure_attribution"), attributions)
        add_values(row.get("verifier_failure_attribution_any"), attributions)
        add_values(row.get("error_kind_any"), error_kinds)
        add_values(row.get("stop_signal"), stop_signals)

missing_attributions = sorted(required_attributions - attributions)
missing_error_kinds = sorted(required_error_kinds - error_kinds)
missing_stop_signals = sorted(required_stop_signals - stop_signals)
if missing_attributions or missing_error_kinds or missing_stop_signals:
    raise SystemExit(
        "Attribution fixture coverage incomplete: "
        f"missing_attributions={missing_attributions} "
        f"missing_error_kinds={missing_error_kinds} "
        f"missing_stop_signals={missing_stop_signals}"
    )

print(
    "ATTRIBUTION_COVERAGE_OK "
    f"categories={','.join(sorted(attributions))} "
    f"error_kinds={','.join(sorted(error_kinds))} "
    f"stop_signals={','.join(sorted(stop_signals))}"
)
PY

echo "Extracting minimal replay reproduction fixture"
python3 "${ROOT_DIR}/scripts/nl_tests/extract_client_like_replay.py" \
  "${ROOT_DIR}/scripts/nl_tests/fixtures/client_like_runs/contract_rejection_attribution" \
  --case-jsonl /tmp/rustclaw-contract-replay.jsonl \
  --expectations /tmp/rustclaw-contract-replay.expectations.jsonl \
  --min-repro /tmp/rustclaw-contract-replay.min-repro.jsonl
python3 - <<'PY'
import json
from pathlib import Path

path = Path("/tmp/rustclaw-contract-replay.min-repro.jsonl")
rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
assert len(rows) == 1, f"expected one min-repro row, got {len(rows)}"
row = rows[0]
assert row["request"], "min repro must include request"
assert row["route_contract"]["contract_match"], "min repro must include route contract"
assert row["planned_actions"], "min repro must include planned actions"
assert "missing_evidence" in row, "min repro must include evidence fields"
assert row["final_answer_preview"], "min repro must include final answer preview"
print("MIN_REPRO_EXTRACT_OK")
PY

echo "CONTRACT_MATRIX_OFFLINE_SUITE_OK"

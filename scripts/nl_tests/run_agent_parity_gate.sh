#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
if [[ -n "${NL_SUITE_RUN_DIR:-}" ]]; then
  OUT_DIR="${NL_SUITE_RUN_DIR}/agent_parity_gate"
else
  OUT_DIR="${ROOT_DIR}/logs/agent_parity_gate/${RUN_STAMP}"
fi
RUN_DIRS=()
SKIP_COVERAGE=0
SKIP_MODEL_CATALOG=0
SKIP_PROVIDER_SMOKE=0
SKIP_CODING_FIXTURE=0
SKIP_METRICS=0
DEDUPE_LATEST_CASE=0
EXPECT_CASE_COUNT=0
LIVE_METRICS_RAN=0

MIN_PASS_RATE="${MIN_PASS_RATE:-1.0}"
MAX_AVG_LLM_CALLS="${MAX_AVG_LLM_CALLS:-4}"
MAX_PROMPT_TRUNCATIONS="${MAX_PROMPT_TRUNCATIONS:-0}"
MAX_PROVIDER_FINAL_ERRORS="${MAX_PROVIDER_FINAL_ERRORS:-0}"
MAX_PROVIDER_RETRYABLE_ERRORS="${MAX_PROVIDER_RETRYABLE_ERRORS:-}"
MAX_VERIFIER_CALLS="${MAX_VERIFIER_CALLS:-}"
MAX_PROMPT_BYTES_BEFORE="${MAX_PROMPT_BYTES_BEFORE:-}"
CHINESE_PROVIDER_LIVE_PROVIDERS="${CHINESE_PROVIDER_LIVE_PROVIDERS:-minimax}"
CHINESE_PROVIDER_ENV_FILE="${CHINESE_PROVIDER_ENV_FILE:-${ROOT_DIR}/../runtime_env_filled.sh}"
CHINESE_PROVIDER_ENV_FILE_STATE="auto"
CHINESE_PROVIDER_ENV_FILE_SOURCE="default"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_agent_parity_gate.sh [options] [run-dir ...]

What it gates:
  - Static compact NL metadata coverage, including agent parity, async, media dry-run, and no X/Twitter live publish rows.
  - Offline coding-loop repair fixture expectations and bounded metrics.
  - Optional real client-like run metrics when one or more run directories are provided.

Options:
  --run-dir PATH                  Add a client-like run directory to summarize.
  --out-dir PATH                  Gate artifact directory. Default: logs/agent_parity_gate/<timestamp>
  --skip-coverage                 Skip compact metadata coverage.
  --skip-model-catalog            Skip Chinese-provider model catalog guard.
  --skip-provider-smoke           Skip Chinese-provider dry-run smoke matrix.
  --skip-coding-fixture           Skip offline coding-loop repair fixture.
  --skip-metrics                  Skip metrics gates for provided run dirs.
  --dedupe-latest-case            For rerun shards, keep latest valid turn per numeric case id.
  --expect-case-count N           Require at least N unique case ids when deduping.
  --min-pass-rate N               Metrics gate. Default: MIN_PASS_RATE or 1.0.
  --max-avg-llm-calls N           Metrics gate. Default: MAX_AVG_LLM_CALLS or 4.
  --max-prompt-truncations N      Metrics gate. Default: MAX_PROMPT_TRUNCATIONS or 0.
  --max-provider-final-errors N   Metrics gate. Default: MAX_PROVIDER_FINAL_ERRORS or 0.
  --max-provider-retryable-errors N
  --max-verifier-calls N
  --max-prompt-bytes-before N
  --chinese-live-providers CSV  Chinese-provider live scope for smoke matrix. Default: CHINESE_PROVIDER_LIVE_PROVIDERS or minimax. Use all for every requested provider.
  --chinese-env-file PATH       Env file passed to Chinese-provider smoke preflight. Default: CHINESE_PROVIDER_ENV_FILE or ../runtime_env_filled.sh when present.
  --no-chinese-env-file         Do not pass an env file to Chinese-provider smoke preflight.
  -h, --help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --run-dir)
      RUN_DIRS+=("${2:-}")
      shift 2
      ;;
    --out-dir)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    --skip-coverage)
      SKIP_COVERAGE=1
      shift
      ;;
    --skip-model-catalog)
      SKIP_MODEL_CATALOG=1
      shift
      ;;
    --skip-provider-smoke)
      SKIP_PROVIDER_SMOKE=1
      shift
      ;;
    --skip-coding-fixture)
      SKIP_CODING_FIXTURE=1
      shift
      ;;
    --skip-metrics)
      SKIP_METRICS=1
      shift
      ;;
    --dedupe-latest-case)
      DEDUPE_LATEST_CASE=1
      shift
      ;;
    --expect-case-count)
      EXPECT_CASE_COUNT="${2:-}"
      shift 2
      ;;
    --min-pass-rate)
      MIN_PASS_RATE="${2:-}"
      shift 2
      ;;
    --max-avg-llm-calls)
      MAX_AVG_LLM_CALLS="${2:-}"
      shift 2
      ;;
    --max-prompt-truncations)
      MAX_PROMPT_TRUNCATIONS="${2:-}"
      shift 2
      ;;
    --max-provider-final-errors)
      MAX_PROVIDER_FINAL_ERRORS="${2:-}"
      shift 2
      ;;
    --max-provider-retryable-errors)
      MAX_PROVIDER_RETRYABLE_ERRORS="${2:-}"
      shift 2
      ;;
    --max-verifier-calls)
      MAX_VERIFIER_CALLS="${2:-}"
      shift 2
      ;;
    --max-prompt-bytes-before)
      MAX_PROMPT_BYTES_BEFORE="${2:-}"
      shift 2
      ;;
    --chinese-live-providers)
      CHINESE_PROVIDER_LIVE_PROVIDERS="${2:-}"
      shift 2
      ;;
    --chinese-env-file)
      CHINESE_PROVIDER_ENV_FILE="${2:-}"
      CHINESE_PROVIDER_ENV_FILE_STATE="explicit"
      CHINESE_PROVIDER_ENV_FILE_SOURCE="explicit"
      shift 2
      ;;
    --no-chinese-env-file)
      CHINESE_PROVIDER_ENV_FILE=""
      CHINESE_PROVIDER_ENV_FILE_STATE="disabled"
      CHINESE_PROVIDER_ENV_FILE_SOURCE="disabled"
      shift
      ;;
    -*)
      echo "Unknown option: $1" >&2
      exit 2
      ;;
    *)
      RUN_DIRS+=("$1")
      shift
      ;;
  esac
done

mkdir -p "$OUT_DIR"

chinese_provider_env_file_args=()
if [[ -n "$CHINESE_PROVIDER_ENV_FILE" && -f "$CHINESE_PROVIDER_ENV_FILE" ]]; then
  chinese_provider_env_file_args+=(--env-file "$CHINESE_PROVIDER_ENV_FILE")
  CHINESE_PROVIDER_ENV_FILE_STATE="present"
elif [[ "$CHINESE_PROVIDER_ENV_FILE_STATE" != "disabled" ]]; then
  CHINESE_PROVIDER_ENV_FILE_STATE="missing"
fi

metrics_args() {
  local out_path="$1"
  shift
  local args=(
    "$@"
    --output "$out_path"
    --min-pass-rate "$MIN_PASS_RATE"
    --max-avg-llm-calls "$MAX_AVG_LLM_CALLS"
    --max-prompt-truncations "$MAX_PROMPT_TRUNCATIONS"
    --max-provider-final-errors "$MAX_PROVIDER_FINAL_ERRORS"
  )
  if [[ -n "$MAX_PROVIDER_RETRYABLE_ERRORS" ]]; then
    args+=(--max-provider-retryable-errors "$MAX_PROVIDER_RETRYABLE_ERRORS")
  fi
  if [[ -n "$MAX_VERIFIER_CALLS" ]]; then
    args+=(--max-verifier-calls "$MAX_VERIFIER_CALLS")
  fi
  if [[ -n "$MAX_PROMPT_BYTES_BEFORE" ]]; then
    args+=(--max-prompt-bytes-before "$MAX_PROMPT_BYTES_BEFORE")
  fi
  if [[ "$DEDUPE_LATEST_CASE" -eq 1 ]]; then
    args+=(--dedupe-latest-case)
    if [[ "$EXPECT_CASE_COUNT" != "0" ]]; then
      args+=(--expect-case-count "$EXPECT_CASE_COUNT")
    fi
  fi
  printf '%s\n' "${args[@]}"
}

run_metrics_gate() {
  local out_path="$1"
  shift
  mapfile -t args < <(metrics_args "$out_path" "$@")
  python3 "${SCRIPT_DIR}/summarize_rollout_metrics.py" "${args[@]}"
}

path_ref() {
  local value="$1"
  python3 - "$ROOT_DIR" "$OUT_DIR" "$value" <<'PY'
import sys
from pathlib import Path, PurePosixPath

root = Path(sys.argv[1]).resolve()
out_dir = Path(sys.argv[2]).resolve()
raw = sys.argv[3]

try:
    candidate = Path(raw).resolve()
except OSError:
    print("external_path")
    raise SystemExit

try:
    rel = candidate.relative_to(out_dir)
    print("out_dir" if str(rel) == "." else f"out_dir/{rel.as_posix()}")
    raise SystemExit
except ValueError:
    pass

try:
    print(candidate.relative_to(root).as_posix())
    raise SystemExit
except ValueError:
    pass

if not raw.startswith("/") and "\\" not in raw:
    rel = PurePosixPath(raw)
    if rel.parts and all(part not in {"", ".", ".."} for part in rel.parts):
        print(rel.as_posix())
        raise SystemExit

print("external_path")
PY
}

echo "AGENT_PARITY_GATE out_dir_ref=$(path_ref "$OUT_DIR")"

echo "AGENT_PARITY_GATE_STEP runtime_hard_reply_baseline"
python3 "${ROOT_DIR}/scripts/check_no_runtime_hard_reply.py" --all \
  --baseline "${ROOT_DIR}/scripts/baselines/runtime_hard_reply_baseline.txt" \
  --fail-on-new \
  > "${OUT_DIR}/runtime_hard_reply_baseline.txt"

echo "AGENT_PARITY_GATE_STEP no_agent_mode_payload"
{
  python3 "${ROOT_DIR}/scripts/check_no_agent_mode_payload.py" --self-test
  python3 "${ROOT_DIR}/scripts/check_no_agent_mode_payload.py"
} > "${OUT_DIR}/no_agent_mode_payload.txt"

echo "AGENT_PARITY_GATE_STEP agent_loop_static_contracts"
{
  echo "AGENT_LOOP_STATIC_SELF_TEST check_route_authority_legacy_keys.py"
  python3 "${ROOT_DIR}/scripts/check_route_authority_legacy_keys.py" --self-test
  echo "AGENT_LOOP_STATIC_SELF_TEST check_legacy_route_boundary.py"
  python3 "${ROOT_DIR}/scripts/check_legacy_route_boundary.py" --self-test
  echo "AGENT_LOOP_STATIC_SELF_TEST check_pre_planner_exit_inventory.py"
  python3 "${ROOT_DIR}/scripts/check_pre_planner_exit_inventory.py" --self-test
  echo "AGENT_LOOP_STATIC_SELF_TEST check_no_nl_hardmatch.py"
  python3 "${ROOT_DIR}/scripts/check_no_nl_hardmatch.py" --self-test
  echo "AGENT_LOOP_STATIC_SELF_TEST check_historical_hardcoded_language.py"
  python3 "${ROOT_DIR}/scripts/check_historical_hardcoded_language.py" --self-test
  python3 "${ROOT_DIR}/scripts/check_route_authority_legacy_keys.py"
  python3 "${ROOT_DIR}/scripts/check_legacy_route_boundary.py"
  python3 "${ROOT_DIR}/scripts/check_pre_planner_exit_inventory.py"
  python3 "${ROOT_DIR}/scripts/check_no_nl_hardmatch.py"
  python3 "${ROOT_DIR}/scripts/check_historical_hardcoded_language.py" \
    --fail-on-runtime \
    --fail-on-ui-visible
} > "${OUT_DIR}/agent_loop_static_contracts.txt"

echo "AGENT_PARITY_GATE_STEP evidence_extractor_contracts"
{
  python3 "${ROOT_DIR}/scripts/check_evidence_extractor_contracts.py" --self-test
  python3 "${ROOT_DIR}/scripts/check_evidence_extractor_contracts.py"
} > "${OUT_DIR}/evidence_extractor_contracts.txt"

echo "AGENT_PARITY_GATE_STEP secret_scan_contract"
python3 "${SCRIPT_DIR}/check_secret_scan_contract.py" --json \
  > "${OUT_DIR}/secret_scan_contract.json"

echo "AGENT_PARITY_GATE_STEP suite_wrapper_contract"
python3 "${SCRIPT_DIR}/check_suite_wrapper_contract.py" --json \
  > "${OUT_DIR}/suite_wrapper_contract.json"

echo "AGENT_PARITY_GATE_STEP runner_path_ref_contract"
python3 "${SCRIPT_DIR}/check_runner_path_ref_contract.py" --json \
  > "${OUT_DIR}/runner_path_ref_contract.json"

echo "AGENT_PARITY_GATE_STEP suite_artifact_contract_self_test"
python3 "${SCRIPT_DIR}/check_suite_artifact_contract.py" --self-test \
  > "${OUT_DIR}/suite_artifact_contract_self_test.txt"

echo "AGENT_PARITY_GATE_STEP llm_raw_trace_runner_contract"
{
  python3 "${SCRIPT_DIR}/print_llm_raw_trace.py" --self-test
  python3 "${SCRIPT_DIR}/check_llm_raw_trace_runner_contract.py"
} > "${OUT_DIR}/llm_raw_trace_runner_contract.txt"

echo "AGENT_PARITY_GATE_STEP rollout_metrics_contract"
python3 "${SCRIPT_DIR}/summarize_rollout_metrics.py" --self-test \
  > "${OUT_DIR}/rollout_metrics_contract.txt"

if [[ "$SKIP_COVERAGE" -eq 0 ]]; then
  echo "AGENT_PARITY_GATE_STEP compact_coverage"
  python3 "${SCRIPT_DIR}/check_compact_coverage.py" --report \
    > "${OUT_DIR}/compact_coverage.json"
fi

if [[ "$SKIP_MODEL_CATALOG" -eq 0 ]]; then
  echo "AGENT_PARITY_GATE_STEP chinese_model_catalog_self_test"
  python3 "${ROOT_DIR}/scripts/check_chinese_model_catalog.py" \
    --self-test \
    > "${OUT_DIR}/chinese_model_catalog_self_test.txt"

  echo "AGENT_PARITY_GATE_STEP chinese_model_catalog"
  python3 "${ROOT_DIR}/scripts/check_chinese_model_catalog.py" \
    --json \
    "${chinese_provider_env_file_args[@]}" \
    > "${OUT_DIR}/chinese_model_catalog.json"
fi

if [[ "$SKIP_PROVIDER_SMOKE" -eq 0 ]]; then
  echo "AGENT_PARITY_GATE_STEP chinese_provider_smoke_dry_run"
  chinese_provider_smoke_args=(
    --dry-run \
    --live-providers "$CHINESE_PROVIDER_LIVE_PROVIDERS" \
    --out-dir "${OUT_DIR}/chinese_provider_smoke"
  )
  chinese_provider_smoke_args+=("${chinese_provider_env_file_args[@]}")
  bash "${SCRIPT_DIR}/run_chinese_provider_smoke_matrix.sh" \
    "${chinese_provider_smoke_args[@]}" \
    > "${OUT_DIR}/chinese_provider_smoke.txt"
fi

if [[ "$SKIP_CODING_FIXTURE" -eq 0 ]]; then
  echo "AGENT_PARITY_GATE_STEP coding_loop_repair_fixture"
  python3 "${SCRIPT_DIR}/evaluate_client_like_run.py" \
    "${SCRIPT_DIR}/fixtures/client_like_runs/coding_loop_repair" \
    --expectations "${SCRIPT_DIR}/expectations/coding_loop_repair_fixture.jsonl" \
    > "${OUT_DIR}/coding_loop_repair_eval.txt"

  run_metrics_gate \
    "${OUT_DIR}/coding_loop_repair_metrics.json" \
    "${SCRIPT_DIR}/fixtures/client_like_runs/coding_loop_repair" \
    > "${OUT_DIR}/coding_loop_repair_metrics.txt"
fi

if [[ "${#RUN_DIRS[@]}" -gt 0 && "$SKIP_METRICS" -eq 0 ]]; then
  echo "AGENT_PARITY_GATE_STEP live_run_metrics count=${#RUN_DIRS[@]}"
  run_metrics_gate "${OUT_DIR}/run_metrics.json" "${RUN_DIRS[@]}" \
    > "${OUT_DIR}/run_metrics.txt"
  LIVE_METRICS_RAN=1
elif [[ "${#RUN_DIRS[@]}" -eq 0 ]]; then
  echo "AGENT_PARITY_GATE_NO_RUN_DIR live metrics skipped"
fi

{
  echo "out_dir_ref=$(path_ref "$OUT_DIR")"
  echo "no_agent_mode_payload=1"
  echo "agent_loop_static_contracts=1"
  echo "evidence_extractor_contracts=1"
  echo "secret_scan_contract=1"
  echo "suite_wrapper_contract=1"
  echo "runner_path_ref_contract=1"
  echo "suite_artifact_contract_self_test=1"
  echo "llm_raw_trace_runner_contract=1"
  echo "rollout_metrics_contract=1"
  echo "coverage=$((1 - SKIP_COVERAGE))"
  echo "model_catalog=$((1 - SKIP_MODEL_CATALOG))"
  echo "provider_smoke=$((1 - SKIP_PROVIDER_SMOKE))"
  echo "coding_fixture=$((1 - SKIP_CODING_FIXTURE))"
  echo "run_dir_count=${#RUN_DIRS[@]}"
  echo "metrics=$((1 - SKIP_METRICS))"
  echo "live_metrics=${LIVE_METRICS_RAN}"
  echo "min_pass_rate=${MIN_PASS_RATE}"
  echo "max_avg_llm_calls=${MAX_AVG_LLM_CALLS}"
  echo "max_prompt_truncations=${MAX_PROMPT_TRUNCATIONS}"
  echo "max_provider_final_errors=${MAX_PROVIDER_FINAL_ERRORS}"
  echo "chinese_provider_live_providers=${CHINESE_PROVIDER_LIVE_PROVIDERS}"
  echo "chinese_provider_env_file_state=${CHINESE_PROVIDER_ENV_FILE_STATE}"
  echo "chinese_provider_env_file_source=${CHINESE_PROVIDER_ENV_FILE_SOURCE}"
} > "${OUT_DIR}/gate_summary.env"

echo "AGENT_PARITY_GATE_OK out_dir_ref=$(path_ref "$OUT_DIR")"

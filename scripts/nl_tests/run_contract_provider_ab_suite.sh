#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

COUNT=20
OUT_DIR="${ROOT_DIR}/scripts/nl_suite_logs/provider_ab/$(date +%Y%m%d_%H%M%S)"
CASE_JSONL=""
EXPECTATIONS_JSONL=""
PHASE=""
SIDE=""
PROVIDER=""
RUN_DIR=""
LEFT_RUN_DIR=""
RIGHT_RUN_DIR=""
LEFT_LABEL="left"
RIGHT_LABEL="right"
ENV_FILE=""
RUN_RETRIES="${PROVIDER_AB_RETRIES:-1}"
RETRY_SLEEP_SECONDS="${PROVIDER_AB_RETRY_SLEEP_SECONDS:-30}"

usage() {
  cat <<'EOF'
Usage:
  # 1) Generate one shared case set.
  bash scripts/nl_tests/run_contract_provider_ab_suite.sh --prepare --count 20 --out-dir /tmp/provider-ab

  # 2) Restart or start clawd for the target provider, then run that side.
  bash scripts/nl_tests/run_contract_provider_ab_suite.sh --run-side left --provider minimax \
    --case-jsonl /tmp/provider-ab/cases.jsonl --out-dir /tmp/provider-ab

  # 3) Restart or start clawd for the other provider, then run that side.
  bash scripts/nl_tests/run_contract_provider_ab_suite.sh --run-side right --provider openai \
    --case-jsonl /tmp/provider-ab/cases.jsonl --out-dir /tmp/provider-ab

  # 4) Compare the two run directories.
  bash scripts/nl_tests/run_contract_provider_ab_suite.sh --compare \
    --left-run-dir /tmp/provider-ab/left/run_... \
    --right-run-dir /tmp/provider-ab/right/run_...

Options:
  --prepare                  generate shared contract NL cases and expectations
  --run-side SIDE            run one side against the currently running clawd
  --compare                  compare two completed run directories
  --provider NAME            provider label for --run-side; also exported as RUSTCLAW_PROVIDER_OVERRIDE
  --count N                  case count for --prepare, default 20
  --case-jsonl PATH          case JSONL to replay
  --expectations PATH        expectations JSONL; default is sibling of case JSONL
  --out-dir PATH             output directory for generated cases and side logs
  --left-run-dir PATH        left run directory for --compare
  --right-run-dir PATH       right run directory for --compare
  --left-label LABEL         comparison label, default left
  --right-label LABEL        comparison label, default right
  --env-file PATH            optional shell env file to source before running
  --retries N                attempts for --run-side, default 1
  --retry-sleep-seconds N    delay between attempts, default 30

Important:
  RUSTCLAW_PROVIDER_OVERRIDE is read by clawd when clawd starts. This script
  does not change the provider inside an already running clawd process. Before
  each --run-side invocation, make sure clawd is running with the intended
  provider/config. The env var is exported here for setups that launch clawd
  from the same wrapper, and for clear run metadata.

  Known providers are preflighted for their required local credential env var.
  If a required key is missing, --run-side writes inconclusive metadata and
  exits successfully without calling clawd, so missing credentials are not
  reported as contract drift.
EOF
}

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" --anchor "$OUT_DIR" --anchor-name out_dir "$1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prepare)
      PHASE="prepare"
      shift
      ;;
    --run-side)
      PHASE="run-side"
      SIDE="${2:-}"
      shift 2
      ;;
    --compare)
      PHASE="compare"
      shift
      ;;
    --provider)
      PROVIDER="${2:-}"
      shift 2
      ;;
    --count)
      COUNT="${2:-}"
      shift 2
      ;;
    --case-jsonl)
      CASE_JSONL="${2:-}"
      shift 2
      ;;
    --expectations)
      EXPECTATIONS_JSONL="${2:-}"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    --left-run-dir)
      LEFT_RUN_DIR="${2:-}"
      shift 2
      ;;
    --right-run-dir)
      RIGHT_RUN_DIR="${2:-}"
      shift 2
      ;;
    --left-label)
      LEFT_LABEL="${2:-}"
      shift 2
      ;;
    --right-label)
      RIGHT_LABEL="${2:-}"
      shift 2
      ;;
    --env-file)
      ENV_FILE="${2:-}"
      shift 2
      ;;
    --retries)
      RUN_RETRIES="${2:-}"
      shift 2
      ;;
    --retry-sleep-seconds)
      RETRY_SLEEP_SECONDS="${2:-}"
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

cd "${ROOT_DIR}"

if [[ -n "${ENV_FILE}" ]]; then
  # shellcheck source=/dev/null
  source "${ENV_FILE}"
fi

if [[ -z "${PHASE}" ]]; then
  echo "Choose one phase: --prepare, --run-side, or --compare" >&2
  usage >&2
  exit 2
fi

if ! [[ "${COUNT}" =~ ^[0-9]+$ ]] || [[ "${COUNT}" -le 0 ]]; then
  echo "--count must be a positive integer, got: ${COUNT}" >&2
  exit 2
fi
if ! [[ "${RUN_RETRIES}" =~ ^[0-9]+$ ]] || [[ "${RUN_RETRIES}" -le 0 ]]; then
  echo "--retries must be a positive integer, got: ${RUN_RETRIES}" >&2
  exit 2
fi
if ! [[ "${RETRY_SLEEP_SECONDS}" =~ ^[0-9]+$ ]]; then
  echo "--retry-sleep-seconds must be a non-negative integer, got: ${RETRY_SLEEP_SECONDS}" >&2
  exit 2
fi

prepare_cases() {
  mkdir -p "${OUT_DIR}"
  CASE_JSONL="${OUT_DIR}/cases.jsonl"
  EXPECTATIONS_JSONL="${OUT_DIR}/expectations.jsonl"
  local -a generator_args=(
    --count "${COUNT}" \
    --nl \
    --expectations "${EXPECTATIONS_JSONL}" \
    --report
  )
  if [[ "${COUNT}" -ge 100 ]]; then
    generator_args+=(--check)
  else
    echo "NOTE: --count ${COUNT} is a smoke-sized sample; full generator coverage check starts at count >= 100."
  fi
  python3 "${ROOT_DIR}/scripts/nl_tests/generate_contract_matrix_cases.py" \
    "${generator_args[@]}" \
    > "${CASE_JSONL}"
  cat > "${OUT_DIR}/manifest.env" <<EOF
CASE_JSONL=${CASE_JSONL}
EXPECTATIONS_JSONL=${EXPECTATIONS_JSONL}
CASE_JSONL_REF=$(path_ref "${CASE_JSONL}")
EXPECTATIONS_JSONL_REF=$(path_ref "${EXPECTATIONS_JSONL}")
COUNT=${COUNT}
EOF
  echo "PROVIDER_AB_PREPARE_OK out_dir_ref=$(path_ref "${OUT_DIR}") case_jsonl_ref=$(path_ref "${CASE_JSONL}") expectations_ref=$(path_ref "${EXPECTATIONS_JSONL}")"
}

extract_run_dir() {
  local output_file="$1"
  python3 - "${output_file}" <<'PY'
from pathlib import Path
import re
import sys

text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
matches = re.findall(r"log_dir=([^\s]+)", text)
if not matches:
    raise SystemExit("could not find log_dir in run output")
print(matches[-1])
PY
}

output_looks_provider_inconclusive() {
  local output_file="$1"
  grep -Eiq 'provider_(error|unavailable)|llm_provider_unavailable|rate.?limit|quota|capacity|429|timeout|timed out|connection refused|connection reset|dns|temporar(y|ily unavailable)' "${output_file}"
}

provider_required_env_var() {
  local provider_lc
  provider_lc="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
  case "${provider_lc}" in
    openai|openai_compat|openai-compatible)
      echo "OPENAI_API_KEY"
      ;;
    minimax|mimo)
      echo "MINIMAX_API_KEY"
      ;;
    *)
      return 1
      ;;
  esac
}

run_side() {
  if [[ -z "${SIDE}" ]]; then
    echo "--run-side requires SIDE" >&2
    exit 2
  fi
  if [[ -z "${PROVIDER}" ]]; then
    echo "--run-side requires --provider" >&2
    exit 2
  fi
  if [[ -z "${CASE_JSONL}" ]]; then
    CASE_JSONL="${OUT_DIR}/cases.jsonl"
  fi
  if [[ ! -f "${CASE_JSONL}" ]]; then
    echo "case JSONL not found: ${CASE_JSONL}" >&2
    exit 2
  fi
  if [[ -z "${EXPECTATIONS_JSONL}" ]]; then
    EXPECTATIONS_JSONL="$(dirname "${CASE_JSONL}")/expectations.jsonl"
  fi

  mkdir -p "${OUT_DIR}/${SIDE}"
  local output_file="${OUT_DIR}/${SIDE}/run.output.txt"
  local attempt_output_file=""
  local suite_status=1
  local attempt=1
  local attempts_run=0
  local required_env=""
  required_env="$(provider_required_env_var "${PROVIDER}" || true)"
  if [[ -n "${required_env}" && -z "${!required_env:-}" ]]; then
    local reason="missing_env:${required_env}"
    printf 'PROVIDER_AB_RUN_SIDE_INCONCLUSIVE side=%s provider=%s reason=%s\n' \
      "${SIDE}" "${PROVIDER}" "${reason}" > "${output_file}"
    cat > "${OUT_DIR}/${SIDE}/metadata.json" <<EOF
{"side":"${SIDE}","provider":"${PROVIDER}","status":"inconclusive","attempts":0,"run_dir":"","run_dir_ref":"","case_jsonl":"${CASE_JSONL}","case_jsonl_ref":"$(path_ref "${CASE_JSONL}")","expectations":"${EXPECTATIONS_JSONL}","expectations_ref":"$(path_ref "${EXPECTATIONS_JSONL}")","output_file":"${output_file}","output_file_ref":"$(path_ref "${output_file}")","reason":"${reason}"}
EOF
    cat "${output_file}"
    return 0
  fi
  echo "PROVIDER_AB_RUN_SIDE side=${SIDE} provider=${PROVIDER}"
  echo "NOTE: ensure clawd was started with provider=${PROVIDER}; provider override is startup-scoped."
  while [[ "${attempt}" -le "${RUN_RETRIES}" ]]; do
    attempt_output_file="${OUT_DIR}/${SIDE}/run.attempt_${attempt}.output.txt"
    echo "PROVIDER_AB_RUN_SIDE_ATTEMPT side=${SIDE} provider=${PROVIDER} attempt=${attempt}/${RUN_RETRIES}"
    set +e
    RUSTCLAW_PROVIDER_OVERRIDE="${PROVIDER}" \
    CLIENT_LIKE_TEST_ID="provider-ab-${SIDE}-${PROVIDER}-attempt-${attempt}-$(date +%Y%m%d_%H%M%S)" \
      bash "${ROOT_DIR}/scripts/nl_tests/run_client_like_continuous_suite.sh" \
        --skip-smoke \
        --case-jsonl "${CASE_JSONL}" \
        --case-limit "${COUNT}" \
        --quality-guard \
        --log-root "${OUT_DIR}/${SIDE}" \
        | tee "${attempt_output_file}"
    suite_status="${PIPESTATUS[0]}"
    set -e
    attempts_run="${attempt}"
    cp "${attempt_output_file}" "${output_file}"
    if [[ "${suite_status}" -eq 0 ]]; then
      break
    fi
    if [[ "${attempt}" -lt "${RUN_RETRIES}" ]]; then
      echo "PROVIDER_AB_RUN_SIDE_RETRY side=${SIDE} provider=${PROVIDER} attempt=${attempt} status=${suite_status} sleep_seconds=${RETRY_SLEEP_SECONDS}"
      sleep "${RETRY_SLEEP_SECONDS}"
    fi
    attempt=$((attempt + 1))
  done
  if [[ "${suite_status}" -ne 0 ]]; then
    RUN_DIR="$(extract_run_dir "${output_file}" 2>/dev/null || true)"
    if output_looks_provider_inconclusive "${output_file}"; then
      cat > "${OUT_DIR}/${SIDE}/metadata.json" <<EOF
{"side":"${SIDE}","provider":"${PROVIDER}","status":"inconclusive","attempts":${attempts_run},"run_dir":"${RUN_DIR}","run_dir_ref":"$(path_ref "${RUN_DIR}")","case_jsonl":"${CASE_JSONL}","case_jsonl_ref":"$(path_ref "${CASE_JSONL}")","expectations":"${EXPECTATIONS_JSONL}","expectations_ref":"$(path_ref "${EXPECTATIONS_JSONL}")","output_file":"${output_file}","output_file_ref":"$(path_ref "${output_file}")"}
EOF
      echo "PROVIDER_AB_RUN_SIDE_INCONCLUSIVE side=${SIDE} provider=${PROVIDER} attempts=${attempts_run} run_dir_ref=$(path_ref "${RUN_DIR}")"
      return 0
    fi
    echo "PROVIDER_AB_RUN_SIDE_FAIL side=${SIDE} provider=${PROVIDER} attempts=${attempts_run} status=${suite_status}" >&2
    return "${suite_status}"
  fi
  RUN_DIR="$(extract_run_dir "${output_file}")"
  python3 "${ROOT_DIR}/scripts/nl_tests/evaluate_client_like_run.py" \
    "${RUN_DIR}" \
    --expectations "${EXPECTATIONS_JSONL}"
  cat > "${OUT_DIR}/${SIDE}/metadata.json" <<EOF
{"side":"${SIDE}","provider":"${PROVIDER}","status":"passed","attempts":${attempts_run},"run_dir":"${RUN_DIR}","run_dir_ref":"$(path_ref "${RUN_DIR}")","case_jsonl":"${CASE_JSONL}","case_jsonl_ref":"$(path_ref "${CASE_JSONL}")","expectations":"${EXPECTATIONS_JSONL}","expectations_ref":"$(path_ref "${EXPECTATIONS_JSONL}")"}
EOF
  echo "PROVIDER_AB_RUN_SIDE_OK side=${SIDE} provider=${PROVIDER} attempts=${attempts_run} run_dir_ref=$(path_ref "${RUN_DIR}")"
}

compare_runs() {
  if [[ -z "${LEFT_RUN_DIR}" || -z "${RIGHT_RUN_DIR}" ]]; then
    echo "--compare requires --left-run-dir and --right-run-dir" >&2
    exit 2
  fi
  python3 "${ROOT_DIR}/scripts/nl_tests/compare_contract_provider_runs.py" \
    --left "${LEFT_RUN_DIR}" \
    --right "${RIGHT_RUN_DIR}" \
    --left-label "${LEFT_LABEL}" \
    --right-label "${RIGHT_LABEL}"
}

case "${PHASE}" in
  prepare)
    prepare_cases
    ;;
  run-side)
    run_side
    ;;
  compare)
    compare_runs
    ;;
  *)
    echo "unsupported phase: ${PHASE}" >&2
    exit 2
    ;;
esac

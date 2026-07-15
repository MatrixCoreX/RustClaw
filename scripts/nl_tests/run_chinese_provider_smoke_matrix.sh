#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

CASE_FILE="${ROOT_DIR}/scripts/nl_tests/cases/nl_cases_chinese_model_adapter_20260715.txt"
OUT_DIR="${ROOT_DIR}/scripts/nl_suite_logs/chinese_provider_smoke/$(date +%Y%m%d_%H%M%S)"
BASE_URL_VALUE="${BASE_URL:-http://127.0.0.1:8787}"
ENV_FILE=""
CASE_LIMIT=""
WAIT_SECONDS_VALUE="${MAX_WAIT_SECONDS:-1200}"
POLL_SECONDS_VALUE="${POLL_INTERVAL_SECONDS:-1}"
QUALITY_GUARD=1
DRY_RUN=0
PROVIDERS=()

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_chinese_provider_smoke_matrix.sh [options]

Options:
  --provider NAME         Add one provider to run. May be repeated.
  --providers CSV        Provider list. Default: minimax,mimo,qwen,deepseek.
  --case-file PATH       NL case file. Default: chinese model adapter compact set.
  --case-limit N         Limit appended cases passed to the client-like runner.
  --out-dir PATH         Output directory for matrix metadata and run logs.
  --base-url URL         clawd base URL. Default: http://127.0.0.1:8787.
  --env-file PATH        Optional shell env file to source before preflight.
  --wait-seconds N       Max wait per NL turn. Default: MAX_WAIT_SECONDS or 1200.
  --poll-seconds N       Poll interval. Default: POLL_INTERVAL_SECONDS or 1.
  --no-quality-guard     Do not pass --quality-guard to the NL runner.
  --dry-run              Validate cases and credential state but do not call clawd.
  -h, --help             Show this help.

Credential preflight:
  minimax  -> MINIMAX_API_KEY
  mimo     -> MIMO_API_KEY or XIAOMI_API_KEY
  qwen     -> QWEN_API_KEY or DASHSCOPE_API_KEY
  deepseek -> DEEPSEEK_API_KEY

Notes:
  RUSTCLAW_PROVIDER_OVERRIDE is startup-scoped for clawd. This runner exports it
  for metadata and wrappers that start clawd from the same environment; it does
  not rewrite a running clawd process in place.
EOF
}

add_csv_providers() {
  local raw="$1"
  local item
  IFS=',' read -r -a items <<< "$raw"
  for item in "${items[@]}"; do
    item="$(printf '%s' "$item" | tr '[:upper:]' '[:lower:]' | xargs)"
    if [[ -n "$item" ]]; then
      PROVIDERS+=("$item")
    fi
  done
}

provider_required_env_vars() {
  case "$1" in
    minimax)
      printf '%s\n' "MINIMAX_API_KEY"
      ;;
    mimo)
      printf '%s\n' "MIMO_API_KEY" "XIAOMI_API_KEY"
      ;;
    qwen)
      printf '%s\n' "QWEN_API_KEY" "DASHSCOPE_API_KEY"
      ;;
    deepseek)
      printf '%s\n' "DEEPSEEK_API_KEY"
      ;;
    *)
      return 1
      ;;
  esac
}

provider_has_credentials() {
  local provider="$1"
  local env_name
  while IFS= read -r env_name; do
    if [[ -n "${!env_name:-}" ]]; then
      return 0
    fi
  done < <(provider_required_env_vars "$provider" || true)
  return 1
}

required_env_csv() {
  local provider="$1"
  local env_name
  local out=""
  while IFS= read -r env_name; do
    if [[ -z "$out" ]]; then
      out="$env_name"
    else
      out="${out},${env_name}"
    fi
  done < <(provider_required_env_vars "$provider" || true)
  printf '%s' "$out"
}

write_metadata() {
  local path="$1"
  local provider="$2"
  local status="$3"
  local reason="$4"
  local run_dir="$5"
  local output_file="$6"
  local exit_code="$7"
  python3 - "$path" "$provider" "$status" "$reason" "$run_dir" "$output_file" "$exit_code" "$CASE_FILE" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
provider = sys.argv[2]
status = sys.argv[3]
reason = sys.argv[4]
run_dir = sys.argv[5]
output_file = sys.argv[6]
exit_code = int(sys.argv[7])
case_file = sys.argv[8]
payload = {
    "provider": provider,
    "status": status,
    "reason_code": reason,
    "run_dir": run_dir,
    "output_file": output_file,
    "exit_code": exit_code,
    "case_file": case_file,
}
path.write_text(json.dumps(payload, ensure_ascii=False, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
PY
}

append_summary() {
  local metadata_file="$1"
  cat "$metadata_file" >> "${OUT_DIR}/provider_summary.jsonl"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --provider)
      add_csv_providers "${2:-}"
      shift 2
      ;;
    --providers)
      add_csv_providers "${2:-}"
      shift 2
      ;;
    --case-file)
      CASE_FILE="${2:-}"
      shift 2
      ;;
    --case-limit)
      CASE_LIMIT="${2:-}"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    --base-url)
      BASE_URL_VALUE="${2:-}"
      shift 2
      ;;
    --env-file)
      ENV_FILE="${2:-}"
      shift 2
      ;;
    --wait-seconds)
      WAIT_SECONDS_VALUE="${2:-}"
      shift 2
      ;;
    --poll-seconds)
      POLL_SECONDS_VALUE="${2:-}"
      shift 2
      ;;
    --no-quality-guard)
      QUALITY_GUARD=0
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
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

cd "$ROOT_DIR"

if [[ -n "$ENV_FILE" ]]; then
  # shellcheck source=/dev/null
  source "$ENV_FILE"
fi

if [[ "${#PROVIDERS[@]}" -eq 0 ]]; then
  add_csv_providers "minimax,mimo,qwen,deepseek"
fi

if [[ ! -f "$CASE_FILE" ]]; then
  echo "case file not found: $CASE_FILE" >&2
  exit 2
fi

mkdir -p "$OUT_DIR"
: > "${OUT_DIR}/provider_summary.jsonl"

python3 "${ROOT_DIR}/scripts/nl_tests/check_chinese_provider_smoke_matrix.py" \
  --case-file "$CASE_FILE" \
  --json > "${OUT_DIR}/case_coverage.json"

echo "CHINESE_PROVIDER_SMOKE_MATRIX out_dir=${OUT_DIR}"
echo "case_file=${CASE_FILE}"
echo "providers=${PROVIDERS[*]}"
echo "dry_run=${DRY_RUN}"

matrix_status=0
for provider in "${PROVIDERS[@]}"; do
  provider_dir="${OUT_DIR}/${provider}"
  mkdir -p "$provider_dir"
  metadata_file="${provider_dir}/metadata.json"
  output_file="${provider_dir}/run.output.txt"
  required_env="$(required_env_csv "$provider")"
  if [[ -z "$required_env" ]]; then
    write_metadata "$metadata_file" "$provider" "skipped" "provider_unknown" "" "$output_file" 0
    append_summary "$metadata_file"
    echo "CHINESE_PROVIDER_SMOKE_SKIP provider=${provider} reason_code=provider_unknown"
    continue
  fi
  if ! provider_has_credentials "$provider"; then
    write_metadata "$metadata_file" "$provider" "skipped" "provider_missing_credentials" "" "$output_file" 0
    append_summary "$metadata_file"
    echo "CHINESE_PROVIDER_SMOKE_SKIP provider=${provider} reason_code=provider_missing_credentials required_env=${required_env}"
    continue
  fi
  if [[ "$DRY_RUN" -eq 1 ]]; then
    write_metadata "$metadata_file" "$provider" "planned" "dry_run" "" "$output_file" 0
    append_summary "$metadata_file"
    echo "CHINESE_PROVIDER_SMOKE_PLANNED provider=${provider}"
    continue
  fi

  runner_args=(
    --base-url "$BASE_URL_VALUE"
    --wait-seconds "$WAIT_SECONDS_VALUE"
    --poll-seconds "$POLL_SECONDS_VALUE"
    --skip-smoke
    --case-file "$CASE_FILE"
    --log-root "$provider_dir"
    --llm-trace-max-chars "1600"
  )
  if [[ -n "$CASE_LIMIT" ]]; then
    runner_args+=(--case-limit "$CASE_LIMIT")
  fi
  if [[ "$QUALITY_GUARD" -eq 1 ]]; then
    runner_args+=(--quality-guard)
  fi

  echo "CHINESE_PROVIDER_SMOKE_RUN provider=${provider}"
  set +e
  RUSTCLAW_PROVIDER_OVERRIDE="$provider" \
  CLIENT_LIKE_TEST_ID="chinese-provider-${provider}-$(date +%Y%m%d_%H%M%S)" \
    bash "${ROOT_DIR}/scripts/nl_tests/run_client_like_continuous_suite.sh" "${runner_args[@]}" \
    | tee "$output_file"
  run_status="${PIPESTATUS[0]}"
  set -e
  if [[ "$run_status" -eq 0 ]]; then
    write_metadata "$metadata_file" "$provider" "passed" "ok" "$provider_dir" "$output_file" "$run_status"
    echo "CHINESE_PROVIDER_SMOKE_PASS provider=${provider}"
  else
    matrix_status=1
    write_metadata "$metadata_file" "$provider" "failed" "runner_failed" "$provider_dir" "$output_file" "$run_status"
    echo "CHINESE_PROVIDER_SMOKE_FAIL provider=${provider} exit_code=${run_status}" >&2
  fi
  append_summary "$metadata_file"
done

python3 - "${OUT_DIR}/provider_summary.jsonl" "${OUT_DIR}/matrix_summary.json" <<'PY'
import json
import sys
from collections import Counter
from pathlib import Path

summary_path = Path(sys.argv[1])
out_path = Path(sys.argv[2])
rows = [
    json.loads(line)
    for line in summary_path.read_text(encoding="utf-8").splitlines()
    if line.strip()
]
counts = Counter(str(row.get("status") or "unknown") for row in rows)
payload = {
    "provider_count": len(rows),
    "status_counts": dict(sorted(counts.items())),
    "providers": rows,
}
out_path.write_text(json.dumps(payload, ensure_ascii=False, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
PY

exit "$matrix_status"

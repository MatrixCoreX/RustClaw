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
LIVE_PROVIDERS=()
LIVE_SCOPE_ALL=0
LIVE_SCOPE_SET=0
DEFAULT_LIVE_PROVIDERS="${CHINESE_PROVIDER_LIVE_PROVIDERS:-minimax}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_chinese_provider_smoke_matrix.sh [options]

Options:
  --provider NAME         Add one provider to run. May be repeated.
  --providers CSV        Provider list. Default: minimax,mimo,qwen,deepseek.
  --live-provider NAME    Mark one provider as in current live scope. May be repeated. Use all for every requested provider.
  --live-providers CSV    Current live-scope providers. Default: CHINESE_PROVIDER_LIVE_PROVIDERS or minimax. Use all for every requested provider.
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
  Live provider quota, account, auth, and model-access failures are recorded
  with structured provider_* reason_code values instead of generic runner_failed.
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

add_csv_live_providers() {
  local raw="$1"
  local item
  LIVE_SCOPE_SET=1
  IFS=',' read -r -a items <<< "$raw"
  for item in "${items[@]}"; do
    item="$(printf '%s' "$item" | tr '[:upper:]' '[:lower:]' | xargs)"
    if [[ -n "$item" ]]; then
      if [[ "$item" == "all" ]]; then
        LIVE_SCOPE_ALL=1
        LIVE_PROVIDERS=()
        continue
      fi
      if [[ "$LIVE_SCOPE_ALL" -eq 1 ]]; then
        continue
      fi
      LIVE_PROVIDERS+=("$item")
    fi
  done
}

provider_in_live_scope() {
  local provider="$1"
  local item
  if [[ "$LIVE_SCOPE_ALL" -eq 1 ]]; then
    return 0
  fi
  if [[ "${#LIVE_PROVIDERS[@]}" -eq 0 ]]; then
    return 0
  fi
  for item in "${LIVE_PROVIDERS[@]}"; do
    if [[ "$item" == "$provider" ]]; then
      return 0
    fi
  done
  return 1
}

live_scope_csv() {
  local item
  local out=""
  if [[ "$LIVE_SCOPE_ALL" -eq 1 ]]; then
    printf '%s' "all"
    return 0
  fi
  for item in "${LIVE_PROVIDERS[@]}"; do
    if [[ -z "$out" ]]; then
      out="$item"
    else
      out="${out},${item}"
    fi
  done
  printf '%s' "$out"
}

path_ref() {
  local value="$1"
  python3 - "$ROOT_DIR" "$OUT_DIR" "$value" <<'PY'
import sys
from pathlib import Path, PurePosixPath

root = Path(sys.argv[1]).resolve()
out_dir = Path(sys.argv[2]).resolve()
raw = sys.argv[3]


def safe_relative_text(text: str) -> str | None:
    normalized = text.replace("\\", "/")
    if not normalized or normalized.startswith("/") or any(ch.isspace() for ch in normalized):
        return None
    path = PurePosixPath(normalized)
    if any(part in {"", ".", ".."} for part in path.parts):
        return None
    return path.as_posix()


def path_ref(raw_value: str) -> str:
    if not raw_value:
        return ""
    path = Path(raw_value)
    if path.is_absolute():
        resolved = path.resolve()
        for base, prefix in ((out_dir, "out_dir"), (root, "")):
            try:
                rel = resolved.relative_to(base)
            except ValueError:
                continue
            rel_text = rel.as_posix()
            if rel_text == ".":
                return prefix or "."
            return f"{prefix}/{rel_text}" if prefix else rel_text
        return "external_path"
    return safe_relative_text(raw_value) or "external_path"


print(path_ref(raw))
PY
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

provider_credential_state() {
  local provider="$1"
  local required_env
  required_env="$(required_env_csv "$provider")"
  if [[ -z "$required_env" ]]; then
    printf '%s' "unknown"
    return 0
  fi
  if provider_has_credentials "$provider"; then
    printf '%s' "configured_env"
  else
    printf '%s' "missing"
  fi
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
  local live_scope="all"
  local credential_state
  local credential_required_env
  credential_state="$(provider_credential_state "$provider")"
  credential_required_env="$(required_env_csv "$provider")"
  if [[ "$LIVE_SCOPE_ALL" -eq 1 ]]; then
    live_scope="all"
  elif [[ "${#LIVE_PROVIDERS[@]}" -gt 0 ]]; then
    if provider_in_live_scope "$provider"; then
      live_scope="included"
    else
      live_scope="excluded"
    fi
  fi
  python3 - "$path" "$provider" "$status" "$reason" "$run_dir" "$output_file" "$exit_code" "$CASE_FILE" "$live_scope" "$(live_scope_csv)" "$credential_state" "$credential_required_env" "$ROOT_DIR" "$OUT_DIR" <<'PY'
import json
import sys
from pathlib import Path, PurePosixPath

path = Path(sys.argv[1])
provider = sys.argv[2]
status = sys.argv[3]
reason = sys.argv[4]
run_dir = sys.argv[5]
output_file = sys.argv[6]
exit_code = int(sys.argv[7])
case_file = sys.argv[8]
live_scope = sys.argv[9]
live_scope_providers = [item for item in sys.argv[10].split(",") if item]
credential_state = sys.argv[11]
credential_required_env = [item for item in sys.argv[12].split(",") if item]
root = Path(sys.argv[13]).resolve()
out_dir = Path(sys.argv[14]).resolve()


def safe_relative_text(text: str) -> str | None:
    normalized = text.replace("\\", "/")
    if not normalized or normalized.startswith("/") or any(ch.isspace() for ch in normalized):
        return None
    posix_path = PurePosixPath(normalized)
    if any(part in {"", ".", ".."} for part in posix_path.parts):
        return None
    return posix_path.as_posix()


def path_ref(raw_value: str) -> str:
    if not raw_value:
        return ""
    raw_path = Path(raw_value)
    if raw_path.is_absolute():
        resolved = raw_path.resolve()
        for base, prefix in ((out_dir, "out_dir"), (root, "")):
            try:
                rel = resolved.relative_to(base)
            except ValueError:
                continue
            rel_text = rel.as_posix()
            if rel_text == ".":
                return prefix or "."
            return f"{prefix}/{rel_text}" if prefix else rel_text
        return "external_path"
    return safe_relative_text(raw_value) or "external_path"


payload = {
    "provider": provider,
    "status": status,
    "reason_code": reason,
    "live_scope": live_scope,
    "live_scope_providers": live_scope_providers,
    "credential_state": credential_state,
    "credential_required_env": credential_required_env,
    "run_dir": path_ref(run_dir),
    "output_file": path_ref(output_file),
    "exit_code": exit_code,
    "case_file": path_ref(case_file),
}
path.write_text(json.dumps(payload, ensure_ascii=False, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
PY
}

append_summary() {
  local metadata_file="$1"
  cat "$metadata_file" >> "${OUT_DIR}/provider_summary.jsonl"
}

classify_failure_reason() {
  local output_file="$1"
  python3 "${ROOT_DIR}/scripts/nl_tests/classify_provider_failure.py" --reason-only "$output_file"
}

classify_failure_status() {
  local output_file="$1"
  python3 "${ROOT_DIR}/scripts/nl_tests/classify_provider_failure.py" --status-only "$output_file"
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
    --live-provider)
      add_csv_live_providers "${2:-}"
      shift 2
      ;;
    --live-providers)
      add_csv_live_providers "${2:-}"
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

if [[ "$LIVE_SCOPE_SET" -eq 0 ]]; then
  add_csv_live_providers "$DEFAULT_LIVE_PROVIDERS"
fi

if [[ ! -f "$CASE_FILE" ]]; then
  echo "case file not found: $CASE_FILE" >&2
  exit 2
fi

mkdir -p "$OUT_DIR"
: > "${OUT_DIR}/provider_summary.jsonl"

python3 "${ROOT_DIR}/scripts/nl_tests/check_chinese_provider_smoke_matrix.py" \
  --self-test
python3 "${ROOT_DIR}/scripts/nl_tests/check_chinese_provider_smoke_summary.py" \
  --self-test

python3 "${ROOT_DIR}/scripts/nl_tests/check_chinese_provider_smoke_matrix.py" \
  --case-file "$CASE_FILE" \
  --json > "${OUT_DIR}/case_coverage.json"

echo "CHINESE_PROVIDER_SMOKE_MATRIX out_dir_ref=$(path_ref "$OUT_DIR")"
echo "case_file=$(path_ref "$CASE_FILE")"
echo "providers=${PROVIDERS[*]}"
echo "live_scope_providers=$(live_scope_csv)"
echo "dry_run=${DRY_RUN}"

matrix_status=0
for provider in "${PROVIDERS[@]}"; do
  provider_dir="${OUT_DIR}/${provider}"
  mkdir -p "$provider_dir"
  metadata_file="${provider_dir}/metadata.json"
  output_file="${provider_dir}/run.output.txt"
  required_env="$(required_env_csv "$provider")"
  if ! provider_in_live_scope "$provider"; then
    write_metadata "$metadata_file" "$provider" "skipped" "provider_not_in_live_scope" "" "$output_file" 0
    append_summary "$metadata_file"
    echo "CHINESE_PROVIDER_SMOKE_SKIP provider=${provider} reason_code=provider_not_in_live_scope"
    continue
  fi
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
    failure_reason="$(classify_failure_reason "$output_file")"
    failure_status="$(classify_failure_status "$output_file")"
    write_metadata "$metadata_file" "$provider" "$failure_status" "$failure_reason" "$provider_dir" "$output_file" "$run_status"
    echo "CHINESE_PROVIDER_SMOKE_FAIL provider=${provider} reason_code=${failure_reason} exit_code=${run_status}" >&2
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
status_counts = Counter(str(row.get("status") or "unknown") for row in rows)
reason_code_counts = Counter(str(row.get("reason_code") or "unknown") for row in rows)
credential_state_counts = Counter(str(row.get("credential_state") or "unknown") for row in rows)
live_scope_counts = Counter(str(row.get("live_scope") or "unknown") for row in rows)
payload = {
    "provider_count": len(rows),
    "status_counts": dict(sorted(status_counts.items())),
    "reason_code_counts": dict(sorted(reason_code_counts.items())),
    "credential_state_counts": dict(sorted(credential_state_counts.items())),
    "live_scope_counts": dict(sorted(live_scope_counts.items())),
    "providers": rows,
}
out_path.write_text(json.dumps(payload, ensure_ascii=False, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
PY

python3 "${ROOT_DIR}/scripts/nl_tests/check_chinese_provider_smoke_summary.py" \
  "${OUT_DIR}/matrix_summary.json"

exit "$matrix_status"

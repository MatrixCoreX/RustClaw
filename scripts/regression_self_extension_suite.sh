#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

WAIT_SECONDS="${WAIT_SECONDS:-120}"
KEEP_WORKSPACE="${KEEP_WORKSPACE:-0}"
CLAWD_BIN="${CLAWD_BIN:-}"
RUNTIME_ENV_FILE="${RUNTIME_ENV_FILE:-/home/guagua/runtime_env_filled.sh}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/regression_self_extension_suite.sh [options]

Options:
  --wait-seconds N       Max wait per inner regression (default: 120)
  --keep-workspace       Keep temporary isolated workspaces for inspection
  --clawd-bin PATH       Override clawd binary used by inner regressions
  --runtime-env-file P   Env file sourced by NL handoff smoke
  -h, --help             Show this help

This suite runs:
  1. local backend self-extension runtime-enable regression
  2. provider-backed natural-language self-extension handoff regression

If the NL handoff stage hits provider_unavailable, the suite reports SKIP for
that stage but still exits success as long as the local backend stage
passes.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --wait-seconds)
      WAIT_SECONDS="${2:-}"
      shift 2
      ;;
    --keep-workspace)
      KEEP_WORKSPACE=1
      shift
      ;;
    --clawd-bin)
      CLAWD_BIN="${2:-}"
      shift 2
      ;;
    --runtime-env-file)
      RUNTIME_ENV_FILE="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

runtime_cmd=(
  bash "${ROOT_DIR}/scripts/regression_self_extension_runtime_enable.sh"
  --wait-seconds "$WAIT_SECONDS"
)
nl_cmd=(
  bash "${ROOT_DIR}/scripts/regression_self_extension_nl_handoff.sh"
  --wait-seconds "$WAIT_SECONDS"
  --runtime-env-file "$RUNTIME_ENV_FILE"
)

if [[ "$KEEP_WORKSPACE" == "1" ]]; then
  runtime_cmd+=(--keep-workspace)
  nl_cmd+=(--keep-workspace)
fi
if [[ -n "$CLAWD_BIN" ]]; then
  runtime_cmd+=(--clawd-bin "$CLAWD_BIN")
  nl_cmd+=(--clawd-bin "$CLAWD_BIN")
fi

echo "== Stage 1/2: local self-extension runtime-enable =="
"${runtime_cmd[@]}"

echo
echo "== Stage 2/2: natural-language self-extension handoff =="
if "${nl_cmd[@]}"; then
  echo "PASS: natural-language self-extension handoff regression"
else
  status=$?
  if [[ "$status" == "2" ]]; then
    echo "SKIP: natural-language self-extension handoff regression (provider unavailable)"
  else
    exit "$status"
  fi
fi

echo
echo "PASS: self-extension regression suite finished"

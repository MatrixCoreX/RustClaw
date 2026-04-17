#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG_ROOT="${LOG_ROOT:-$ROOT_DIR/scripts/regression_logs/ops_closed_loop_deterministic}"
SKIP_CHECK=0

usage() {
  cat <<'EOF'
Usage:
  bash scripts/regression_ops_closed_loop_deterministic.sh [options]

Options:
  --log-root DIR   Override log root directory
  --skip-check     Skip final `cargo check -p clawd`
  -h, --help       Show this help

This deterministic suite covers the local closed-loop regression stack for
ops/repair flows:
  1. execution_recipe state transitions
  2. verifier ops_recipe plan guards and rewrites
  3. loop_control stop/continue behavior
  4. skill_execution validation-failure side effects
  5. optional cargo check
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --log-root)
      LOG_ROOT="${2:-}"
      shift 2
      ;;
    --skip-check)
      SKIP_CHECK=1
      shift
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

RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
RUN_LOG="${RUN_DIR}/run.log"
mkdir -p "$RUN_DIR"

exec > >(tee -a "$RUN_LOG") 2>&1

run_case() {
  local label="$1"
  shift
  echo
  echo "== ${label} =="
  "$@"
}

echo "Deterministic ops_closed_loop regression"
echo "  run_dir: ${RUN_DIR}"
echo "  run_log: ${RUN_LOG}"

cd "$ROOT_DIR"

run_case \
  "execution_recipe deterministic state machine" \
  cargo test -p clawd execution_recipe::tests:: -- --nocapture

run_case \
  "verifier ops_recipe guards and rewrites" \
  cargo test -p clawd verifier::tests::ops_recipe_ -- --nocapture

run_case \
  "loop_control stop and continue behavior" \
  cargo test -p clawd agent_engine::loop_control::tests:: -- --nocapture

run_case \
  "skill_execution validation failure side effects" \
  cargo test -p clawd agent_engine::skill_execution::tests:: -- --nocapture

if [[ "$SKIP_CHECK" != "1" ]]; then
  run_case \
    "cargo check clawd" \
    cargo check -p clawd
fi

echo
echo "PASS: deterministic ops_closed_loop regression finished"
echo "Artifacts:"
echo "  - ${RUN_DIR}"
echo "  - ${RUN_LOG}"

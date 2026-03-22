#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_ROOT="${SCRIPT_DIR}/nl_suite_logs/all"
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
RUN_LOG="${RUN_DIR}/run.log"
SUMMARY_FILE="${RUN_DIR}/artifacts.txt"

mkdir -p "$RUN_DIR"
touch "$SUMMARY_FILE"
exec > >(tee -a "$RUN_LOG") 2>&1

echo "NL suite: all"
echo "  run_dir:  $RUN_DIR"
echo "  run_log:  $RUN_LOG"
echo

record_artifact() {
  local name="$1"
  local path="$2"
  printf '%s\t%s\n' "$name" "$path" >> "$SUMMARY_FILE"
}

run_suite() {
  local name="$1"
  local script_path="$2"
  local before after latest

  echo "============================================================"
  echo "[SUITE] $name"
  echo "[SCRIPT] $script_path"

  before="$(
    find "${SCRIPT_DIR}/nl_suite_logs/${name}" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort || true
  )"

  bash "$script_path"

  after="$(
    find "${SCRIPT_DIR}/nl_suite_logs/${name}" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort || true
  )"

  latest="$(
    comm -13 <(printf '%s\n' "$before") <(printf '%s\n' "$after") | tail -n 1
  )"
  if [[ -z "$latest" ]]; then
    latest="$(
      find "${SCRIPT_DIR}/nl_suite_logs/${name}" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort | tail -n 1
    )"
  fi

  if [[ -n "$latest" ]]; then
    echo "[ARTIFACT] $latest"
    record_artifact "$name" "$latest"
  else
    echo "[ARTIFACT] <not found>"
    record_artifact "$name" "<not found>"
  fi
  echo
}

run_suite "manual" "${SCRIPT_DIR}/run_nl_suite_manual.sh"
run_suite "text_match" "${SCRIPT_DIR}/run_nl_suite_text_match.sh"
run_suite "full" "${SCRIPT_DIR}/run_nl_suite_full.sh"
run_suite "trace" "${SCRIPT_DIR}/run_nl_suite_trace.sh"
run_suite "resume" "${SCRIPT_DIR}/run_nl_suite_resume.sh"
run_suite "clarify" "${SCRIPT_DIR}/run_nl_suite_clarify.sh"

echo "Artifacts:"
echo "  - $RUN_DIR"
echo "  - $RUN_LOG"
echo "  - $SUMMARY_FILE"
while IFS=$'\t' read -r name path; do
  [[ -n "${name:-}" ]] || continue
  echo "  - ${name}: ${path}"
done < "$SUMMARY_FILE"

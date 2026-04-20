#!/usr/bin/env bash
# Phase 7 §7.5 Step 3: Regenerate a fixture case from a model_io.log slice.
#
# Wraps `cargo test fixture_replay_e2e::tests::regen_fixture_tool` so the
# operator does not have to remember the env-var contract every time.
#
# Usage:
#   scripts/regen_fixture.sh <case_name> <log_file> [--force] [--dry-run]
#
# Examples:
#   # First-time recording of act_find_service_file:
#   scripts/regen_fixture.sh act_find_service_file /tmp/log.jsonl
#
#   # Re-record after a prompt template change (requires --force):
#   scripts/regen_fixture.sh act_find_service_file /tmp/log.jsonl --force
#
#   # Just inspect what convert_* would produce, no disk write:
#   scripts/regen_fixture.sh act_find_service_file /tmp/log.jsonl --dry-run
#
# Recording prerequisites (do these BEFORE invoking this script):
#   1. Set `[routing] debug_log_prompt = true` in your clawd config.
#   2. Pin the prompt-`__NOW__` field by setting
#      `RUSTCLAW_TEST_FREEZE_NOW=2026-04-19T12:00:00+08:00` in the clawd
#      worker environment when triggering the case (any value is fine, but
#      the SAME value must be used at replay time, otherwise the
#      intent_normalizer prompt will hash differently).
#   3. Trigger the case (telegram message / scripts/nl_tests/run_manual_test.sh
#      / direct HTTP) and wait for completion.
#   4. From the workspace's `logs/model_io.log`, grep the verbose lines for
#      this task (use the task_id printed in clawd stdout):
#        rg -F '"task_id":"<task_id>"' logs/model_io.log > /tmp/log.jsonl
#   5. Run THIS script.
#
# After this script succeeds:
#   * `crates/clawd/tests/fixtures/llm_io/<case>/calls.jsonl` is created/
#     overwritten.
#   * Inspect with `git diff` and commit alongside the §7.5 test that
#     consumes it.
#   * If you were testing the smoke skeleton, remove the `#[ignore]` from
#     `e2e_<case>_replay_smoke` in `fixture_replay_e2e.rs`.

set -euo pipefail

usage() {
  sed -n '2,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit "${1:-1}"
}

if [[ $# -lt 2 ]]; then
  usage 1
fi

CASE="$1"; shift
LOG="$1"; shift

FORCE=""
DRY=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)   FORCE=1 ;;
    --dry-run) DRY=1 ;;
    -h|--help) usage 0 ;;
    *)
      echo "regen_fixture.sh: unknown arg '$1'" >&2
      usage 1
      ;;
  esac
  shift
done

if [[ ! -f "$LOG" ]]; then
  echo "regen_fixture.sh: log file not found: $LOG" >&2
  exit 2
fi
LOG_ABS="$(cd "$(dirname "$LOG")" && pwd)/$(basename "$LOG")"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "regen_fixture.sh: case=$CASE log=$LOG_ABS force=${FORCE:-0} dry_run=${DRY:-0}"

cd "$REPO_ROOT"
RUSTCLAW_REGEN_FIXTURE_CASE="$CASE" \
RUSTCLAW_REGEN_FIXTURE_LOG="$LOG_ABS" \
RUSTCLAW_REGEN_FIXTURE_FORCE="${FORCE:-}" \
RUSTCLAW_REGEN_FIXTURE_DRY="${DRY:-}" \
cargo test \
  -p clawd --bin clawd \
  fixture_replay_e2e::tests::regen_fixture_tool \
  -- --ignored --nocapture --test-threads=1

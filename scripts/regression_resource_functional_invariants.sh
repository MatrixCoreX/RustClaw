#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

run_case() {
  local label="$1"
  local filter="$2"

  printf '[resource-invariant] %s\n' "$label"
  cargo test -p clawd "$filter" -- --exact
}

printf '[resource-invariant] artifact projection and delivery\n'
cargo test -p clawd task_artifacts::tests

printf '[resource-invariant] channel delivery receipts\n'
cargo test -p clawd repo::channel_delivery_receipt::tests

run_case \
  'pinned generation resolves its original receipt' \
  'skill_admission::tests::generation_validation_resolves_its_pinned_receipt_after_current_install_changes'
run_case \
  'runner requires the exact execution binding' \
  'skills::runner::tests::successful_runner_results_require_the_exact_execution_binding'
run_case \
  'poll dispatch preserves the pinned execution binding' \
  'skills::runner::tests::pinned_poll_binding_is_normalized_for_exact_runner_dispatch'
run_case \
  'policy approval is bound to the registry generation' \
  'approval_grant::tests::approval_binding_changes_when_registry_generation_changes'

printf '[resource-invariant] all fixed comparisons passed\n'

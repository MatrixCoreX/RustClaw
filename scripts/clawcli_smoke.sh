#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLAWCLI_BIN="${CLAWCLI_BIN:-$ROOT/target/debug/clawcli}"
BASE_URL="${RUSTCLAW_BASE_URL:-http://127.0.0.1:8787}"
KEY="${RUSTCLAW_CLI_SMOKE_KEY:-${RUSTCLAW_ADMIN_KEY:-}}"
SMOKE_TEXT="${RUSTCLAW_CLI_SMOKE_TEXT:-hello}"
WATCH_TIMEOUT_SECONDS="${RUSTCLAW_CLI_SMOKE_WATCH_TIMEOUT_SECONDS:-120}"
REQUIRE_CAPABILITIES="${RUSTCLAW_CLI_SMOKE_REQUIRE_CAPABILITIES:-0}"

if [[ ! -x "$CLAWCLI_BIN" ]]; then
  echo "clawcli binary not found or not executable: $CLAWCLI_BIN" >&2
  echo "Run: cargo build -p clawcli" >&2
  exit 2
fi

CLI_BASE=("$CLAWCLI_BIN" --base-url "$BASE_URL")
if [[ -n "$KEY" ]]; then
  CLI_BASE+=(--key "$KEY")
fi

run_cli() {
  "${CLI_BASE[@]}" "$@"
}

extract_task_id() {
  python3 -c 'import json,sys; print(json.load(sys.stdin)["task_id"])'
}

echo "SMOKE health"
run_cli health >/dev/null

echo "SMOKE skills"
run_cli skills --json >/dev/null

echo "SMOKE capabilities"
capabilities_error="$(mktemp)"
if ! run_cli capabilities --json >/dev/null 2>"$capabilities_error"; then
  if [[ "$REQUIRE_CAPABILITIES" == "1" ]]; then
    cat "$capabilities_error" >&2
    rm -f "$capabilities_error"
    exit 1
  fi
  echo "SMOKE capabilities skipped endpoint_unavailable" >&2
fi
rm -f "$capabilities_error"

echo "SMOKE exec-effective-config"
run_cli exec --profile quick --print-effective-config "smoke effective config" >/dev/null

echo "SMOKE submit"
submit_json="$(run_cli submit --text "$SMOKE_TEXT" --detach --json)"
task_id="$(printf '%s\n' "$submit_json" | extract_task_id)"
if [[ -z "$task_id" ]]; then
  echo "submit did not return task_id" >&2
  exit 1
fi
echo "SMOKE task_id=$task_id"

echo "SMOKE get"
run_cli get "$task_id" >/dev/null

echo "SMOKE events"
run_cli events "$task_id" --jsonl >/dev/null

echo "SMOKE watch"
timeout "$WATCH_TIMEOUT_SECONDS" \
  "${CLI_BASE[@]}" watch "$task_id" --until-terminal --jsonl >/dev/null

echo "SMOKE wait"
run_cli wait "$task_id" --until terminal --timeout-seconds "$WATCH_TIMEOUT_SECONDS" --json >/dev/null

echo "SMOKE review"
run_cli review "$task_id" --json >/dev/null

echo "SMOKE subagents"
run_cli subagents "$task_id" --json >/dev/null

echo "SMOKE permission inspect"
run_cli permission inspect "$task_id" --json >/dev/null

echo "SMOKE replay"
replay_file="$(mktemp)"
run_cli replay export "$task_id" --output "$replay_file" --json >/dev/null
run_cli replay run "$replay_file" --coverage >/dev/null
rm -f "$replay_file"

if [[ -n "${RUSTCLAW_CLI_SMOKE_USER_ID:-}" && -n "${RUSTCLAW_CLI_SMOKE_CHAT_ID:-}" ]]; then
  echo "SMOKE active"
  run_cli active \
    --user-id "$RUSTCLAW_CLI_SMOKE_USER_ID" \
    --chat-id "$RUSTCLAW_CLI_SMOKE_CHAT_ID" \
    --json >/dev/null
  echo "SMOKE tui"
  run_cli tui \
    --user-id "$RUSTCLAW_CLI_SMOKE_USER_ID" \
    --chat-id "$RUSTCLAW_CLI_SMOKE_CHAT_ID" \
    --task-id "$task_id" \
    --once \
    --json >/dev/null
fi

if [[ -n "${RUSTCLAW_CLI_SMOKE_CANCEL_TASK_ID:-}" ]]; then
  echo "SMOKE cancel-task"
  run_cli cancel-task "$RUSTCLAW_CLI_SMOKE_CANCEL_TASK_ID" >/dev/null
fi

if [[ -n "${RUSTCLAW_CLI_SMOKE_RESUME_TASK_ID:-}" ]]; then
  echo "SMOKE resume-task"
  run_cli resume-task "$RUSTCLAW_CLI_SMOKE_RESUME_TASK_ID" >/dev/null
fi

if [[ -n "${RUSTCLAW_CLI_SMOKE_PAUSE_TASK_ID:-}" ]]; then
  echo "SMOKE pause-task"
  run_cli pause-task \
    "$RUSTCLAW_CLI_SMOKE_PAUSE_TASK_ID" \
    --pause-seconds "${RUSTCLAW_CLI_SMOKE_PAUSE_SECONDS:-3600}" >/dev/null
fi

if [[ -n "${RUSTCLAW_CLI_SMOKE_RUN_SKILL:-}" ]]; then
  echo "SMOKE run-skill"
  run_cli run-skill \
    "$RUSTCLAW_CLI_SMOKE_RUN_SKILL" \
    --args-json "${RUSTCLAW_CLI_SMOKE_RUN_SKILL_ARGS_JSON:-{}}" \
    --wait \
    --json >/dev/null
fi

echo "SMOKE ok"

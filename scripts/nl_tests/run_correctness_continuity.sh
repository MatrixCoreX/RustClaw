#!/usr/bin/env bash
set -euo pipefail
umask 077

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/scripts/shell_compat.sh"
configure_platform_command_path
BINARY="${1:-$ROOT/target/debug/clawd}"
SKILL_BINARY_DIR="${2:-$ROOT/target/debug}"
CASE_FILE="${NL_CASE_FILE:-scripts/nl_tests/cases/nl_cases_correctness_continuity_20261027.txt}"
if [[ -n "${NL_CASE_NAME:-}" ]]; then
  set -- --case-name "$NL_CASE_NAME"
else
  set --
fi
[[ -x "$BINARY" && -x "$SKILL_BINARY_DIR/task-control-skill" ]] || {
  echo 'Build clawd and task-control-skill before running this isolated suite.' >&2
  exit 1
}
cd "$ROOT"
mkdir -p "$ROOT/tmp"
STATE="$(mktemp -d "$ROOT/tmp/agent-runtime-nl-isolated-continuity-XXXXXX")"
echo "isolated_test_state=$STATE"
WORKSPACE="$STATE/workspace"
mkdir -p "$WORKSPACE" "$WORKSPACE/logs" "$WORKSPACE/data/skills"
git ls-files -z --cached --others --exclude-standard configs prompts crates/skills optional_skills README.md Cargo.toml Cargo.lock \
  | tar --null -T - -cf - | tar -C "$WORKSPACE" -xf -
ln -s "$ROOT/target" "$WORKSPACE/target"
python3 scripts/nl_tests/create_isolated_config.py --source configs/config.toml \
  --output "$STATE/config.toml" --sqlite-path "$STATE/tasks.sqlite" \
  --audit-sqlite-path "$STATE/audit.sqlite" --skill-data-root "$WORKSPACE/data/skills"
python3 scripts/project_skill_receipts.py --skill fs_search --skill system_basic \
  --package-root "$WORKSPACE/data/skill-packages"
mkdir -p "$STATE/bin"
cp "$SKILL_BINARY_DIR/task-control-skill" "$STATE/bin/task-control-skill"
python3 scripts/project_skill_receipts.py --skill task_control --binary-dir "$STATE/bin" \
  --package-root "$WORKSPACE/data/skill-packages"
if [[ -f "$ROOT/../runtime_env_filled.sh" ]]; then
  source "$ROOT/../runtime_env_filled.sh"
fi
export APP_CONFIG_PATH="$STATE/config.toml" APP_DB_PATH="$STATE/tasks.sqlite"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
export APP_INTERNAL_LISTEN="127.0.0.1:$PORT"
export NL_MODEL_IO_LOG="$WORKSPACE/logs/model_io.log" NL_RUNTIME_TRACE_LOG="$STATE/server.log"
export NL_CLAWD_BIN="$BINARY" NL_ISOLATED_WORKSPACE="$WORKSPACE" CLIENT_LIKE_CHANNEL=ui
export NL_TRACE_FINALIZE_WAIT_SECONDS=5
SERVER_PID=''
cleanup() {
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  echo "retained_test_state=$STATE"
}
trap cleanup EXIT
(
  cd "$WORKSPACE"
  exec "$BINARY" --config "$STATE/config.toml"
) >"$STATE/server.log" 2>&1 &
SERVER_PID=$!
export USER_KEY='' APP_USER_KEY=''
READY=0
for ((attempt=0; attempt<120; attempt++)); do
  kill -0 "$SERVER_PID" 2>/dev/null || { echo "Isolated server stopped: $STATE/server.log" >&2; exit 1; }
  if [[ -z "$USER_KEY" ]]; then
    AUTH_OUTPUT="$(bash scripts/auth-key.sh list 2>/dev/null || true)"
    USER_KEY="$(awk '$2 == "admin" && $3 == "enabled" {print $1; exit}' <<<"$AUTH_OUTPUT")"
    export APP_USER_KEY="$USER_KEY"
  fi
  if [[ -n "$USER_KEY" ]] && curl -fsS --max-time 2 -H "X-Agent-Key: $USER_KEY" \
      "http://127.0.0.1:$PORT/v1/health" >/dev/null; then
    READY=1
    break
  fi
  sleep 1
done
[[ "$READY" == 1 ]] || { echo "Isolated health timeout: $STATE/server.log" >&2; exit 1; }
echo "run_id=correctness-continuity-$PORT suite=correctness-continuity provider=selected_config model=selected_config dry_run=false isolation=$STATE"
bash scripts/nl_tests/run_manual_test.sh --base-url "http://127.0.0.1:$PORT" \
  --case-file "$CASE_FILE" \
  "$@" \
  --log-root "$STATE/results" --wait-seconds 900 --provider-retries 0 --fail-fast 1 --full-text

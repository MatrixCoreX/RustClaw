#!/usr/bin/env bash
set -euo pipefail
umask 077

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/scripts/shell_compat.sh"
configure_platform_command_path
BINARY="${1:-$ROOT/target/debug/clawd}"
SKILL_BINARY_DIR="${2:-$ROOT/target/debug}"
if [[ $# == 0 ]]; then
  echo 'Preparing current local binaries with separate exact Cargo targets...'
  (
    cd "$ROOT"
    export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
    cargo build -p clawd --bin clawd
    cargo build -p task-control-skill --bin task-control-skill
  )
fi
CASE_FILE="${NL_CASE_FILE:-scripts/nl_tests/cases/nl_cases_correctness_continuity_20261027.txt}"
WAIT_SECONDS="${NL_WAIT_SECONDS:-1800}"
[[ "$WAIT_SECONDS" =~ ^[1-9][0-9]*$ ]] || { echo 'NL_WAIT_SECONDS must be a positive integer.' >&2; exit 1; }
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
SNAPSHOT_ROOT="${NL_BINARY_SNAPSHOT_ROOT:-$ROOT/target/nl-runtime-snapshots}"
mkdir -p "$SNAPSHOT_ROOT"
python3 scripts/nl_tests/check_workspace_space.py --binary "$BINARY" \
  --state-root "$ROOT/tmp" --snapshot-root "$SNAPSHOT_ROOT"
STATE="$(mktemp -d "$ROOT/tmp/agent-runtime-nl-isolated-continuity-XXXXXX")"
echo "isolated_test_state=$STATE"
SNAPSHOT="$(mktemp -d "$SNAPSHOT_ROOT/run-XXXXXX")/clawd"
cp -p "$BINARY" "$SNAPSHOT"
python3 - "$BINARY" "$SNAPSHOT" "$STATE/binary-evidence.json" <<'PY'
import hashlib,json,sys
from pathlib import Path
source,snapshot,evidence=map(Path,sys.argv[1:])
with snapshot.open('rb') as stream:
    digest=hashlib.file_digest(stream,'sha256').hexdigest()
with source.open('rb') as stream:
    if hashlib.file_digest(stream,'sha256').hexdigest() != digest:
        raise SystemExit('test_binary_changed_during_snapshot')
evidence.write_text(json.dumps({'source_binary':str(source),'pinned_binary':str(snapshot),
    'sha256':digest,'size_bytes':snapshot.stat().st_size})+'\n')
print('pinned_runtime_binary='+str(snapshot)+' sha256='+digest)
PY
BINARY="$SNAPSHOT"
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
start_server() {
  python3 - "$STATE/binary-evidence.json" <<'PY'
import hashlib,json,sys
from pathlib import Path
evidence=json.loads(Path(sys.argv[1]).read_text())
with Path(evidence['pinned_binary']).open('rb') as stream:
    if hashlib.file_digest(stream,'sha256').hexdigest() != evidence['sha256']:
        raise SystemExit('test_binary_snapshot_digest_changed')
PY
  (
    cd "$WORKSPACE"
    exec "$BINARY" --config "$STATE/config.toml"
  ) >>"$STATE/server.log" 2>&1 &
  SERVER_PID=$!
}
start_server
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
if [[ -n "${NL_CASE_SEQUENCE:-}" ]]; then
  IFS=',' read -r -a sequence <<< "$NL_CASE_SEQUENCE"
  position=0
  for case_name in "${sequence[@]}"; do
    position=$((position + 1))
    bash scripts/nl_tests/run_manual_test.sh --base-url "http://127.0.0.1:$PORT" \
      --case-file "$CASE_FILE" --case-name "$case_name" --reuse-chat-id-base --chat-id "$PORT" \
      --log-root "$STATE/results" --wait-seconds "$WAIT_SECONDS" --provider-retries 0 --fail-fast 1 --full-text
    if [[ "$position" == "${NL_RESTART_AFTER_CASE:-0}" && "$position" -lt "${#sequence[@]}" ]]; then
      old_pid="$SERVER_PID"
      kill "$SERVER_PID"
      wait "$SERVER_PID" || true
      start_server
      echo "test_runtime_restart=completed after_case=$case_name old_pid=$old_pid new_pid=$SERVER_PID"
      python3 -c 'import json,sys; print(json.dumps({"after_case":sys.argv[1],"old_pid":int(sys.argv[2]),"new_pid":int(sys.argv[3])}))' \
        "$case_name" "$old_pid" "$SERVER_PID" > "$STATE/restart-evidence.json"
      ready=0
      for ((attempt=0; attempt<120; attempt++)); do
        kill -0 "$SERVER_PID" 2>/dev/null || { echo 'Restarted isolated server stopped' >&2; exit 1; }
        if curl -fsS --max-time 2 -H "X-Agent-Key: $USER_KEY" "http://127.0.0.1:$PORT/v1/health" >/dev/null; then
          ready=1
          break
        fi
        sleep 1
      done
      [[ "$ready" == 1 ]] || { echo 'Restarted isolated server health timeout' >&2; exit 1; }
    fi
  done
else
  bash scripts/nl_tests/run_manual_test.sh --base-url "http://127.0.0.1:$PORT" \
    --case-file "$CASE_FILE" "$@" \
    --log-root "$STATE/results" --wait-seconds "$WAIT_SECONDS" --provider-retries 0 --fail-fast 1 --full-text
fi

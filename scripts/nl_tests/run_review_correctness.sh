#!/usr/bin/env bash
set -euo pipefail
umask 077

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BINARY="${1:-${ROOT}/target/debug/clawd}"
[[ -x "$BINARY" ]] || { echo "Compiled server binary missing" >&2; exit 1; }
cd "$ROOT"
mkdir -p "${ROOT}/tmp"
STATE="$(mktemp -d "${ROOT}/tmp/review-correctness-XXXXXX")"
WORKSPACE="${STATE}/workspace"
mkdir -p "$WORKSPACE" "$WORKSPACE/data/skills" "$WORKSPACE/logs"
# A small source snapshot excludes unrelated untracked workspaces and all caches.
git ls-files -z --cached --others --exclude-standard configs prompts crates/skills optional_skills README.md Cargo.toml \
  | tar --null -T - -cf - | tar -C "$WORKSPACE" -xf -
ln -s "$ROOT/target" "$WORKSPACE/target"
python3 scripts/nl_tests/create_isolated_config.py --source configs/config.toml \
  --output "$STATE/config.toml" --sqlite-path "$STATE/tasks.sqlite" \
  --audit-sqlite-path "$STATE/audit.sqlite" --skill-data-root "$WORKSPACE/data/skills"
python3 scripts/project_skill_receipts.py --skill fs_basic --skill system_basic \
  --package-root "$WORKSPACE/data/skill-packages"
if [[ -f "${ROOT}/../runtime_env_filled.sh" ]]; then
  source "${ROOT}/../runtime_env_filled.sh"
fi
export APP_CONFIG_PATH="$STATE/config.toml" APP_DB_PATH="$STATE/tasks.sqlite"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
export APP_INTERNAL_LISTEN="127.0.0.1:$PORT"
export NL_MODEL_IO_LOG="$WORKSPACE/logs/model_io.log"
export NL_RUNTIME_TRACE_LOG="$STATE/server.log"
export NL_CLAWD_BIN="$BINARY"
SERVER_PID=""
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
export USER_KEY="" APP_USER_KEY=""
READY=0
for ((attempt=0; attempt<120; attempt++)); do
  kill -0 "$SERVER_PID" 2>/dev/null || { echo "Isolated server exited; see $STATE/server.log" >&2; exit 1; }
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
[[ "$READY" == 1 ]] || { echo "Isolated server did not become ready" >&2; exit 1; }
export NL_ISOLATED_WORKSPACE="$WORKSPACE" CLIENT_LIKE_CHANNEL=ui
bash scripts/nl_tests/run_manual_test.sh \
  --base-url "http://127.0.0.1:$PORT" \
  --case-file scripts/nl_tests/cases/nl_cases_review_correctness_20261023.txt \
  --log-root "$STATE/results" --wait-seconds 600 --provider-retries 0 --fail-fast 1 --full-text

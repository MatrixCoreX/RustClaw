#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

CASE_FILE="${SCRIPT_DIR}/nl_manual_cases.txt"
BASE_URL_VALUE="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID_VALUE="${USER_ID:-1985996990}"
CHAT_ID_VALUE="${CHAT_ID:-1985996990}"
USER_KEY_VALUE="${RUSTCLAW_USER_KEY:-${USER_KEY:-}}"
WAIT_SECONDS_VALUE="${MAX_WAIT_SECONDS:-180}"
POLL_SECONDS_VALUE="${POLL_INTERVAL_SECONDS:-1}"
FULL_TEXT=0
EXTRA_ARGS=()
SUITE_ARGS=()
LOCK_DIR="/tmp/rustclaw_nl_manual_test.lock"
CHILD_PID=""

usage() {
  cat <<'EOF'
Usage:
  bash scripts/run_nl_manual_test.sh [options] [-- extra args for regression_user_instruction.sh]

Options:
  --case-file PATH      Case file to run. Default: scripts/nl_manual_cases.txt
  --base-url URL        clawd base url. Default: http://127.0.0.1:8787
  --user-id ID          User id for submit
  --chat-id ID          Base chat id for submit
  --user-key KEY        RustClaw user key
  --wait-seconds N      Max wait seconds per case
  --poll-seconds N      Poll interval seconds
  --full-text           Print full response text
  -h, --help            Show this help

Case format:
  suite|name|tags|prompt

Typical flow:
  1. Edit scripts/nl_manual_cases.txt
  2. Run: bash scripts/run_nl_manual_test.sh --user-key <your-key>
  3. Send me the printed log directory, or directly send:
     - logs/regression_user_instruction_<timestamp>/run.log
     - logs/regression_user_instruction_<timestamp>/results.jsonl
     - logs/regression_user_instruction_<timestamp>/unresolved.log
EOF
}

cleanup() {
  local exit_code=$?
  if [[ -n "${CHILD_PID:-}" ]] && kill -0 "$CHILD_PID" >/dev/null 2>&1; then
    kill "$CHILD_PID" >/dev/null 2>&1 || true
    wait "$CHILD_PID" >/dev/null 2>&1 || true
  fi
  if [[ -d "$LOCK_DIR" ]]; then
    rm -rf "$LOCK_DIR"
  fi
  exit "$exit_code"
}

acquire_lock() {
  if mkdir "$LOCK_DIR" 2>/dev/null; then
    printf '%s\n' "$$" > "${LOCK_DIR}/pid"
    return 0
  fi

  local existing_pid=""
  if [[ -f "${LOCK_DIR}/pid" ]]; then
    existing_pid="$(tr -d ' \t\r\n' < "${LOCK_DIR}/pid" 2>/dev/null || true)"
  fi
  if [[ -n "$existing_pid" ]] && kill -0 "$existing_pid" >/dev/null 2>&1; then
    echo "Another manual NL regression run is already active (pid=${existing_pid})." >&2
    exit 2
  fi

  rm -rf "$LOCK_DIR"
  mkdir "$LOCK_DIR"
  printf '%s\n' "$$" > "${LOCK_DIR}/pid"
}

build_suite_args_from_case_file() {
  local case_file="$1"
  mapfile -t SUITE_ARGS < <(
    python3 - "$case_file" <<'PY'
import sys
from pathlib import Path

seen = set()
for raw in Path(sys.argv[1]).read_text(encoding="utf-8").splitlines():
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = [part.strip() for part in line.split("|", 3)]
    if len(parts) != 4:
        continue
    suite = parts[0]
    if suite and suite not in seen:
        seen.add(suite)
        print(suite)
PY
  )
}

trap cleanup EXIT INT TERM

while [[ $# -gt 0 ]]; do
  case "$1" in
    --case-file)
      CASE_FILE="$2"
      shift 2
      ;;
    --base-url)
      BASE_URL_VALUE="$2"
      shift 2
      ;;
    --user-id)
      USER_ID_VALUE="$2"
      shift 2
      ;;
    --chat-id)
      CHAT_ID_VALUE="$2"
      shift 2
      ;;
    --user-key)
      USER_KEY_VALUE="$2"
      shift 2
      ;;
    --wait-seconds)
      WAIT_SECONDS_VALUE="$2"
      shift 2
      ;;
    --poll-seconds)
      POLL_SECONDS_VALUE="$2"
      shift 2
      ;;
    --full-text)
      FULL_TEXT=1
      shift
      ;;
    --)
      shift
      EXTRA_ARGS+=("$@")
      break
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

if [[ ! -f "$CASE_FILE" ]]; then
  echo "Case file not found: $CASE_FILE" >&2
  exit 2
fi

acquire_lock
build_suite_args_from_case_file "$CASE_FILE"

echo "Natural-language manual regression"
echo "  case_file: $CASE_FILE"
echo "  base_url:  $BASE_URL_VALUE"
echo "  user_id:   $USER_ID_VALUE"
echo "  chat_id:   $CHAT_ID_VALUE"
echo "  user_key:  ${USER_KEY_VALUE:-<auto-detect admin key>}"
echo "  wait:      ${WAIT_SECONDS_VALUE}s"
echo "  poll:      ${POLL_SECONDS_VALUE}s"
if [[ "${#SUITE_ARGS[@]}" -gt 0 ]]; then
  echo "  suites:    ${SUITE_ARGS[*]}"
fi
echo
echo "Running existing regression engine with your case file..."
echo

CMD=(
  bash "${ROOT_DIR}/scripts/regression_user_instruction.sh"
  --no-defaults
  --case-file "$CASE_FILE"
  --base-url "$BASE_URL_VALUE"
  --user-id "$USER_ID_VALUE"
  --chat-id "$CHAT_ID_VALUE"
  --wait-seconds "$WAIT_SECONDS_VALUE"
  --poll-seconds "$POLL_SECONDS_VALUE"
)

if [[ -n "$USER_KEY_VALUE" ]]; then
  CMD+=(--user-key "$USER_KEY_VALUE")
fi

if [[ "$FULL_TEXT" -eq 1 ]]; then
  CMD+=(--full-text)
fi

if [[ "${#EXTRA_ARGS[@]}" -gt 0 ]]; then
  CMD+=("${EXTRA_ARGS[@]}")
fi

if [[ "${#SUITE_ARGS[@]}" -gt 0 ]]; then
  for suite in "${SUITE_ARGS[@]}"; do
    CMD+=(--suite "$suite")
  done
fi

"${CMD[@]}" &
CHILD_PID=$!
wait "$CHILD_PID"
CHILD_PID=""

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

CASE_FILE="${SCRIPT_DIR}/nl_full_suite_cases.txt"
TRACE_CASE_FILE="${SCRIPT_DIR}/nl_full_suite_trace_cases.txt"
LOG_ROOT="${SCRIPT_DIR}/nl_full_suite_logs"
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
RUN_LOG="${RUN_DIR}/run.log"
BASE_URL_VALUE="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID_VALUE="${USER_ID:-1985996990}"
CHAT_ID_VALUE="${CHAT_ID:-1985996990}"
USER_KEY_VALUE="${RUSTCLAW_USER_KEY:-${USER_KEY:-}}"
WAIT_SECONDS_VALUE="${MAX_WAIT_SECONDS:-180}"
POLL_SECONDS_VALUE="${POLL_INTERVAL_SECONDS:-1}"
WITH_TRACE=0
WITH_RESUME=0
FULL_TEXT=0
RESUME_DIR=""
RESUME_LINE=""

usage() {
  cat <<'EOF'
Usage:
  bash scripts/run_nl_full_suite.sh [options]

Default:
  Run the comprehensive natural-language instruction suite using run_nl_manual_test.sh.

Options:
  --case-file PATH      Main NL case file. Default: scripts/nl_full_suite_cases.txt
  --trace-case-file P   Trace case file. Default: scripts/nl_full_suite_trace_cases.txt
  --log-root PATH       Root log dir. Default: scripts/nl_full_suite_logs
  --base-url URL        clawd base url
  --user-id ID          user id for submits
  --chat-id ID          base chat id for submits
  --user-key KEY        RustClaw user key
  --wait-seconds N      max wait per case
  --poll-seconds N      poll interval
  --full-text           pass through to child NL runner
  --resume-dir PATH     existing child run dir for the main NL runner
  --resume-line N       continue after this tested source line in the main case file
  --with-trace          additionally run regression_trace_ask.sh on focused trace cases
  --with-resume         additionally run regression_resume_continue.sh
  -h, --help            show this help

Artifacts:
  scripts/nl_full_suite_logs/<timestamp>/
    run.log
    simple_nl.log
    trace_ask.log          (if --with-trace)
    resume_continue.log    (if --with-resume)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --case-file)
      CASE_FILE="$2"
      shift 2
      ;;
    --trace-case-file)
      TRACE_CASE_FILE="$2"
      shift 2
      ;;
    --log-root)
      LOG_ROOT="$2"
      RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
      RUN_LOG="${RUN_DIR}/run.log"
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
    --resume-dir)
      RESUME_DIR="$2"
      shift 2
      ;;
    --resume-line)
      RESUME_LINE="$2"
      shift 2
      ;;
    --with-trace)
      WITH_TRACE=1
      shift
      ;;
    --with-resume)
      WITH_RESUME=1
      shift
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
  echo "Main case file not found: $CASE_FILE" >&2
  exit 2
fi

if [[ "$WITH_TRACE" -eq 1 ]] && [[ ! -f "$TRACE_CASE_FILE" ]]; then
  echo "Trace case file not found: $TRACE_CASE_FILE" >&2
  exit 2
fi

mkdir -p "$RUN_DIR"
exec > >(tee -a "$RUN_LOG") 2>&1

echo "NL full regression suite"
echo "  run_dir:          $RUN_DIR"
echo "  case_file:        $CASE_FILE"
echo "  trace_case_file:  $TRACE_CASE_FILE"
echo "  base_url:         $BASE_URL_VALUE"
echo "  user_id:          $USER_ID_VALUE"
echo "  chat_id:          $CHAT_ID_VALUE"
echo "  user_key:         ${USER_KEY_VALUE:-<auto-detect admin key>}"
echo "  wait:             ${WAIT_SECONDS_VALUE}s"
echo "  poll:             ${POLL_SECONDS_VALUE}s"
echo "  resume_dir:       ${RESUME_DIR:-<new run>}"
echo "  resume_line:      ${RESUME_LINE:-<none>}"
echo "  with_trace:       $WITH_TRACE"
echo "  with_resume:      $WITH_RESUME"
echo

simple_cmd=(
  bash "${SCRIPT_DIR}/run_nl_manual_test.sh"
  --case-file "$CASE_FILE"
  --base-url "$BASE_URL_VALUE"
  --user-id "$USER_ID_VALUE"
  --chat-id "$CHAT_ID_VALUE"
  --wait-seconds "$WAIT_SECONDS_VALUE"
  --poll-seconds "$POLL_SECONDS_VALUE"
  --log-root "${RUN_DIR}/simple_nl_outputs"
)
if [[ -n "$USER_KEY_VALUE" ]]; then
  simple_cmd+=(--user-key "$USER_KEY_VALUE")
fi
if [[ "$FULL_TEXT" -eq 1 ]]; then
  simple_cmd+=(--full-text)
fi
if [[ -n "$RESUME_DIR" ]]; then
  simple_cmd+=(--resume-dir "$RESUME_DIR")
fi
if [[ -n "$RESUME_LINE" ]]; then
  simple_cmd+=(--resume-line "$RESUME_LINE")
fi

echo "== Section 1/3: comprehensive NL cases =="
"${simple_cmd[@]}" | tee "${RUN_DIR}/simple_nl.log"

if [[ "$WITH_TRACE" -eq 1 ]]; then
  trace_cmd=(
    bash "${SCRIPT_DIR}/regression_trace_ask.sh"
    --no-defaults
    --case-file "$TRACE_CASE_FILE"
    --base-url "$BASE_URL_VALUE"
    --user-id "$USER_ID_VALUE"
    --chat-id "$((CHAT_ID_VALUE + 50000))"
    --wait-seconds "$WAIT_SECONDS_VALUE"
    --poll-seconds "$POLL_SECONDS_VALUE"
  )
  if [[ -n "$USER_KEY_VALUE" ]]; then
    trace_cmd+=(--user-key "$USER_KEY_VALUE")
  fi
  if [[ "$FULL_TEXT" -eq 1 ]]; then
    trace_cmd+=(--full-text)
  fi

  echo
  echo "== Section 2/3: focused trace ask cases =="
  "${trace_cmd[@]}" | tee "${RUN_DIR}/trace_ask.log"
fi

if [[ "$WITH_RESUME" -eq 1 ]]; then
  resume_cmd=(
    bash "${SCRIPT_DIR}/regression_resume_continue.sh"
    --base-url "$BASE_URL_VALUE"
    --user-id "$USER_ID_VALUE"
    --chat-id "$((CHAT_ID_VALUE + 90000))"
    --wait-seconds "$WAIT_SECONDS_VALUE"
  )

  echo
  echo "== Section 3/3: resume / continue flow =="
  "${resume_cmd[@]}" | tee "${RUN_DIR}/resume_continue.log"
fi

echo
echo "Artifacts:"
echo "  - $RUN_LOG"
echo "  - ${RUN_DIR}/simple_nl.log"
if [[ "$WITH_TRACE" -eq 1 ]]; then
  echo "  - ${RUN_DIR}/trace_ask.log"
fi
if [[ "$WITH_RESUME" -eq 1 ]]; then
  echo "  - ${RUN_DIR}/resume_continue.log"
fi

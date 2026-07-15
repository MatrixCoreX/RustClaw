#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

CASE_FILE="${SCRIPT_DIR}/cases/nl_cases_full.txt"
TRACE_CASE_FILE="${SCRIPT_DIR}/cases/nl_cases_trace.txt"
LOG_ROOT="${ROOT_DIR}/scripts/nl_suite_logs/full"
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
RUN_LOG="${RUN_DIR}/run.log"
BASE_URL_VALUE="${BASE_URL:-http://127.0.0.1:8787}"
USER_ID_VALUE="${USER_ID:-1985996990}"
CHAT_ID_VALUE="${CHAT_ID:-1985996990}"
USER_KEY_VALUE="${RUSTCLAW_USER_KEY:-${USER_KEY:-}}"
WAIT_SECONDS_VALUE="${MAX_WAIT_SECONDS:-180}"
POLL_SECONDS_VALUE="${POLL_INTERVAL_SECONDS:-1}"
NETWORK_RETRY_COUNT_VALUE="${NETWORK_RETRY_COUNT:-5}"
NETWORK_RETRY_SLEEP_SECONDS_VALUE="${NETWORK_RETRY_SLEEP_SECONDS:-15}"
MODEL_EXHAUST_SLEEP_SECONDS_VALUE="${MODEL_EXHAUST_SLEEP_SECONDS:-3600}"
MODEL_EXHAUST_MAX_RETRIES_VALUE="${MODEL_EXHAUST_MAX_RETRIES:-24}"
WITH_TRACE=0
WITH_RESUME=0
WITH_SELF_EXTENSION=0
FULL_TEXT=0
PROMPT_REPLY_ONLY=0
RESUME_DIR=""
RESUME_LINE=""

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_full_suite.sh [options]
  Preferred unified entry:
    bash scripts/nl_tests/run_suite.sh full [options]

Default:
  Run the comprehensive natural-language instruction suite using run_manual_test.sh.

Options:
  --case-file PATH      Main NL case file. Default: scripts/nl_tests/cases/nl_cases_full.txt
  --trace-case-file P   Trace case file. Default: scripts/nl_tests/cases/nl_cases_trace.txt
  --log-root PATH       Root log dir. Default: scripts/nl_suite_logs/full
  --base-url URL        clawd base url
  --user-id ID          user id for submits
  --chat-id ID          base chat id for submits
  --user-key KEY        RustClaw user key
  --wait-seconds N      max wait per case
  --poll-seconds N      poll interval
  --network-retries N   retry count for submit/query network failures
  --network-sleep N     sleep seconds between network retries
  --model-sleep N       sleep seconds after model exhausted/capacity
  --model-retries N     max per-case retries after model exhausted/capacity
  --provider-retry-sleep N
                       alias of --model-sleep for unified suite entry
  --provider-retries N alias of --model-retries for unified suite entry
  --no-llm-trace       accepted for compatibility with unified suite entry; ignored here
  --prompt-reply-only  Print only prompt and assistant reply for each case
  --full-text           pass through to child NL runner
  --resume-dir PATH     existing child run dir for the main NL runner
  --resume-line N       continue after this tested source line in the main case file
  --with-trace          additionally run regression_trace_ask.sh on focused trace cases
  --with-resume         additionally run regression_resume_continue.sh
  --with-self-extension additionally run self-extension regression suite
  -h, --help            show this help

Artifacts:
  scripts/nl_suite_logs/full/<timestamp>/
    run.log
    simple_nl.log
    trace_ask.log          (if --with-trace)
    resume_continue.log    (if --with-resume)
    self_extension.log     (if --with-self-extension)
EOF
}

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" --anchor "$1" "$2"
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
    --network-retries)
      NETWORK_RETRY_COUNT_VALUE="$2"
      shift 2
      ;;
    --network-sleep)
      NETWORK_RETRY_SLEEP_SECONDS_VALUE="$2"
      shift 2
      ;;
    --model-sleep)
      MODEL_EXHAUST_SLEEP_SECONDS_VALUE="$2"
      shift 2
      ;;
    --model-retries)
      MODEL_EXHAUST_MAX_RETRIES_VALUE="$2"
      shift 2
      ;;
    --provider-retry-sleep)
      MODEL_EXHAUST_SLEEP_SECONDS_VALUE="$2"
      shift 2
      ;;
    --provider-retries)
      MODEL_EXHAUST_MAX_RETRIES_VALUE="$2"
      shift 2
      ;;
    --no-llm-trace)
      shift
      ;;
    --prompt-reply-only)
      PROMPT_REPLY_ONLY=1
      shift
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
    --with-self-extension)
      WITH_SELF_EXTENSION=1
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

if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
  echo "NL full regression suite"
  echo "  run_dir_ref:      $(path_ref "$RUN_DIR" "$RUN_DIR")"
  echo "  run_log_ref:      $(path_ref "$RUN_DIR" "$RUN_LOG")"
  echo "  case_file_ref:    $(path_ref "$RUN_DIR" "$CASE_FILE")"
  echo "  trace_case_file_ref: $(path_ref "$RUN_DIR" "$TRACE_CASE_FILE")"
  echo "  base_url:         $BASE_URL_VALUE"
  echo "  user_id:          $USER_ID_VALUE"
  echo "  chat_id:          $CHAT_ID_VALUE"
  echo "  user_key:         ${USER_KEY_VALUE:-<auto-detect admin key>}"
  echo "  wait:             ${WAIT_SECONDS_VALUE}s"
  echo "  poll:             ${POLL_SECONDS_VALUE}s"
  echo "  net_retries:      ${NETWORK_RETRY_COUNT_VALUE}"
  echo "  net_sleep:        ${NETWORK_RETRY_SLEEP_SECONDS_VALUE}s"
  echo "  model_sleep:      ${MODEL_EXHAUST_SLEEP_SECONDS_VALUE}s"
  echo "  model_retries:    ${MODEL_EXHAUST_MAX_RETRIES_VALUE}"
  if [[ -n "$RESUME_DIR" ]]; then
    echo "  resume_dir_ref:   $(path_ref "$RUN_DIR" "$RESUME_DIR")"
  else
    echo "  resume_dir_ref:   new_run"
  fi
  echo "  resume_line:      ${RESUME_LINE:-<none>}"
  echo "  with_trace:       $WITH_TRACE"
  echo "  with_resume:      $WITH_RESUME"
  echo "  with_self_ext:    $WITH_SELF_EXTENSION"
  echo
fi

simple_cmd=(
  bash "${SCRIPT_DIR}/run_manual_test.sh"
  --case-file "$CASE_FILE"
  --base-url "$BASE_URL_VALUE"
  --user-id "$USER_ID_VALUE"
  --chat-id "$CHAT_ID_VALUE"
  --wait-seconds "$WAIT_SECONDS_VALUE"
  --poll-seconds "$POLL_SECONDS_VALUE"
  --network-retries "$NETWORK_RETRY_COUNT_VALUE"
  --network-sleep "$NETWORK_RETRY_SLEEP_SECONDS_VALUE"
  --model-sleep "$MODEL_EXHAUST_SLEEP_SECONDS_VALUE"
  --model-retries "$MODEL_EXHAUST_MAX_RETRIES_VALUE"
  --log-root "${RUN_DIR}/simple_nl_outputs"
)
if [[ -n "$USER_KEY_VALUE" ]]; then
  simple_cmd+=(--user-key "$USER_KEY_VALUE")
fi
if [[ "$FULL_TEXT" -eq 1 ]]; then
  simple_cmd+=(--full-text)
fi
if [[ "$PROMPT_REPLY_ONLY" -eq 1 ]]; then
  simple_cmd+=(--prompt-reply-only)
fi
if [[ -n "$RESUME_DIR" ]]; then
  simple_cmd+=(--resume-dir "$RESUME_DIR")
fi
if [[ -n "$RESUME_LINE" ]]; then
  simple_cmd+=(--resume-line "$RESUME_LINE")
fi

if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
  echo "== Section 1/3: comprehensive NL cases =="
fi
"${simple_cmd[@]}" | tee "${RUN_DIR}/simple_nl.log"

if [[ "$WITH_TRACE" -eq 1 ]]; then
  trace_cmd=(
    bash "${ROOT_DIR}/scripts/regression_trace_ask.sh"
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

  if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
    echo
    echo "== Section 2/3: focused trace ask cases =="
  fi
  "${trace_cmd[@]}" | tee "${RUN_DIR}/trace_ask.log"
fi

if [[ "$WITH_RESUME" -eq 1 ]]; then
  resume_cmd=(
    bash "${ROOT_DIR}/scripts/regression_resume_continue.sh"
    --base-url "$BASE_URL_VALUE"
    --user-id "$USER_ID_VALUE"
    --chat-id "$((CHAT_ID_VALUE + 90000))"
    --wait-seconds "$WAIT_SECONDS_VALUE"
  )

  if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
    echo
    echo "== Section 3/3: resume / continue flow =="
  fi
  "${resume_cmd[@]}" | tee "${RUN_DIR}/resume_continue.log"
fi

if [[ "$WITH_SELF_EXTENSION" -eq 1 ]]; then
  self_extension_cmd=(
    bash "${ROOT_DIR}/scripts/regression_self_extension_suite.sh"
    --wait-seconds "$WAIT_SECONDS_VALUE"
  )
  if [[ -x "${ROOT_DIR}/target/debug/clawd" ]]; then
    self_extension_cmd+=(--clawd-bin "${ROOT_DIR}/target/debug/clawd")
  fi

  if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
    echo
    echo "== Self-extension regressions =="
  fi
  "${self_extension_cmd[@]}" | tee "${RUN_DIR}/self_extension.log"
fi

if [[ "$PROMPT_REPLY_ONLY" -ne 1 ]]; then
  echo
  echo "Artifacts:"
  echo "  - run_log_ref=$(path_ref "$RUN_DIR" "$RUN_LOG")"
  echo "  - simple_nl_log_ref=$(path_ref "$RUN_DIR" "${RUN_DIR}/simple_nl.log")"
  if [[ "$WITH_TRACE" -eq 1 ]]; then
    echo "  - trace_ask_log_ref=$(path_ref "$RUN_DIR" "${RUN_DIR}/trace_ask.log")"
  fi
  if [[ "$WITH_RESUME" -eq 1 ]]; then
    echo "  - resume_continue_log_ref=$(path_ref "$RUN_DIR" "${RUN_DIR}/resume_continue.log")"
  fi
  if [[ "$WITH_SELF_EXTENSION" -eq 1 ]]; then
    echo "  - self_extension_log_ref=$(path_ref "$RUN_DIR" "${RUN_DIR}/self_extension.log")"
  fi
fi

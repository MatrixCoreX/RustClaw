#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${PROFILE:-release}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/logs/base_skill_contracts_$(date +%Y%m%d_%H%M%S)}"
REPORT_PATH="${REPORT_PATH:-$LOG_DIR/report.md}"

PASS=0
FAIL=0
SKIP=0
RESULT_LINES=()
TMP_DIR=""
HTTP_SERVER_PID=""

usage() {
  cat <<EOF
Usage:
  bash scripts/check_base_skill_response_contracts.sh [options]

Options:
  --profile release         Wrapper profile (default: release)
  --log-dir PATH            Directory for logs
  --report PATH             Markdown report path (default: <log-dir>/report.md)
  --help, -h                Show help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    --log-dir)
      LOG_DIR="${2:-}"
      shift 2
      ;;
    --report)
      REPORT_PATH="${2:-}"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ "$PROFILE" != "release" ]]; then
  echo "--profile must be release" >&2
  exit 2
fi

command -v jq >/dev/null 2>&1 || {
  echo "Missing command: jq" >&2
  exit 2
}

mkdir -p "$LOG_DIR"
TMP_DIR="$(mktemp -d /tmp/base-skill-contracts-XXXXXX)"

cleanup() {
  if [[ -n "$HTTP_SERVER_PID" ]]; then
    kill "$HTTP_SERVER_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$TMP_DIR" && -d "$TMP_DIR" ]]; then
    rm -rf "$TMP_DIR"
  fi
}
trap cleanup EXIT

check_ok_extra() {
  local skill="$1"
  local args_json="$2"
  local extra_expr="${3:-.extra != null}"
  local wrapper="$ROOT_DIR/scripts/skill_calls/call_${skill}.sh"
  local stdout_log="$LOG_DIR/${skill}.stdout.log"
  local stderr_log="$LOG_DIR/${skill}.stderr.log"

  echo "== $skill =="
  set +e
  bash "$wrapper" --profile "$PROFILE" --raw --args "$args_json" >"$stdout_log" 2>"$stderr_log"
  local rc=$?
  set -e

  if [[ "$rc" -ne 0 ]]; then
    FAIL=$((FAIL + 1))
    echo "FAIL $skill exit=$rc"
    RESULT_LINES+=("- FAIL: \`$skill\` exit=$rc ([stdout]($stdout_log), [stderr]($stderr_log))")
    return
  fi

  local resp
  resp="$(tr -d '\r' <"$stdout_log" | tail -n 1)"
  if [[ -z "$resp" ]]; then
    FAIL=$((FAIL + 1))
    echo "FAIL $skill empty output"
    RESULT_LINES+=("- FAIL: \`$skill\` empty output ([stdout]($stdout_log), [stderr]($stderr_log))")
    return
  fi

  if ! printf '%s\n' "$resp" | jq -e "
    .status == \"ok\"
    and ($extra_expr)
  " >/dev/null 2>&1; then
    FAIL=$((FAIL + 1))
    echo "FAIL $skill extra contract mismatch"
    RESULT_LINES+=("- FAIL: \`$skill\` extra contract mismatch ([stdout]($stdout_log), [stderr]($stderr_log))")
    return
  fi

  PASS=$((PASS + 1))
  echo "PASS $skill"
  RESULT_LINES+=("- PASS: \`$skill\`")
}

check_error_contract() {
  local skill="$1"
  local args_json="$2"
  local error_expr="${3:-((.extra.error_kind? // .error_kind?) | type == \"string\" and length > 0)}"
  local wrapper="$ROOT_DIR/scripts/skill_calls/call_${skill}.sh"
  local stdout_log="$LOG_DIR/${skill}.error.stdout.log"
  local stderr_log="$LOG_DIR/${skill}.error.stderr.log"

  echo "== $skill error contract =="
  set +e
  bash "$wrapper" --profile "$PROFILE" --raw --args "$args_json" >"$stdout_log" 2>"$stderr_log"
  local rc=$?
  set -e

  if [[ "$rc" -ne 0 ]]; then
    FAIL=$((FAIL + 1))
    echo "FAIL $skill error contract exit=$rc"
    RESULT_LINES+=("- FAIL: \`$skill\` error contract exit=$rc ([stdout]($stdout_log), [stderr]($stderr_log))")
    return
  fi

  local resp
  resp="$(tr -d '\r' <"$stdout_log" | tail -n 1)"
  if [[ -z "$resp" ]]; then
    FAIL=$((FAIL + 1))
    echo "FAIL $skill error contract empty output"
    RESULT_LINES+=("- FAIL: \`$skill\` error contract empty output ([stdout]($stdout_log), [stderr]($stderr_log))")
    return
  fi

  if ! printf '%s\n' "$resp" | jq -e "
    .status == \"error\"
    and (.error_text | type == \"string\" and length > 0)
    and ($error_expr)
  " >/dev/null 2>&1; then
    FAIL=$((FAIL + 1))
    echo "FAIL $skill error contract mismatch"
    RESULT_LINES+=("- FAIL: \`$skill\` error contract mismatch ([stdout]($stdout_log), [stderr]($stderr_log))")
    return
  fi

  PASS=$((PASS + 1))
  echo "PASS $skill error contract"
  RESULT_LINES+=("- PASS: \`$skill\` error contract")
}

skip_check() {
  local skill="$1"
  local reason="$2"
  SKIP=$((SKIP + 1))
  echo "SKIP $skill ($reason)"
  RESULT_LINES+=("- SKIP: \`$skill\` ($reason)")
}

start_http_contract_server() {
  local serve_dir="$TMP_DIR/http_basic"
  mkdir -p "$serve_dir"
  printf '%s\n' '{"ok":true}' >"$serve_dir/index.json"
  python3 -m http.server 18087 --bind 127.0.0.1 --directory "$serve_dir" >"$LOG_DIR/http_basic.server.stdout.log" 2>"$LOG_DIR/http_basic.server.stderr.log" &
  HTTP_SERVER_PID=$!
  sleep 1
  kill -0 "$HTTP_SERVER_PID" >/dev/null 2>&1
}

check_ok_extra "system_basic" '{"action":"info"}' '.extra != null and (.extra | has("hostname")) and (.extra | has("workspace_root"))'
check_ok_extra "fs_search" '{"action":"find_ext","ext":"md","root":"crates/skills","max_results":2}' '.extra != null and .extra.action == "find_ext" and (.extra | has("count")) and (.extra | has("results"))'
check_ok_extra "health_check" '{}' '.extra != null and (.extra | has("workspace_root")) and (.extra | has("log_dir"))'
check_ok_extra "process_basic" '{"action":"ps","limit":2}' '.extra != null and .extra.action == "ps" and .extra.exit_code == 0 and (.extra | has("output"))'
check_ok_extra "git_basic" '{"action":"status"}' '.extra != null and .extra.action == "status" and .extra.subcommand == "status" and .extra.exit_code == 0 and (.extra | has("output"))'
check_ok_extra "package_manager" '{"action":"detect"}' '.extra != null and .extra.action == "detect" and (.extra | has("manager")) and (.extra | has("output"))'
check_ok_extra "archive_basic" '{"action":"pack","source":"scripts/skill_calls","archive":"tmp/archive-basic-contract.zip","format":"zip"}' '.extra != null and .extra.action == "pack" and .extra.format == "zip" and (.extra | has("source")) and (.extra | has("archive")) and (.extra | has("output"))'
check_ok_extra "db_basic" '{"action":"sqlite_query","db_path":"data/db-basic-contract.sqlite","sql":"PRAGMA schema_version;"}' '.extra != null and .extra.action == "sqlite_query" and (.extra | has("db_path")) and (.extra | has("sql")) and (.extra.result | has("columns")) and (.extra.result | has("rows"))'
check_ok_extra "config_guard" '{"path":"configs/config.toml"}' '.extra != null and .extra.action == "scan" and (.extra | has("path")) and (.extra | has("risk_count")) and (.extra | has("risks"))'
if start_http_contract_server; then
  check_ok_extra "http_basic" '{"action":"get","url":"http://127.0.0.1:18087/index.json","timeout_seconds":5}' '.extra != null and .extra.action == "get" and .extra.status_code == 200 and (.extra | has("url")) and (.extra | has("body_preview"))'
  kill "$HTTP_SERVER_PID" >/dev/null 2>&1 || true
  HTTP_SERVER_PID=""
else
  skip_check "http_basic" "local http server unavailable"
fi

if command -v docker >/dev/null 2>&1; then
  check_ok_extra "docker_basic" '{"action":"ps"}' '.extra != null and .extra.action == "ps" and .extra.exit_code == 0 and (.extra | has("docker_args")) and (.extra | has("output"))'
else
  skip_check "docker_basic" "docker command not available"
fi

check_error_contract "db_basic" '{"action":"sqlite_query","db_path":"data/db-basic-contract.sqlite","sql":"DELETE FROM demo"}' '.extra.error_kind == "unsafe_sql"'
check_error_contract "config_guard" '{"path":"configs/does-not-exist.toml"}' '.extra.error_kind == "not_found"'
check_error_contract "system_basic" '{"action":"read_range","path":"."}' '((.extra.error_kind? // .error_kind?) == "is_directory")'
check_error_contract "archive_basic" '{"action":"nope"}' '((.extra.error_kind? // .error_kind?) == "invalid_input")'

echo
echo "==== Base Skill Contract Summary ===="
echo "PASS: $PASS"
echo "FAIL: $FAIL"
echo "SKIP: $SKIP"
echo "Logs: $LOG_DIR"

mkdir -p "$(dirname "$REPORT_PATH")"
{
  echo "# Base Skill Response Contract Report"
  echo
  echo "- Time: $(date '+%Y-%m-%d %H:%M:%S %Z')"
  echo "- Profile: \`$PROFILE\`"
  echo "- PASS: $PASS"
  echo "- FAIL: $FAIL"
  echo "- SKIP: $SKIP"
  echo "- Logs: \`$LOG_DIR\`"
  echo
  for line in "${RESULT_LINES[@]}"; do
    echo "$line"
  done
} >"$REPORT_PATH"

echo "Report: $REPORT_PATH"

if [[ "$FAIL" -ne 0 ]]; then
  exit 1
fi

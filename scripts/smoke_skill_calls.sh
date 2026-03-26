#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CALLS_DIR="$ROOT_DIR/scripts/skill_calls"
PROFILE="${PROFILE:-release}"
AUTO_BUILD=0
TIMEOUT_SECS="${TIMEOUT_SECS:-60}"
LIST_ONLY=0
CONTINUE_ON_ERROR=1
SKILLS_FILTER="${SKILLS_FILTER:-}"
EXCLUDE_FILTER="${EXCLUDE_FILTER:-}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/logs/skill_call_smoke_$(date +%Y%m%d_%H%M%S)}"
REPORT_PATH="${REPORT_PATH:-$LOG_DIR/report.md}"

PASS=0
WARN=0
FAIL=0
SKIP=0
RESULT_LINES=()

usage() {
  cat <<EOF
Usage:
  bash scripts/smoke_skill_calls.sh [options]

Options:
  --profile debug|release   Wrapper profile (default: release)
  --auto-build              Pass --auto-build to wrappers
  --timeout N               Timeout per wrapper in seconds (default: 60)
  --skills a,b,c            Only run selected skills
  --exclude a,b,c           Skip selected skills
  --log-dir PATH            Directory for per-skill stdout/stderr logs
  --report PATH             Markdown report output path (default: <log-dir>/report.md)
  --stop-on-error           Stop immediately on first hard failure
  --list                    Only list discovered wrappers
  --help, -h                Show help

Notes:
  - This is a protocol-level smoke runner for scripts/skill_calls/call_*.sh.
  - PASS: wrapper returned valid JSON with required top-level fields and status=ok.
  - WARN: wrapper returned valid JSON with required top-level fields but status=error.
  - FAIL: wrapper crashed, timed out, or returned invalid JSON shape.
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1" >&2
    exit 2
  }
}

csv_has() {
  local csv="$1"
  local needle="$2"
  [[ -z "$csv" ]] && return 1
  IFS=',' read -r -a parts <<<"$csv"
  for part in "${parts[@]}"; do
    if [[ "${part// /}" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

discover_skills() {
  local path stem
  for path in "$CALLS_DIR"/call_*.sh; do
    [[ -e "$path" ]] || continue
    stem="$(basename "$path")"
    echo "${stem#call_}" | sed 's/\.sh$//'
  done | sort
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    --auto-build)
      AUTO_BUILD=1
      shift
      ;;
    --timeout)
      TIMEOUT_SECS="${2:-}"
      shift 2
      ;;
    --skills)
      SKILLS_FILTER="${2:-}"
      shift 2
      ;;
    --exclude)
      EXCLUDE_FILTER="${2:-}"
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
    --stop-on-error)
      CONTINUE_ON_ERROR=0
      shift
      ;;
    --list)
      LIST_ONLY=1
      shift
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

if [[ "$PROFILE" != "debug" && "$PROFILE" != "release" ]]; then
  echo "--profile must be debug or release" >&2
  exit 2
fi

need_cmd jq
need_cmd timeout

mapfile -t ALL_SKILLS < <(discover_skills)

if [[ "$LIST_ONLY" == "1" ]]; then
  printf '%s\n' "${ALL_SKILLS[@]}"
  exit 0
fi

mkdir -p "$LOG_DIR"
echo "Log dir: $LOG_DIR"

for skill in "${ALL_SKILLS[@]}"; do
  if [[ -n "$SKILLS_FILTER" ]] && ! csv_has "$SKILLS_FILTER" "$skill"; then
    SKIP=$((SKIP + 1))
    echo "SKIP $skill (not selected)"
    RESULT_LINES+=("- SKIP: \`$skill\` (not selected)")
    continue
  fi
  if csv_has "$EXCLUDE_FILTER" "$skill"; then
    SKIP=$((SKIP + 1))
    echo "SKIP $skill (excluded)"
    RESULT_LINES+=("- SKIP: \`$skill\` (excluded)")
    continue
  fi

  wrapper="$CALLS_DIR/call_${skill}.sh"
  stdout_log="$LOG_DIR/${skill}.stdout.log"
  stderr_log="$LOG_DIR/${skill}.stderr.log"

  cmd=(bash "$wrapper" --profile "$PROFILE" --raw)
  if [[ "$AUTO_BUILD" == "1" ]]; then
    cmd+=(--auto-build)
  fi

  echo
  echo "== $skill =="
  set +e
  timeout "$TIMEOUT_SECS" "${cmd[@]}" >"$stdout_log" 2>"$stderr_log"
  rc=$?
  set -e

  if [[ "$rc" -ne 0 ]]; then
    FAIL=$((FAIL + 1))
    echo "FAIL $skill exit=$rc"
    echo "  stdout: $stdout_log"
    echo "  stderr: $stderr_log"
    RESULT_LINES+=("- FAIL: \`$skill\` exit=$rc ([stdout]($stdout_log), [stderr]($stderr_log))")
    if [[ "$CONTINUE_ON_ERROR" == "0" ]]; then
      break
    fi
    continue
  fi

  resp="$(tr -d '\r' <"$stdout_log" | tail -n 1)"
  if [[ -z "$resp" ]]; then
    FAIL=$((FAIL + 1))
    echo "FAIL $skill empty output"
    echo "  stdout: $stdout_log"
    echo "  stderr: $stderr_log"
    RESULT_LINES+=("- FAIL: \`$skill\` empty output ([stdout]($stdout_log), [stderr]($stderr_log))")
    if [[ "$CONTINUE_ON_ERROR" == "0" ]]; then
      break
    fi
    continue
  fi

  if ! printf '%s\n' "$resp" | jq -e '
    type == "object"
    and (.request_id | type == "string")
    and (.status | type == "string")
    and has("text")
    and has("error_text")
  ' >/dev/null 2>&1; then
    FAIL=$((FAIL + 1))
    echo "FAIL $skill invalid response shape"
    echo "  stdout: $stdout_log"
    echo "  stderr: $stderr_log"
    RESULT_LINES+=("- FAIL: \`$skill\` invalid response shape ([stdout]($stdout_log), [stderr]($stderr_log))")
    if [[ "$CONTINUE_ON_ERROR" == "0" ]]; then
      break
    fi
    continue
  fi

  status="$(printf '%s\n' "$resp" | jq -r '.status')"
  if [[ "$status" == "ok" ]]; then
    PASS=$((PASS + 1))
    echo "PASS $skill"
    RESULT_LINES+=("- PASS: \`$skill\`")
  else
    WARN=$((WARN + 1))
    echo "WARN $skill status=$status"
    echo "  stdout: $stdout_log"
    echo "  stderr: $stderr_log"
    RESULT_LINES+=("- WARN: \`$skill\` status=\`$status\` ([stdout]($stdout_log), [stderr]($stderr_log))")
  fi
done

echo
echo "==== Skill Call Smoke Summary ===="
echo "PASS: $PASS"
echo "WARN: $WARN"
echo "FAIL: $FAIL"
echo "SKIP: $SKIP"
echo "Logs: $LOG_DIR"

mkdir -p "$(dirname "$REPORT_PATH")"
{
  echo "# Skill Call Smoke Report"
  echo
  echo "- Time: $(date '+%Y-%m-%d %H:%M:%S %Z')"
  echo "- Profile: \`$PROFILE\`"
  echo "- PASS: $PASS"
  echo "- WARN: $WARN"
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

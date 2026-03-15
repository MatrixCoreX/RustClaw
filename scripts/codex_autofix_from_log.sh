#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
LOG_FILE="${ROOT_DIR}/logs/clawd.log"
LINES=3000
CONTEXT=30
TASKS=10
MODE="full-auto"
MODEL=""
EXTRA_PROMPT=""
DRY_RUN=0
OUTPUT_FILE="${ROOT_DIR}/logs/codex_autofix.last.txt"
PROMPT_FILE="${ROOT_DIR}/logs/codex_autofix.prompt.txt"

usage() {
  cat <<'EOF'
Usage:
  scripts/codex_autofix_from_log.sh [options]

Options:
  --log <path>         Log file to inspect (default: logs/clawd.log)
  --lines <n>          Max log lines to search for task markers (default: 3000)
  --tasks <n>          Consider only the last n task log blocks (default: 10)
  --context <n>        Extra lines before/after the task range (default: 30)
  --model <name>       Pass a specific Codex model
  --dangerous          Use codex --dangerously-bypass-approvals-and-sandbox
  --extra <text>       Extra repair instructions appended to the Codex prompt
  --dry-run            Only generate and print the prompt, do not call Codex
  -h, --help           Show this help

Examples:
  scripts/codex_autofix_from_log.sh --dry-run
  scripts/codex_autofix_from_log.sh --tasks 20 --context 50
  scripts/codex_autofix_from_log.sh --dangerous --extra "Focus on parser failures first"
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --log)
      LOG_FILE="$2"
      shift 2
      ;;
    --lines)
      LINES="$2"
      shift 2
      ;;
    --tasks)
      TASKS="$2"
      shift 2
      ;;
    --context)
      CONTEXT="$2"
      shift 2
      ;;
    --model)
      MODEL="$2"
      shift 2
      ;;
    --dangerous)
      MODE="dangerous"
      shift
      ;;
    --extra)
      EXTRA_PROMPT="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ ! -f "$LOG_FILE" ]]; then
  echo "Log file not found: $LOG_FILE" >&2
  exit 1
fi

if ! command -v codex >/dev/null 2>&1; then
  echo "codex command not found in PATH" >&2
  exit 1
fi

mkdir -p "${ROOT_DIR}/logs"
TMP_LOG="$(mktemp)"
trap 'rm -f "$TMP_LOG"' EXIT

tail -n "$LINES" "$LOG_FILE" > "$TMP_LOG"
# Recent N tasks: lines matching task_call_end / task_call_begin / claim_next_task: claimed
TASK_LINES="$(
  rg -n 'task_call_end|task_call_begin|claim_next_task: claimed' "$TMP_LOG" | tail -n "$TASKS" | cut -d: -f1
)"
if [[ -z "$TASK_LINES" ]]; then
  echo "No task markers found in last ${LINES} lines of $LOG_FILE" >&2
  exit 1
fi
FIRST_LINE="$(echo "$TASK_LINES" | head -n 1)"
LAST_LINE="$(echo "$TASK_LINES" | tail -n 1)"
START=$(( FIRST_LINE > CONTEXT ? FIRST_LINE - CONTEXT : 1 ))
END=$(( LAST_LINE + CONTEXT ))
EXCERPT="$(sed -n "${START},${END}p" "$TMP_LOG")"

# Only fix when there is a problem in this excerpt
if ! echo "$EXCERPT" | rg -q -i 'task_call_end.*status=failed|ERROR .* failed|error\[E[0-9]{4}\]|could not compile|panicked|panic'; then
  if ! echo "$EXCERPT" | rg -q -i 'error|failed:|failed status='; then
    echo "No issue found in recent ${TASKS} task(s). Nothing to fix."
    exit 0
  fi
fi

GIT_STATUS="$(git -C "$ROOT_DIR" status --short 2>/dev/null || true)"
if [[ -z "$GIT_STATUS" ]]; then
  GIT_STATUS='(clean or unavailable)'
fi

cat > "$PROMPT_FILE" <<EOF
You are fixing a bug in the RustClaw repository.

Workspace: ${ROOT_DIR}
Primary signal: log excerpt for the last ${TASKS} task(s) from ${LOG_FILE}

Goals:
1. Infer the most likely root cause from the log excerpt.
2. Inspect the relevant code and apply a minimal fix.
3. Preserve existing behavior outside the bug fix.
4. Run the narrowest useful verification command if appropriate.
5. Summarize the root cause, files changed, and verification result.

Constraints:
- Work only inside this repository.
- Prefer small, targeted edits over broad refactors.
- Respect existing uncommitted changes.
- If the log excerpt is insufficient, inspect nearby code paths before changing anything.

Recent git status:
${GIT_STATUS}

Recent log excerpt:

aaaLOG_STARTaaa
${EXCERPT}
aaaLOG_ENDaaa
EOF

if [[ -n "$EXTRA_PROMPT" ]]; then
  {
    echo
    echo "Extra instructions:"
    echo "$EXTRA_PROMPT"
  } >> "$PROMPT_FILE"
fi

if [[ "$DRY_RUN" == "1" ]]; then
  echo "Prompt written to: $PROMPT_FILE"
  sed -n '1,240p' "$PROMPT_FILE"
  exit 0
fi

CODEX_ARGS=(exec -C "$ROOT_DIR" -o "$OUTPUT_FILE")
if [[ "$MODE" == "dangerous" ]]; then
  CODEX_ARGS+=(--dangerously-bypass-approvals-and-sandbox)
else
  CODEX_ARGS+=(--full-auto)
fi
if [[ -n "$MODEL" ]]; then
  CODEX_ARGS+=(--model "$MODEL")
fi
CODEX_ARGS+=(-)

printf 'Running: codex %s\n' "${CODEX_ARGS[*]}"
codex "${CODEX_ARGS[@]}" < "$PROMPT_FILE"

echo
echo "Prompt: $PROMPT_FILE"
echo "Last message: $OUTPUT_FILE"

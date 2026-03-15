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
OUTPUT_FILE="${ROOT_DIR}/logs/codex_autofix_prompt.last.txt"
PROMPT_FILE="${ROOT_DIR}/logs/codex_autofix_prompt.prompt.txt"
VENDOR=""
PROMPT_NAME=""

usage() {
  cat <<'EOF'
Usage:
  scripts/codex_autofix_prompt_from_log.sh [options]

Same as codex_autofix_from_log.sh but restricts fixes to prompt files only
(prompts/*.md). Use when the log suggests LLM/output issues and you want
to tune prompts without touching code.

Options:
  --log <path>         Log file to inspect (default: logs/clawd.log)
  --lines <n>          Max log lines to search for task markers (default: 3000)
  --tasks <n>          Consider only the last n task log blocks (default: 10)
  --context <n>        Extra lines before/after the task range (default: 30)
  --vendor <name>      Optional: scope to vendor (qwen|minimax|openai|google|claude)
  --prompt <name>      Optional: prompt file stem, e.g. chat_response_prompt
  --model <name>       Pass a specific Codex model
  --dangerous          Use codex --dangerously-bypass-approvals-and-sandbox
  --extra <text>       Extra repair instructions appended to the Codex prompt
  --dry-run            Only generate and print the prompt, do not call Codex
  -h, --help           Show this help

Examples:
  scripts/codex_autofix_prompt_from_log.sh --dry-run
  scripts/codex_autofix_prompt_from_log.sh --tasks 20 --prompt intent_router_prompt --vendor qwen
  scripts/codex_autofix_prompt_from_log.sh --dangerous --extra "Tighten JSON output rules"
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
    --vendor)
      VENDOR="$2"
      shift 2
      ;;
    --prompt)
      PROMPT_NAME="$2"
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

SCOPE_TEXT=""
if [[ -n "$VENDOR" ]] || [[ -n "$PROMPT_NAME" ]]; then
  SCOPE_TEXT="Scope (if applicable):"
  [[ -n "$VENDOR" ]]      && SCOPE_TEXT="$SCOPE_TEXT vendor=$VENDOR"
  [[ -n "$PROMPT_NAME" ]] && SCOPE_TEXT="$SCOPE_TEXT prompt stem=$PROMPT_NAME (e.g. prompts/${PROMPT_NAME}.md or prompts/skills/*.md)"
fi

cat > "$PROMPT_FILE" <<EOF
You are fixing a bug in the RustClaw repository by adjusting PROMPT FILES ONLY.

Workspace: ${ROOT_DIR}
Primary signal: log excerpt for the last ${TASKS} task(s) from ${LOG_FILE}

STRICT CONSTRAINT — PROMPTS ONLY:
- You may ONLY create or modify files under the prompts/ directory (e.g. prompts/*.md, prompts/skills/*.md).
- Do NOT modify any Rust, TypeScript, TOML, or other code. Do NOT change crates/, UI/, configs/ (except if you are explicitly told to touch a specific config that only holds prompt text).
- If the log clearly indicates a code bug (e.g. compile error, panic, wrong logic in .rs/.ts), do not attempt a code fix here; instead briefly state that the user should run codex_autofix_from_log.sh or codex_fix.sh for code fixes.

Goals:
1. Infer from the log excerpt whether the failure is likely due to prompt wording, instructions, or examples (e.g. LLM output format, routing, or skill behavior).
2. Identify the most relevant prompt file(s) under prompts/ and apply minimal edits to improve behavior.
3. Preserve existing style and structure of the prompt; only change what is needed to address the failure.
4. If you suggest verification, limit it to re-running the same flow or a narrow prompt-level check; do not change or run code unless it is already part of the repo's prompt-test flow.

${SCOPE_TEXT}

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
  sed -n '1,260p' "$PROMPT_FILE"
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

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
MODE="auto"
VENDOR=""
PROMPT_NAME=""
LOG_FILE="${ROOT_DIR}/logs/clawd.log"
MODEL_IO_LOG="${ROOT_DIR}/logs/model_io.log"
LINES=400
CONTEXT=40
DRY_RUN=0
DANGEROUS=0
MODEL=""
EXTRA_PROMPT=""
OUTPUT_FILE="${ROOT_DIR}/logs/codex_fix.last.txt"
PROMPT_FILE="${ROOT_DIR}/logs/codex_fix.prompt.txt"
DETECTION_REASON=""
DETECTED_VENDOR=""
DETECTED_PROMPT=""

usage() {
	cat <<'EOF'
Usage:
  scripts/codex_fix.sh [auto|code|prompt|both] [options]

Modes:
  auto                  Inspect recent logs and decide whether to fix code, prompt, or both
  code                  Focus on code fixes from recent logs
  prompt                Focus on vendor prompt tuning only
  both                  Allow both code and prompt fixes

Options:
  --vendor <name>       Vendor for prompt mode: qwen|minimax|openai|google|claude
  --prompt <name>       Prompt file stem, e.g. chat_response_prompt
  --log <path>          Runtime/build log file (default: logs/clawd.log)
  --lines <n>           Tail last n log lines (default: 400)
  --context <n>         Keep n lines around latest error match (default: 40)
  --model <name>        Codex model override
  --dangerous           Use codex --dangerously-bypass-approvals-and-sandbox
  --extra <text>        Extra instructions appended to the prompt
  --dry-run             Only generate prompt, do not call codex
  -h, --help            Show help

Examples:
  scripts/codex_fix.sh auto --dry-run
  scripts/codex_fix.sh code --dry-run
  scripts/codex_fix.sh prompt --vendor minimax --prompt chat_response_prompt
  scripts/codex_fix.sh both --vendor qwen --prompt single_plan_execution_prompt --dangerous
EOF
}

if [[ $# -gt 0 ]]; then
	case "$1" in
	auto | code | prompt | both)
		MODE="$1"
		shift
		;;
	esac
fi

while [[ $# -gt 0 ]]; do
	case "$1" in
	--vendor)
		VENDOR="$2"
		shift 2
		;;
	--prompt)
		PROMPT_NAME="$2"
		shift 2
		;;
	--log)
		LOG_FILE="$2"
		shift 2
		;;
	--lines)
		LINES="$2"
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
		DANGEROUS=1
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
	-h | --help)
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

if ! command -v codex >/dev/null 2>&1; then
	echo "codex command not found in PATH" >&2
	exit 1
fi

mkdir -p "${ROOT_DIR}/logs"
GIT_STATUS="$(git -C "$ROOT_DIR" status --short 2>/dev/null || true)"
[[ -n "$GIT_STATUS" ]] || GIT_STATUS='(clean or unavailable)'

TMP_LOG=""
cleanup() {
	[[ -n "$TMP_LOG" && -f "$TMP_LOG" ]] && rm -f "$TMP_LOG"
}
trap cleanup EXIT

RUNTIME_EXCERPT=""
LAST_ERROR_LINE=""
LOG_TAIL=""
if [[ -f "$LOG_FILE" ]]; then
	TMP_LOG="$(mktemp)"
	tail -n "$LINES" "$LOG_FILE" >"$TMP_LOG"
	LOG_TAIL="$(cat "$TMP_LOG")"
	LAST_ERROR_LINE="$(
		rg -n -i 'task_call_end.*status=failed|ERROR .* failed|error\[E[0-9]{4}\]|could not compile|panicked|panic|traceback|exception' "$TMP_LOG" | tail -n 1 | cut -d: -f1 || true
	)"
	if [[ -z "$LAST_ERROR_LINE" ]]; then
		LAST_ERROR_LINE="$(rg -n -i 'error|failed:|failed status=|parse_failed|invalid json|json parse|unexpected token|tool call parse' "$TMP_LOG" | tail -n 1 | cut -d: -f1 || true)"
	fi
	if [[ -n "$LAST_ERROR_LINE" ]]; then
		START=$((LAST_ERROR_LINE > CONTEXT ? LAST_ERROR_LINE - CONTEXT : 1))
		END=$((LAST_ERROR_LINE + CONTEXT))
		RUNTIME_EXCERPT="$(sed -n "${START},${END}p" "$TMP_LOG")"
	else
		RUNTIME_EXCERPT="$(tail -n "$CONTEXT" "$TMP_LOG")"
	fi
fi

extract_vendor() {
	local source_text="$1"
	printf '%s\n' "$source_text" | rg -o 'vendor=(qwen|minimax|openai|google|claude)' -N | tail -n 1 | sed 's/^vendor=//' || true
}

extract_prompt() {
	local source_text="$1"
	local prompt
	prompt="$(printf '%s\n' "$source_text" | rg -o 'prompt_file=[^ ]*([A-Za-z0-9_/-]+)\.md' -N | tail -n 1 | sed -E 's#.*prompt_file=.*/([^/]+)\.md#\1#' || true)"
	if [[ -z "$prompt" ]]; then
		prompt="$(printf '%s\n' "$source_text" | rg -o 'prompt_name=[A-Za-z0-9_-]+' -N | tail -n 1 | sed 's/^prompt_name=//' || true)"
	fi
	printf '%s' "$prompt"
}

has_code_signal=0
has_prompt_signal=0
if [[ -n "$LOG_TAIL" ]]; then
	if printf '%s\n' "$LOG_TAIL" | rg -qi 'error\[E[0-9]{4}\]|could not compile|panicked|panic|traceback|exception|segmentation fault|borrow checker|mismatched types|no method named'; then
		has_code_signal=1
	fi
	if printf '%s\n' "$LOG_TAIL" | rg -qi '\[LLM_CALL\]|\[PROMPT\]|prompt_invocation|prompt_file=|prompt_name=|parse_failed|invalid json|json parse|needs_clarify|resolved_user_intent'; then
		has_prompt_signal=1
	fi
fi

if [[ -z "$VENDOR" ]]; then
	DETECTED_VENDOR="$(extract_vendor "$RUNTIME_EXCERPT")"
	if [[ -z "$DETECTED_VENDOR" && -f "$MODEL_IO_LOG" ]]; then
		DETECTED_VENDOR="$(tail -n "$LINES" "$MODEL_IO_LOG" | rg -o '"vendor"\s*:\s*"(qwen|minimax|openai|google|claude)"|vendor=(qwen|minimax|openai|google|claude)' -N | tail -n 1 | sed -E 's/.*"(qwen|minimax|openai|google|claude)".*/\1/; s/^vendor=//' || true)"
	fi
	VENDOR="$DETECTED_VENDOR"
fi

if [[ -z "$PROMPT_NAME" ]]; then
	DETECTED_PROMPT="$(extract_prompt "$RUNTIME_EXCERPT")"
	if [[ -z "$DETECTED_PROMPT" && -f "$MODEL_IO_LOG" ]]; then
		DETECTED_PROMPT="$(extract_prompt "$(tail -n "$LINES" "$MODEL_IO_LOG")")"
	fi
	PROMPT_NAME="$DETECTED_PROMPT"
fi

if [[ "$MODE" == "auto" ]]; then
	if [[ $has_code_signal -eq 1 && $has_prompt_signal -eq 1 && -n "$PROMPT_NAME" && -n "$VENDOR" ]]; then
		MODE="both"
		DETECTION_REASON="recent logs contain both code-failure signals and prompt/LLM signals; vendor=${VENDOR}; prompt=${PROMPT_NAME}"
	elif [[ $has_code_signal -eq 1 ]]; then
		MODE="code"
		DETECTION_REASON="recent logs mainly show compile/runtime failure signals"
	elif [[ $has_prompt_signal -eq 1 && -n "$PROMPT_NAME" && -n "$VENDOR" ]]; then
		MODE="prompt"
		DETECTION_REASON="recent logs mainly show LLM/prompt behavior signals; vendor=${VENDOR}; prompt=${PROMPT_NAME}"
	elif [[ -n "$PROMPT_NAME" && -n "$VENDOR" ]]; then
		MODE="prompt"
		DETECTION_REASON="inferred latest active prompt from logs; defaulting to prompt tuning for vendor=${VENDOR}; prompt=${PROMPT_NAME}"
	else
		MODE="code"
		DETECTION_REASON="no reliable prompt target found; defaulting to code fix"
	fi
else
	DETECTION_REASON="mode provided by user: ${MODE}"
fi

if [[ "$MODE" != "code" ]]; then
	[[ -n "$VENDOR" ]] || {
		echo "Unable to determine vendor; pass --vendor explicitly" >&2
		exit 1
	}
	[[ -n "$PROMPT_NAME" ]] || {
		echo "Unable to determine prompt; pass --prompt explicitly" >&2
		exit 1
	}
fi

PROMPT_CONTEXT=""
if [[ "$MODE" != "code" ]]; then
	BASE_PROMPT="${ROOT_DIR}/prompts/vendors/default/${PROMPT_NAME}.md"
	VENDOR_PROMPT="${ROOT_DIR}/prompts/vendors/${VENDOR}/${PROMPT_NAME}.md"
	[[ -f "$VENDOR_PROMPT" ]] || {
		echo "Vendor prompt not found: $VENDOR_PROMPT" >&2
		exit 1
	}
	BASE_TEXT=""
	[[ -f "$BASE_PROMPT" ]] && BASE_TEXT="$(sed -n '1,220p' "$BASE_PROMPT")"
	VENDOR_TEXT="$(sed -n '1,260p' "$VENDOR_PROMPT")"
	CODE_HINTS="$(rg -n "$PROMPT_NAME|${PROMPT_NAME}.md|prompt_file=.*${PROMPT_NAME}|prompt_name=.*${PROMPT_NAME}" "$ROOT_DIR/crates" 2>/dev/null || true)"
	MODEL_IO_EXCERPT=""
	if [[ -f "$MODEL_IO_LOG" ]]; then
		MODEL_IO_EXCERPT="$(tail -n "$LINES" "$MODEL_IO_LOG" | rg -i "${VENDOR}|${PROMPT_NAME}" -n || true)"
	fi
	PROMPT_CONTEXT=$(
		cat <<EOF
Prompt target vendor: ${VENDOR}
Prompt target file: ${VENDOR_PROMPT}
Base prompt file: ${BASE_PROMPT}

Current vendor prompt:
aaaVENDOR_PROMPT_STARTaaa
${VENDOR_TEXT}
aaaVENDOR_PROMPT_ENDaaa

Base prompt reference:
aaaBASE_PROMPT_STARTaaa
${BASE_TEXT}
aaaBASE_PROMPT_ENDaaa

Relevant code references:
aaaCODE_HINTS_STARTaaa
${CODE_HINTS}
aaaCODE_HINTS_ENDaaa

Recent model I/O hints:
aaaMODEL_IO_STARTaaa
${MODEL_IO_EXCERPT}
aaaMODEL_IO_ENDaaa
EOF
	)
fi

TASK_BLOCK=""
case "$MODE" in
code)
	TASK_BLOCK=$(
		cat <<'EOF'
Task mode: CODE FIX ONLY
- Focus on code/runtime/build failures from the log excerpt.
- You may edit Rust/Python/shell/config files as needed.
- Do not rewrite vendor prompt files unless absolutely required to fix the logged failure.
EOF
	)
	;;
prompt)
	TASK_BLOCK=$(
		cat <<'EOF'
Task mode: PROMPT TUNING ONLY
- Focus on improving the selected vendor prompt file.
- Do not edit Rust/source code unless the prompt file path is plainly wrong and cannot be tuned otherwise.
- Preserve output format compatibility with the current parser and caller expectations.
- Keep changes scoped to the selected vendor prompt file when possible.
EOF
	)
	;;
both)
	TASK_BLOCK=$(
		cat <<'EOF'
Task mode: CODE + PROMPT FIX
- You may fix code and the selected vendor prompt together.
- Prefer the smallest change set that resolves the issue.
- If the issue is prompt-quality only, avoid unnecessary code edits.
EOF
	)
	;;
esac

cat >"$PROMPT_FILE" <<EOF
You are fixing behavior in the RustClaw repository.

Workspace: ${ROOT_DIR}

Auto decision summary:
- selected_mode: ${MODE}
- reason: ${DETECTION_REASON}
- vendor: ${VENDOR:-n/a}
- prompt: ${PROMPT_NAME:-n/a}
- log_file: ${LOG_FILE}

${TASK_BLOCK}

Global goals:
1. Infer the most likely root cause from the available evidence.
2. Inspect the relevant code and/or prompt files.
3. Apply a minimal, targeted fix.
4. Preserve existing behavior outside the fix.
5. Summarize root cause, files changed, and any verification you performed.

Constraints:
- Work only inside this repository.
- Respect existing uncommitted changes.
- Prefer small, precise edits over broad rewrites.
- If output format is strict JSON/labels/single-sentence, preserve parser compatibility.

Recent git status:
${GIT_STATUS}

Recent runtime/build log excerpt:
aaaLOG_STARTaaa
${RUNTIME_EXCERPT}
aaaLOG_ENDaaa

${PROMPT_CONTEXT}
EOF

if [[ -n "$EXTRA_PROMPT" ]]; then
	{
		echo
		echo "Extra instructions:"
		echo "$EXTRA_PROMPT"
	} >>"$PROMPT_FILE"
fi

printf 'Auto decision: mode=%s' "$MODE"
if [[ -n "$VENDOR" ]]; then
	printf ' vendor=%s' "$VENDOR"
fi
if [[ -n "$PROMPT_NAME" ]]; then
	printf ' prompt=%s' "$PROMPT_NAME"
fi
printf '\n'
printf 'Reason: %s\n' "$DETECTION_REASON"
printf 'Prompt file: %s\n' "$PROMPT_FILE"

if [[ "$DRY_RUN" == "1" ]]; then
	sed -n '1,260p' "$PROMPT_FILE"
	exit 0
fi

CODEX_ARGS=(exec -C "$ROOT_DIR" -o "$OUTPUT_FILE")
if [[ "$DANGEROUS" == "1" ]]; then
	CODEX_ARGS+=(--dangerously-bypass-approvals-and-sandbox)
else
	CODEX_ARGS+=(--full-auto)
fi
if [[ -n "$MODEL" ]]; then
	CODEX_ARGS+=(--model "$MODEL")
fi
CODEX_ARGS+=(-)

printf 'Running: codex %s\n' "${CODEX_ARGS[*]}"
codex "${CODEX_ARGS[@]}" <"$PROMPT_FILE"

echo
echo "Prompt: $PROMPT_FILE"
echo "Last message: $OUTPUT_FILE"

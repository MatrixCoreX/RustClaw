#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/lib.sh"

DEFAULT_CASE_FILE="${SCRIPT_DIR}/regression_user_instruction_cases.txt"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-1}"
MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-180}"
PRINT_FULL_TEXT=0
USE_DEFAULT_CASES=1

CASE_SUITES=()
CASE_NAMES=()
CASE_TAGS=()
CASE_PROMPTS=()
SUITE_FILTERS=()

RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
OUTPUT_DIR="${ROOT_DIR}/logs/regression_user_instruction_${RUN_STAMP}"
RESULTS_JSONL="${OUTPUT_DIR}/results.jsonl"
PROMPT_CANDIDATES_LOG="${OUTPUT_DIR}/prompt_candidates.log"
GUARD_CANDIDATES_LOG="${OUTPUT_DIR}/guard_candidates.log"
UNRESOLVED_LOG="${OUTPUT_DIR}/unresolved.log"
RUN_LOG="${OUTPUT_DIR}/run.log"
RAW_DIR="${OUTPUT_DIR}/raw"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1"
    exit 2
  }
}

usage() {
  cat <<'EOF'
Usage:
  bash scripts/regression_user_instruction.sh [options]

Options:
  --base-url URL         Task API base url (default from BASE_URL or lib.sh)
  --user-id ID           Base user id used when submitting tasks
  --chat-id ID           Base chat id; script derives isolated chat ids per case
  --user-key KEY         User key used for submit/query auth and identity
  --suite NAME           Run only one suite (can repeat): chat, act, chat_act, failure, file, crypto, resume
  --case-file PATH       Read cases from file: suite|name|tags|prompt
  --no-defaults          Run only explicitly provided case files
  --wait-seconds N       Max wait seconds per case (default: 180)
  --poll-seconds N       Poll interval seconds (default: 1)
  --full-text            Print full text instead of compact summary
  -h, --help             Show this help

Artifacts:
  - logs/regression_user_instruction_<timestamp>/run.log
  - results.jsonl
  - prompt_candidates.log
  - guard_candidates.log
  - unresolved.log

Default flow:
  - auto-detect first enabled admin key when --user-key is not provided
  - run single-turn user-operation cases from scripts/regression_user_instruction_cases.txt
  - run a dedicated multi-turn resume/follow-up suite
  - classify findings into prompt / guard / unresolved buckets
EOF
}

add_case() {
  local suite="$1"
  local name="$2"
  local tags="$3"
  local prompt="$4"
  CASE_SUITES+=("$suite")
  CASE_NAMES+=("$name")
  CASE_TAGS+=("$tags")
  CASE_PROMPTS+=("$prompt")
}

load_case_file() {
  local case_file="$1"
  python3 - "$case_file" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
for idx, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = [part.strip() for part in line.split("|", 3)]
    if len(parts) != 4:
        raise SystemExit(f"invalid case line {idx}: expected suite|name|tags|prompt")
    suite, name, tags, prompt = parts
    if not suite or not name or not prompt:
        raise SystemExit(f"invalid case line {idx}: suite/name/prompt required")
    print(f"{suite}\x1f{name}\x1f{tags}\x1f{prompt}")
PY
}

load_default_cases() {
  if [[ ! -f "$DEFAULT_CASE_FILE" ]]; then
    echo "Default case file not found: $DEFAULT_CASE_FILE" >&2
    exit 2
  fi
  local suite name tags prompt
  while IFS=$'\x1f' read -r suite name tags prompt; do
    add_case "$suite" "$name" "$tags" "$prompt"
  done < <(load_case_file "$DEFAULT_CASE_FILE")
}

case_selected() {
  local suite="$1"
  if [[ "${#SUITE_FILTERS[@]}" -eq 0 ]]; then
    return 0
  fi
  local item
  for item in "${SUITE_FILTERS[@]}"; do
    if [[ "$item" == "$suite" ]]; then
      return 0
    fi
  done
  return 1
}

resolve_admin_key() {
  if [[ -n "${USER_KEY:-}" ]]; then
    return 0
  fi
  USER_KEY="$("${SCRIPT_DIR}/auth-key.sh" list | awk '$2 == "admin" && $3 == "enabled" { print $1; exit }')"
  if [[ -z "${USER_KEY:-}" ]]; then
    echo "No enabled admin key found. Pass --user-key explicitly." >&2
    exit 2
  fi
}

summarize_text() {
  local text="${1:-}"
  local limit="${2:-160}"
  python3 - "$text" "$limit" <<'PY'
import sys
text = sys.argv[1]
limit = int(sys.argv[2])
text = " ".join((text or "").split())
if len(text) > limit:
    text = text[:limit] + "...(truncated)"
print(text)
PY
}

extract_poll_fields() {
  local raw_file="$1"
  python3 - "$raw_file" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
data = obj.get("data") or {}
result = data.get("result_json") or {}
status = str(data.get("status") or "")
progress = result.get("progress_messages") or []
if not isinstance(progress, list):
    progress = []
print(f"{status}\t{len(progress)}")
for item in progress:
    text = str(item or "").replace("\r", " ").replace("\n", "\\n").replace("\t", " ")
    print(text)
PY
}

poll_task_with_progress() {
  local task_id="$1"
  local raw_file="$2"
  local waited=0
  local last_status=""
  local progress_seen=0

  while [[ "$waited" -le "$MAX_WAIT_SECONDS" ]]; do
    local raw parsed status progress_count
    raw="$(query_task "$task_id")"
    printf '%s\n' "$raw" > "$raw_file"
    parsed="$(extract_poll_fields "$raw_file")"
    status="$(printf '%s\n' "$parsed" | awk -F'\t' 'NR==1{print $1}')"
    progress_count="$(printf '%s\n' "$parsed" | awk -F'\t' 'NR==1{print $2}')"

    if [[ "$status" != "$last_status" ]]; then
      echo "  [status] ${last_status:-<none>} -> ${status:-<empty>}"
      last_status="$status"
    fi

    if [[ "$progress_count" =~ ^[0-9]+$ ]] && (( progress_count > progress_seen )); then
      local idx msg summary
      for (( idx = progress_seen; idx < progress_count; idx++ )); do
        msg="$(printf '%s\n' "$parsed" | sed -n "$((idx + 2))p")"
        summary="$(summarize_text "$msg" 180)"
        echo "  [progress:$((idx + 1))] $summary"
      done
      progress_seen="$progress_count"
    fi

    case "$status" in
      succeeded|failed|canceled|timeout)
        return 0
        ;;
      *)
        sleep "$POLL_INTERVAL_SECONDS"
        waited=$((waited + POLL_INTERVAL_SECONDS))
        ;;
    esac
  done

  echo "poll timeout for task_id=${task_id}" >&2
  return 1
}

print_result_summary() {
  local raw_file="$1"
  local mode="${2:-summary}"
  python3 - "$raw_file" "$mode" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
mode = sys.argv[2]
data = obj.get("data") or {}
result = data.get("result_json") or {}
status = data.get("status") or ""
error_text = data.get("error_text") or ""
text = result.get("text") or ""
messages = result.get("messages") or []
progress = result.get("progress_messages") or []
resume_context = result.get("resume_context")

def compact(s, limit=220):
    s = " ".join(str(s).split())
    if len(s) > limit:
        return s[:limit] + "...(truncated)"
    return s

print(f"  [final] status={status}")
if error_text:
    print(f"  [error] {compact(error_text)}")
if text:
    if mode == "full":
        print("  [text]")
        print(text)
    else:
        print(f"  [text] {compact(text)}")
if messages:
    print(f"  [messages] count={len(messages)}")
    for idx, msg in enumerate(messages, start=1):
        if mode == "full":
            print(f"    - [{idx}]")
            print(msg)
        else:
            print(f"    - [{idx}] {compact(msg)}")
if progress:
    print(f"  [progress_total] count={len(progress)}")
if resume_context is not None:
    print("  [resume_context] yes")
PY
}

print_clawd_trace() {
  local task_id="$1"
  python3 - "${ROOT_DIR}/logs/clawd.log" "$task_id" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
task_id = sys.argv[2]
if not path.exists():
    print("  [trace] logs/clawd.log not found")
    raise SystemExit(0)

keywords = [
    "task_call_begin",
    "worker_once: ask task_id=",
    "route_request_mode",
    "prompt_invocation",
    "executor_step_execute",
    "executor_result_ok",
    "executor_result_error",
    "task_call_end",
    "bind_resume_context",
]

def compact(line: str, limit: int = 220) -> str:
    line = " ".join(line.split())
    if len(line) > limit:
        return line[:limit] + "...(truncated)"
    return line

hits = []
for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
    if task_id not in raw:
        continue
    if any(k in raw for k in keywords):
        hits.append(compact(raw))

if not hits:
    print("  [trace] no clawd.log lines matched task_id")
else:
    print("  [trace] clawd.log")
    for line in hits:
        print(f"    {line}")
PY
}

print_model_io_summary() {
  local task_id="$1"
  python3 - "${ROOT_DIR}/logs/model_io.log" "$task_id" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
task_id = sys.argv[2]
if not path.exists():
    print("  [llm] logs/model_io.log not found")
    raise SystemExit(0)

def compact(s: str, limit: int = 180) -> str:
    s = " ".join((s or "").split())
    if len(s) > limit:
        return s[:limit] + "...(truncated)"
    return s

def prompt_head(prompt: str) -> str:
    lines = (prompt or "").splitlines()
    for line in lines:
        t = line.strip()
        if not t or t in ("<!--", "-->"):
            continue
        return compact(t, 120)
    return "<empty>"

rows = []
for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
    raw = raw.strip()
    if not raw:
        continue
    try:
        obj = json.loads(raw)
    except Exception:
        continue
    if str(obj.get("task_id") or "") != task_id:
        continue
    rows.append(obj)

if not rows:
    print("  [llm] no model_io.log rows matched task_id")
else:
    print("  [llm] model_io.log")
    for idx, row in enumerate(rows, start=1):
        status = row.get("status") or ""
        model = row.get("model") or ""
        phead = prompt_head(row.get("prompt") or "")
        rhead = compact(row.get("response") or "", 160)
        ehead = compact(row.get("error") or "", 160)
        print(f"    [{idx}] status={status} model={model} prompt={phead}")
        if rhead:
            print(f"         response={rhead}")
        if ehead:
            print(f"         error={ehead}")
PY
}

analyze_case_json() {
  local suite="$1"
  local name="$2"
  local tags_csv="$3"
  local prompt="$4"
  local task_id="$5"
  local raw_file="$6"

  python3 - "$suite" "$name" "$tags_csv" "$prompt" "$task_id" "$raw_file" "${ROOT_DIR}" <<'PY'
import json
import re
import sys
from pathlib import Path

suite, name, tags_csv, prompt, task_id, raw_file, root_dir = sys.argv[1:8]
tags = {item.strip() for item in tags_csv.split(',') if item.strip()}
raw = json.loads(Path(raw_file).read_text(encoding='utf-8'))
clawd_path = Path(root_dir) / 'logs' / 'clawd.log'
model_io_path = Path(root_dir) / 'logs' / 'model_io.log'

data = raw.get('data') or {}
result = data.get('result_json') or {}
status = str(data.get('status') or '')
error_text = str(data.get('error_text') or '')
text = str(result.get('text') or '')
messages = result.get('messages') or []
progress = result.get('progress_messages') or []
resume_context = result.get('resume_context')
joined_output = '\n'.join([str(x) for x in progress] + [str(x) for x in messages] + [text])
combined_trace = joined_output

clawd_lines = []
if clawd_path.exists():
    clawd_lines = [line for line in clawd_path.read_text(encoding='utf-8', errors='replace').splitlines() if task_id in line]
combined_trace = joined_output + '\n' + '\n'.join(clawd_lines)

model_rows = []
if model_io_path.exists():
    for raw_line in model_io_path.read_text(encoding='utf-8', errors='replace').splitlines():
        raw_line = raw_line.strip()
        if not raw_line:
            continue
        try:
            row = json.loads(raw_line)
        except Exception:
            continue
        if str(row.get('task_id') or '') == task_id:
            model_rows.append(row)

route_chat = any('routed_mode=Chat' in line for line in clawd_lines)
route_act = any('routed_mode=Act' in line for line in clawd_lines)
route_chat_act = any('routed_mode=ChatAct' in line for line in clawd_lines)
route_clarify = any('routed_mode=AskClarify' in line for line in clawd_lines)
has_exec = any('executor_step_execute' in line for line in clawd_lines)
has_write_file = any('tool=write_file' in line for line in clawd_lines) or bool(re.search(r'written \d+ bytes to ', combined_trace, re.I))
used_crypto = any('tool=crypto' in line for line in clawd_lines) or bool(re.search(r'\btrade_preview\b|\btrade_submit\b|BTCUSDT|ETHUSDT|SMA|USDT|CoinDesk|CoinTelegraph|order_id|positions?|open orders?|cancel', combined_trace, re.I))
final_delivery = bool(re.search(r'(?:FILE:|IMAGE_FILE:)\S+', text))
not_found_text = bool(re.search('(\u6ca1\u627e\u5230|\u672a\u627e\u5230|not found|does not exist|no such file)', text, re.I))
trade_submit_detected = bool(re.search(r'trade_submit|trade_submitted', combined_trace, re.I))
trade_preview_detected = bool(re.search('trade_preview|awaiting_confirmation|\u9884\u89c8|\u98ce\u9669\u63d0\u793a|\u786e\u8ba4\u540e\u6267\u884c', combined_trace, re.I))
route_fallback = any('route_request_mode llm failed' in line or 'route_request_mode parse failed' in line for line in clawd_lines)
duplicate_final = bool(progress) and text.strip() and str(progress[-1]).strip() == text.strip()
response_mentions_remaining = bool(re.search('(\u5269\u4f59|remaining|\u8fd8\u5269|undone|pending)', text, re.I))
response_mentions_failure = bool(re.search('(\u5931\u8d25|\u62a5\u9519|failed|error)', text, re.I))

issues = {'prompt': [], 'guard': [], 'unresolved': []}

def add(bucket, message):
    issues[bucket].append(message)

if route_fallback:
    add('prompt', 'route_request_mode fell back; review router prompt / parser output first.')
if duplicate_final:
    add('prompt', 'final text duplicates the last progress message and may show twice to the user.')

if 'chat_only' in tags and (has_exec or route_act or route_chat_act):
    add('prompt', 'chat-only case still triggered an execution path.')

allow_clarify = 'allow_clarify' in tags
if 'exec' in tags and not has_exec and not (allow_clarify and route_clarify):
    add('prompt', 'case expected execution, but no executor_step_execute was observed.')

if 'no_write_file' in tags and has_write_file:
    add('guard', 'user explicitly asked for reply-only behavior, but write_file still happened.')

if 'deliver_file' in tags and not final_delivery:
    add('prompt', 'user asked to send a file, but no FILE/IMAGE_FILE delivery marker appeared.')

if 'missing_file_graceful' in tags:
    if status != 'succeeded':
        add('prompt', 'missing-file request did not converge to a graceful succeeded response.')
    if not not_found_text:
        add('prompt', 'missing-file request did not clearly explain that the file was not found.')
    if final_delivery:
        add('guard', 'missing-file request still produced a file delivery marker.')

if 'write_and_deliver' in tags:
    if not has_write_file:
        add('prompt', 'request asked to generate and send a file, but no write_file signal appeared.')
    if not final_delivery:
        add('prompt', 'request asked to send a generated file, but no FILE/IMAGE_FILE delivery marker appeared.')

if 'tool_crypto' in tags and not used_crypto and not (allow_clarify and route_clarify):
    add('prompt', 'crypto natural-language request did not appear to reach crypto behavior.')

if 'preview_only' in tags:
    if trade_submit_detected:
        add('guard', 'preview-only trading case showed trade_submit / trade_submitted signals.')
    if not (trade_preview_detected or route_clarify or used_crypto):
        add('prompt', 'preview-only trading case did not end in a preview / confirmation state.')

if 'resume_context' in tags and status in {'failed', 'timeout', 'canceled'} and resume_context is None:
    add('prompt', 'failed multi-step task did not return resume_context.')

if 'followup_explain_only' in tags:
    if not response_mentions_failure or not response_mentions_remaining:
        add('prompt', 'explain-only follow-up did not clearly describe both the failed step and the remaining step.')
    if has_exec and not route_clarify:
        add('prompt', 'explain-only follow-up still executed work instead of only discussing context.')

if 'followup_resume_with_change' in tags:
    old_tokens = re.findall(r'AFTER_OLD_[A-Z0-9_]+', prompt)
    new_tokens = re.findall(r'AFTER_NEW_[A-Z0-9_]+', prompt)
    response_mentions_old = any(token in combined_trace for token in old_tokens)
    response_mentions_new = any(token in combined_trace for token in new_tokens)
    if not has_exec:
        add('prompt', 'resume-with-change follow-up did not execute the remaining step.')
    if not response_mentions_new:
        add('prompt', 'resume-with-change follow-up never reflected the new replacement step token.')
    if response_mentions_old:
        add('guard', 'resume-with-change follow-up still referenced the old step token after the change request.')

if 'no_post_fail_steps' in tags:
    echo_tokens = re.findall(r'echo\s+([A-Za-z0-9_]+)', prompt)
    if len(echo_tokens) >= 2:
        tail_token = echo_tokens[-1]
        if tail_token and tail_token in combined_trace:
            add('guard', f'post-failure step token {tail_token} still appeared after the break point.')

allowed_failure = 'resume_context' in tags and status in {'failed', 'timeout', 'canceled'} and resume_context is not None
clarify_ok = allow_clarify and route_clarify and status == 'succeeded'

classification = 'pass'
if issues['guard']:
    classification = 'guard'
elif issues['prompt']:
    classification = 'prompt'
elif status in {'failed', 'timeout', 'canceled'} and not allowed_failure:
    classification = 'unresolved'
elif status not in {'succeeded', 'failed', 'timeout', 'canceled'}:
    classification = 'unresolved'
elif status == 'succeeded' or allowed_failure or clarify_ok:
    classification = 'pass'
else:
    classification = 'unresolved'

if classification == 'unresolved' and not issues['unresolved']:
    if error_text:
        add('unresolved', f'terminal status={status}; manual review needed: {error_text}')
    else:
        add('unresolved', f'terminal status={status}; heuristics did not isolate a clearer cause.')

summary = {
    'suite': suite,
    'name': name,
    'tags': sorted(tags),
    'task_id': task_id,
    'status': status,
    'classification': classification,
    'route': {
        'chat': route_chat,
        'act': route_act,
        'chat_act': route_chat_act,
        'clarify': route_clarify,
    },
    'signals': {
        'has_exec': has_exec,
        'has_write_file': has_write_file,
        'used_crypto': used_crypto,
        'final_delivery': final_delivery,
        'resume_context': resume_context is not None,
        'llm_calls': len(model_rows),
    },
    'issues': issues,
}
print(json.dumps(summary, ensure_ascii=False))
PY
}

render_case_analysis() {
  local analysis_json="$1"
  python3 - "$analysis_json" <<'PY'
import json
import sys
obj = json.loads(sys.argv[1])
print(f"  [analysis] classification={obj['classification']} status={obj['status']} llm_calls={obj['signals']['llm_calls']}")
print(
    "  [signals] "
    f"exec={'yes' if obj['signals']['has_exec'] else 'no'} "
    f"write_file={'yes' if obj['signals']['has_write_file'] else 'no'} "
    f"crypto={'yes' if obj['signals']['used_crypto'] else 'no'} "
    f"delivery={'yes' if obj['signals']['final_delivery'] else 'no'} "
    f"resume={'yes' if obj['signals']['resume_context'] else 'no'}"
)
print("  [issues]")
any_issue = False
for bucket in ('prompt', 'guard', 'unresolved'):
    for item in obj['issues'][bucket]:
        any_issue = True
        print(f"    - [{bucket}] {item}")
if not any_issue:
    print("    - no heuristic issues detected")
PY
}

append_bucket_logs() {
  local analysis_json="$1"
  local prompt="$2"
  python3 - "$analysis_json" "$prompt" "$PROMPT_CANDIDATES_LOG" "$GUARD_CANDIDATES_LOG" "$UNRESOLVED_LOG" <<'PY'
import json
import sys
from pathlib import Path

obj = json.loads(sys.argv[1])
prompt = sys.argv[2]
prompt_log = Path(sys.argv[3])
guard_log = Path(sys.argv[4])
unresolved_log = Path(sys.argv[5])

for bucket, path in [('prompt', prompt_log), ('guard', guard_log), ('unresolved', unresolved_log)]:
    items = obj['issues'][bucket]
    if bucket == 'unresolved' and obj['classification'] == 'unresolved' and not items:
        items = [f"terminal status={obj['status']} with no finer-grained heuristic result"]
    if not items:
        continue
    with path.open('a', encoding='utf-8') as fh:
        fh.write(f"[{obj['suite']}] {obj['name']} task_id={obj['task_id']} status={obj['status']} class={obj['classification']}\n")
        fh.write(f"prompt: {prompt}\n")
        for item in items:
            fh.write(f"- {item}\n")
        fh.write("\n")
PY
}

prepare_output_dir() {
  mkdir -p "$OUTPUT_DIR" "$RAW_DIR"
  : > "$RESULTS_JSONL"
  printf 'Prompt-first candidates\n\n' > "$PROMPT_CANDIDATES_LOG"
  printf 'Hard-guard candidates\n\n' > "$GUARD_CANDIDATES_LOG"
  printf 'Unresolved items\n\n' > "$UNRESOLVED_LOG"
  exec > >(tee -a "$RUN_LOG") 2>&1
}

run_one_case() {
  local suite="$1"
  local name="$2"
  local tags="$3"
  local prompt="$4"
  local chat_id_override="$5"
  local raw_file="${RAW_DIR}/${name}.json"
  local old_chat_id="$CHAT_ID"
  local submit_resp task_id analysis_json

  CHAT_ID="$chat_id_override"

  echo
  echo "============================================================"
  echo "[CASE] ${name}"
  echo "[SUITE] ${suite}"
  echo "[TAGS] ${tags}"
  echo "[CHAT] ${CHAT_ID}"
  echo "[PROMPT] ${prompt}"

  submit_resp="$(submit_task "$prompt")"
  task_id="$(extract_submit_task_id "$submit_resp")"
  echo "[TASK] ${task_id}"
  echo "[POLL]"
  poll_task_with_progress "$task_id" "$raw_file"

  echo "[RESULT]"
  if [[ "$PRINT_FULL_TEXT" == "1" ]]; then
    print_result_summary "$raw_file" "full"
  else
    print_result_summary "$raw_file" "summary"
  fi

  echo "[TRACE]"
  print_clawd_trace "$task_id"

  echo "[LLM]"
  print_model_io_summary "$task_id"

  echo "[CHECK]"
  analysis_json="$(analyze_case_json "$suite" "$name" "$tags" "$prompt" "$task_id" "$raw_file")"
  render_case_analysis "$analysis_json"
  printf '%s\n' "$analysis_json" >> "$RESULTS_JSONL"
  append_bucket_logs "$analysis_json" "$prompt"

  CHAT_ID="$old_chat_id"
}

run_resume_suite() {
  local base_resume_chat_id="$1"
  local token="USER_OPS_${RUN_STAMP}"
  local fail_prompt explain_prompt continue_prompt

  fail_prompt="??? echo BEFORE_${token}???? definitely_missing_command_${token}???? echo AFTER_OLD_${token}"
  explain_prompt="????????????????????????????????????? AFTER_OLD_${token}?"
  continue_prompt="?????????????? echo AFTER_NEW_${token}???????? AFTER_OLD_${token}?"

  run_one_case "resume" "resume_seed_failure" "exec,resume_context,no_post_fail_steps" "$fail_prompt" "$base_resume_chat_id"
  run_one_case "resume" "resume_explain_only" "followup_explain_only" "$explain_prompt" "$base_resume_chat_id"
  run_one_case "resume" "resume_continue_patched" "followup_resume_with_change" "$continue_prompt" "$base_resume_chat_id"
}

print_final_summary() {
  python3 - "$RESULTS_JSONL" <<'PY'
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

path = Path(sys.argv[1])
rows = []
for raw in path.read_text(encoding='utf-8').splitlines():
    raw = raw.strip()
    if not raw:
        continue
    rows.append(json.loads(raw))

by_class = Counter(row['classification'] for row in rows)
by_suite = defaultdict(Counter)
for row in rows:
    by_suite[row['suite']][row['classification']] += 1

print()
print('================ Final Summary ================')
print(f"TOTAL_CASES={len(rows)} PASS={by_class.get('pass', 0)} PROMPT={by_class.get('prompt', 0)} GUARD={by_class.get('guard', 0)} UNRESOLVED={by_class.get('unresolved', 0)}")
for suite in sorted(by_suite):
    counter = by_suite[suite]
    print(
        f"  - {suite}: total={sum(counter.values())} "
        f"pass={counter.get('pass', 0)} prompt={counter.get('prompt', 0)} "
        f"guard={counter.get('guard', 0)} unresolved={counter.get('unresolved', 0)}"
    )
PY

  echo
  echo "Artifacts:"
  echo "  - ${RUN_LOG}"
  echo "  - ${RESULTS_JSONL}"
  echo "  - ${PROMPT_CANDIDATES_LOG}"
  echo "  - ${GUARD_CANDIDATES_LOG}"
  echo "  - ${UNRESOLVED_LOG}"
}

main() {
  local case_file=""

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --base-url)
        BASE_URL="${2:-}"
        shift 2
        ;;
      --user-id)
        USER_ID="${2:-}"
        shift 2
        ;;
      --chat-id)
        CHAT_ID="${2:-}"
        shift 2
        ;;
      --user-key)
        USER_KEY="${2:-}"
        shift 2
        ;;
      --suite)
        SUITE_FILTERS+=("${2:-}")
        shift 2
        ;;
      --case-file)
        case_file="${2:-}"
        shift 2
        ;;
      --no-defaults)
        USE_DEFAULT_CASES=0
        shift
        ;;
      --wait-seconds)
        MAX_WAIT_SECONDS="${2:-}"
        shift 2
        ;;
      --poll-seconds)
        POLL_INTERVAL_SECONDS="${2:-}"
        shift 2
        ;;
      --full-text)
        PRINT_FULL_TEXT=1
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        echo "Unknown argument: $1"
        usage
        exit 2
        ;;
    esac
  done

  need_cmd curl
  need_cmd python3
  need_cmd awk
  need_cmd sed
  need_cmd tee

  resolve_admin_key

  if [[ "$USE_DEFAULT_CASES" == "1" ]]; then
    load_default_cases
  fi

  if [[ -n "$case_file" ]]; then
    local suite name tags prompt
    while IFS=$'\x1f' read -r suite name tags prompt; do
      add_case "$suite" "$name" "$tags" "$prompt"
    done < <(load_case_file "$case_file")
  fi

  if [[ "${#CASE_NAMES[@]}" -eq 0 ]]; then
    echo "No cases to run."
    exit 2
  fi

  prepare_output_dir

  BASE_USER_ID="$USER_ID"
  BASE_CHAT_ID="$CHAT_ID"

  echo "== User instruction regression suite =="
  echo "BASE_URL=${BASE_URL}"
  echo "BASE_USER_ID=${BASE_USER_ID} BASE_CHAT_ID=${BASE_CHAT_ID} USER_KEY_SET=$( [[ -n "${USER_KEY:-}" ]] && echo yes || echo no )"
  echo "WAIT=${MAX_WAIT_SECONDS}s POLL=${POLL_INTERVAL_SECONDS}s"
  if [[ "${#SUITE_FILTERS[@]}" -gt 0 ]]; then
    echo "SUITE_FILTERS=${SUITE_FILTERS[*]}"
  else
    echo "SUITE_FILTERS=<all>"
  fi
  echo "OUTPUT_DIR=${OUTPUT_DIR}"

  health_check

  local i run_index=0 suite name tags prompt chat_id_for_case
  for (( i = 0; i < ${#CASE_NAMES[@]}; i++ )); do
    suite="${CASE_SUITES[$i]}"
    name="${CASE_NAMES[$i]}"
    tags="${CASE_TAGS[$i]}"
    prompt="${CASE_PROMPTS[$i]}"
    if ! case_selected "$suite"; then
      continue
    fi
    run_index=$((run_index + 1))
    chat_id_for_case=$((BASE_CHAT_ID + run_index))
    run_one_case "$suite" "$name" "$tags" "$prompt" "$chat_id_for_case"
  done

  if case_selected "resume"; then
    run_resume_suite "$((BASE_CHAT_ID + 9000))"
  fi

  print_final_summary
}

main "$@"

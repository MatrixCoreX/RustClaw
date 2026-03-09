#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/document/regression_llm_first/lib.sh"

POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-1}"
MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-180}"
PRINT_FULL_TEXT=0
USE_DEFAULT_CASES=1
DEFAULT_CASE_FILE="${SCRIPT_DIR}/regression_trace_ask_cases_real.txt"

CASE_NAMES=()
CASE_PROMPTS=()

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1"
    exit 2
  }
}

usage() {
  cat <<'EOF'
Usage:
  bash scripts/regression_trace_ask.sh [options]

Options:
  --base-url URL         Task API base url (default from BASE_URL or lib.sh)
  --user-id ID           User id used when submitting tasks
  --chat-id ID           Chat id used when submitting tasks
  --wait-seconds N       Max wait seconds per case (default: 180)
  --poll-seconds N       Poll interval seconds (default: 1)
  --prompt TEXT          Add one custom case (can repeat)
  --case-file PATH       Read cases from file: "name|prompt" or just "prompt"
  --no-defaults          Run only custom/file cases
  --full-text            Print full text instead of compact summary
  -h, --help             Show this help

Default cases:
  - loaded from scripts/regression_trace_ask_cases_real.txt

What it prints:
  - submit / polling progress
  - progress_messages 增量
  - final status / text / messages
  - clawd.log 路由与执行摘要
  - model_io.log 每次 LLM prompt/response/error 摘要
  - 简单问题提示（如：命令请求却路由到 Chat）
EOF
}

add_case() {
  local name="$1"
  local prompt="$2"
  CASE_NAMES+=("$name")
  CASE_PROMPTS+=("$prompt")
}

load_default_cases() {
  if [[ ! -f "$DEFAULT_CASE_FILE" ]]; then
    echo "Default case file not found: $DEFAULT_CASE_FILE" >&2
    exit 2
  fi
  local line name prompt
  while IFS=$'\t' read -r name prompt; do
    [[ -n "$prompt" ]] || continue
    add_case "$name" "$prompt"
  done < <(load_case_file "$DEFAULT_CASE_FILE")
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
    if "|" in line:
        name, prompt = line.split("|", 1)
        name = name.strip() or f"case_{idx}"
        prompt = prompt.strip()
    else:
        name = f"case_{idx}"
        prompt = line
    if prompt:
        print(f"{name}\t{prompt}")
PY
}

summarize_text() {
  local text="${1:-}"
  local limit="${2:-140}"
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
  local raw="$1"
  python3 - "$raw" <<'PY'
import json
import sys

raw = sys.argv[1]
obj = json.loads(raw)
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
  local waited=0
  local last_status=""
  local progress_seen=0

  while [[ "$waited" -le "$MAX_WAIT_SECONDS" ]]; do
    local raw parsed status progress_count
    raw="$(query_task "$task_id")"
    parsed="$(extract_poll_fields "$raw")"
    status="$(printf '%s\n' "$parsed" | awk -F'\t' 'NR==1{print $1}')"
    progress_count="$(printf '%s\n' "$parsed" | awk -F'\t' 'NR==1{print $2}')"

    if [[ "$status" != "$last_status" ]]; then
      echo "  [status] ${last_status:-<none>} -> ${status:-<empty>}" >&2
      last_status="$status"
    fi

    if [[ "$progress_count" =~ ^[0-9]+$ ]] && (( progress_count > progress_seen )); then
      local idx msg summary
      for (( idx = progress_seen; idx < progress_count; idx++ )); do
        msg="$(printf '%s\n' "$parsed" | sed -n "$((idx + 2))p")"
        summary="$(summarize_text "$msg" 160)"
        echo "  [progress:$((idx + 1))] $summary" >&2
      done
      progress_seen="$progress_count"
    fi

    case "$status" in
      succeeded|failed|canceled|timeout)
        printf '%s\n' "$raw"
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
  local raw="$1"
  local mode="${2:-summary}"
  python3 - "$raw" "$mode" <<'PY'
import json
import sys

raw = sys.argv[1]
mode = sys.argv[2]
obj = json.loads(raw)
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

print_issue_hints() {
  local task_id="$1"
  local prompt="$2"
  local raw="$3"
  python3 - "${ROOT_DIR}/logs/clawd.log" "$task_id" "$prompt" "$raw" <<'PY'
import json
import re
import sys
from pathlib import Path

log_path = Path(sys.argv[1])
task_id = sys.argv[2]
prompt = sys.argv[3]
raw = sys.argv[4]

obj = json.loads(raw)
data = obj.get("data") or {}
result = data.get("result_json") or {}
status = str(data.get("status") or "")
text = result.get("text") or ""
progress = result.get("progress_messages") or []
messages = result.get("messages") or []
resume_context = result.get("resume_context")

log_text = ""
if log_path.exists():
    log_text = log_path.read_text(encoding="utf-8", errors="replace")
lines = [line for line in log_text.splitlines() if task_id in line]

route_chat = any("routed_mode=Chat" in line for line in lines)
route_act = any("routed_mode=Act" in line for line in lines)
route_chat_act = any("routed_mode=ChatAct" in line for line in lines)
route_clarify = any("routed_mode=AskClarify" in line for line in lines)
has_exec = any("executor_step_execute" in line for line in lines)
has_route_fallback = any("route_request_mode llm failed" in line or "route_request_mode parse failed" in line for line in lines)

prompt_action_like = bool(re.search(r"(执行|查看当前目录|\bls\b|\bpwd\b|\bdf\b|\brun\s+|列出|查看目录)", prompt, re.I))
final_like_env_block = "不支持命令执行" in text or "无法执行" in text
duplicate_final = bool(progress) and text.strip() and progress[-1].strip() == text.strip()
creative_mixed = bool(re.search(r"(笑话|故事|评书|段子|poem|joke|story)", prompt, re.I))
multi_step_prompt = bool(re.search(r"(先|然后|再执行|接着|并且)", prompt))
missing_cmd_case = "definitely_missing_command_rustclaw_12345" in prompt
after_fail_marker = "AFTER_FAIL_TRACE" in prompt
trace_multi_marker = "TRACE_MULTI_OK" in prompt
progress_join = "\n".join(progress + messages + ([text] if text else []))
failure_followup_like = bool(re.search(r"(哪一步失败|后面还剩|why .*failed|which step failed|what work is still remaining|what remains)", prompt, re.I))
explicit_file_intent = bool(re.search(r"(保存|存成|写入文件|文件形式|发文件|save(?:\s+it)?(?:\s+to)?\s+(?:a\s+)?file|save\b|write(?:\s+it)?\s+to\s+file|create\s+(?:a\s+)?file|send\s+(?:me\s+)?the\s+file)", prompt, re.I))
has_write_file = any("tool=write_file" in line for line in lines)
same_turn_failure_explain_like = failure_followup_like and bool(re.search(r"(先不要继续|不要继续|do not continue|don't resume yet)", prompt, re.I))
listing_based_digest_like = bool(re.search(r"(目录|directory listing|项目核心|core project)", prompt, re.I))
hidden_files_like = bool(re.search(r"(隐藏文件|点开头|dotfiles?|hidden files?|hidden entries?)", prompt, re.I))
named_file_delivery_like = bool(re.search(r"(发给我|发我|发过来|send me|send\b|deliver\b).*(?:`?)([A-Za-z0-9_.-]+\.[A-Za-z0-9_.-]+)(?:`?)", prompt, re.I))
named_missing_file_like = "definitely_missing_named_file_rustclaw_" in prompt
generated_file_delivery_like = bool(
    re.search(r"(写个|写一个|生成|create|write)\b.*(\.sh|shell script|脚本)", prompt, re.I)
    and re.search(r"(保存|存成|写入文件|save|create file)", prompt, re.I)
    and re.search(r"(发给我|发我|发过来|send me|deliver)", prompt, re.I)
)

def trim_token(raw: str) -> str:
    return raw.strip().strip("`'\"，,:：;。()（）")

def extract_planned_step_count(blobs):
    max_step = 0
    for blob in blobs:
        for match in re.finditer(r"(?:^|\n)\s*(\d+)\.", str(blob)):
            try:
                max_step = max(max_step, int(match.group(1)))
            except Exception:
                pass
    return max_step

def extract_max_executed_step(lines):
    max_step = 0
    for line in lines:
        m = re.search(r"step=(\d+)", line)
        if not m:
            continue
        try:
            max_step = max(max_step, int(m.group(1)))
        except Exception:
            pass
    return max_step

def extract_written_paths(blobs):
    out = []
    for blob in blobs:
        for match in re.finditer(r"written \d+ bytes to (\S+)", str(blob)):
            out.append(trim_token(match.group(1)))
    return out

def extract_delivery_paths(blob):
    out = []
    for match in re.finditer(r"(?:FILE:|IMAGE_FILE:)(\S+)", str(blob)):
        out.append(trim_token(match.group(1)))
    return out

def extract_listing_entries(blobs):
    entries = set()
    for blob in blobs:
        text_blob = str(blob)
        if "/" not in text_blob and "." not in text_blob:
            continue
        for token in text_blob.replace("\n", " ").split():
            cleaned = trim_token(token)
            if cleaned:
                entries.add(cleaned)
    return entries

issues = []
if prompt_action_like and route_chat and not has_exec:
    issues.append("命令/目录请求被路由到 Chat，且没有 executor_step_execute。")
if has_route_fallback:
    issues.append("检测到 route_request_mode fallback，建议核对路由模型返回。")
if prompt_action_like and final_like_env_block and not has_exec:
    issues.append("结果像是聊天兜底文案，不像真实执行失败。")
if duplicate_final:
    issues.append("final text 与最后一条 progress_messages 相同，可能造成重复显示。")
if creative_mixed and route_chat_act:
    issues.append("该 case 走了 ChatAct，需人工关注后续创作是否被前序 act 输出污染。")
if prompt_action_like and route_clarify:
    issues.append("该 case 进入 AskClarify；若目标其实清晰，可能过度澄清。")
if prompt_action_like and route_act and not has_exec:
    issues.append("路由是 Act，但没有执行步骤，可能在规划阶段提前 respond。")
if status in {"failed", "timeout", "canceled"} and multi_step_prompt and resume_context is None:
    issues.append("多步任务失败但未返回 resume_context，不利于后续继续执行。")
if missing_cmd_case and not progress:
    issues.append("中间失败 case 在失败前没有任何 progress，需确认前序步骤是否真的执行。")
if trace_multi_marker and "TRACE_MULTI_OK" not in progress_join:
    issues.append("多步成功标记 TRACE_MULTI_OK 未出现在输出中，需确认顺序执行是否完整。")
if missing_cmd_case and after_fail_marker and "AFTER_FAIL_TRACE" in progress_join:
    issues.append("失败后的后续步骤似乎仍然执行了 AFTER_FAIL_TRACE，需确认失败后是否正确中断。")
if missing_cmd_case and creative_mixed and re.search(r"(pwd|/home|RustClaw)", progress_join):
    issues.append("失败混合 chat case 里出现前序执行内容，需人工检查 chat 是否被 act 输出污染。")
if prompt.strip() == "继续" and not route_clarify and not route_act:
    issues.append("“继续” 未落到 AskClarify/Act，需检查 follow-up 路由。")
if failure_followup_like and re.search(r"(未执行任何步骤|No step failed|no prior action or execution)", text, re.I):
    issues.append("失败追问没有正确绑定到最近的 interrupted task context。")
if has_write_file and not explicit_file_intent:
    issues.append("检测到 write_file，但用户未明确要求保存/创建文件，可能把文本生成误判成文件写入。")
if generated_file_delivery_like and has_write_file and not re.search(r"(?:FILE:|IMAGE_FILE:)\S+", text):
    issues.append("用户要求先生成文件再发送，但最终没有明确返回 FILE/IMAGE_FILE 交付。")
if hidden_files_like and re.search(r"(运行 `?ls -a`?|run `?ls -a`?|是否需要我执行|do you want me to run)", text, re.I):
    issues.append("用户已经在问隐藏文件结果，但系统只是在建议/询问是否执行命令，没有直接回答。")
if named_file_delivery_like:
    requested_files = [trim_token(x) for x in re.findall(r"`?([A-Za-z0-9_.-]+\.[A-Za-z0-9_.-]+)`?", prompt)]
    delivered_blob = "\n".join(list(progress) + list(messages) + [text])
    final_delivery_like = bool(re.search(r"(?:FILE:|IMAGE_FILE:)\S+", text))
    matched_requested = False
    for req in requested_files:
        if not req:
            continue
        req_lower = req.lower()
        if req_lower in text.lower():
            matched_requested = True
            break
    if not matched_requested and not final_delivery_like:
        issues.append("用户要求发送指定文件，但最终输出里没有出现目标文件名/路径。")
    if not final_delivery_like and not named_missing_file_like:
        issues.append("用户要求发送指定文件，但最终回复不是明确的 FILE/IMAGE_FILE 交付。")
    if re.search(r"(?m)^[.A-Za-z0-9_-]+/?$", text) and "\n" in text:
        issues.append("用户要求发送指定文件，但系统退化成了目录列表。")
    if any("tool=list_dir" in line for line in lines) and not final_delivery_like and not named_missing_file_like:
        issues.append("用户要求发送指定文件，但执行里走了 list_dir 且没有明确文件交付。")
if named_missing_file_like:
    if status != "succeeded":
        issues.append("用户请求发送不存在的指定文件，但整次 ask 直接失败了，没有收敛成明确的未找到回复。")
    if not re.search(r"(没找到|未找到|not found|no such file|does not exist)", text, re.I):
        issues.append("用户请求发送不存在的指定文件，但最终没有明确返回文件未找到。")
    if re.search(r"(?:FILE:|IMAGE_FILE:)\S+", text):
        issues.append("用户请求发送不存在的指定文件，但系统仍然返回了文件交付标记。")

plan_blobs = list(progress) + list(messages)
planned_step_count = extract_planned_step_count(plan_blobs)
max_executed_step = extract_max_executed_step(lines)
if status == "succeeded" and planned_step_count >= 2 and max_executed_step and max_executed_step < planned_step_count and not named_missing_file_like:
    issues.append("计划里还有后续步骤，但执行在更早步骤就结束了，疑似提前 respond / 提前收尾。")

written_paths = extract_written_paths(plan_blobs + [text])
delivery_paths = extract_delivery_paths(text)
if written_paths:
    written_path = written_paths[-1]
    if delivery_paths:
        delivered_path = delivery_paths[-1]
        if delivered_path != written_path:
            issues.append("返回/发送的文件路径与真实写入路径不一致。")
    elif re.search(r"(saved path|保存路径|文件路径|path only)", prompt, re.I) and written_path not in text:
        issues.append("用户要求返回保存路径，但最终回复未包含真实写入路径。")

if same_turn_failure_explain_like and status in {"failed", "timeout", "canceled"} and resume_context is not None:
    issues.append("同一轮里用户要求先说明失败步骤和剩余工作，但系统直接返回失败而没有先解释。")

if listing_based_digest_like:
    listing_entries = extract_listing_entries(plan_blobs)
    mentioned_paths = [trim_token(x) for x in re.findall(r"`([^`]+)`", text)]
    unseen = []
    for item in mentioned_paths:
        if not item or ("." not in item and "/" not in item):
            continue
        if item in listing_entries or item.rstrip("/") in listing_entries or f"{item.rstrip('/')}/" in listing_entries:
            continue
        unseen.append(item)
    if unseen:
        issues.append(f"总结里引用了目录输出中未出现的条目: {', '.join(unseen[:3])}")

print("  [issues]")
if issues:
    for issue in issues:
        print(f"    - {issue}")
else:
    print("    - 未命中脚本内置启发式问题。")
PY
}

run_case() {
  local name="$1"
  local prompt="$2"
  local submit_resp task_id final_raw

  echo
  echo "============================================================"
  echo "[CASE] ${name}"
  echo "[PROMPT] ${prompt}"

  submit_resp="$(submit_task "$prompt")"
  task_id="$(extract_submit_task_id "$submit_resp")"
  echo "[TASK] ${task_id}"
  echo "[POLL]"
  final_raw="$(poll_task_with_progress "$task_id")"

  echo "[RESULT]"
  if [[ "$PRINT_FULL_TEXT" == "1" ]]; then
    print_result_summary "$final_raw" "full"
  else
    print_result_summary "$final_raw" "summary"
  fi

  echo "[TRACE]"
  print_clawd_trace "$task_id"

  echo "[LLM]"
  print_model_io_summary "$task_id"

  echo "[CHECK]"
  print_issue_hints "$task_id" "$prompt" "$final_raw"
}

main() {
  local prompt_count=0
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
      --wait-seconds)
        MAX_WAIT_SECONDS="${2:-}"
        shift 2
        ;;
      --poll-seconds)
        POLL_INTERVAL_SECONDS="${2:-}"
        shift 2
        ;;
      --prompt)
        prompt_count=$((prompt_count + 1))
        add_case "custom_${prompt_count}" "${2:-}"
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

  if [[ "$USE_DEFAULT_CASES" == "1" ]]; then
    load_default_cases
  fi

  if [[ -n "$case_file" ]]; then
    while IFS=$'\t' read -r name prompt; do
      [[ -n "${prompt:-}" ]] || continue
      add_case "$name" "$prompt"
    done < <(load_case_file "$case_file")
  fi

  if [[ "${#CASE_NAMES[@]}" -eq 0 ]]; then
    echo "No cases to run."
    usage
    exit 2
  fi

  echo "== Ask trace regression =="
  echo "BASE_URL=${BASE_URL}"
  echo "USER_ID=${USER_ID} CHAT_ID=${CHAT_ID}"
  echo "WAIT=${MAX_WAIT_SECONDS}s POLL=${POLL_INTERVAL_SECONDS}s"
  echo "CASES=${#CASE_NAMES[@]}"

  health_check

  local i
  for (( i = 0; i < ${#CASE_NAMES[@]}; i++ )); do
    run_case "${CASE_NAMES[$i]}" "${CASE_PROMPTS[$i]}"
  done

  echo
  echo "DONE: all cases finished"
}

main "$@"

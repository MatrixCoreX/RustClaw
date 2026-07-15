#!/usr/bin/env bash
#
# test_circuit_breaker.sh — observe RustClaw provider fallback + circuit
# breaker behaviour end-to-end, WITHOUT modifying config.toml or restarting
# clawd.
#
# What this script actually does:
#   1. Reads configs/config.toml + relevant API_KEY env vars to figure out
#      how many LLM vendors are usable. Aborts if only 1 → no fallback can
#      ever happen, so observing the breaker would be meaningless.
#   2. Asks clawd to run N small act-style cases back-to-back via the normal
#      /v1/tasks endpoint.
#   3. After each case, parses the new lines that landed in
#      logs/model_io.log and tags them per-task — counting:
#        - per-vendor LLM call attempts
#        - per-vendor failures (status != ok)
#        - vendor switches inside a single task (= a fallback occurred)
#        - tasks where the FINAL serving vendor != selected_vendor
#          (= fallback also won)
#   4. Prints a summary that flags whether the circuit breaker likely fired
#      (consecutive failures on a single vendor ≥ 5 within the run).
#
# This script is read-only on the configuration. To actually FORCE the
# breaker to trip you would need a deliberately broken vendor in the
# fallback chain (e.g. a custom vendor pointing to 127.0.0.1:1). That kind
# of mutation is intentionally NOT done here.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/lib.sh"

CASE_COUNT="${CASE_COUNT:-12}"
RUN_STAMP="$(date +%Y%m%d_%H%M%S)"
LOG_ROOT_DEFAULT="${ROOT_DIR}/scripts/nl_suite_logs/circuit_breaker"
LOG_ROOT="${LOG_ROOT:-${LOG_ROOT_DEFAULT}}"
RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"
RUN_LOG="${RUN_DIR}/run.log"
SUMMARY_JSON="${RUN_DIR}/summary.json"
MODEL_IO_LOG="${ROOT_DIR}/logs/model_io.log"

usage() {
  cat <<EOF
Usage: $(basename "$0") [--case-count N] [--log-root PATH]

Observes provider fallback + circuit breaker behaviour end-to-end without
mutating configs/config.toml.

  --case-count N    Number of probe cases to send (default: 12)
  --log-root PATH   Where to drop logs (default: ${LOG_ROOT_DEFAULT})
  -h, --help        Show this help

Pre-requisites:
  - clawd must be running (default base url ${BASE_URL})
  - At least 2 vendors must have api_key set (in config.toml or via env vars
    ANTHROPIC_API_KEY / OPENAI_API_KEY / DEEPSEEK_API_KEY / MINIMAX_API_KEY /
    GOOGLE_API_KEY / GROK_API_KEY / QWEN_API_KEY / MIMO_API_KEY / XIAOMI_API_KEY)
    — otherwise nothing can fall back over and this script will refuse to run.
EOF
}

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" --anchor "$1" "$2"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --case-count) CASE_COUNT="$2"; shift 2 ;;
    --log-root)   LOG_ROOT="$2"; RUN_DIR="${LOG_ROOT}/${RUN_STAMP}"; shift 2 ;;
    -h|--help)    usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

mkdir -p "$RUN_DIR"
RUN_LOG="${RUN_DIR}/run.log"
SUMMARY_JSON="${RUN_DIR}/summary.json"
exec > >(tee -a "$RUN_LOG") 2>&1

echo "Circuit breaker probe"
echo "  run_dir_ref: $(path_ref "$RUN_DIR" "$RUN_DIR")"
echo "  run_log_ref: $(path_ref "$RUN_DIR" "$RUN_LOG")"
echo "  summary_json_ref: $(path_ref "$RUN_DIR" "$SUMMARY_JSON")"
echo "  base_url:   $BASE_URL"
echo "  case_count: $CASE_COUNT"
echo

# ---- Step 1: vendor inventory ------------------------------------------
mapfile -t USABLE_VENDORS < <(python3 - <<'PY'
import os, tomllib, pathlib, sys

cfg_path = pathlib.Path("configs/config.toml")
if not cfg_path.exists():
    sys.stderr.write("configs/config.toml not found\n")
    raise SystemExit(2)
cfg = tomllib.loads(cfg_path.read_text(encoding="utf-8"))
llm = cfg.get("llm") or {}
selected = llm.get("selected_vendor") or ""

env_keys = {
    "anthropic": "ANTHROPIC_API_KEY",
    "deepseek":  "DEEPSEEK_API_KEY",
    "google":    "GOOGLE_API_KEY",
    "grok":      "GROK_API_KEY",
    "minimax":   "MINIMAX_API_KEY",
    "mimo":      "MIMO_API_KEY",
    "openai":    "OPENAI_API_KEY",
    "qwen":      "QWEN_API_KEY",
    "custom":    "CUSTOM_API_KEY",
}

usable = []
for name, env_var in env_keys.items():
    section = llm.get(name) or {}
    has_cfg = bool((section.get("api_key") or "").strip())
    has_env = bool((os.environ.get(env_var) or "").strip())
    if name == "mimo":
        has_env = has_env or bool((os.environ.get("XIAOMI_API_KEY") or "").strip())
    if has_cfg or has_env:
        usable.append(name)

# Print selected first to make it identifiable, then the rest.
ordered = []
if selected and selected in usable:
    ordered.append(selected)
for n in usable:
    if n not in ordered:
        ordered.append(n)
for n in ordered:
    print(n)
PY
)

if (( ${#USABLE_VENDORS[@]} == 0 )); then
  echo "[fatal] No vendor has an api_key set. Cannot run circuit breaker probe."
  exit 2
fi

echo "[vendors] usable: ${USABLE_VENDORS[*]} (selected first)"
if (( ${#USABLE_VENDORS[@]} < 2 )); then
  echo "[fatal] Only 1 vendor usable; fallback / circuit breaker cannot be"
  echo "        observed in this configuration."
  echo "        Add an api_key for at least one more vendor (in config.toml"
  echo "        or via the matching *_API_KEY env var) and rerun."
  exit 3
fi
echo

# ---- Step 2: health + admin key ---------------------------------------
if [[ -z "${USER_KEY:-}" ]]; then
  USER_KEY="$("${ROOT_DIR}/scripts/auth-key.sh" list | awk '$2 == "admin" && $3 == "enabled" { print $1; exit }' || true)"
  if [[ -z "$USER_KEY" ]]; then
    echo "[fatal] No enabled admin key (set USER_KEY env or run scripts/auth-key.sh)"
    exit 2
  fi
fi

echo "[health] checking $BASE_URL/v1/health"
health_check
echo "[health] OK"
echo

# ---- Step 3: probe cases ----------------------------------------------
PROBE_PROMPTS=(
  "只输出当前工作目录的绝对路径，不要解释"
  "只输出当前用户名，不要解释"
  "只输出当前机器 hostname"
  "把当前仓库顶层目录列出来，简单分组就行"
  "看看仓库里有没有 rustclaw.service，只回答有或没有"
  "读取 Cargo.toml 的 package.name，只输出值"
  "列出 logs 目录下的前 10 个文件名"
  "看看当前 git 分支叫什么，只给我分支名"
  "把这句话翻成英文：服务已经恢复"
  "用一句话告诉新手为什么要写单元测试"
  "比较 Cargo.toml 和 Cargo.lock 哪个更大"
  "检查 telegramd 现在是不是在运行"
)

# Make sure CASE_COUNT doesn't exceed PROBE_PROMPTS — we cycle if needed.
PROBE_LEN="${#PROBE_PROMPTS[@]}"

# Capture starting offset of model_io.log for the per-task diff.
log_size() { stat -c '%s' "$MODEL_IO_LOG" 2>/dev/null || echo 0; }

INITIAL_OFFSET="$(log_size)"
echo "[model_io] initial offset: ${INITIAL_OFFSET} bytes"
echo

declare -a TASK_IDS=()

for ((i = 1; i <= CASE_COUNT; i++)); do
  prompt="${PROBE_PROMPTS[$(( (i - 1) % PROBE_LEN ))]}"
  echo "----- probe $i / $CASE_COUNT -----"
  echo "  prompt: $prompt"
  raw="$(submit_task "$prompt" 2>&1)" || { echo "  submit failed: $raw"; continue; }
  task_id="$(extract_submit_task_id "$raw")"
  echo "  task_id: $task_id"
  TASK_IDS+=("$task_id")
  out_file="$(mktemp)"
  # Wait until terminal but cap at 90s per case to keep the probe quick.
  MAX_WAIT_SECONDS=90 wait_task_until_terminal_with_limit "$task_id" 90 1 "$out_file" >/dev/null || true
  status="$(python3 -c 'import json,sys; print((json.load(open(sys.argv[1])).get("data") or {}).get("status") or "?")' "$out_file" 2>/dev/null || echo "?")"
  echo "  final status: $status"
  rm -f "$out_file"
done

echo
echo "[model_io] post-probe offset: $(log_size) bytes"
echo

# ---- Step 4: per-task LLM call analysis -------------------------------
python3 - "$MODEL_IO_LOG" "$INITIAL_OFFSET" "${TASK_IDS[@]}" >"$SUMMARY_JSON" <<'PY'
import json, sys
from collections import defaultdict

log_path = sys.argv[1]
offset = int(sys.argv[2])
task_ids = set(sys.argv[3:])

with open(log_path, "rb") as fh:
    fh.seek(offset)
    chunk = fh.read().decode("utf-8", errors="replace")

per_task = defaultdict(list)  # task_id -> list of (vendor, status, error)
for line in chunk.splitlines():
    line = line.strip()
    if not line:
        continue
    try:
        obj = json.loads(line)
    except Exception:
        continue
    tid = str(obj.get("task_id") or "")
    if tid not in task_ids:
        continue
    per_task[tid].append({
        "vendor": str(obj.get("vendor") or ""),
        "status": str(obj.get("status") or ""),
        "error":  str(obj.get("error") or ""),
        "ts":     obj.get("ts") or "",
    })

vendor_attempts = defaultdict(int)
vendor_failures = defaultdict(int)
fallback_tasks = 0
multi_vendor_tasks = 0
final_vendor_counter = defaultdict(int)
consecutive_per_vendor = defaultdict(int)
max_consecutive_failures = defaultdict(int)
running_consecutive = defaultdict(int)

# We need ordered (across tasks) failure stream per vendor to estimate
# whether the breaker likely tripped (default cooldown threshold = 5
# consecutive failures inside the LlmProviderRuntime).
all_calls_in_order = []
for tid in task_ids:
    for call in per_task.get(tid, []):
        all_calls_in_order.append((tid, call))

for tid, call in all_calls_in_order:
    v = call["vendor"]
    vendor_attempts[v] += 1
    if call["status"] != "ok":
        vendor_failures[v] += 1
        running_consecutive[v] += 1
        if running_consecutive[v] > max_consecutive_failures[v]:
            max_consecutive_failures[v] = running_consecutive[v]
    else:
        running_consecutive[v] = 0

for tid, calls in per_task.items():
    vendors = [c["vendor"] for c in calls]
    distinct = set(vendors)
    if len(distinct) > 1:
        multi_vendor_tasks += 1
    if calls:
        ok_calls = [c for c in calls if c["status"] == "ok"]
        if ok_calls:
            final_vendor = ok_calls[-1]["vendor"]
            final_vendor_counter[final_vendor] += 1
        else:
            final_vendor_counter["<all_failed>"] += 1
        if vendors[0] != (ok_calls[-1]["vendor"] if ok_calls else vendors[0]):
            fallback_tasks += 1

summary = {
    "tasks_observed": len(per_task),
    "per_vendor_attempts": dict(vendor_attempts),
    "per_vendor_failures": dict(vendor_failures),
    "max_consecutive_failures_per_vendor": dict(max_consecutive_failures),
    "tasks_with_vendor_switch": multi_vendor_tasks,
    "tasks_where_final_vendor_differed_from_first": fallback_tasks,
    "final_vendor_distribution": dict(final_vendor_counter),
}
print(json.dumps(summary, ensure_ascii=False, indent=2))
PY

echo "================ Summary ================"
cat "$SUMMARY_JSON"
echo

# ---- Step 5: human-readable verdict -----------------------------------
python3 - "$SUMMARY_JSON" <<'PY'
import json, sys, pathlib

s = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))

print()
print("================ Verdict ================")
total = s["tasks_observed"]
if total == 0:
    print("  NO LLM CALLS recorded — model_io.log may be disabled, or no probes ran.")
    raise SystemExit(0)

vsw = s["tasks_with_vendor_switch"]
fbt = s["tasks_where_final_vendor_differed_from_first"]
print(f"  tasks observed                         : {total}")
print(f"  tasks with vendor switch (any kind)    : {vsw}")
print(f"  tasks where final vendor != first call : {fbt}  (= true fallback wins)")

# Per-vendor signals
print()
print("  per-vendor stats:")
attempts = s["per_vendor_attempts"]
failures = s["per_vendor_failures"]
maxcons  = s["max_consecutive_failures_per_vendor"]
for v in sorted(attempts.keys()):
    a = attempts.get(v, 0)
    f = failures.get(v, 0)
    m = maxcons.get(v, 0)
    rate = (f / a * 100.0) if a else 0
    flag = ""
    if m >= 5:
        flag = "  ← likely tripped circuit breaker (>=5 consecutive failures)"
    elif m >= 3:
        flag = "  ← approaching breaker threshold (3-4 consecutive failures)"
    print(f"    {v:10s} attempts={a:3d}  failures={f:3d} ({rate:5.1f}%)  max_consecutive_failures={m:2d}{flag}")

# Final fallback distribution
print()
print("  final serving vendor distribution:")
for v, n in s["final_vendor_distribution"].items():
    print(f"    {v:15s} {n}")

if fbt == 0:
    print()
    print("  No fallback occurred — primary vendor handled every call.")
    print("  This is normal in a healthy run, but means the circuit breaker")
    print("  fallback path was NOT exercised. To force it, point the primary")
    print("  vendor's base_url at an unreachable host temporarily.")
PY

echo
echo "Artifacts:"
echo "  - run_dir_ref=$(path_ref "$RUN_DIR" "$RUN_DIR")"
echo "  - run_log_ref=$(path_ref "$RUN_DIR" "$RUN_LOG")"
echo "  - summary_json_ref=$(path_ref "$RUN_DIR" "$SUMMARY_JSON")"

#!/usr/bin/env bash
# §5.1 inspect_task: 给定 task_id，把 task_journal + model_io.log + tracing 日志拉齐到 stdout。
#
# 用法：
#   scripts/inspect_task.sh <task_id> [--verbose]
#
# 默认（slim）输出：
#   * task row 状态 + task_journal summary
#   * 一行 SUMMARY：调用次数、provider/model 分布、ts 跨度、状态分布
#   * 每一条 LLM 调用一行：ts | status | vendor:model | prompt_source | usage tokens
#   * tracing 行（grep clawd.run.log）总数 + 前 20 行
#
# --verbose 时：
#   * 打印 task_journal 完整 trace
#   * 打印每条 model_io.log 完整 prompt（已截断）+ response 摘要
#   * 打印全部 tracing 行
#
# 依赖：jq, awk, grep。logs/ 路径由 LOGS_DIR 环境变量覆盖（默认 ./logs）。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

TASK_ID=""
VERBOSE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --verbose|-v)
      VERBOSE=1
      shift
      ;;
    --help|-h)
      sed -n '2,18p' "$0"
      exit 0
      ;;
    -*)
      echo "[inspect_task] unknown flag: $1" >&2
      exit 2
      ;;
    *)
      if [[ -z "$TASK_ID" ]]; then
        TASK_ID="$1"
      else
        echo "[inspect_task] unexpected positional arg: $1" >&2
        exit 2
      fi
      shift
      ;;
  esac
done

if [[ -z "$TASK_ID" ]]; then
  echo "usage: $0 <task_id> [--verbose]" >&2
  exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "[inspect_task] jq is required (apt install jq)" >&2
  exit 1
fi

LOGS_DIR="${LOGS_DIR:-logs}"
MODEL_IO_LOG="${LOGS_DIR}/model_io.log"
DEFAULT_TRACING_LOG="${LOGS_DIR}/clawd.run.log"
LEGACY_TRACING_LOG="${TMPDIR:-/tmp}/clawd.out"
TRACING_LOG="${TRACING_LOG:-${DEFAULT_TRACING_LOG}}"
if [[ ! -f "$TRACING_LOG" && -f "$LEGACY_TRACING_LOG" ]]; then
  TRACING_LOG="$LEGACY_TRACING_LOG"
fi
CONFIG_PATH="${CONFIG_PATH:-configs/config.toml}"

resolve_db_path() {
  python3 - "$CONFIG_PATH" <<'PY'
from pathlib import Path
import sys
import tomllib

config_path = Path(sys.argv[1])
if not config_path.is_absolute():
    config_path = (Path.cwd() / config_path).resolve()
root = config_path.parent.parent if config_path.name == "config.toml" and config_path.parent.name == "configs" else Path.cwd()
with config_path.open("rb") as f:
    data = tomllib.load(f)
db_path = (((data.get("database") or {}).get("sqlite_path")) or "").strip()
if not db_path:
    print((root / "data" / "rustclaw.db").resolve())
else:
    p = Path(db_path)
    print((p if p.is_absolute() else (root / p)).resolve())
PY
}

fetch_task_row_json() {
  python3 - "$DB_PATH" "$TASK_ID" <<'PY'
import json
import sqlite3
import sys

db_path = sys.argv[1]
task_id = sys.argv[2]

conn = sqlite3.connect(db_path)
row = conn.execute(
    "SELECT status, result_json, error_text, created_at, updated_at "
    "FROM tasks WHERE task_id = ? LIMIT 1",
    (task_id,),
).fetchone()
if not row:
    print("")
    raise SystemExit(0)

status, result_json, error_text, created_at, updated_at = row
parsed = None
parse_error = None
if result_json:
    try:
        parsed = json.loads(result_json)
    except Exception as exc:
        parse_error = str(exc)

journal = {}
if isinstance(parsed, dict):
    journal = parsed.get("task_journal") or {}

print(json.dumps({
    "status": status,
    "created_at": created_at,
    "updated_at": updated_at,
    "error_text": error_text,
    "has_result_json": bool(result_json),
    "result_json_parse_error": parse_error,
    "task_journal_summary": journal.get("summary"),
    "task_journal_trace": journal.get("trace"),
}, ensure_ascii=False))
PY
}

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" "$1"
}

DB_PATH="${DB_PATH:-$(resolve_db_path)}"
TASK_ROW_JSON=""
if [[ -f "$DB_PATH" ]]; then
  TASK_ROW_JSON="$(fetch_task_row_json || true)"
fi

# ---- 1. model_io.log: 拉所有匹配本 task_id 的行（grep + jq filter） -----------
# 先用 grep 粗筛减少 jq 解析量（数百 MB 文件全 jq 会很慢）。
COUNT=0
MATCHED=()
if [[ -f "$MODEL_IO_LOG" ]]; then
  grep_filter="\"task_id\":\"${TASK_ID}\""
  mapfile -t MATCHED < <(grep -F "$grep_filter" "$MODEL_IO_LOG" || true)
  COUNT=${#MATCHED[@]}
fi

echo "===== inspect_task: ${TASK_ID} ====="
echo "db_path_ref     : $(path_ref "${DB_PATH}")"
echo "model_io_log_ref: $(path_ref "${MODEL_IO_LOG}")"
echo "tracing_log_ref : $(path_ref "${TRACING_LOG}")"
echo "calls_total : ${COUNT}"

echo
if [[ ! -f "$DB_PATH" ]]; then
  echo "[inspect_task] sqlite db missing: $(path_ref "${DB_PATH}")"
elif [[ -z "$TASK_ROW_JSON" ]]; then
  echo "[inspect_task] no task row found for task_id=${TASK_ID}"
else
  echo "----- task row / task_journal summary -----"
  printf '%s\n' "$TASK_ROW_JSON" | jq '{
    status,
    created_at,
    updated_at,
    error_text,
    has_result_json,
    result_json_parse_error,
    task_journal_summary
  }'
  if [[ "$VERBOSE" -eq 1 ]]; then
    echo "----- task_journal trace -----"
    printf '%s\n' "$TASK_ROW_JSON" | jq '.task_journal_trace'
  fi
fi

if [[ ! -f "$MODEL_IO_LOG" ]]; then
  echo "[inspect_task] model_io log missing: $(path_ref "${MODEL_IO_LOG}")"
  echo "[inspect_task] continuing with task_journal + tracing only"
elif [[ "$COUNT" -eq 0 ]]; then
  echo "[inspect_task] no model_io entries found for task_id=${TASK_ID}"
else
  # 用 jq 把 MATCHED 数组合成单行 ndjson 流，做一次聚合 + 一次明细。
  printf '%s\n' "${MATCHED[@]}" | jq -s --arg verbose "$VERBOSE" '
    sort_by(.ts) as $rows |
    {
      summary: {
        first_ts: ($rows[0].ts // null),
        last_ts:  ($rows[-1].ts // null),
        span_secs: (if ($rows | length) > 1 then ($rows[-1].ts - $rows[0].ts) else 0 end),
        status_counts: ($rows | group_by(.status) | map({(.[0].status // "unknown"): length}) | add),
        vendor_model_counts: ($rows | group_by(.vendor + ":" + (.model // "?"))
                                    | map({key: (.[0].vendor + ":" + (.[0].model // "?")), count: length})),
        prompt_source_counts: ($rows | group_by(.prompt_source // "unknown")
                                     | map({key: (.[0].prompt_source // "unknown"), count: length})),
        total_input_tokens: ($rows | map(.usage.input_tokens // .usage.prompt_tokens // 0) | add),
        total_output_tokens: ($rows | map(.usage.output_tokens // .usage.completion_tokens // 0) | add),
      },
      calls: ($rows | map({
        ts: .ts,
        status: .status,
        vendor_model: (.vendor + ":" + (.model // "?")),
        prompt_source: .prompt_source,
        in_tokens: (.usage.input_tokens // .usage.prompt_tokens // null),
        out_tokens: (.usage.output_tokens // .usage.completion_tokens // null),
        error: .error,
        prompt_chars: (.prompt_chars // (.prompt | if . then length else null end)),
        response_chars: (.response_chars // (.clean_response // .response | if . then length else null end)),
        prompt_preview: (if $verbose == "1" then (.prompt // null) else null end),
        response_preview: (if $verbose == "1" then ((.clean_response // .response) // null) else null end),
      }))
    }
  '
fi

# ---- 2. tracing log: grep task_id 行 -----------------------------------------
echo
if [[ -f "$TRACING_LOG" ]]; then
  TRACE_COUNT=$(grep -c -F "$TASK_ID" "$TRACING_LOG" 2>/dev/null || true)
  TRACE_COUNT="${TRACE_COUNT:-0}"
  echo "tracing_lines: ${TRACE_COUNT}"
  if [[ "$TRACE_COUNT" -gt 0 ]]; then
    if [[ "$VERBOSE" -eq 1 ]]; then
      echo "----- tracing (full) -----"
      grep -F "$TASK_ID" "$TRACING_LOG"
    else
      echo "----- tracing (first 20 lines) -----"
      grep -F "$TASK_ID" "$TRACING_LOG" | head -n 20
      if [[ "$TRACE_COUNT" -gt 20 ]]; then
        echo "... (+$((TRACE_COUNT - 20)) more lines, rerun with --verbose to see all)"
      fi
    fi
  fi
else
  echo "tracing_lines: 0  (no ${TRACING_LOG})"
fi

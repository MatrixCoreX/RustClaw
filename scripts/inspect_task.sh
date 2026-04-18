#!/usr/bin/env bash
# §5.1 inspect_task: 给定 task_id，把 model_io.log + tracing 日志 + 概要拉齐到 stdout。
#
# 用法：
#   scripts/inspect_task.sh <task_id> [--verbose]
#
# 默认（slim）输出：
#   * 一行 SUMMARY：调用次数、provider/model 分布、ts 跨度、状态分布
#   * 每一条 LLM 调用一行：ts | status | vendor:model | prompt_source | usage tokens
#   * tracing 行（grep clawd.run.log）总数 + 前 20 行
#
# --verbose 时：
#   * 打印每条 model_io.log 完整 prompt（已截断）+ response 摘要
#   * 打印全部 tracing 行
#
# 依赖：jq, awk, grep。logs/ 路径由 LOGS_DIR 环境变量覆盖（默认 ./logs）。

set -euo pipefail

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
TRACING_LOG="${TRACING_LOG:-${LOGS_DIR}/clawd.run.log}"

if [[ ! -f "$MODEL_IO_LOG" ]]; then
  echo "[inspect_task] model_io log missing: $MODEL_IO_LOG" >&2
  echo "[inspect_task] (override with LOGS_DIR=/path)" >&2
  exit 1
fi

# ---- 1. model_io.log: 拉所有匹配本 task_id 的行（grep + jq filter） -----------
# 先用 grep 粗筛减少 jq 解析量（数百 MB 文件全 jq 会很慢）。
grep_filter="\"task_id\":\"${TASK_ID}\""

mapfile -t MATCHED < <(grep -F "$grep_filter" "$MODEL_IO_LOG" || true)
COUNT=${#MATCHED[@]}

echo "===== inspect_task: ${TASK_ID} ====="
echo "model_io_log: ${MODEL_IO_LOG}"
echo "tracing_log : ${TRACING_LOG}"
echo "calls_total : ${COUNT}"

if [[ "$COUNT" -eq 0 ]]; then
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

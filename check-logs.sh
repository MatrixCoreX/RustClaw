#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="${SCRIPT_DIR}/logs"
LINES=120
FOLLOW=0

usage() {
  cat <<'EOF'
Usage: # zh: 用法
  ./check-logs.sh [-n NUM] [-f] # zh: ./check-logs.sh [-n 行数] [-f]

Options: # zh: 参数
  -n NUM   Show latest NUM lines per log file (default 120) # zh: 每个日志文件展示最近 NUM 行（默认 120）
  -f       Follow clawd/telegramd logs continuously # zh: 持续跟踪 clawd/telegramd 日志
  -h       Show help # zh: 显示帮助
EOF
}

while getopts ":n:fh" opt; do
  case "$opt" in
    n)
      if [[ ! "${OPTARG}" =~ ^[0-9]+$ ]] || [[ "${OPTARG}" -le 0 ]]; then
        echo "Error: -n must be a positive integer" # zh: 错误: -n 必须是正整数
        exit 1
      fi
      LINES="${OPTARG}"
      ;;
    f)
      FOLLOW=1
      ;;
    h)
      usage
      exit 0
      ;;
    :)
      echo "Error: option -${OPTARG} requires an argument" # zh: 错误: 选项 -${OPTARG} 缺少参数
      usage
      exit 1
      ;;
    \?)
      echo "Error: unknown option -${OPTARG}" # zh: 错误: 未知选项 -${OPTARG}
      usage
      exit 1
      ;;
  esac
done

if [[ ! -d "${LOG_DIR}" ]]; then
  echo "Log directory not found: ${LOG_DIR}" # zh: 未找到日志目录: ${LOG_DIR}
  exit 1
fi

print_file_report() {
  local file="$1"
  local name
  name="$(basename "${file}")"

  echo
  echo "==================== ${name} (latest ${LINES} lines) ====================" # zh: ==================== ${name} (最近 ${LINES} 行) ====================
  if [[ ! -f "${file}" ]]; then
    echo "Log file does not exist: ${file}" # zh: 日志文件不存在: ${file}
    return
  fi

  tail -n "${LINES}" "${file}" || true

  echo
  echo "--- ${name} key error matches (latest 30) ---" # zh: --- ${name} 关键异常命中（最近 30 条）---
  if command -v rg >/dev/null 2>&1; then
    rg -n -i "error|failed|timeout|panic|terminatedbyothergetupdates|processing failed|queued|queue full" "${file}" | tail -n 30 || echo "No key error keywords found" # zh: 未发现关键异常关键词
  else
    grep -nEi "error|failed|timeout|panic|terminatedbyothergetupdates|processing failed|queued|queue full" "${file}" | tail -n 30 || echo "No key error keywords found" # zh: 未发现关键异常关键词
  fi
}

echo "Log directory: ${LOG_DIR}" # zh: 日志目录: ${LOG_DIR}
echo "Check time: $(date '+%F %T')" # zh: 检查时间: $(date '+%F %T')

print_file_report "${LOG_DIR}/clawd.log"
print_file_report "${LOG_DIR}/telegramd.log"

if [[ "${FOLLOW}" == "1" ]]; then
  echo
  echo "Start following logs (Ctrl+C to exit)..." # zh: 开始持续跟踪日志（Ctrl+C 退出）...
  tail -F "${LOG_DIR}/clawd.log" "${LOG_DIR}/telegramd.log"
fi

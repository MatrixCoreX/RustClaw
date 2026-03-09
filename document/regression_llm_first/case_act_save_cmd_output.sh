#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check

target_rel="document/llm_save_cmd_output_${USER_ID}_${CHAT_ID}_$(date +%s).txt"
target_abs="${REPO_ROOT}/${target_rel}"
confirm_token="SAVED_FILE:${target_rel}"

rm -f "${target_abs}"

prompt="先执行 ls -l，再把输出保存到文件 ${target_rel}，并在执行输出里打印 ${confirm_token}"
echo "[CASE] act_save_cmd_output"
echo "prompt: ${prompt}"

submit_resp="$(submit_task "${prompt}")"
task_id="$(extract_submit_task_id "${submit_resp}")"
echo "task_id: ${task_id}"

row="$(wait_task_until_terminal "${task_id}")"
status="$(printf '%s' "${row}" | awk -F'\t' '{print $1}')"
text="$(printf '%s' "${row}" | awk -F'\t' '{print $2}')"
error="$(printf '%s' "${row}" | awk -F'\t' '{print $3}')"

if [ "${status}" != "succeeded" ]; then
  echo "FAIL: status=${status} error=${error}"
  exit 1
fi

if [ ! -f "${target_abs}" ]; then
  echo "FAIL: target file not created: ${target_abs}"
  exit 1
fi

if [ ! -s "${target_abs}" ]; then
  echo "FAIL: target file is empty: ${target_abs}"
  exit 1
fi

if ! python3 - "${target_abs}" <<'PY'
from pathlib import Path
import sys

p = Path(sys.argv[1])
text = p.read_text(encoding="utf-8", errors="replace")
if "Cargo.toml" not in text:
    raise SystemExit(1)
PY
then
  echo "FAIL: target file does not look like ls -l output: ${target_abs}"
  exit 1
fi

raw_task="$(query_task "${task_id}")"
if ! python3 - "${confirm_token}" "${raw_task}" <<'PY'
import json
import sys

token = sys.argv[1]
raw = sys.argv[2]
obj = json.loads(raw)

def contains(v):
    if isinstance(v, dict):
        return any(contains(x) for x in v.values())
    if isinstance(v, list):
        return any(contains(x) for x in v)
    if isinstance(v, str):
        return token in v
    return False

if not contains(obj):
    raise SystemExit(1)
PY
then
  echo "FAIL: confirmation token missing in task response payload: ${confirm_token}"
  echo "result_text=${text}"
  echo "error_text=${error}"
  exit 1
fi

echo "PASS: file created and confirmed at ${target_rel}"

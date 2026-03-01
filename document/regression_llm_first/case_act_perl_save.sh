#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check

run_case_expect \
  "act_perl_save" \
  "帮我写一个perl的代码例子，保存在 ./perl 目录下，文件名 perl_example.pl" \
  "succeeded" \
  "Saved successfully:" \
  "text" \
  "agent tool call limit exceeded" \
  "either"

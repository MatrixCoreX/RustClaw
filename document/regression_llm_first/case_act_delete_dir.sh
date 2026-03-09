#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check

# Prepare a deletable directory to ensure the test is meaningful.
mkdir -p ./perl_test
echo "delete-me" > ./perl_test/sample.txt

run_case_expect \
  "act_delete_perl_test_dir" \
  "删掉 ./perl_test 目录和里面文件" \
  "succeeded" \
  "" \
  "text" \
  "agent repeated same action too many times" \
  "either"

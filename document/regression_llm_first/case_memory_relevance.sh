#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
run_case_expect "memory_relevance_seed" "记住我的昵称是龙虾王。回复 MEMORY_REL_SEED_OK" "succeeded" "MEMORY_REL_SEED_OK" "text"
run_case_expect "memory_relevance_math" "请计算 2+2，只回复 MEM_REL_MATH_OK 4" "succeeded" "MEM_REL_MATH_OK 4" "text" "龙虾王" "text"

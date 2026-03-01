#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
run_case_expect "memory_preference_override_en" "记住：以后默认用英文回复。请回复 MEMORY_PREF_EN_OK" "succeeded" "MEMORY_PREF_EN_OK" "text"
run_case_expect "memory_preference_override_zh" "更新偏好：以后默认用中文回复。请回复 MEMORY_PREF_ZH_OK" "succeeded" "MEMORY_PREF_ZH_OK" "text"
run_case_expect "memory_preference_override_query" "我现在默认用什么语言回复？只回复语言代号。" "succeeded" "zh" "text"

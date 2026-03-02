#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
run_case_expect "memory_preference_persist_set" "记住：以后默认用英文回复。请回复 MEMORY_PREF_SET_OK" "succeeded" "MEMORY_PREF_SET_OK" "text"
run_case_expect "memory_preference_persist_query" "我之前让你默认用什么语言？只回复语言代号。" "succeeded" "en" "text"

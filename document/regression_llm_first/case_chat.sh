#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check
run_case "chat" "请用中文原样回复：LLM_FIRST_CHAT_OK" "LLM_FIRST_CHAT_OK"

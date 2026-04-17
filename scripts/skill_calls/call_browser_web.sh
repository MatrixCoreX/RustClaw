#!/usr/bin/env bash
SKILL_NAME="browser_web"
DEFAULT_ARGS='{"action":"open_extract","url":"https://example.com","max_pages":1,"save_screenshot":false,"capture_images":false}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

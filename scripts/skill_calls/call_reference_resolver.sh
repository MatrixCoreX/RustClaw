#!/usr/bin/env bash
SKILL_NAME="reference_resolver"
DEFAULT_ARGS='{"action":"resolve_reference","request_text":"用上个回复那个文件","target_type":"file","language_hint":"zh-CN","max_candidates":5,"recent_turns":[{"role":"assistant","turn_id":"a1","text":"已保存到 /tmp/report.md"}],"recent_results":[]}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

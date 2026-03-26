#!/usr/bin/env bash
SKILL_NAME="doc_parse"
DEFAULT_ARGS='{"action":"parse_doc","path":"README.md","mode":"text_only","max_chars":4000,"include_metadata":true}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

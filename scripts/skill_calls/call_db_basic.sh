#!/usr/bin/env bash
SKILL_NAME="db_basic"
DEFAULT_ARGS='{"action":"sqlite_query","db_path":"data/skill-calls-smoke.sqlite","sql":"PRAGMA schema_version;"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

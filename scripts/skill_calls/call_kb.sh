#!/usr/bin/env bash
SKILL_NAME="kb"
DEFAULT_ARGS='{"action":"ingest","namespace":"demo_docs","paths":["README.md"],"overwrite":true,"chunk_size":800,"max_file_size":2097152}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

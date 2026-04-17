#!/usr/bin/env bash
SKILL_NAME="crypto"
DEFAULT_ARGS='{"action":"quote","symbol":"BTCUSDT"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"

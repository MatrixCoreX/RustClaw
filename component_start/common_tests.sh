#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/common.sh"

APP_MODEL_SELECT=0 component_start_init "$ROOT_DIR" release "test-entry"
[[ "$COMPONENT_ROOT" == "$ROOT_DIR" ]]
[[ "$COMPONENT_PROFILE" == "release" ]]
[[ "$COMPONENT_CONFIG_PATH" == "$ROOT_DIR/configs/config.toml" ]]
[[ "$COMPONENT_CHANNEL_CONFIG_DIR" == "$ROOT_DIR/configs/channels" ]]
[[ "$(component_binary_path clawd)" == "$ROOT_DIR/target/release/clawd" ]]

unset MINIMAX_API_KEY MIMO_API_KEY XIAOMI_API_KEY
[[ -z "$(component_vendor_api_key_from_env minimax)" ]]
MINIMAX_API_KEY="minimax-test-key"
export MINIMAX_API_KEY
[[ "$(component_vendor_api_key_from_env MiniMax)" == "minimax-test-key" ]]
XIAOMI_API_KEY="xiaomi-test-key"
MIMO_API_KEY="mimo-test-key"
export XIAOMI_API_KEY MIMO_API_KEY
[[ "$(component_vendor_api_key_from_env mimo)" == "mimo-test-key" ]]
MIMO_API_KEY="REPLACE_ME"
export MIMO_API_KEY
[[ "$(component_vendor_api_key_from_env mimo)" == "xiaomi-test-key" ]]
unset MINIMAX_API_KEY MIMO_API_KEY XIAOMI_API_KEY

if component_start_init "$ROOT_DIR" debug "test-entry" >/dev/null 2>&1; then
  echo "Unsupported profile was accepted." >&2
  exit 1
fi

stale_pid="$COMPONENT_PID_DIR/component-common-test.pid"
printf '%s\n' "999999999" > "$stale_pid"
if component_pid_is_running "component-common-test"; then
  echo "Stale PID was treated as running." >&2
  exit 1
fi
[[ ! -e "$stale_pid" ]]

detached_log="$(mktemp)"
component_launch_detached "$detached_log" sleep 30
detached_pid="$COMPONENT_LAUNCHED_PID"
component_process_is_running "$detached_pid"

sleep 30 &
unrelated_pid=$!
trap 'kill "$unrelated_pid" "$detached_pid" >/dev/null 2>&1 || true; rm -f "$detached_log" "$COMPONENT_PID_DIR/component-common-test.pid"' EXIT
printf '%s\n' "$unrelated_pid" > "$stale_pid"
if component_pid_is_running \
  "component-common-test" \
  "$ROOT_DIR/target/release/component-common-not-running"; then
  echo "A reused PID was accepted for the wrong component." >&2
  exit 1
fi
[[ ! -e "$stale_pid" ]]
kill -0 "$unrelated_pid"

component_write_pid_file "component-common-test" "$unrelated_pid"
grep -Fxq "$unrelated_pid" "$stale_pid"

echo "COMPONENT_START_COMMON_TESTS ok"

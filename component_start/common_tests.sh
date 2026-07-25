#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/common.sh"

RUSTCLAW_MODEL_SELECT=0 component_start_init "$ROOT_DIR" release "test-entry"
[[ "$COMPONENT_ROOT" == "$ROOT_DIR" ]]
[[ "$COMPONENT_PROFILE" == "release" ]]
[[ "$COMPONENT_CONFIG_PATH" == "$ROOT_DIR/configs/config.toml" ]]
[[ "$COMPONENT_CHANNEL_CONFIG_DIR" == "$ROOT_DIR/configs/channels" ]]
[[ "$(component_binary_path clawd)" == "$ROOT_DIR/target/release/clawd" ]]

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

sleep 30 &
unrelated_pid=$!
trap 'kill "$unrelated_pid" >/dev/null 2>&1 || true; rm -f "$COMPONENT_PID_DIR/component-common-test.pid"' EXIT
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

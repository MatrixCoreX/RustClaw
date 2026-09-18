#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"
temporary="$(mktemp -d)"
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  rm -r -- "$temporary"
}
trap cleanup EXIT
COMPONENT_CONFIG_PATH="$temporary/production.toml"
COMPONENT_PID_DIR="$temporary"
python3 -c 'import time; time.sleep(60)' --config "$temporary/test.toml" &
other=$!
pids+=("$other")
python3 -c 'import time; time.sleep(60)' --config="$COMPONENT_CONFIG_PATH" &
matching=$!
pids+=("$matching")
python3 -c 'import time; time.sleep(60)' &
default_pid=$!
pids+=("$default_pid")
sleep 0.2
component_process_config_matches "$matching"
component_process_config_matches "$default_pid"
if component_process_config_matches "$other"; then
  echo "An isolated config was treated as the production service." >&2
  exit 1
fi
component_write_pid_file identity-test "$other"
if component_pid_is_running identity-test "$temporary/absent-executable"; then
  echo "An isolated instance was adopted from the PID file." >&2
  exit 1
fi
[[ ! -e "$temporary/identity-test.pid" ]]
kill -0 "$other"
echo "COMPONENT_PROCESS_CONFIG_TESTS ok"

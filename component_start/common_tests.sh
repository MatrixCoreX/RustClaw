#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/common.sh"

runtime_env_fixture="$(mktemp)"
printf '%s\n' 'export COMPONENT_RUNTIME_ENV_TEST=loaded' > "$runtime_env_fixture"
APP_RUNTIME_ENV_SCRIPT="$runtime_env_fixture" component_load_runtime_environment
[[ "${COMPONENT_RUNTIME_ENV_TEST:-}" == "loaded" ]]
grep -Fq 'component_load_runtime_environment' "$ROOT_DIR/start-all.sh"
rm -f "$runtime_env_fixture"
unset COMPONENT_RUNTIME_ENV_TEST

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

relay_config_fixture="$(mktemp)"
cat > "$relay_config_fixture" <<'TOML'
[llm]
selected_vendor = "custom"
selected_model = "relay-model"

[llm.hosted_relay]
enabled = true
vendor = "custom"
model = "relay-model"
base_url = "https://relay.example/v1"

[llm.custom]
base_url = "https://relay.example/v1/"
TOML
component_model_uses_hosted_relay_enrollment "$relay_config_fixture" custom relay-model
if component_model_uses_hosted_relay_enrollment "$relay_config_fixture" custom other-model; then
  echo "Hosted relay enrollment accepted the wrong model." >&2
  exit 1
fi
if component_model_uses_hosted_relay_enrollment "$relay_config_fixture" minimax relay-model; then
  echo "Hosted relay enrollment accepted the wrong vendor." >&2
  exit 1
fi
sed -i.bak 's#https://relay.example/v1/#https://other.example/v1#' "$relay_config_fixture"
if component_model_uses_hosted_relay_enrollment "$relay_config_fixture" custom relay-model; then
  echo "Hosted relay enrollment accepted the wrong provider endpoint." >&2
  exit 1
fi
rm -f "$relay_config_fixture" "$relay_config_fixture.bak"

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

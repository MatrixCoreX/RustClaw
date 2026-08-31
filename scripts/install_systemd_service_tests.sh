#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/product_identity.sh"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

WORKSPACE="$TMP_ROOT/runtime space"
mkdir -p "$WORKSPACE/configs"
touch "$WORKSPACE/start-all-bin.sh"
touch "$WORKSPACE/stop-agent.sh"
touch "$WORKSPACE/configs/config.toml"
touch "$TMP_ROOT/runtime_env.sh"
printf '%s\n' 'test-secret' > "$TMP_ROOT/provider_key"

OUTPUT="$TMP_ROOT/agent-system.service"
bash "$SCRIPT_DIR/install-systemd-service.sh" \
  --workspace "$WORKSPACE" \
  --user "$(id -un)" \
  --runtime-env "$TMP_ROOT/runtime_env.sh" \
  --credential PROVIDER_API_KEY:"$TMP_ROOT/provider_key" \
  --output "$OUTPUT"

grep -Fq "WorkingDirectory=$WORKSPACE" "$OUTPUT"
grep -Fq "ExecStart=/bin/bash \"$WORKSPACE/start-all-bin.sh\" release" "$OUTPUT"
grep -Fq "Environment=\"APP_SYSTEMD_UNIT=${APP_SERVICE_NAME}.service\"" "$OUTPUT"
grep -Fq "Environment=\"APP_RUNTIME_ENV_SCRIPT=$TMP_ROOT/runtime_env.sh\"" "$OUTPUT"
grep -Fq "Environment=\"HOME=" "$OUTPUT"
grep -Fq "TimeoutStartSec=0" "$OUTPUT"
grep -Fq "UMask=0077" "$OUTPUT"
grep -Fq "LoadCredential=PROVIDER_API_KEY:$TMP_ROOT/provider_key" "$OUTPUT"
grep -Fq "NoNewPrivileges=yes" "$OUTPUT"
grep -Fq "CapabilityBoundingSet=" "$OUTPUT"
grep -Fq "AmbientCapabilities=" "$OUTPUT"
grep -Fq "ProtectSystem=strict" "$OUTPUT"
grep -Fq "ProtectHome=read-only" "$OUTPUT"
grep -Fq "ReadWritePaths=$WORKSPACE" "$OUTPUT"
if grep -Fq "/home/example/old-product" "$OUTPUT"; then
  echo "Rendered unit contains the former hardcoded workspace path." >&2
  exit 1
fi

if bash "$SCRIPT_DIR/install-systemd-service.sh" \
  --workspace "$WORKSPACE" \
  --user "$(id -un)" \
  --credential 'INVALID-NAME:/does/not/exist' \
  --output "$TMP_ROOT/invalid-credential.service" >/dev/null 2>&1; then
  echo "Unsafe systemd credential binding was accepted." >&2
  exit 1
fi

if bash "$SCRIPT_DIR/install-systemd-service.sh" \
  --workspace "$WORKSPACE" \
  --user "$(id -un)" \
  --unit-name "invalid.service;reboot" \
  --output "$TMP_ROOT/invalid.service" >/dev/null 2>&1; then
  echo "Unsafe unit name was accepted." >&2
  exit 1
fi

echo "SYSTEMD_SERVICE_INSTALLER_TESTS ok"

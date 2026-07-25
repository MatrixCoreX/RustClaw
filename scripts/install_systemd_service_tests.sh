#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

WORKSPACE="$TMP_ROOT/runtime space"
mkdir -p "$WORKSPACE/configs"
touch "$WORKSPACE/start-all-bin.sh"
touch "$WORKSPACE/stop-rustclaw.sh"
touch "$WORKSPACE/configs/config.toml"
touch "$TMP_ROOT/runtime_env.sh"

OUTPUT="$TMP_ROOT/rustclaw.service"
bash "$SCRIPT_DIR/install-systemd-service.sh" \
  --workspace "$WORKSPACE" \
  --user "$(id -un)" \
  --runtime-env "$TMP_ROOT/runtime_env.sh" \
  --output "$OUTPUT"

grep -Fq "WorkingDirectory=$WORKSPACE" "$OUTPUT"
grep -Fq "ExecStart=/bin/bash \"$WORKSPACE/start-all-bin.sh\" release" "$OUTPUT"
grep -Fq "Environment=\"RUSTCLAW_SYSTEMD_UNIT=rustclaw.service\"" "$OUTPUT"
grep -Fq "Environment=\"RUSTCLAW_RUNTIME_ENV_SCRIPT=$TMP_ROOT/runtime_env.sh\"" "$OUTPUT"
grep -Fq "Environment=\"HOME=" "$OUTPUT"
grep -Fq "TimeoutStartSec=0" "$OUTPUT"
if grep -Fq "/home/guagua/rustclaw" "$OUTPUT"; then
  echo "Rendered unit contains the former hardcoded workspace path." >&2
  exit 1
fi

if bash "$SCRIPT_DIR/install-systemd-service.sh" \
  --workspace "$WORKSPACE" \
  --user "$(id -un)" \
  --unit-name "rustclaw.service;reboot" \
  --output "$TMP_ROOT/invalid.service" >/dev/null 2>&1; then
  echo "Unsafe unit name was accepted." >&2
  exit 1
fi

echo "SYSTEMD_SERVICE_INSTALLER_TESTS ok"

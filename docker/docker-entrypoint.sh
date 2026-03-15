#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$SCRIPT_DIR"

PROFILE="${RUSTCLAW_PROFILE:-release}"
PID_DIR="$SCRIPT_DIR/.pids"
MOUNTED_CONFIG_DIR="$SCRIPT_DIR/docker/config"
MOUNTED_CONFIG_FILE="$MOUNTED_CONFIG_DIR/config.toml"
MOUNTED_REGISTRY_FILE="$MOUNTED_CONFIG_DIR/skills_registry.toml"
ACTIVE_CONFIG_FILE="$SCRIPT_DIR/configs/config.toml"
ACTIVE_REGISTRY_FILE="$SCRIPT_DIR/configs/skills_registry.toml"

sync_config_from_mount() {
  if [[ -f "$MOUNTED_CONFIG_FILE" ]]; then
    cp "$MOUNTED_CONFIG_FILE" "$ACTIVE_CONFIG_FILE"
    echo "Loaded config override from $MOUNTED_CONFIG_FILE"
  elif [[ ! -f "$MOUNTED_CONFIG_FILE" ]]; then
    cp "$ACTIVE_CONFIG_FILE" "$MOUNTED_CONFIG_FILE"
    echo "Seeded mounted config at $MOUNTED_CONFIG_FILE"
  fi

  if [[ -f "$MOUNTED_REGISTRY_FILE" ]]; then
    cp "$MOUNTED_REGISTRY_FILE" "$ACTIVE_REGISTRY_FILE"
    echo "Loaded skills registry override from $MOUNTED_REGISTRY_FILE"
  elif [[ -f "$ACTIVE_REGISTRY_FILE" ]]; then
    cp "$ACTIVE_REGISTRY_FILE" "$MOUNTED_REGISTRY_FILE"
    echo "Seeded mounted skills registry at $MOUNTED_REGISTRY_FILE"
  fi
}

cleanup() {
  "$SCRIPT_DIR/stop-rustclaw.sh" || true
}

handle_signal() {
  cleanup
  exit 0
}

trap handle_signal TERM INT

mkdir -p "$PID_DIR" "$SCRIPT_DIR/logs" "$SCRIPT_DIR/data" "$SCRIPT_DIR/image" "$SCRIPT_DIR/audio" "$SCRIPT_DIR/document" "$MOUNTED_CONFIG_DIR"
sync_config_from_mount

"$SCRIPT_DIR/start-all-bin.sh" "$PROFILE"

while true; do
  shopt -s nullglob
  pid_files=("$PID_DIR"/*.pid)
  shopt -u nullglob

  if [[ ${#pid_files[@]} -eq 0 ]]; then
    echo "No service PID files found under $PID_DIR, exiting."
    cleanup
    exit 1
  fi

  for pid_file in "${pid_files[@]}"; do
    pid="$(<"$pid_file")"
    if [[ -z "${pid}" || ! "$pid" =~ ^[0-9]+$ ]]; then
      echo "Invalid pid file: $pid_file"
      cleanup
      exit 1
    fi
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      echo "Service stopped unexpectedly: $pid_file (PID=$pid)"
      cleanup
      exit 1
    fi
  done

  sleep 2
done

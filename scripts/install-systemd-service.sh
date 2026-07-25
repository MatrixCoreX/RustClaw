#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

ACTION="install"
WORKSPACE="$ROOT_DIR"
RUN_USER="${SUDO_USER:-$(id -un)}"
UNIT_NAME="rustclaw.service"
UNIT_DIR="${RUSTCLAW_SYSTEMD_UNIT_DIR:-/etc/systemd/system}"
RUNTIME_ENV_SCRIPT="${RUSTCLAW_RUNTIME_ENV_SCRIPT:-}"
OUTPUT_PATH=""
ENABLE_SERVICE=0
START_SERVICE=0

usage() {
  cat <<'EOF'
Usage:
  bash scripts/install-systemd-service.sh [options]

Options:
  --workspace PATH       RustClaw workspace/runtime directory
  --user USER            Operating-system user that runs RustClaw
  --runtime-env PATH     Optional shell environment file sourced by startup scripts
  --unit-name NAME       Systemd unit name (default: rustclaw.service)
  --unit-dir PATH        Systemd unit directory (default: /etc/systemd/system)
  --output PATH          Render the unit to PATH without installing it
  --print                Render the unit to stdout without installing it
  --enable               Enable the unit after installation
  --start                Start or restart the unit after installation
  --uninstall            Stop, disable, and remove the installed unit
  -h, --help             Show this help

Installation is supported only on Linux hosts running systemd. Rendering with
--print or --output is available on other hosts for review and testing.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace)
      WORKSPACE="${2:-}"
      shift 2
      ;;
    --user)
      RUN_USER="${2:-}"
      shift 2
      ;;
    --runtime-env)
      RUNTIME_ENV_SCRIPT="${2:-}"
      shift 2
      ;;
    --unit-name)
      UNIT_NAME="${2:-}"
      shift 2
      ;;
    --unit-dir)
      UNIT_DIR="${2:-}"
      shift 2
      ;;
    --output)
      OUTPUT_PATH="${2:-}"
      ACTION="render"
      shift 2
      ;;
    --print)
      ACTION="print"
      shift
      ;;
    --enable)
      ENABLE_SERVICE=1
      shift
      ;;
    --start)
      START_SERVICE=1
      shift
      ;;
    --uninstall)
      ACTION="uninstall"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ ! "$UNIT_NAME" =~ ^[A-Za-z0-9_.@-]+\.service$ ]]; then
  echo "Invalid systemd unit name: $UNIT_NAME" >&2
  exit 2
fi

run_privileged() {
  if [[ "$(id -u)" == "0" ]]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    echo "Root access or sudo is required for systemd installation." >&2
    return 1
  fi
}

require_systemd_host() {
  if [[ "$(uname -s)" != "Linux" ]]; then
    echo "Systemd service installation is supported only on Linux." >&2
    exit 3
  fi
  if ! command -v systemctl >/dev/null 2>&1; then
    echo "systemctl is unavailable; use direct RustClaw process startup." >&2
    exit 3
  fi
  if [[ ! -d /run/systemd/system ]]; then
    echo "Systemd is not the active service manager on this host." >&2
    exit 3
  fi
}

if [[ "$ACTION" == "uninstall" ]]; then
  require_systemd_host
  UNIT_PATH="${UNIT_DIR%/}/$UNIT_NAME"
  run_privileged systemctl disable --now "$UNIT_NAME" >/dev/null 2>&1 || true
  run_privileged rm -f "$UNIT_PATH"
  run_privileged systemctl daemon-reload
  echo "Removed systemd unit: $UNIT_PATH"
  exit 0
fi

if [[ -z "$WORKSPACE" || ! -d "$WORKSPACE" ]]; then
  echo "Workspace directory does not exist: $WORKSPACE" >&2
  exit 2
fi
WORKSPACE="$(cd "$WORKSPACE" && pwd)"

for required in start-all-bin.sh stop-rustclaw.sh; do
  if [[ ! -f "$WORKSPACE/$required" ]]; then
    echo "Missing runtime script: $WORKSPACE/$required" >&2
    exit 2
  fi
done

if ! id "$RUN_USER" >/dev/null 2>&1; then
  echo "Operating-system user does not exist: $RUN_USER" >&2
  exit 2
fi
RUN_GROUP="$(id -gn "$RUN_USER")"
RUN_HOME="$(
  if command -v getent >/dev/null 2>&1; then
    getent passwd "$RUN_USER" | awk -F: '{print $6}'
  fi
)"
if [[ -z "$RUN_HOME" ]]; then
  RUN_HOME="$HOME"
fi

if [[ -z "$RUNTIME_ENV_SCRIPT" && -f "$(dirname "$WORKSPACE")/runtime_env_filled.sh" ]]; then
  RUNTIME_ENV_SCRIPT="$(dirname "$WORKSPACE")/runtime_env_filled.sh"
fi
if [[ -n "$RUNTIME_ENV_SCRIPT" ]]; then
  if [[ ! -f "$RUNTIME_ENV_SCRIPT" ]]; then
    echo "Runtime environment script does not exist: $RUNTIME_ENV_SCRIPT" >&2
    exit 2
  fi
  RUNTIME_ENV_SCRIPT="$(cd "$(dirname "$RUNTIME_ENV_SCRIPT")" && pwd)/$(basename "$RUNTIME_ENV_SCRIPT")"
fi

systemd_quote() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '"%s"' "$value"
}

systemd_path() {
  local value="$1"
  if [[ "$value" == *$'\n'* || "$value" == *$'\r'* ]]; then
    echo "Systemd paths must not contain line breaks." >&2
    return 1
  fi
  value="${value//\\/\\\\}"
  printf '%s' "$value"
}

render_unit() {
  local workspace_path home_env_q config_env_q unit_env_q start_q stop_q pid_path
  workspace_path="$(systemd_path "$WORKSPACE")"
  home_env_q="$(systemd_quote "HOME=$RUN_HOME")"
  config_env_q="$(systemd_quote "RUSTCLAW_CONFIG_PATH=$WORKSPACE/configs/config.toml")"
  unit_env_q="$(systemd_quote "RUSTCLAW_SYSTEMD_UNIT=$UNIT_NAME")"
  start_q="$(systemd_quote "$WORKSPACE/start-all-bin.sh")"
  stop_q="$(systemd_quote "$WORKSPACE/stop-rustclaw.sh")"
  pid_path="$(systemd_path "$WORKSPACE/.pids/clawd.pid")"

  cat <<EOF
[Unit]
Description=RustClaw Runtime
Wants=network-online.target
After=network-online.target

[Service]
Type=forking
User=$RUN_USER
Group=$RUN_GROUP
WorkingDirectory=$workspace_path
Environment=$home_env_q
Environment=$config_env_q
Environment=$unit_env_q
Environment="RUSTCLAW_MODEL_SELECT=0"
Environment="RUSTCLAW_LOG_COLOR=0"
Environment="RUST_LOG=info"
EOF
  if [[ -n "$RUNTIME_ENV_SCRIPT" ]]; then
    printf 'Environment=%s\n' \
      "$(systemd_quote "RUSTCLAW_RUNTIME_ENV_SCRIPT=$RUNTIME_ENV_SCRIPT")"
  fi
  cat <<EOF
ExecStart=/bin/bash $start_q release
ExecStop=/bin/bash $stop_q
PIDFile=$pid_path
Restart=on-failure
RestartSec=5
TimeoutStartSec=0
TimeoutStopSec=45
KillMode=control-group
TasksMax=512

[Install]
WantedBy=multi-user.target
EOF
}

if [[ "$ACTION" == "print" ]]; then
  render_unit
  exit 0
fi

if [[ "$ACTION" == "render" ]]; then
  if [[ -z "$OUTPUT_PATH" ]]; then
    echo "--output requires a path." >&2
    exit 2
  fi
  mkdir -p "$(dirname "$OUTPUT_PATH")"
  render_unit > "$OUTPUT_PATH"
  echo "Rendered systemd unit: $OUTPUT_PATH"
  exit 0
fi

require_systemd_host
UNIT_PATH="${UNIT_DIR%/}/$UNIT_NAME"
TMP_UNIT="$(mktemp)"
trap 'rm -f "$TMP_UNIT"' EXIT
render_unit > "$TMP_UNIT"
run_privileged install -D -m 0644 "$TMP_UNIT" "$UNIT_PATH"
run_privileged systemctl daemon-reload
if [[ "$ENABLE_SERVICE" == "1" ]]; then
  run_privileged systemctl enable "$UNIT_NAME"
fi
if [[ "$START_SERVICE" == "1" ]]; then
  run_privileged systemctl restart "$UNIT_NAME"
fi
echo "Installed systemd unit: $UNIT_PATH"

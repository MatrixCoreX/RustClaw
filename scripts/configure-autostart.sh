#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/product_identity.sh"

ACTION="enable"
WORKSPACE="$ROOT_DIR"
NON_INTERACTIVE=0
SYSTEMD_RUNTIME_DIR="${APP_SYSTEMD_RUNTIME_DIR:-/run/systemd/system}"
SYSTEMD_UNIT_DIR="${APP_SYSTEMD_UNIT_DIR:-/etc/systemd/system}"
UNIT_NAME="${APP_SERVICE_NAME}.service"
LAUNCHD_LABEL="$APP_SERVICE_NAME"

usage() {
  cat <<'EOF'
Usage: bash scripts/configure-autostart.sh [options]

Options:
  --enable             Register startup at boot/login (default)
  --disable            Remove the registered startup entry
  --status             Print the current startup registration
  --workspace PATH     Runtime installation directory
  --non-interactive    Never prompt for elevated privileges
  -h, --help           Show this help

Linux uses systemd. Interactive installation prefers a system unit; a normal
runtime start without elevation falls back to a user unit. macOS uses a
per-user LaunchAgent. Unsupported service managers return an explicit status.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --enable) ACTION="enable"; shift ;;
    --disable) ACTION="disable"; shift ;;
    --status) ACTION="status"; shift ;;
    --workspace)
      [[ $# -ge 2 && -n "$2" ]] || { usage >&2; exit 2; }
      WORKSPACE="$2"
      shift 2
      ;;
    --non-interactive) NON_INTERACTIVE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

[[ -d "$WORKSPACE" ]] || {
  echo "Autostart workspace does not exist: $WORKSPACE" >&2
  exit 2
}
WORKSPACE="$(cd "$WORKSPACE" && pwd -P)"
for required in start-all-bin.sh stop-agent.sh; do
  [[ -f "$WORKSPACE/$required" ]] || {
    echo "Autostart runtime script is missing: $WORKSPACE/$required" >&2
    exit 2
  }
done

systemd_available() {
  [[ "$(uname -s 2>/dev/null || true)" == "Linux" ]] \
    && command -v systemctl >/dev/null 2>&1 \
    && [[ -d "$SYSTEMD_RUNTIME_DIR" ]]
}

system_unit_enabled() {
  systemd_available && systemctl is-enabled --quiet "$UNIT_NAME" >/dev/null 2>&1
}

user_unit_path() {
  printf '%s/.config/systemd/user/%s\n' "$HOME" "$UNIT_NAME"
}

user_unit_enabled() {
  local unit_path
  unit_path="$(user_unit_path)"
  [[ -f "$unit_path" ]] || return 1
  systemctl --user is-enabled --quiet "$UNIT_NAME" >/dev/null 2>&1
}

systemd_quote() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '"%s"' "$value"
}

render_user_unit() {
  local start_q stop_q home_q config_q pid_path
  start_q="$(systemd_quote "$WORKSPACE/start-all-bin.sh")"
  stop_q="$(systemd_quote "$WORKSPACE/stop-agent.sh")"
  home_q="$(systemd_quote "HOME=$HOME")"
  config_q="$(systemd_quote "APP_CONFIG_PATH=$WORKSPACE/configs/config.toml")"
  pid_path="$WORKSPACE/.pids/clawd.pid"
  cat <<EOF
[Unit]
Description=Agent Runtime
Wants=network-online.target
After=network-online.target

[Service]
Type=forking
WorkingDirectory=$WORKSPACE
Environment=$home_q
Environment=$config_q
Environment="APP_AUTOSTART_MANAGED=1"
Environment="APP_MODEL_SELECT=0"
Environment="APP_LOG_COLOR=0"
Environment="RUST_LOG=info"
ExecStart=/bin/bash $start_q release
ExecStop=/bin/bash $stop_q
PIDFile=$pid_path
Restart=on-failure
RestartSec=5
TimeoutStartSec=0
TimeoutStopSec=45
UMask=0077

[Install]
WantedBy=default.target
EOF
}

enable_user_systemd() {
  local unit_path tmp_path
  unit_path="$(user_unit_path)"
  mkdir -p "$(dirname "$unit_path")"
  tmp_path="$(mktemp "${TMPDIR:-/tmp}/agent-autostart-unit.XXXXXX")"
  if ! render_user_unit > "$tmp_path" || ! install -m 0600 "$tmp_path" "$unit_path"; then
    rm -f "$tmp_path"
    return 1
  fi
  rm -f "$tmp_path"
  systemctl --user daemon-reload
  systemctl --user enable "$UNIT_NAME" >/dev/null
  if command -v loginctl >/dev/null 2>&1; then
    loginctl enable-linger "$(id -un)" >/dev/null 2>&1 || true
  fi
  printf 'autostart=enabled manager=systemd-user unit=%s\n' "$UNIT_NAME"
}

disable_user_systemd() {
  local unit_path
  unit_path="$(user_unit_path)"
  systemctl --user disable "$UNIT_NAME" >/dev/null 2>&1 || true
  rm -f "$unit_path"
  systemctl --user daemon-reload >/dev/null 2>&1 || true
}

can_install_system_unit_without_prompt() {
  [[ "$(id -u)" == "0" ]] && return 0
  command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1
}

enable_linux() {
  if ! systemd_available; then
    echo "autostart=unsupported manager=none reason=systemd_unavailable" >&2
    return 3
  fi
  if system_unit_enabled; then
    disable_user_systemd
    printf 'autostart=enabled manager=systemd-system unit=%s\n' "$UNIT_NAME"
    return 0
  fi
  if [[ "$NON_INTERACTIVE" == "0" ]] || can_install_system_unit_without_prompt; then
    APP_SYSTEMD_RUNTIME_DIR="$SYSTEMD_RUNTIME_DIR" \
      APP_SYSTEMD_UNIT_DIR="$SYSTEMD_UNIT_DIR" \
      bash "$SCRIPT_DIR/install-systemd-service.sh" \
        --workspace "$WORKSPACE" \
        --user "${SUDO_USER:-$(id -un)}" \
        --enable
    disable_user_systemd
    printf 'autostart=enabled manager=systemd-system unit=%s\n' "$UNIT_NAME"
    return 0
  fi
  enable_user_systemd
}

disable_linux() {
  local status=0
  if system_unit_enabled || [[ -f "${SYSTEMD_UNIT_DIR%/}/$UNIT_NAME" ]]; then
    if [[ "$NON_INTERACTIVE" == "1" ]] && ! can_install_system_unit_without_prompt; then
      echo "autostart=disable_failed manager=systemd-system reason=privilege_required" >&2
      status=1
    elif ! APP_SYSTEMD_RUNTIME_DIR="$SYSTEMD_RUNTIME_DIR" \
      APP_SYSTEMD_UNIT_DIR="$SYSTEMD_UNIT_DIR" \
      bash "$SCRIPT_DIR/install-systemd-service.sh" --uninstall; then
      status=1
    fi
  fi
  disable_user_systemd
  if [[ "$status" == "0" ]]; then
    echo "autostart=disabled manager=systemd"
  fi
  return "$status"
}

status_linux() {
  if system_unit_enabled; then
    printf 'autostart=enabled manager=systemd-system unit=%s\n' "$UNIT_NAME"
  elif user_unit_enabled; then
    printf 'autostart=enabled manager=systemd-user unit=%s\n' "$UNIT_NAME"
  else
    echo "autostart=disabled manager=systemd"
  fi
}

launch_agent_path() {
  printf '%s/Library/LaunchAgents/%s.plist\n' "$HOME" "$LAUNCHD_LABEL"
}

enable_macos() {
  local plist_path
  plist_path="$(launch_agent_path)"
  mkdir -p "$(dirname "$plist_path")" "$WORKSPACE/logs"
  python3 - "$plist_path" "$LAUNCHD_LABEL" "$WORKSPACE" "$HOME" <<'PY'
import os
import plistlib
import sys
from pathlib import Path

path, label, workspace, home = sys.argv[1:]
payload = {
    "Label": label,
    "ProgramArguments": ["/bin/bash", f"{workspace}/start-all-bin.sh", "release"],
    "WorkingDirectory": workspace,
    "RunAtLoad": True,
    "ProcessType": "Background",
    "EnvironmentVariables": {
        "HOME": home,
        "APP_AUTOSTART_MANAGED": "1",
        "APP_CONFIG_PATH": f"{workspace}/configs/config.toml",
        "APP_MODEL_SELECT": "0",
        "APP_LOG_COLOR": "0",
        "RUST_LOG": "info",
    },
    "StandardOutPath": f"{workspace}/logs/launchd-autostart.log",
    "StandardErrorPath": f"{workspace}/logs/launchd-autostart.log",
}
target = Path(path)
temporary = target.with_name(f".{target.name}.tmp")
with temporary.open("wb") as stream:
    plistlib.dump(payload, stream, fmt=plistlib.FMT_XML, sort_keys=True)
os.chmod(temporary, 0o600)
os.replace(temporary, target)
PY
  if command -v launchctl >/dev/null 2>&1; then
    launchctl enable "gui/$(id -u)/$LAUNCHD_LABEL" >/dev/null 2>&1 || true
  fi
  printf 'autostart=enabled manager=launchd label=%s\n' "$LAUNCHD_LABEL"
}

disable_macos() {
  local plist_path
  plist_path="$(launch_agent_path)"
  if command -v launchctl >/dev/null 2>&1; then
    launchctl bootout "gui/$(id -u)" "$plist_path" >/dev/null 2>&1 || true
    launchctl disable "gui/$(id -u)/$LAUNCHD_LABEL" >/dev/null 2>&1 || true
  fi
  rm -f "$plist_path"
  echo "autostart=disabled manager=launchd"
}

status_macos() {
  if [[ -f "$(launch_agent_path)" ]]; then
    printf 'autostart=enabled manager=launchd label=%s\n' "$LAUNCHD_LABEL"
  else
    echo "autostart=disabled manager=launchd"
  fi
}

case "$(uname -s 2>/dev/null || true)" in
  Linux)
    case "$ACTION" in
      enable) enable_linux ;;
      disable) disable_linux ;;
      status) status_linux ;;
    esac
    ;;
  Darwin)
    case "$ACTION" in
      enable) enable_macos ;;
      disable) disable_macos ;;
      status) status_macos ;;
    esac
    ;;
  *)
    echo "autostart=unsupported manager=none reason=unsupported_platform" >&2
    exit 3
    ;;
esac

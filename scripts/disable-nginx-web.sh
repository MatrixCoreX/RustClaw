#!/usr/bin/env bash
# Stop nginx and remove only the RustClaw nginx site and its dedicated UI root.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"

HOST_OS="$(detect_host_os || printf '%s' "unknown")"

nginx_config_candidates() {
  if [[ "$HOST_OS" == "macos" ]]; then
    local brew_prefix=""
    if command -v brew >/dev/null 2>&1; then
      brew_prefix="$(brew --prefix 2>/dev/null || true)"
    fi
    if [[ -n "$brew_prefix" ]]; then
      printf '%s\n' "$brew_prefix/etc/nginx/servers/rustclaw-ui.conf"
    fi
    printf '%s\n' "/opt/homebrew/etc/nginx/servers/rustclaw-ui.conf"
    printf '%s\n' "/usr/local/etc/nginx/servers/rustclaw-ui.conf"
    return
  fi
  printf '%s\n' "/etc/nginx/sites-available/rustclaw-ui.conf"
  printf '%s\n' "/etc/nginx/conf.d/rustclaw-ui.conf"
}

rustclaw_ui_root() {
  local config="$1"
  awk '
    /^[[:space:]]*root[[:space:]]+/ {
      value = $0
      sub(/^[[:space:]]*root[[:space:]]+/, "", value)
      sub(/;[[:space:]]*$/, "", value)
      print value
      exit
    }
  ' "$config"
}

remove_path() {
  local path="$1"
  if [[ -w "$(dirname "$path")" ]]; then
    rm -rf -- "$path"
  else
    sudo rm -rf -- "$path"
  fi
}

remove_rustclaw_site() {
  local config="$1"
  [[ -f "$config" ]] || return 0
  if ! grep -Fq "# RustClaw UI" "$config"; then
    echo "Refusing to remove unrecognized nginx config: $config" >&2
    return 1
  fi

  local ui_root=""
  ui_root="$(rustclaw_ui_root "$config")"
  if [[ -n "$ui_root" ]]; then
    case "$ui_root" in
      */rustclaw|*/nginx-ui)
        if [[ -e "$ui_root" ]]; then
          remove_path "$ui_root"
          echo "Removed RustClaw UI deployment: $ui_root"
        fi
        ;;
      *)
        echo "Refusing to delete non-dedicated UI root: $ui_root" >&2
        echo "Use a dedicated path ending in /rustclaw or /nginx-ui, or remove it manually." >&2
        return 1
        ;;
    esac
  fi

  local site_link="/etc/nginx/sites-enabled/$(basename "$config")"
  if [[ "$HOST_OS" != "macos" && ( -e "$site_link" || -L "$site_link" ) ]]; then
    remove_path "$site_link"
  fi
  remove_path "$config"
  echo "Removed RustClaw nginx site: $config"
}

stop_nginx() {
  if [[ "$HOST_OS" == "macos" ]]; then
    if ! command -v brew >/dev/null 2>&1; then
      echo "Homebrew is required to manage nginx on macOS." >&2
      return 1
    fi
    brew services stop nginx >/dev/null 2>&1 || true
    echo "Stopped nginx through Homebrew services."
    return
  fi

  if command -v systemctl >/dev/null 2>&1 && [[ -d /run/systemd/system ]]; then
    sudo systemctl disable --now nginx
    echo "Stopped and disabled nginx through systemd."
  elif command -v rc-service >/dev/null 2>&1; then
    sudo rc-service nginx stop || true
    if command -v rc-update >/dev/null 2>&1; then
      sudo rc-update del nginx default >/dev/null 2>&1 || true
    fi
    echo "Stopped nginx through OpenRC."
  elif command -v service >/dev/null 2>&1; then
    sudo service nginx stop
    echo "Stopped nginx through the service manager."
  elif command -v nginx >/dev/null 2>&1; then
    sudo nginx -s stop
    echo "Stopped nginx directly."
  else
    echo "nginx is not installed or no supported service manager was found."
  fi
}

stop_nginx

found=0
while IFS= read -r config; do
  [[ -n "$config" && -f "$config" ]] || continue
  found=1
  remove_rustclaw_site "$config"
done < <(nginx_config_candidates)

if [[ "$found" == "0" ]]; then
  echo "No RustClaw nginx site configuration was found."
fi

echo "nginx_disabled=ok"

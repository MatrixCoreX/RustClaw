#!/usr/bin/env bash
# Set the local mDNS hostname without changing the runtime's internal identity.

set -euo pipefail

PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${PATH:-}"

LOCAL_HOSTNAME=""

usage() {
  cat <<'EOF'
Usage: configure-local-mdns.sh --set <hostname>

Set the single-label hostname advertised as <hostname>.local on the local network.
The value may be supplied with or without the trailing .local suffix.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --set)
      [[ $# -ge 2 ]] || { echo "--set requires a value" >&2; exit 2; }
      LOCAL_HOSTNAME="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

LOCAL_HOSTNAME="$(printf '%s' "$LOCAL_HOSTNAME" | tr '[:upper:]' '[:lower:]')"
LOCAL_HOSTNAME="${LOCAL_HOSTNAME%.local}"
if [[ -z "$LOCAL_HOSTNAME" \
  || ${#LOCAL_HOSTNAME} -gt 63 \
  || ! "$LOCAL_HOSTNAME" =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ ]]; then
  echo "Invalid local mDNS hostname." >&2
  exit 2
fi

if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "Run this script as root." >&2
    exit 1
  fi
  exec sudo -- "$0" --set "$LOCAL_HOSTNAME"
fi

update_linux_hosts() {
  local hosts_file="/etc/hosts"
  local staged
  staged="$(mktemp /etc/.hosts.agent-runtime.XXXXXX)"
  awk -v hostname="$LOCAL_HOSTNAME" '
    BEGIN { replaced = 0 }
    $1 == "127.0.1.1" && replaced == 0 {
      print "127.0.1.1\t" hostname " " hostname
      replaced = 1
      next
    }
    { print }
    END {
      if (replaced == 0) print "127.0.1.1\t" hostname " " hostname
    }
  ' "$hosts_file" > "$staged"
  chmod --reference="$hosts_file" "$staged" 2>/dev/null || chmod 644 "$staged"
  chown --reference="$hosts_file" "$staged" 2>/dev/null || chown root:root "$staged"
  mv -f "$staged" "$hosts_file"
}

configure_linux() {
  command -v hostnamectl >/dev/null 2>&1 || {
    echo "hostnamectl is required on Linux." >&2
    exit 1
  }
  if ! command -v avahi-daemon >/dev/null 2>&1; then
    if ! command -v apt-get >/dev/null 2>&1; then
      echo "avahi-daemon is required for local mDNS discovery." >&2
      exit 1
    fi
    apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y avahi-daemon
  fi

  hostnamectl set-hostname "$LOCAL_HOSTNAME"
  update_linux_hosts
  if command -v systemctl >/dev/null 2>&1 && [[ -d /run/systemd/system ]]; then
    systemctl enable avahi-daemon >/dev/null 2>&1 || true
    systemctl restart avahi-daemon
  elif command -v service >/dev/null 2>&1; then
    service avahi-daemon restart
  else
    echo "Unable to restart avahi-daemon." >&2
    exit 1
  fi
}

configure_macos() {
  command -v scutil >/dev/null 2>&1 || {
    echo "scutil is required on macOS." >&2
    exit 1
  }
  scutil --set LocalHostName "$LOCAL_HOSTNAME"
  killall -HUP mDNSResponder >/dev/null 2>&1 || true
}

case "$(uname -s)" in
  Linux) configure_linux ;;
  Darwin) configure_macos ;;
  *)
    echo "Local mDNS configuration is supported on Linux and macOS." >&2
    exit 1
    ;;
esac

cat <<EOF
mdns_name_updated=true
hostname=$LOCAL_HOSTNAME
mdns_name=$LOCAL_HOSTNAME.local
EOF

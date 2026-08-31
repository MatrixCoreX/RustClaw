#!/usr/bin/env bash
# Restore the nginx site that existed before local-LAN HTTPS was enabled.

set -euo pipefail

PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${PATH:-}"

NGINX_CONF="${APP_LAN_HTTPS_NGINX_CONF:-/etc/nginx/sites-available/agent-runtime-ui.conf}"
CA_ROOT="${APP_LAN_HTTPS_CA_ROOT:-/etc/agent-runtime/local-ca}"
TLS_ROOT="${APP_LAN_HTTPS_TLS_ROOT:-/etc/agent-runtime/tls}"
PUBLIC_ROOT="${APP_LAN_HTTPS_PUBLIC_ROOT:-/var/lib/agent-runtime/public}"
PURGE=0

usage() {
  cat <<'EOF'
Usage: restore-local-lan-http.sh [--nginx-conf <path>] [--purge]

Restore the nginx configuration saved before local-LAN HTTPS was enabled.
Certificate material is retained by default so HTTPS can be enabled again
without reinstalling the CA. --purge deletes the local CA and leaf keys only
after nginx has been restored successfully.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --nginx-conf)
      [[ $# -ge 2 ]] || { echo "--nginx-conf requires a value" >&2; exit 2; }
      NGINX_CONF="$2"
      shift 2
      ;;
    --purge)
      PURGE=1
      shift
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

if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "Run this script as root." >&2
    exit 1
  fi
  if [[ "$PURGE" == "1" ]]; then
    exec sudo -- "$0" --nginx-conf "$NGINX_CONF" --purge
  fi
  exec sudo -- "$0" --nginx-conf "$NGINX_CONF"
fi

BACKUP_PATH="${NGINX_CONF}.before-local-https"
if [[ ! -f "$BACKUP_PATH" ]]; then
  echo "No pre-HTTPS nginx backup exists: $BACKUP_PATH" >&2
  exit 1
fi

CURRENT_SNAPSHOT="$(mktemp "$(dirname "$NGINX_CONF")/.agent-lan-restore.XXXXXX")"
cp -a "$NGINX_CONF" "$CURRENT_SNAPSHOT"
trap 'rm -f "$CURRENT_SNAPSHOT"' EXIT
cp -a "$BACKUP_PATH" "$NGINX_CONF"

if ! nginx -t; then
  cp -a "$CURRENT_SNAPSHOT" "$NGINX_CONF"
  nginx -t || true
  echo "nginx rejected the restored configuration; returned to HTTPS." >&2
  exit 1
fi

if command -v systemctl >/dev/null 2>&1 && [[ -d /run/systemd/system ]]; then
  systemctl reload nginx
elif command -v service >/dev/null 2>&1; then
  service nginx reload
else
  nginx -s reload
fi

restored_without_https=0
if ! grep -Eq '^[[:space:]]*listen[[:space:]]+.*443' "$NGINX_CONF"; then
  for _ in {1..50}; do
    if ! ss -H -ltn 2>/dev/null | awk '$4 ~ /:443$/ { found = 1 } END { exit(found ? 0 : 1) }'; then
      restored_without_https=1
      break
    fi
    sleep 0.2
  done
  if [[ "$restored_without_https" != "1" ]]; then
    cp -a "$CURRENT_SNAPSHOT" "$NGINX_CONF"
    nginx -t || true
    if command -v systemctl >/dev/null 2>&1 && [[ -d /run/systemd/system ]]; then
      systemctl reload nginx || true
    elif command -v service >/dev/null 2>&1; then
      service nginx reload || true
    else
      nginx -s reload || true
    fi
    echo "nginx did not finish restoring the pre-HTTPS listener state." >&2
    exit 1
  fi
fi

if [[ "$PURGE" == "1" ]]; then
  if [[ -d "$CA_ROOT" ]] && command -v mkcert >/dev/null 2>&1; then
    CAROOT="$CA_ROOT" mkcert -uninstall || true
  fi
  rm -rf -- "$CA_ROOT" "$TLS_ROOT"
  rm -f -- "$PUBLIC_ROOT/local-device-ca.crt" "$PUBLIC_ROOT/local-device-ca.sha256"
fi

echo "local_lan_https=restored_to_previous_nginx_config"
echo "certificate_material=$([[ "$PURGE" == "1" ]] && echo removed || echo retained)"

#!/usr/bin/env bash
# Configure a local-CA HTTPS nginx entry for a LAN device without exposing the internal runtime.

set -euo pipefail

PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${PATH:-}"

NGINX_CONF="${APP_LAN_HTTPS_NGINX_CONF:-/etc/nginx/sites-available/agent-runtime-ui.conf}"
NGINX_SITE_LINK="${APP_LAN_HTTPS_NGINX_SITE_LINK:-/etc/nginx/sites-enabled/agent-runtime-ui.conf}"
UI_ROOT="${APP_LAN_HTTPS_UI_ROOT:-/var/www/html/agent-runtime}"
UPSTREAM="${APP_LAN_HTTPS_UPSTREAM:-http://127.0.0.1:8788}"
CA_ROOT="${APP_LAN_HTTPS_CA_ROOT:-/etc/agent-runtime/local-ca}"
TLS_ROOT="${APP_LAN_HTTPS_TLS_ROOT:-/etc/agent-runtime/tls}"
PUBLIC_ROOT="${APP_LAN_HTTPS_PUBLIC_ROOT:-/var/lib/agent-runtime/public}"
LAN_IP=""
LAN_HOSTNAME=""
PREPARE_ONLY=0

usage() {
  cat <<'EOF'
Usage: configure-local-lan-https.sh [options]

Configure nginx HTTPS for the device's current private IPv4 address.

Options:
  --ip <address>       Override automatic default-route IPv4 detection.
  --hostname <name>    Add a stable local hostname to the certificate.
  --nginx-conf <path>  Override the nginx site configuration path.
  --ui-root <path>     Override the deployed UI root.
  --upstream <url>     Override the loopback WEBD upstream.
  --prepare-only       Create/renew certificates without changing nginx.
  -h, --help           Show this help.

Use --prepare-only before asking a browser to trust the local CA. Re-running
the script reuses that CA and renews the leaf certificate for the current IP.
Without --prepare-only, the original nginx file is backed up on the first run
and HTTPS is activated.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ip)
      [[ $# -ge 2 ]] || { echo "--ip requires a value" >&2; exit 2; }
      LAN_IP="$2"
      shift 2
      ;;
    --hostname)
      [[ $# -ge 2 ]] || { echo "--hostname requires a value" >&2; exit 2; }
      LAN_HOSTNAME="$2"
      shift 2
      ;;
    --nginx-conf)
      [[ $# -ge 2 ]] || { echo "--nginx-conf requires a value" >&2; exit 2; }
      NGINX_CONF="$2"
      shift 2
      ;;
    --ui-root)
      [[ $# -ge 2 ]] || { echo "--ui-root requires a value" >&2; exit 2; }
      UI_ROOT="$2"
      shift 2
      ;;
    --upstream)
      [[ $# -ge 2 ]] || { echo "--upstream requires a value" >&2; exit 2; }
      UPSTREAM="$2"
      shift 2
      ;;
    --prepare-only)
      PREPARE_ONLY=1
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
  sudo_args=(
    --nginx-conf "$NGINX_CONF"
    --ui-root "$UI_ROOT"
    --upstream "$UPSTREAM"
  )
  [[ -z "$LAN_HOSTNAME" ]] || sudo_args=(--hostname "$LAN_HOSTNAME" "${sudo_args[@]}")
  [[ -z "$LAN_IP" ]] || sudo_args=(--ip "$LAN_IP" "${sudo_args[@]}")
  [[ "$PREPARE_ONLY" != "1" ]] || sudo_args=(--prepare-only "${sudo_args[@]}")
  exec sudo -- "$0" "${sudo_args[@]}"
fi

[[ "$(uname -s)" == "Linux" ]] || {
  echo "Automatic local-LAN HTTPS configuration currently supports Linux only." >&2
  exit 1
}

install_dependencies() {
  local required_commands=(mkcert openssl ip python3)
  if [[ "$PREPARE_ONLY" != "1" ]]; then
    required_commands+=(nginx curl)
  fi
  local missing=0
  for command_name in "${required_commands[@]}"; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
      missing=1
      break
    fi
  done
  [[ "$missing" == "0" ]] && return 0
  if ! command -v apt-get >/dev/null 2>&1; then
    echo "Install nginx, mkcert, openssl, iproute2, and python3, then retry." >&2
    exit 1
  fi
  apt-get update -qq
  local packages=(mkcert libnss3-tools openssl iproute2 python3)
  if [[ "$PREPARE_ONLY" != "1" ]]; then
    packages+=(nginx curl)
  fi
  DEBIAN_FRONTEND=noninteractive apt-get install -y "${packages[@]}"
}

detect_lan_ip() {
  local detected=""
  detected="$(ip -4 route get 1.1.1.1 2>/dev/null \
    | awk 'NR == 1 { for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit } }')"
  if [[ -z "$detected" ]]; then
    detected="$(ip -4 route show default 2>/dev/null \
      | awk 'NR == 1 { for (i = 1; i <= NF; i++) if ($i == "dev") { print $(i + 1); exit } }' \
      | xargs -r -I{} ip -4 -o addr show dev {} scope global 2>/dev/null \
      | awk 'NR == 1 { split($4, address, "/"); print address[1] }')"
  fi
  printf '%s\n' "$detected"
}

validate_private_ipv4() {
  python3 - "$1" <<'PY'
import ipaddress
import sys

address = ipaddress.ip_address(sys.argv[1])
private_networks = (
    ipaddress.ip_network("10.0.0.0/8"),
    ipaddress.ip_network("172.16.0.0/12"),
    ipaddress.ip_network("192.168.0.0/16"),
)
if address.version != 4 or not any(address in network for network in private_networks):
    raise SystemExit(1)
PY
}

validate_path_value() {
  local label="$1"
  local value="$2"
  if [[ -z "$value" || "$value" == *$'\n'* || "$value" == *';'* ]]; then
    echo "$label contains unsupported characters: $value" >&2
    exit 2
  fi
}

install_dependencies

if [[ -z "$LAN_IP" ]]; then
  LAN_IP="$(detect_lan_ip)"
fi
if [[ -z "$LAN_IP" ]] || ! validate_private_ipv4 "$LAN_IP"; then
  echo "Unable to detect a private default-route IPv4 address; use --ip." >&2
  exit 1
fi

if [[ -z "$LAN_HOSTNAME" ]]; then
  LAN_HOSTNAME="$(hostname -s 2>/dev/null || true)"
fi
if [[ ! "$LAN_HOSTNAME" =~ ^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?$ ]]; then
  echo "Invalid local hostname: $LAN_HOSTNAME" >&2
  exit 2
fi
if [[ ! "$UPSTREAM" =~ ^http://127[.]0[.]0[.]1:[0-9]+$ ]]; then
  echo "The WEBD upstream must be an explicit loopback HTTP URL." >&2
  exit 2
fi
validate_path_value "nginx config path" "$NGINX_CONF"
validate_path_value "UI root" "$UI_ROOT"

install -d -m 700 "$CA_ROOT" "$TLS_ROOT"
install -d -m 755 "$PUBLIC_ROOT"

if [[ ! -s "$CA_ROOT/rootCA.pem" || ! -s "$CA_ROOT/rootCA-key.pem" ]]; then
  CAROOT="$CA_ROOT" mkcert -install
fi

CAROOT="$CA_ROOT" mkcert \
  -cert-file "$TLS_ROOT/lan.pem" \
  -key-file "$TLS_ROOT/lan-key.pem" \
  "$LAN_IP" "$LAN_HOSTNAME" "$LAN_HOSTNAME.local" "$LAN_HOSTNAME.home.arpa" \
  localhost 127.0.0.1 ::1

chown root:root \
  "$CA_ROOT/rootCA.pem" "$CA_ROOT/rootCA-key.pem" \
  "$TLS_ROOT/lan.pem" "$TLS_ROOT/lan-key.pem"
chmod 644 "$CA_ROOT/rootCA.pem" "$TLS_ROOT/lan.pem"
chmod 600 "$CA_ROOT/rootCA-key.pem" "$TLS_ROOT/lan-key.pem"
install -m 644 "$CA_ROOT/rootCA.pem" "$PUBLIC_ROOT/local-device-ca.crt"
openssl x509 -in "$CA_ROOT/rootCA.pem" -noout -fingerprint -sha256 \
  | sed 's/^sha256 Fingerprint=//' > "$PUBLIC_ROOT/local-device-ca.sha256"
chmod 644 "$PUBLIC_ROOT/local-device-ca.sha256"

CA_FINGERPRINT="$(cat "$PUBLIC_ROOT/local-device-ca.sha256")"
if [[ "$PREPARE_ONLY" == "1" ]]; then
  cat <<EOF
local_lan_https=prepared
detected_ip=$LAN_IP
ca_api_path=/v1/system/local-https-ca
ca_fingerprint_sha256=$CA_FINGERPRINT
next_step=install_ca_then_enable_https
EOF
  exit 0
fi

mkdir -p "$(dirname "$NGINX_CONF")" "$(dirname "$NGINX_SITE_LINK")"
BACKUP_PATH="${NGINX_CONF}.before-local-https"
if [[ -f "$NGINX_CONF" && ! -e "$BACKUP_PATH" ]]; then
  cp -a "$NGINX_CONF" "$BACKUP_PATH"
fi

CURRENT_SNAPSHOT="$(mktemp "$(dirname "$NGINX_CONF")/.agent-lan-current.XXXXXX")"
if [[ -f "$NGINX_CONF" ]]; then
  cp -a "$NGINX_CONF" "$CURRENT_SNAPSHOT"
else
  : > "$CURRENT_SNAPSHOT"
fi
NEW_CONFIG="$(mktemp "$(dirname "$NGINX_CONF")/.agent-lan-https.XXXXXX")"
cleanup() {
  rm -f "$CURRENT_SNAPSHOT" "$NEW_CONFIG"
}
trap cleanup EXIT

cat > "$NEW_CONFIG" <<NGINX
# Agent Runtime UI: local-CA HTTPS entry with a loopback-only WEBD upstream.
server {
    listen 0.0.0.0:80;
    listen [::]:80;
    server_name _;
    root $UI_ROOT;
    index index.html;
    client_max_body_size 100m;

    add_header X-Content-Type-Options "nosniff" always;
    add_header X-Frame-Options "DENY" always;
    add_header Referrer-Policy "no-referrer" always;
    add_header Permissions-Policy "camera=(), geolocation=(), microphone=()" always;
    add_header Content-Security-Policy "default-src 'self'; base-uri 'none'; frame-ancestors 'none'; object-src 'none'; form-action 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; media-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self' https: wss:" always;

    location = /local-device-ca.crt {
        alias $PUBLIC_ROOT/local-device-ca.crt;
        default_type application/x-x509-ca-cert;
        add_header Content-Disposition 'attachment; filename="local-device-ca.crt"';
        add_header Cache-Control "no-store";
    }

    location = /local-device-ca.sha256 {
        alias $PUBLIC_ROOT/local-device-ca.sha256;
        default_type text/plain;
        add_header Cache-Control "no-store";
    }

    location ^~ /v1/ {
        proxy_pass $UPSTREAM;
        proxy_http_version 1.1;
        proxy_buffering off;
        proxy_read_timeout 21600s;
        proxy_send_timeout 300s;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto http;
    }

    location ^~ /webd/ {
        proxy_pass $UPSTREAM;
        proxy_http_version 1.1;
        proxy_buffering off;
        proxy_read_timeout 21600s;
        proxy_send_timeout 300s;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto http;
    }

    location = /index.html {
        add_header Cache-Control "no-store, no-cache, must-revalidate" always;
        add_header Pragma "no-cache" always;
        expires -1;
    }

    location / {
        try_files \$uri \$uri/ /index.html;
    }
}

server {
    listen 0.0.0.0:443 ssl;
    listen [::]:443 ssl;
    server_name _;

    root $UI_ROOT;
    index index.html;
    client_max_body_size 100m;

    ssl_certificate $TLS_ROOT/lan.pem;
    ssl_certificate_key $TLS_ROOT/lan-key.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_session_cache shared:LAN_TLS:10m;
    ssl_session_timeout 10m;

    add_header X-Content-Type-Options "nosniff" always;
    add_header X-Frame-Options "DENY" always;
    add_header Referrer-Policy "no-referrer" always;
    add_header Permissions-Policy "camera=(), geolocation=(), microphone=(self)" always;
    add_header Content-Security-Policy "default-src 'self'; base-uri 'none'; frame-ancestors 'none'; object-src 'none'; form-action 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; media-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self' https: wss:" always;

    location ^~ /v1/ {
        proxy_pass $UPSTREAM;
        proxy_http_version 1.1;
        proxy_buffering off;
        proxy_read_timeout 21600s;
        proxy_send_timeout 300s;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto https;
    }

    location ^~ /webd/ {
        proxy_pass $UPSTREAM;
        proxy_http_version 1.1;
        proxy_buffering off;
        proxy_read_timeout 21600s;
        proxy_send_timeout 300s;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto https;
    }

    location = /index.html {
        add_header Cache-Control "no-store, no-cache, must-revalidate" always;
        add_header Pragma "no-cache" always;
        add_header X-Content-Type-Options "nosniff" always;
        add_header X-Frame-Options "DENY" always;
        add_header Referrer-Policy "no-referrer" always;
        add_header Permissions-Policy "camera=(), geolocation=(), microphone=(self)" always;
        add_header Content-Security-Policy "default-src 'self'; base-uri 'none'; frame-ancestors 'none'; object-src 'none'; form-action 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; media-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self' https: wss:" always;
        expires -1;
    }

    location / {
        try_files \$uri \$uri/ /index.html;
    }
}
NGINX

chmod 644 "$NEW_CONFIG"
mv -f "$NEW_CONFIG" "$NGINX_CONF"
ln -sfn "$NGINX_CONF" "$NGINX_SITE_LINK"

if ! nginx -t; then
  if [[ -s "$CURRENT_SNAPSHOT" ]]; then
    cp -a "$CURRENT_SNAPSHOT" "$NGINX_CONF"
  else
    rm -f "$NGINX_CONF"
  fi
  nginx -t || true
  echo "nginx rejected the HTTPS configuration; restored the prior file." >&2
  exit 1
fi

if command -v systemctl >/dev/null 2>&1 && [[ -d /run/systemd/system ]]; then
  systemctl enable nginx >/dev/null 2>&1 || true
  systemctl reload nginx
elif command -v service >/dev/null 2>&1; then
  service nginx reload
else
  nginx -s reload
fi

https_ready=0
for _ in {1..50}; do
  http_status="$(curl --connect-timeout 1 --max-time 2 --silent \
    --output /dev/null --write-out '%{http_code}' http://127.0.0.1/ || true)"
  if [[ "$http_status" == "200" ]] && curl --cacert "$CA_ROOT/rootCA.pem" \
    --connect-timeout 1 --max-time 2 --silent --fail \
    --output /dev/null https://127.0.0.1/; then
    https_ready=1
    break
  fi
  sleep 0.2
done
if [[ "$https_ready" != "1" ]]; then
  if [[ -s "$CURRENT_SNAPSHOT" ]]; then
    cp -a "$CURRENT_SNAPSHOT" "$NGINX_CONF"
  else
    rm -f "$NGINX_CONF"
  fi
  nginx -t || true
  if command -v systemctl >/dev/null 2>&1 && [[ -d /run/systemd/system ]]; then
    systemctl reload nginx || true
  elif command -v service >/dev/null 2>&1; then
    service nginx reload || true
  else
    nginx -s reload || true
  fi
  echo "HTTPS did not become ready after nginx reload; restored the prior file." >&2
  exit 1
fi

cat <<EOF
local_lan_https=enabled
detected_ip=$LAN_IP
http_url=http://$LAN_IP/
https_url=https://$LAN_IP/
ca_download=http://$LAN_IP/local-device-ca.crt
ca_fingerprint_sha256=$CA_FINGERPRINT
restore_command=$(dirname "$0")/restore-local-lan-http.sh
EOF

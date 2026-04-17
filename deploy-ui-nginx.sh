#!/usr/bin/env bash
# 将已构建的 UI/dist 复制到 nginx 目录并刷新 nginx 配置。
# 参数：--deploy / --copy 仅作兼容保留；脚本默认直接部署现有产物。
# 可选：--build 先编译 UI；--path /path/to/nginx/root 指定 nginx 站点根。

set -euo pipefail

# 脚本所在目录 = RustClaw 根目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
UI_DIR="$SCRIPT_DIR/UI"
DIST_DIR="$UI_DIR/dist"
HOST_OS="$(detect_host_os || printf '%s' "unknown")"
HOST_ARCH="$(detect_host_arch || printf '%s' "unknown")"
HOST_TARGET="$(host_rust_target 2>/dev/null || true)"

default_nginx_root() {
  if [[ "$HOST_OS" == "macos" ]]; then
    printf '%s\n' "$HOME/.rustclaw/nginx-ui"
    return
  fi

  printf '%s\n' "/var/www/html/rustclaw"
}

NGINX_ROOT_DEFAULT="$(default_nginx_root)"
NGINX_CONF=""
NGINX_CONF_DIR=""
NGINX_SITE_LINK=""
NGINX_ROOT="$NGINX_ROOT_DEFAULT"
BUILD_UI=0

path_writable_or_creatable() {
  local target="$1"
  if [[ -e "$target" ]]; then
    [[ -w "$target" ]]
    return
  fi
  local parent
  parent="$(dirname "$target")"
  while [[ ! -e "$parent" && "$parent" != "/" ]]; do
    parent="$(dirname "$parent")"
  done
  [[ -w "$parent" ]]
}

usage() {
  echo "Usage: $0 [--deploy|--copy] [--build] [--path DIR]"
  echo "  --deploy        Compatibility flag; deploy existing UI/dist."
  echo "  --copy          Same as --deploy."
  echo "  --build         Build UI before deploy."
  echo "  --path DIR      Nginx site root (default: $NGINX_ROOT_DEFAULT)."
  echo "  (no args)       Copy existing UI/dist to nginx root and configure reverse proxy (default)."
  echo "  host platform   Auto-detected as ${HOST_OS}/${HOST_ARCH} ${HOST_TARGET:+($HOST_TARGET)}."
  echo "  UI source       Reads from UI/dist; use --build to compile first when needed."
  echo ""
  echo "Examples:"
  echo "  $0                    # deploy existing UI/dist to default nginx root"
  echo "  $0 --copy             # deploy existing UI/dist"
  echo "  $0 --build            # build UI, then deploy"
  echo "  $0 --path /srv/http/rustclaw   # deploy to custom path"
}

nginx_conf_path() {
  if [[ "$HOST_OS" == "macos" ]]; then
    local brew_prefix=""
    if command -v brew >/dev/null 2>&1; then
      brew_prefix="$(brew --prefix 2>/dev/null || true)"
    fi
    if [[ -n "$brew_prefix" ]]; then
      printf '%s\n' "$brew_prefix/etc/nginx/servers/rustclaw-ui.conf"
      return
    fi
    if [[ -d "/opt/homebrew/etc/nginx" ]]; then
      printf '%s\n' "/opt/homebrew/etc/nginx/servers/rustclaw-ui.conf"
      return
    fi
    if [[ -d "/usr/local/etc/nginx" ]]; then
      printf '%s\n' "/usr/local/etc/nginx/servers/rustclaw-ui.conf"
      return
    fi
  fi
  if [[ -d "/etc/nginx/sites-available" ]]; then
    printf '%s\n' "/etc/nginx/sites-available/rustclaw-ui.conf"
    return
  fi
  printf '%s\n' "/etc/nginx/conf.d/rustclaw-ui.conf"
}

nginx_main_conf_path() {
  if [[ "$HOST_OS" == "macos" ]]; then
    local brew_prefix=""
    if command -v brew >/dev/null 2>&1; then
      brew_prefix="$(brew --prefix 2>/dev/null || true)"
    fi
    if [[ -n "$brew_prefix" ]]; then
      printf '%s\n' "$brew_prefix/etc/nginx/nginx.conf"
      return
    fi
    if [[ -f "/opt/homebrew/etc/nginx/nginx.conf" ]]; then
      printf '%s\n' "/opt/homebrew/etc/nginx/nginx.conf"
      return
    fi
    if [[ -f "/usr/local/etc/nginx/nginx.conf" ]]; then
      printf '%s\n' "/usr/local/etc/nginx/nginx.conf"
      return
    fi
  fi
  printf '%s\n' "/etc/nginx/nginx.conf"
}

nginx_main_conf_includes_dir() {
  local main_conf="$1"
  local include_dir="$2"
  local include_line="include ${include_dir}/*.conf;"
  [[ -f "$main_conf" ]] || return 1
  grep -Fq "$include_line" "$main_conf"
}

nginx_site_link_path() {
  local conf_path="$1"
  local main_conf="${2:-}"
  if [[ "$HOST_OS" == "macos" ]]; then
    return 0
  fi
  if [[ "$conf_path" == /etc/nginx/sites-available/* ]] && [[ -d "/etc/nginx/sites-enabled" ]]; then
    if [[ -n "$main_conf" ]] && nginx_main_conf_includes_dir "$main_conf" "/etc/nginx/sites-available"; then
      return 0
    fi
    printf '%s\n' "/etc/nginx/sites-enabled/$(basename "$conf_path")"
  fi
}

nginx_include_dir_for_conf() {
  local main_conf="$1"
  local conf_path="$2"
  local site_link="$3"

  if [[ "$conf_path" == /etc/nginx/sites-available/* ]]; then
    if [[ -n "$site_link" ]]; then
      printf '%s\n' "/etc/nginx/sites-enabled"
      return
    fi
    if nginx_main_conf_includes_dir "$main_conf" "/etc/nginx/sites-available"; then
      return 0
    fi
  fi

  printf '%s\n' "$(dirname "$conf_path")"
}

resolve_webd_proxy_upstream() {
  local default_upstream="http://127.0.0.1:8788"
  if ! command -v python3 >/dev/null 2>&1; then
    printf '%s\n' "$default_upstream"
    return
  fi
  python3 - "$SCRIPT_DIR/configs/channels/webd.toml" <<'PY'
import sys
from pathlib import Path

default_upstream = "http://127.0.0.1:8788"
config_path = Path(sys.argv[1])
if not config_path.exists():
    print(default_upstream)
    raise SystemExit

try:
    import tomllib  # py311+
except ModuleNotFoundError:
    try:
        import tomli as tomllib  # backport
    except ModuleNotFoundError:
        print(default_upstream)
        raise SystemExit

try:
    data = tomllib.loads(config_path.read_text(encoding="utf-8"))
except Exception:
    print(default_upstream)
    raise SystemExit

listen = str((data.get("webd") or {}).get("listen") or "0.0.0.0:8788").strip()
if not listen:
    print(default_upstream)
    raise SystemExit

host = ""
port = ""
if listen.startswith("[") and "]:" in listen:
    host, port = listen[1:].split("]:", 1)
else:
    host, sep, port = listen.rpartition(":")
    if not sep:
        print(default_upstream)
        raise SystemExit

host = host.strip() or "127.0.0.1"
port = port.strip() or "8788"
if host in {"0.0.0.0", "*"}:
    host = "127.0.0.1"
elif host == "::":
    host = "::1"

if ":" in host and not host.startswith("["):
    host = f"[{host}]"

print(f"http://{host}:{port}")
PY
}

ensure_nginx_site_include() {
  local main_conf="$1"
  local include_dir="$2"
  local include_line="    include ${include_dir}/*.conf;"
  [[ -f "$main_conf" ]] || return 0
  if grep -Fq "$include_line" "$main_conf"; then
    return 0
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 is required to patch nginx.conf includes."
    exit 1
  fi
  if [[ -w "$main_conf" ]]; then
    python3 - "$main_conf" "$include_line" <<'PY'
from pathlib import Path
import sys

conf_path = Path(sys.argv[1])
include_line = sys.argv[2]
text = conf_path.read_text(encoding="utf-8")
if include_line in text:
    raise SystemExit(0)
needle = "http {"
idx = text.find(needle)
if idx < 0:
    raise SystemExit("nginx.conf missing `http {` block")
insert_at = text.find("\n", idx)
if insert_at < 0:
    raise SystemExit("nginx.conf malformed after `http {`")
updated = text[: insert_at + 1] + include_line + "\n" + text[insert_at + 1 :]
conf_path.write_text(updated, encoding="utf-8")
PY
  else
    local tmp_file
    tmp_file="$(mktemp)"
    python3 - "$main_conf" "$include_line" "$tmp_file" <<'PY'
from pathlib import Path
import sys

conf_path = Path(sys.argv[1])
include_line = sys.argv[2]
tmp_path = Path(sys.argv[3])
text = conf_path.read_text(encoding="utf-8")
if include_line in text:
    tmp_path.write_text(text, encoding="utf-8")
    raise SystemExit(0)
needle = "http {"
idx = text.find(needle)
if idx < 0:
    raise SystemExit("nginx.conf missing `http {` block")
insert_at = text.find("\n", idx)
if insert_at < 0:
    raise SystemExit("nginx.conf malformed after `http {`")
updated = text[: insert_at + 1] + include_line + "\n" + text[insert_at + 1 :]
tmp_path.write_text(updated, encoding="utf-8")
PY
    sudo cp "$tmp_file" "$main_conf"
    rm -f "$tmp_file"
  fi
  echo "Ensured nginx include in $main_conf: $include_line"
}

ensure_nginx_site_link() {
  local conf_path="$1"
  local site_link="$2"
  [[ -n "$site_link" ]] || return 0
  if [[ -L "$site_link" ]]; then
    local current_target=""
    current_target="$(readlink "$site_link" 2>/dev/null || true)"
    if [[ "$current_target" == "$conf_path" ]]; then
      return 0
    fi
  elif [[ -e "$site_link" ]]; then
    echo "Refusing to overwrite existing nginx site entry: $site_link"
    exit 1
  fi

  if path_writable_or_creatable "$site_link"; then
    mkdir -p "$(dirname "$site_link")"
    ln -sfn "$conf_path" "$site_link"
  else
    sudo mkdir -p "$(dirname "$site_link")"
    sudo ln -sfn "$conf_path" "$site_link"
  fi
  echo "Ensured nginx site link: $site_link -> $conf_path"
}

remove_stale_nginx_ui_entries() {
  local conf_path="$1"
  local site_link="$2"
  local stale_path=""
  local removed_any=1

  for stale_path in \
    "/etc/nginx/conf.d/rustclaw-ui.conf" \
    "/etc/nginx/sites-available/rustclaw-ui.conf" \
    "/etc/nginx/sites-enabled/rustclaw-ui.conf"
  do
    if [[ "$stale_path" == "$conf_path" ]] || [[ -n "$site_link" && "$stale_path" == "$site_link" ]]; then
      continue
    fi
    if [[ ! -e "$stale_path" && ! -L "$stale_path" ]]; then
      continue
    fi

    if path_writable_or_creatable "$stale_path"; then
      rm -f "$stale_path"
      echo "Removed stale nginx entry: $stale_path"
    else
      sudo rm -f "$stale_path"
      echo "Removed stale nginx entry: $stale_path (sudo)."
    fi
    removed_any=0
  done

  return "$removed_any"
}

systemctl_available() {
  command -v systemctl >/dev/null 2>&1 && systemctl list-unit-files >/dev/null 2>&1
}

service_available() {
  command -v service >/dev/null 2>&1
}

openrc_available() {
  command -v rc-service >/dev/null 2>&1
}

nginx_ui_config_matches() {
  local conf_path="$1"
  local ui_root="$2"
  local proxy_upstream="$3"
  [[ -f "$conf_path" ]] || return 1
  grep -Fq "root $ui_root;" "$conf_path" || return 1
  grep -Fq "location ^~ /v1/" "$conf_path" || return 1
  grep -Fq "location ^~ /webd/" "$conf_path" || return 1
  grep -Fq "proxy_pass $proxy_upstream;" "$conf_path" || return 1
  grep -Fq "try_files \$uri \$uri/ /index.html;" "$conf_path" || return 1
  grep -qE "listen[[:space:]]+.*80[[:space:]]*(default_server)?;" "$conf_path" || return 1
  return 0
}

ensure_nginx() {
  if command -v nginx >/dev/null 2>&1; then
    return 0
  fi

  echo "nginx not found. Attempting to install nginx..."
  if command -v brew >/dev/null 2>&1; then
    brew install nginx
  elif command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -qq && sudo apt-get install -y nginx
  elif command -v zypper >/dev/null 2>&1; then
    sudo zypper --non-interactive install nginx
  elif command -v pacman >/dev/null 2>&1; then
    sudo pacman -Sy --noconfirm nginx
  elif command -v apk >/dev/null 2>&1; then
    sudo apk add nginx
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y nginx
  elif command -v yum >/dev/null 2>&1; then
    sudo yum install -y nginx
  else
    echo "Unsupported package manager. Please install nginx manually, then rerun."
    exit 1
  fi

  if ! command -v nginx >/dev/null 2>&1; then
    echo "nginx still not found after install attempt."
    exit 1
  fi

  if systemctl_available; then
    sudo systemctl enable nginx >/dev/null 2>&1 || true
    sudo systemctl start nginx >/dev/null 2>&1 || true
  elif service_available; then
    sudo service nginx start >/dev/null 2>&1 || true
  elif openrc_available; then
    sudo rc-service nginx start >/dev/null 2>&1 || true
  elif [[ "$HOST_OS" == "macos" ]] && command -v brew >/dev/null 2>&1; then
    brew services start nginx >/dev/null 2>&1 || true
  fi
}

write_nginx_config() {
  local conf_path="$1"
  local ui_root="$2"
  local proxy_upstream="$3"
  cat <<NGX
# RustClaw UI: 静态资源由 nginx 托管，/v1 与 /webd 反代到 webd。
server {
    listen 0.0.0.0:80;
    listen [::]:80;
    root $ui_root;
    index index.html;

    location ^~ /v1/ {
        proxy_pass $proxy_upstream;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }

    location ^~ /webd/ {
        proxy_pass $proxy_upstream;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }

    location / {
        try_files \$uri \$uri/ /index.html;
    }
}
NGX
}

reload_nginx() {
  echo "Reloading nginx..."

  if systemctl_available; then
    sudo nginx -t
    sudo systemctl reload nginx
    echo "Nginx reloaded via systemctl."
    return
  fi

  if service_available; then
    sudo nginx -t
    if sudo service nginx reload >/dev/null 2>&1; then
      echo "Nginx reloaded via service."
      return
    fi
    if sudo service nginx restart >/dev/null 2>&1; then
      echo "Nginx restarted via service."
      return
    fi
  fi

  if openrc_available; then
    sudo nginx -t
    if sudo rc-service nginx reload >/dev/null 2>&1; then
      echo "Nginx reloaded via rc-service."
      return
    fi
    if sudo rc-service nginx restart >/dev/null 2>&1; then
      echo "Nginx restarted via rc-service."
      return
    fi
  fi

  if [[ "$HOST_OS" == "macos" ]] && command -v brew >/dev/null 2>&1; then
    if nginx -t >/dev/null 2>&1 && brew services restart nginx >/dev/null 2>&1; then
      echo "Nginx restarted via brew services."
      return
    fi
  fi

  if command -v nginx >/dev/null 2>&1; then
    sudo nginx -t
    sudo nginx -s reload
    echo "Nginx reloaded via nginx -s reload."
    return
  fi

  echo "Warning: nginx reload skipped. Please reload nginx manually."
}

build_ui() {
  local build_output=""
  if build_output="$(
    cd "$UI_DIR"
    npm run build 2>&1
  )"; then
    printf '%s\n' "$build_output"
    return 0
  fi

  printf '%s\n' "$build_output"

  if [[ "$build_output" == *"npm has a bug related to optional dependencies"* ]] || [[ "$build_output" == *"Cannot find module @rollup/"* ]]; then
    echo "Detected missing Rollup optional dependency. Running npm install and retrying build once..."
    (
      cd "$UI_DIR"
      npm install
      npm run build
    )
    return $?
  fi

  return 1
}

NGINX_CONF="$(nginx_conf_path)"
NGINX_CONF_DIR="$(dirname "$NGINX_CONF")"
NGINX_MAIN_CONF="$(nginx_main_conf_path)"
NGINX_SITE_LINK="$(nginx_site_link_path "$NGINX_CONF" "$NGINX_MAIN_CONF" || true)"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --deploy|--copy)
      shift
      ;;
    --build)
      BUILD_UI=1
      shift
      ;;
    --path)
      NGINX_ROOT="${2:?Missing argument for --path}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1"
      usage
      exit 1
      ;;
  esac
done

echo "Host platform: ${HOST_OS}/${HOST_ARCH} ${HOST_TARGET:+($HOST_TARGET)}"
echo "UI source: $DIST_DIR"

if [[ "$BUILD_UI" == "1" ]]; then
  if ! command -v npm >/dev/null 2>&1; then
    echo "Error: npm not found, cannot build UI automatically."
    exit 1
  fi
  echo "Building UI..."
  build_ui
fi

if [[ ! -d "$DIST_DIR" ]] || [[ ! -f "$DIST_DIR/index.html" ]]; then
  echo "Error: UI/dist not built. Run '$0 --build' or build manually with: cd UI && npm run build"
  exit 1
fi

ensure_nginx
if path_writable_or_creatable "$NGINX_ROOT"; then
  mkdir -p "$NGINX_ROOT"
  cp -R "$DIST_DIR/." "$NGINX_ROOT/"
  echo "Copied UI to $NGINX_ROOT (no sudo)."
else
  sudo mkdir -p "$NGINX_ROOT"
  sudo cp -R "$DIST_DIR/." "$NGINX_ROOT/"
  echo "Copied UI to $NGINX_ROOT (sudo)."
fi
PROXY_UPSTREAM="$(resolve_webd_proxy_upstream)"
NGINX_INCLUDE_DIR="$(nginx_include_dir_for_conf "$NGINX_MAIN_CONF" "$NGINX_CONF" "$NGINX_SITE_LINK" || true)"
if [[ -n "$NGINX_INCLUDE_DIR" ]]; then
  ensure_nginx_site_include "$NGINX_MAIN_CONF" "$NGINX_INCLUDE_DIR"
fi
NGINX_CONFIG_CHANGED=0
if nginx_ui_config_matches "$NGINX_CONF" "$NGINX_ROOT" "$PROXY_UPSTREAM"; then
  echo "Nginx config already up-to-date, skip configure: $NGINX_CONF"
elif path_writable_or_creatable "$NGINX_CONF_DIR"; then
  mkdir -p "$NGINX_CONF_DIR"
  write_nginx_config "$NGINX_CONF" "$NGINX_ROOT" "$PROXY_UPSTREAM" > "$NGINX_CONF"
  echo "Wrote nginx config: $NGINX_CONF"
  NGINX_CONFIG_CHANGED=1
else
  sudo mkdir -p "$NGINX_CONF_DIR"
  write_nginx_config "$NGINX_CONF" "$NGINX_ROOT" "$PROXY_UPSTREAM" | sudo tee "$NGINX_CONF" >/dev/null
  echo "Wrote nginx config: $NGINX_CONF (sudo)."
  NGINX_CONFIG_CHANGED=1
fi

ensure_nginx_site_link "$NGINX_CONF" "$NGINX_SITE_LINK"
if remove_stale_nginx_ui_entries "$NGINX_CONF" "$NGINX_SITE_LINK"; then
  NGINX_CONFIG_CHANGED=1
fi

if [[ "$HOST_OS" != "macos" ]] && { [[ -f /etc/nginx/sites-enabled/default ]] || [[ -L /etc/nginx/sites-enabled/default ]]; }; then
  if [[ -w /etc/nginx/sites-enabled ]]; then
    rm -f /etc/nginx/sites-enabled/default
    echo "Disabled nginx default site: removed /etc/nginx/sites-enabled/default"
  else
    sudo rm -f /etc/nginx/sites-enabled/default
    echo "Disabled nginx default site: removed /etc/nginx/sites-enabled/default (sudo)."
  fi
  NGINX_CONFIG_CHANGED=1
fi

if [[ "$NGINX_CONFIG_CHANGED" == "1" ]]; then
  reload_nginx
else
  echo "Skip nginx reload (no config changes)."
fi
echo "Deploy completed. Open http://<host-ip>/ to access the UI. Same-origin API requests will be proxied by nginx to $PROXY_UPSTREAM"

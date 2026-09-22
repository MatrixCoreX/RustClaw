#!/usr/bin/env bash
# Canonical brand-neutral installer entry point.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
configure_platform_command_path
TARGET="$SCRIPT_DIR/agentctl"
DEFAULT_INSTALL_DIR="/usr/local/bin"
USER_INSTALL_DIR="${HOME}/.local/bin"
USE_USER_DIR=0
INSTALL_DIR="$DEFAULT_INSTALL_DIR"
HOST_OS="$(detect_host_os || printf '%s' "unknown")"
HOST_ARCH="$(detect_host_arch || printf '%s' "unknown")"
REQUESTED_TARGET="host"
HOST_RUST_TARGET="$(host_rust_target 2>/dev/null || true)"
# 本地安装默认不配置 nginx；云服务器可显式传 --deploy-ui-nginx。
DEPLOY_UI_NGINX=""
# --pi-app：配置 Pi App 桌面快捷方式 + 开机自启（小屏）
CONFIGURE_PI_APP=0

# Install packaged binaries, never build missing artifacts on the destination.
TRACKED_RELEASE_DIR="$SCRIPT_DIR/release-bin"
REQUIRED_BIN_NAME="clawd"

default_nginx_root() {
  if [[ "$HOST_OS" == "macos" ]]; then
    printf '%s\n' "${APP_DATA_HOME:-$HOME/.$APP_DATA_NAMESPACE}/nginx-ui"
    return
  fi
  printf '%s\n' "/var/www/html/$APP_DATA_NAMESPACE"
}

NGINX_SITE_LINK=""

nginx_conf_path() {
  if [[ "$HOST_OS" == "macos" ]]; then
    local brew_prefix=""
    if command -v brew >/dev/null 2>&1; then
      brew_prefix="$(brew --prefix 2>/dev/null || true)"
    fi
    if [[ -n "$brew_prefix" ]]; then
      printf '%s\n' "$brew_prefix/etc/nginx/servers/${APP_SERVICE_NAME}-ui.conf"
      return
    fi
    if [[ -d "/opt/homebrew/etc/nginx" ]]; then
      printf '%s\n' "/opt/homebrew/etc/nginx/servers/${APP_SERVICE_NAME}-ui.conf"
      return
    fi
    if [[ -d "/usr/local/etc/nginx" ]]; then
      printf '%s\n' "/usr/local/etc/nginx/servers/${APP_SERVICE_NAME}-ui.conf"
      return
    fi
  fi
  if [[ -d "/etc/nginx/sites-available" ]]; then
    printf '%s\n' "/etc/nginx/sites-available/${APP_SERVICE_NAME}-ui.conf"
    return
  fi
  printf '%s\n' "/etc/nginx/conf.d/${APP_SERVICE_NAME}-ui.conf"
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
  local include_conf="include ${include_dir}/*.conf;"
  local include_any="include ${include_dir}/*;"
  [[ -f "$main_conf" ]] || return 1
  awk -v include_conf="$include_conf" -v include_any="$include_any" '
    {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      sub(/[[:space:]]+$/, "", line)
      if (line ~ /^#/) next
      if (line == include_conf || line == include_any) found = 1
    }
    END { exit found ? 0 : 1 }
  ' "$main_conf"
}

nginx_site_link_path() {
  local conf_path="$1"
  if [[ "$HOST_OS" == "macos" ]]; then
    return 0
  fi
  if [[ "$conf_path" == /etc/nginx/sites-available/* ]] && [[ -d "/etc/nginx/sites-enabled" ]]; then
    printf '%s\n' "/etc/nginx/sites-enabled/$(basename "$conf_path")"
  fi
}

nginx_include_dir_for_conf() {
  local conf_path="$1"
  local site_link="$2"
  if [[ "$conf_path" == /etc/nginx/sites-available/* ]] && [[ -n "$site_link" ]]; then
    printf '%s\n' "/etc/nginx/sites-enabled"
    return
  fi
  printf '%s\n' "$(dirname "$conf_path")"
}

is_raspberry_pi() {
  if [[ "${APP_FORCE_PI_APP:-0}" == "1" ]]; then
    return 0
  fi

  if [[ "$HOST_OS" != "linux" ]]; then
    return 1
  fi

  local model_file model_text
  for model_file in \
    /proc/device-tree/model \
    /sys/firmware/devicetree/base/model; do
    if [[ -r "$model_file" ]]; then
      model_text="$(tr -d '\0' < "$model_file" 2>/dev/null || true)"
      if [[ "$model_text" == *"Raspberry Pi"* ]]; then
        return 0
      fi
    fi
  done

  if [[ -r /proc/cpuinfo ]] && grep -qiE 'Raspberry Pi|BCM27|BCM28' /proc/cpuinfo; then
    return 0
  fi

  return 1
}

NGINX_CONF="$(nginx_conf_path)"
NGINX_CONF_DIR="$(dirname "$NGINX_CONF")"
NGINX_MAIN_CONF="$(nginx_main_conf_path)"
NGINX_SITE_LINK="$(nginx_site_link_path "$NGINX_CONF" || true)"

usage() {
  cat <<'EOF'
Usage:
  bash install-agent-cmd.sh [options]

Options:
  --target TARGET  Verify the host target triple, or use 'host' (default)
  --user           Install to ~/.local/bin (no sudo)
  --dir <path>     Install to custom directory
  --deploy-ui-nginx [path]   Deploy UI to path (default: auto-detect per OS), configure nginx, reload nginx
  --no-deploy-ui   Compatibility no-op; local/default installs already skip nginx
  --pi-app         Configure Pi App on Raspberry Pi only: desktop shortcut + autostart on login
  -h, --help       Show this help

Default: install verified Release command entrypoints without nginx. webd serves the local UI.
Cloud/server deployments may opt in with --deploy-ui-nginx [path].
No compiler or UI build is invoked, even when required files are missing.
Download a matching verified Release first; update with deploy-github-release.sh.
Developers may build manually before installing: docs/developer_build.md.

Verify after install:
  command -v agentctl
  agentctl -h
  agentctl -status

Key management:
  agentctl -key list
  agentctl -key generate user
  agentctl -key generate admin
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --build|--force-build)
      echo "Compilation is not an installation step. Use a verified Release package."
      echo "Developer-only source builds: docs/developer_build.md"
      exit 2
      ;;
    --target)
      shift
      if [[ $# -lt 1 ]]; then
        echo "Missing value for --target"
        exit 1
      fi
      REQUESTED_TARGET="$1"
      ;;
    --user)
      USE_USER_DIR=1
      INSTALL_DIR="$USER_INSTALL_DIR"
      ;;
    --dir)
      shift
      if [[ $# -lt 1 ]]; then
        echo "Missing value for --dir"
        exit 1
      fi
      INSTALL_DIR="$1"
      ;;
    --no-deploy-ui)
      DEPLOY_UI_NGINX=""
      ;;
    --pi-app)
      CONFIGURE_PI_APP=1
      ;;
    --deploy-ui-nginx)
      shift
      if [[ $# -ge 1 && "$1" != --* ]]; then
        DEPLOY_UI_NGINX="$1"
        shift
        continue
      else
        DEPLOY_UI_NGINX="$(default_nginx_root)"
      fi
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      usage
      exit 1
      ;;
  esac
  [[ $# -gt 0 ]] && shift
done

INSTALL_TARGET="$(resolve_requested_target "$REQUESTED_TARGET")"
if [[ "$INSTALL_TARGET" != "$HOST_RUST_TARGET" ]]; then
  echo "Release target does not match this host: $INSTALL_TARGET (host: $HOST_RUST_TARGET)" >&2
  exit 1
fi
BUILD_RELEASE_DIR="$(preferred_release_dir_for_target "$SCRIPT_DIR" "$INSTALL_TARGET")"
HOST_BUILD_RELEASE_DIR="$(preferred_release_dir_for_target "$SCRIPT_DIR" "$HOST_RUST_TARGET")"
FALLBACK_RELEASE_DIR="$(target_release_dir "$SCRIPT_DIR" "")"
PACKAGE_FLAVOR="$(package_flavor_for_target "$INSTALL_TARGET" 2>/dev/null || printf '%s' "$INSTALL_TARGET")"

LINK_PATH="$INSTALL_DIR/agentctl"

ensure_python_runtime() {
  if configure_python3_with_tomllib 2>/dev/null; then
    echo "Python runtime ready: $APP_PYTHON_BIN ($($APP_PYTHON_BIN --version 2>&1))"
    return 0
  fi

  echo "Python 3.11+ with tomllib not found. Installing the agent-runtime Python dependency..."
  if [[ "$HOST_OS" == "macos" ]]; then
    if ! command -v brew >/dev/null 2>&1; then
      echo "Homebrew is required to install current Python on macOS: https://brew.sh" >&2
      exit 1
    fi
    brew install python
  elif command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -qq
    if apt-cache show python3.11 >/dev/null 2>&1; then
      sudo apt-get install -y python3.11
    else
      sudo apt-get install -y python3
    fi
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y python3
  elif command -v yum >/dev/null 2>&1; then
    sudo yum install -y python3
  elif command -v zypper >/dev/null 2>&1; then
    sudo zypper --non-interactive install python3
  elif command -v pacman >/dev/null 2>&1; then
    sudo pacman -Sy --noconfirm python
  elif command -v apk >/dev/null 2>&1; then
    sudo apk add python3
  else
    echo "No supported package manager could install Python 3.11+." >&2
    exit 1
  fi

  configure_platform_command_path
  if ! configure_python3_with_tomllib; then
    echo "Python was installed, but the agent runtime still cannot find Python 3.11+ with tomllib." >&2
    exit 1
  fi
  echo "Python runtime installed: $APP_PYTHON_BIN ($($APP_PYTHON_BIN --version 2>&1))"
}

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

find_nginx_bin() {
  local candidate=""
  if candidate="$(command -v nginx 2>/dev/null)"; then
    printf '%s\n' "$candidate"
    return 0
  fi

  for candidate in /usr/sbin/nginx /sbin/nginx /usr/local/sbin/nginx /usr/local/bin/nginx /opt/homebrew/bin/nginx; do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  return 1
}

nginx_available() {
  find_nginx_bin >/dev/null 2>&1
}

# ----- 确保 nginx 已安装并启动（用于 --deploy-ui-nginx） -----
ensure_nginx() {
  if nginx_available; then
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

  if ! nginx_available; then
    echo "nginx still not found after install attempt."
    exit 1
  fi

  if command -v systemctl >/dev/null 2>&1 && systemctl list-unit-files >/dev/null 2>&1; then
    sudo systemctl enable nginx >/dev/null 2>&1 || true
    sudo systemctl start nginx >/dev/null 2>&1 || true
  elif command -v service >/dev/null 2>&1; then
    sudo service nginx start >/dev/null 2>&1 || true
  elif command -v rc-service >/dev/null 2>&1; then
    sudo rc-service nginx start >/dev/null 2>&1 || true
  elif [[ "$HOST_OS" == "macos" ]] && command -v brew >/dev/null 2>&1; then
    brew services start nginx >/dev/null 2>&1 || true
  fi
  echo "nginx is installed."
}

reload_nginx() {
  local nginx_bin=""
  nginx_bin="$(find_nginx_bin || true)"
  if [[ -z "$nginx_bin" ]]; then
    echo "Warning: nginx reload skipped. Please install or reload nginx manually."
    return
  fi

  if command -v systemctl >/dev/null 2>&1 && systemctl list-unit-files >/dev/null 2>&1; then
    sudo "$nginx_bin" -t
    sudo systemctl reload nginx
    echo "Nginx config OK and reloaded via systemctl."
    return
  fi

  if command -v service >/dev/null 2>&1; then
    sudo "$nginx_bin" -t
    if sudo service nginx reload >/dev/null 2>&1; then
      echo "Nginx config OK and reloaded via service."
      return
    fi
    if sudo service nginx restart >/dev/null 2>&1; then
      echo "Nginx config OK and restarted via service."
      return
    fi
  fi

  if command -v rc-service >/dev/null 2>&1; then
    sudo "$nginx_bin" -t
    if sudo rc-service nginx reload >/dev/null 2>&1; then
      echo "Nginx config OK and reloaded via rc-service."
      return
    fi
    if sudo rc-service nginx restart >/dev/null 2>&1; then
      echo "Nginx config OK and restarted via rc-service."
      return
    fi
  fi

  if [[ "$HOST_OS" == "macos" ]] && command -v brew >/dev/null 2>&1; then
    if [[ -n "$nginx_bin" ]] && "$nginx_bin" -t >/dev/null 2>&1; then
      brew services restart nginx >/dev/null 2>&1
      echo "Nginx config OK and restarted via brew services."
      return
    fi
    echo "Nginx test failed. Check $NGINX_CONF and run: nginx -t"
    exit 1
  fi

  if [[ -n "$nginx_bin" ]]; then
    if sudo "$nginx_bin" -t; then
      sudo "$nginx_bin" -s reload
      echo "Nginx config OK and reloaded via nginx -s reload."
      return
    fi
    echo "Nginx test failed. Check $NGINX_CONF and run: sudo nginx -t"
    exit 1
  fi

  echo "Warning: nginx reload skipped. Please reload nginx manually."
}

resolve_release_dir() {
  local candidate
  for candidate in "$BUILD_RELEASE_DIR" "$HOST_BUILD_RELEASE_DIR" "$FALLBACK_RELEASE_DIR" "$TRACKED_RELEASE_DIR"; do
    if [[ -x "$candidate/$REQUIRED_BIN_NAME" ]]; then
      printf '%s\n' "$candidate"
      return
    fi
  done
  printf '%s\n' "$BUILD_RELEASE_DIR"
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
  grep -Fq 'add_header Cache-Control "no-store, no-cache, must-revalidate" always;' "$conf_path" || return 1
  grep -qE "listen[[:space:]]+.*80[[:space:]]*(default_server)?;" "$conf_path" || return 1
  return 0
}

nginx_ui_config_is_tls_managed() {
  local conf_path="$1"
  [[ -f "$conf_path" ]] || return 1
  grep -Eq '^[[:space:]]*(listen[[:space:]]+.*443|ssl_certificate(_key)?[[:space:]])' "$conf_path"
}

ensure_deployed_ui_readable() {
  local ui_root="$1"
  if [[ -w "$ui_root" ]]; then
    chmod -R a+rX "$ui_root"
  else
    sudo chmod -R a+rX "$ui_root"
  fi
}

resolve_webd_proxy_upstream() {
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
        import tomli as tomllib
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
  if [[ "$include_dir" == "/etc/nginx/sites-enabled" ]]; then
    disable_nginx_sites_available_include "$main_conf"
  fi
  if nginx_main_conf_includes_dir "$main_conf" "$include_dir"; then
    return 0
  fi
  if [[ -w "$main_conf" ]]; then
    python3 - "$main_conf" "$include_line" <<'PY'
from pathlib import Path
import sys

conf_path = Path(sys.argv[1])
include_line = sys.argv[2]
text = conf_path.read_text(encoding="utf-8")
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

disable_nginx_sites_available_include() {
  local main_conf="$1"
  [[ "$HOST_OS" != "macos" ]] || return 0
  [[ -f "$main_conf" && -d "/etc/nginx/sites-enabled" ]] || return 0
  if ! grep -Eq '^[[:space:]]*include[[:space:]]+/etc/nginx/sites-available/\*\.conf;' "$main_conf"; then
    return 0
  fi

  local tmp_file
  tmp_file="$(mktemp)"
  python3 - "$main_conf" "$tmp_file" <<'PY'
from pathlib import Path
import re
import sys

src = Path(sys.argv[1])
dst = Path(sys.argv[2])
out = []
changed = False
for line in src.read_text(encoding="utf-8").splitlines():
    if re.match(r'^\s*include\s+/etc/nginx/sites-available/\*\.conf;\s*$', line):
        indent = line[: len(line) - len(line.lstrip())]
        out.append(f"{indent}# include /etc/nginx/sites-available/*.conf; disabled: active sites are loaded from sites-enabled")
        changed = True
    else:
        out.append(line)
dst.write_text("\n".join(out) + "\n", encoding="utf-8")
raise SystemExit(0 if changed else 2)
PY
  local rc=$?
  if [[ "$rc" == "0" ]]; then
    if [[ -w "$main_conf" ]]; then
      cp "$tmp_file" "$main_conf"
    else
      sudo cp "$tmp_file" "$main_conf"
    fi
    echo "Disabled nginx sites-available include to avoid duplicate default_server entries."
  fi
  rm -f "$tmp_file"
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

[[ -f "$TARGET" ]] || { echo "Missing launcher script: $TARGET" >&2; exit 1; }
echo "Host platform: ${HOST_OS}/${HOST_ARCH} ${HOST_RUST_TARGET:+($HOST_RUST_TARGET)}"
echo "Selected target: $INSTALL_TARGET"
echo "Primary output: $BUILD_RELEASE_DIR"
echo "Flavor tag: $PACKAGE_FLAVOR"

ensure_python_runtime

SELECTED_RELEASE_DIR="$(resolve_release_dir)"
REQUIRED_BIN="$SELECTED_RELEASE_DIR/$REQUIRED_BIN_NAME"

if [[ ! -x "$REQUIRED_BIN" || ! -x "$SELECTED_RELEASE_DIR/webd" || ! -x "$SELECTED_RELEASE_DIR/clawcli" ]]; then
  echo "Release binaries are missing or incomplete: $SELECTED_RELEASE_DIR" >&2
  echo "Download a matching Release or run: bash deploy-github-release.sh --no-restart" >&2
  exit 1
fi
if [[ ! -f "$SCRIPT_DIR/UI/dist/index.html" ]]; then
  echo "Release UI assets are missing. Reinstall a complete Release package; no build was attempted." >&2
  exit 1
fi
for runtime_bin in "$REQUIRED_BIN" "$SELECTED_RELEASE_DIR/webd" "$SELECTED_RELEASE_DIR/clawcli"; do
  python3 "$SCRIPT_DIR/scripts/verify_release_binary.py" "$runtime_bin" "$INSTALL_TARGET"
done
echo "Prebuilt runtime and UI are present. Installing command entrypoints only."

chmod +x "$TARGET"

install_without_sudo() {
  mkdir -p "$INSTALL_DIR"
  rm -f "$LINK_PATH"
  ln -s "$TARGET" "$LINK_PATH"
}

install_with_sudo() {
  sudo mkdir -p "$INSTALL_DIR"
  sudo rm -f "$LINK_PATH"
  sudo ln -s "$TARGET" "$LINK_PATH"
}

if [[ "$USE_USER_DIR" == "1" ]]; then
  install_without_sudo
elif [[ -w "$INSTALL_DIR" ]]; then
  install_without_sudo
elif command -v sudo >/dev/null 2>&1; then
  echo "Installing launcher to $LINK_PATH (sudo required)..."
  install_with_sudo
else
  echo "No write permission to $INSTALL_DIR and sudo is unavailable."
  echo "Falling back to user install path: $USER_INSTALL_DIR"
  INSTALL_DIR="$USER_INSTALL_DIR"
  LINK_PATH="$INSTALL_DIR/agentctl"
  install_without_sudo
fi

echo "Installed: $LINK_PATH -> $TARGET"

# Install clawcli if present (terminal CLI to talk to clawd)
SELECTED_RELEASE_DIR="$(resolve_release_dir)"
CLAWCLI_BIN="$SELECTED_RELEASE_DIR/clawcli"
CLAWCLI_LINK="$INSTALL_DIR/clawcli"
if [[ -x "$CLAWCLI_BIN" ]]; then
  if [[ "$USE_USER_DIR" == "1" ]] || [[ -w "$INSTALL_DIR" ]]; then
    rm -f "$CLAWCLI_LINK"
    ln -s "$CLAWCLI_BIN" "$CLAWCLI_LINK"
    echo "Installed: $CLAWCLI_LINK -> $CLAWCLI_BIN"
  else
    sudo rm -f "$CLAWCLI_LINK"
    sudo ln -s "$CLAWCLI_BIN" "$CLAWCLI_LINK"
    echo "Installed: $CLAWCLI_LINK -> $CLAWCLI_BIN (sudo)"
  fi
else
  echo "Missing Release CLI: $CLAWCLI_BIN. Reinstall the matching Release package." >&2
  exit 1
fi

if [[ "$LINK_PATH" == "$USER_INSTALL_DIR/agentctl" ]]; then
  case ":$PATH:" in
    *":$USER_INSTALL_DIR:"*) ;;
    *)
      echo "Note: $USER_INSTALL_DIR is not in PATH."
      echo "Add this to your shell profile:"
      echo "  export PATH=\"$USER_INSTALL_DIR:\$PATH\""
      ;;
  esac
fi
echo "Check install:"
echo "  command -v agentctl"
echo "  agentctl -h"
echo "  agentctl -status"
echo "  command -v clawcli && clawcli --help   # terminal chat CLI (if installed)"
echo
echo "Key management:"
echo "  agentctl -key list"
echo "  agentctl -key generate user"
echo "  agentctl -key generate admin"
echo
echo "Try:"
echo "  agentctl -status"
echo "  agentctl -start release all                        # 配置通信端后直接启动"
echo "  agentctl -restart release all                      # 配置通信端后直接重启"
echo "  agentctl -start release"
echo "  agentctl -stop"
echo
echo "Tip:"
echo "  bash deploy-github-release.sh --check-only   # check compatible Release"
echo "  bash deploy-github-release.sh                # verified Release update"
echo "Uninstall (removes command only, does not touch configs):"
echo "To uninstall, remove the installed agentctl and compatibility links from: $INSTALL_DIR"
if [[ "$CONFIGURE_PI_APP" == "1" ]]; then
  PI_APP_DIR="$SCRIPT_DIR/pi_app"
  if ! is_raspberry_pi; then
    # zh: 小屏桌面程序只面向树莓派；普通电脑安装时不要写桌面自启动。
    echo "Skip Pi App: current device is not detected as Raspberry Pi."
    echo "Set APP_FORCE_PI_APP=1 to override this check."
  elif [[ -d "$PI_APP_DIR" && -x "$PI_APP_DIR/install-desktop.sh" && -x "$PI_APP_DIR/enable-autostart.sh" ]]; then
    # zh: 可选安装树莓派小屏桌面入口和开机自启。
    echo
    echo "Configuring Pi App: desktop shortcut + autostart..."
    (cd "$PI_APP_DIR" && bash install-desktop.sh)
    (cd "$PI_APP_DIR" && bash enable-autostart.sh)
    echo "Pi App: desktop shortcut created and autostart enabled."
  else
    # zh: pi_app 脚本不存在或不可执行时，只跳过小屏集成，不影响主命令安装。
    echo "Skip Pi App: $PI_APP_DIR not found or scripts not executable."
  fi
fi
if [[ -n "$DEPLOY_UI_NGINX" ]]; then
  echo
  echo "Deploying UI to nginx directory: $DEPLOY_UI_NGINX"
  if [[ ! -d "$SCRIPT_DIR/UI" ]]; then
    echo "Error: UI directory not found: $SCRIPT_DIR/UI"
    exit 1
  fi
  if path_writable_or_creatable "$DEPLOY_UI_NGINX"; then
    mkdir -p "$DEPLOY_UI_NGINX"
    cp -R "$SCRIPT_DIR/UI/dist/." "$DEPLOY_UI_NGINX/"
    echo "Copied UI to $DEPLOY_UI_NGINX (no sudo)."
  else
    sudo mkdir -p "$DEPLOY_UI_NGINX"
    sudo cp -R "$SCRIPT_DIR/UI/dist/." "$DEPLOY_UI_NGINX/"
    echo "Copied UI to $DEPLOY_UI_NGINX (sudo)."
  fi
  ensure_deployed_ui_readable "$DEPLOY_UI_NGINX"
  ensure_nginx
  NGINX_INCLUDE_DIR="$(nginx_include_dir_for_conf "$NGINX_CONF" "$NGINX_SITE_LINK")"
  ensure_nginx_site_include "$NGINX_MAIN_CONF" "$NGINX_INCLUDE_DIR"
  PROXY_UPSTREAM="$(resolve_webd_proxy_upstream)"
  NGINX_CONFIG_CHANGED=0
  if nginx_ui_config_matches "$NGINX_CONF" "$DEPLOY_UI_NGINX" "$PROXY_UPSTREAM"; then
    echo "Nginx config already up-to-date, skip configure: $NGINX_CONF"
  elif nginx_ui_config_is_tls_managed "$NGINX_CONF"; then
    echo "Preserving existing TLS-enabled nginx config: $NGINX_CONF"
    echo "Verify its UI root and proxy upstream manually if the deployment path changed."
  elif path_writable_or_creatable "$NGINX_CONF_DIR"; then
    mkdir -p "$NGINX_CONF_DIR"
    cat > "$NGINX_CONF" << NGX
# Agent Runtime UI: 静态资源由 nginx 托管，/v1 与 /webd 反代到 webd。
server {
    listen 0.0.0.0:80;
    listen [::]:80;
    root $DEPLOY_UI_NGINX;
    index index.html;

    location ^~ /v1/ {
        proxy_pass $PROXY_UPSTREAM;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }

    location ^~ /webd/ {
        proxy_pass $PROXY_UPSTREAM;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
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
NGX
    echo "Wrote nginx config: $NGINX_CONF"
    NGINX_CONFIG_CHANGED=1
  else
    sudo mkdir -p "$NGINX_CONF_DIR"
    sudo tee "$NGINX_CONF" >/dev/null << NGX
# Agent Runtime UI: 静态资源由 nginx 托管，/v1 与 /webd 反代到 webd。
server {
    listen 0.0.0.0:80;
    listen [::]:80;
    root $DEPLOY_UI_NGINX;
    index index.html;

    location ^~ /v1/ {
        proxy_pass $PROXY_UPSTREAM;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }

    location ^~ /webd/ {
        proxy_pass $PROXY_UPSTREAM;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
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
NGX
    echo "Wrote nginx config: $NGINX_CONF (sudo)."
    NGINX_CONFIG_CHANGED=1
  fi
  ensure_nginx_site_link "$NGINX_CONF" "$NGINX_SITE_LINK"
  # 禁用 nginx 自带默认页，否则 80 端口会优先显示 default 页面
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
    echo "If the server cannot be reached via its public IP, check: 1) firewall rules allow port 80 (for example: sudo ufw allow 80); 2) cloud security group / inbound rules allow port 80."
  else
    echo "Skip nginx reload (no config changes)."
  fi
  echo "UI and API are now unified on nginx port 80. Open http://<host-ip>/ directly."
fi

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="$SCRIPT_DIR/rustclaw"
DEFAULT_INSTALL_DIR="/usr/local/bin"
USER_INSTALL_DIR="${HOME}/.local/bin"
FORCE_BUILD=0
DO_BUILD=0
USE_USER_DIR=0
INSTALL_DIR="$DEFAULT_INSTALL_DIR"
# 默认部署 UI 到 nginx：配置 nginx、复制 UI、重启 nginx；--no-deploy-ui 可跳过
DEPLOY_UI_NGINX="/var/www/html/rustclaw"

# 无构建模式要求至少存在此 bin（默认不构建，适合已交叉编译好 bin 的场景）
REQUIRED_BIN="$SCRIPT_DIR/target/release/clawd"
# 交叉编译拉回路径（与 cross-build-upload.sh 一致）
CROSS_TARGET="${RUSTCLAW_CROSS_TARGET:-aarch64-unknown-linux-gnu}"
CROSS_RELEASE="$SCRIPT_DIR/target/$CROSS_TARGET/release"

usage() {
  cat <<'EOF'
Usage:
  bash install-rustclaw-cmd.sh [options]

Options:
  --build          Install deps and build if needed, then install launcher
  --force-build    Force rebuild before install (implies --build)
  --user           Install to ~/.local/bin (no sudo)
  --dir <path>     Install to custom directory
  --deploy-ui-nginx [path]   Deploy UI to path (default: /var/www/html/rustclaw), configure nginx, reload nginx
  --no-deploy-ui   Skip nginx config and UI deploy (launcher only)
  -h, --help       Show this help

Default: install launcher and deploy UI to nginx (config nginx, copy UI, reload nginx). Use --no-deploy-ui to skip UI/nginx.
No build unless --build/--force-build; requires target/release/clawd to exist.
Use --build or --force-build when building from source.

Verify after install:
  command -v rustclaw
  rustclaw -h
  rustclaw -status

Key management:
  rustclaw -key list
  rustclaw -key generate user
  rustclaw -key generate admin
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force-build)
      FORCE_BUILD=1
      DO_BUILD=1
      ;;
    --build)
      DO_BUILD=1
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
    --deploy-ui-nginx)
      shift
      if [[ $# -ge 1 && "$1" != --* ]]; then
        DEPLOY_UI_NGINX="$1"
        shift
        continue
      else
        DEPLOY_UI_NGINX="/var/www/html/rustclaw"
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

LINK_PATH="$INSTALL_DIR/rustclaw"

# ----- 确保 Cargo (Rust) 已安装 -----
ensure_cargo() {
  if command -v cargo >/dev/null 2>&1; then
    return 0
  fi
  echo "cargo not found. Installing Rust toolchain (rustup)..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  if [[ -f "$HOME/.cargo/env" ]]; then
    . "$HOME/.cargo/env"
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Rust install failed or cargo not in PATH. Please run: source \"\$HOME/.cargo/env\""
    exit 1
  fi
  echo "Rust toolchain installed."
}

# ----- 确保 npm 已安装（存在 UI 目录时） -----
ensure_npm() {
  if [[ ! -d "$SCRIPT_DIR/UI" ]]; then
    return 0
  fi
  if command -v npm >/dev/null 2>&1; then
    return 0
  fi
  echo "npm not found. Attempting to install Node.js/npm..."
  if [[ -s "${NVM_DIR:-$HOME/.nvm}/nvm.sh" ]]; then
    . "${NVM_DIR:-$HOME/.nvm}/nvm.sh"
    nvm install --lts
    nvm use --lts
  elif command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -qq && sudo apt-get install -y nodejs npm
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y nodejs npm
  elif command -v yum >/dev/null 2>&1; then
    sudo yum install -y nodejs npm
  else
    echo "Please install Node.js and npm first: https://nodejs.org or: curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash"
    exit 1
  fi
  if ! command -v npm >/dev/null 2>&1; then
    echo "npm still not found after install attempt."
    exit 1
  fi
  echo "Node.js/npm ready."
}

# ----- 确保 nginx 已安装并启动（用于 --deploy-ui-nginx） -----
ensure_nginx() {
  if command -v nginx >/dev/null 2>&1; then
    return 0
  fi

  echo "nginx not found. Attempting to install nginx..."
  if command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -qq && sudo apt-get install -y nginx
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

  if command -v systemctl >/dev/null 2>&1; then
    sudo systemctl enable nginx >/dev/null 2>&1 || true
    sudo systemctl start nginx >/dev/null 2>&1 || true
  fi
  echo "nginx is installed."
}

nginx_ui_config_matches() {
  local conf_path="$1"
  local ui_root="$2"
  [[ -f "$conf_path" ]] || return 1
  grep -Fq "root $ui_root;" "$conf_path" || return 1
  grep -Fq "try_files \$uri \$uri/ /index.html;" "$conf_path" || return 1
  grep -qE "listen[[:space:]]+.*80[[:space:]]*(default_server)?;" "$conf_path" || return 1
  return 0
}

# 判断是否需要构建 release（源码更新或二进制缺失/过期）
need_release_build() {
  local force="$1"
  [[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "1"
    return
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    echo "1"
    return
  fi
  local need
  need="$(python3 - "$SCRIPT_DIR" "$force" <<'PY'
import json
import os
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
mode = sys.argv[2].strip().lower()
release_dir = root / "target" / "release"

def latest_source_mtime(base: Path) -> float:
    latest = 0.0
    tracked_ext = {".rs", ".toml", ".lock"}
    tracked_names = {"Cargo.toml", "Cargo.lock"}
    for current, dirs, files in os.walk(base):
        p = Path(current)
        if any(seg in {"target", ".git", "node_modules"} for seg in p.parts):
            continue
        for name in files:
            fp = p / name
            if fp.name in tracked_names or fp.suffix in tracked_ext:
                try:
                    latest = max(latest, fp.stat().st_mtime)
                except OSError:
                    pass
    return latest

if mode == "--force-build":
    print("1")
    raise SystemExit(0)

try:
    metadata_raw = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=str(root),
        text=True,
    )
except (subprocess.CalledProcessError, FileNotFoundError):
    print("1")
    raise SystemExit(0)

meta = json.loads(metadata_raw)
workspace_members = set(meta.get("workspace_members", []))
bins = set()
for pkg in meta.get("packages", []):
    if pkg.get("id") not in workspace_members:
        continue
    for target in pkg.get("targets", []):
        if "bin" in (target.get("kind", []) or []):
            name = (target.get("name") or "").strip()
            if name:
                bins.add(name)

if not bins:
    print("1")
    raise SystemExit(0)

latest_src = latest_source_mtime(root)
if latest_src <= 0:
    print("1")
    raise SystemExit(0)

oldest_bin = None
for name in sorted(bins):
    bp = release_dir / name
    if not bp.exists():
        print("1")
        raise SystemExit(0)
    try:
        m = bp.stat().st_mtime
    except OSError:
        print("1")
        raise SystemExit(0)
    oldest_bin = m if oldest_bin is None else min(oldest_bin, m)

if oldest_bin is None or oldest_bin < latest_src:
    print("1")
else:
    print("0")
PY
  )"
  echo "$need"
}

# 仅构建 release：UI（若有）+ cargo build --workspace --release，不调用其他脚本
do_release_build() {
  [[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
  ensure_cargo
  if [[ -d "$SCRIPT_DIR/UI" ]]; then
    ensure_npm
    if [[ ! -d "$SCRIPT_DIR/UI/node_modules" ]]; then
      echo "Installing UI dependencies..."
      (cd "$SCRIPT_DIR/UI" && npm install)
    fi
    echo "Building UI assets..."
    (cd "$SCRIPT_DIR/UI" && npm run build)
  fi
  echo "Building workspace (release)..."
  (cd "$SCRIPT_DIR" && cargo build --workspace --release)
  if [[ ! -x "$SCRIPT_DIR/target/release/clawd" ]]; then
    echo "Build finished but target/release/clawd missing."
    exit 1
  fi
  echo "Release build completed."
}

ensure_build() {
  local force="$1"
  ensure_cargo
  ensure_npm
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 not found."
    exit 1
  fi
  local need
  need="$(need_release_build "$force")"
  if [[ "$need" == "1" ]]; then
    echo "Release binaries are missing or outdated. Building release..."
    do_release_build
  else
    echo "Release binaries are up-to-date. Skip build."
  fi
}

if [[ ! -f "$TARGET" ]]; then
  echo "Missing launcher script: $TARGET"
  exit 1
fi

if [[ "$DO_BUILD" == "1" ]]; then
  if [[ "$FORCE_BUILD" == "1" ]]; then
    ensure_build "--force-build"
  else
    ensure_build ""
  fi
else
  if [[ ! -f "$REQUIRED_BIN" ]]; then
    if [[ -x "$CROSS_RELEASE/clawd" ]]; then
      echo "Using cross-compiled binaries from $CROSS_RELEASE"
      mkdir -p "$SCRIPT_DIR/target/release"
      for f in "$CROSS_RELEASE"/*; do
        [[ ! -f "$f" || ! -x "$f" ]] && continue
        [[ "$f" == *.rlib || "$f" == *.d ]] && continue
        ln -sf "$f" "$SCRIPT_DIR/target/release/$(basename "$f")" 2>/dev/null || cp -f "$f" "$SCRIPT_DIR/target/release/$(basename "$f")"
      done
    fi
  fi
  if [[ ! -f "$REQUIRED_BIN" ]]; then
    echo "Error: binary not found: $REQUIRED_BIN"
    echo "Copy your built clawd here, or run with --build to build from source."
    echo "Cross path checked: $CROSS_RELEASE/clawd (set RUSTCLAW_CROSS_TARGET if different)."
    exit 1
  fi
  echo "Skipping build (binary present). Installing launcher only."
fi

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
  LINK_PATH="$INSTALL_DIR/rustclaw"
  install_without_sudo
fi

echo "Installed: $LINK_PATH -> $TARGET"
if [[ "$LINK_PATH" == "$USER_INSTALL_DIR/rustclaw" ]]; then
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
echo "  command -v rustclaw"
echo "  rustclaw -h"
echo "  rustclaw -status"
echo
echo "Key management:"
echo "  rustclaw -key list"
echo "  rustclaw -key generate user"
echo "  rustclaw -key generate admin"
echo
echo "Try:"
echo "  rustclaw -status"
echo "  rustclaw -start release all --quick --skip-setup   # 免配置直接启动"
echo "  rustclaw -restart release all --quick --skip-setup # 免配置直接重启（跳过所有配置)"
echo "  rustclaw -start release"
echo "  rustclaw -stop"
echo
echo "Tip:"
echo "  bash install-rustclaw-cmd.sh --build     # build from source then install"
echo "  bash install-rustclaw-cmd.sh --force-build   # force rebuild then install"
echo "Uninstall (removes command only, does not touch configs):"
echo "  bash uninstall-rustclaw-cmd.sh [--user|--dir <path>]"
if [[ -n "$DEPLOY_UI_NGINX" ]]; then
  echo
  echo "Deploying UI to nginx directory: $DEPLOY_UI_NGINX"
  if [[ ! -d "$SCRIPT_DIR/UI" ]]; then
    echo "Error: UI directory not found: $SCRIPT_DIR/UI"
    exit 1
  fi
  ensure_npm
  if [[ ! -d "$SCRIPT_DIR/UI/dist" ]] || [[ "$DO_BUILD" == "1" ]] || [[ "$FORCE_BUILD" == "1" ]]; then
    if [[ ! -d "$SCRIPT_DIR/UI/node_modules" ]]; then
      echo "Installing UI dependencies..."
      (cd "$SCRIPT_DIR/UI" && npm install)
    fi
    echo "Building UI assets..."
    (cd "$SCRIPT_DIR/UI" && npm run build)
  fi
  if [[ ! -d "$SCRIPT_DIR/UI/dist" ]]; then
    echo "Error: UI build failed (UI/dist missing)."
    exit 1
  fi
  if [[ -w "$DEPLOY_UI_NGINX" ]]; then
    mkdir -p "$DEPLOY_UI_NGINX"
    cp -r "$SCRIPT_DIR/UI/dist/"* "$DEPLOY_UI_NGINX/"
    echo "Copied UI to $DEPLOY_UI_NGINX (no sudo)."
  else
    sudo mkdir -p "$DEPLOY_UI_NGINX"
    sudo cp -r "$SCRIPT_DIR/UI/dist/"* "$DEPLOY_UI_NGINX/"
    echo "Copied UI to $DEPLOY_UI_NGINX (sudo)."
  fi
  ensure_nginx
  NGINX_CONF="/etc/nginx/conf.d/rustclaw-ui.conf"
  NGINX_CONFIG_CHANGED=0
  if nginx_ui_config_matches "$NGINX_CONF" "$DEPLOY_UI_NGINX"; then
    echo "Nginx config already up-to-date, skip configure: $NGINX_CONF"
  elif [[ -w /etc/nginx/conf.d ]]; then
    mkdir -p /etc/nginx/conf.d
    cat > "$NGINX_CONF" << NGX
# RustClaw UI 仅静态托管，root 指向部署目录；API 地址在 UI 中填写。
# default_server 使本虚拟主机处理 80 端口所有未匹配请求；不用 server_name _ 避免与其它配置冲突。
server {
    listen 0.0.0.0:80 default_server;
    listen [::]:80 default_server;
    root $DEPLOY_UI_NGINX;
    index index.html;
    location / {
        try_files \$uri \$uri/ /index.html;
    }
}
NGX
    echo "Wrote nginx config: $NGINX_CONF"
    NGINX_CONFIG_CHANGED=1
  else
    sudo mkdir -p /etc/nginx/conf.d
    sudo tee "$NGINX_CONF" >/dev/null << NGX
# RustClaw UI 仅静态托管，root 指向部署目录；API 地址在 UI 中填写。
# default_server 使本虚拟主机处理 80 端口所有未匹配请求；不用 server_name _ 避免与其它配置冲突。
server {
    listen 0.0.0.0:80 default_server;
    listen [::]:80 default_server;
    root $DEPLOY_UI_NGINX;
    index index.html;
    location / {
        try_files \$uri \$uri/ /index.html;
    }
}
NGX
    echo "Wrote nginx config: $NGINX_CONF (sudo)."
    NGINX_CONFIG_CHANGED=1
  fi
  # 禁用 nginx 自带默认页，否则 80 端口会优先显示 default 页面
  if [[ -f /etc/nginx/sites-enabled/default ]] || [[ -L /etc/nginx/sites-enabled/default ]]; then
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
    if sudo nginx -t; then
      if command -v systemctl >/dev/null 2>&1; then
        sudo systemctl reload nginx
        echo "Nginx config OK and reloaded via systemctl."
      else
        sudo nginx -s reload
        echo "Nginx config OK and reloaded via nginx -s reload."
      fi
      echo "若外网用 IP 无法访问，请检查：1) 防火墙放行 80 端口（如 sudo ufw allow 80）；2) 云安全组/入站规则放行 80。"
    else
      echo "Nginx test failed. Check $NGINX_CONF and run: sudo nginx -t"
      exit 1
    fi
  else
    echo "Skip nginx reload (no config changes)."
  fi
  echo "UI 仅静态；用户在页面中填写 clawd 公网地址访问 API。"
fi

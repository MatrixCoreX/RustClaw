#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
TARGET="$SCRIPT_DIR/rustclaw"
DEFAULT_INSTALL_DIR="/usr/local/bin"
USER_INSTALL_DIR="${HOME}/.local/bin"
FORCE_BUILD=0
DO_BUILD=0
USE_USER_DIR=0
INSTALL_DIR="$DEFAULT_INSTALL_DIR"
HOST_OS="$(detect_host_os || printf '%s' "unknown")"
HOST_ARCH="$(detect_host_arch || printf '%s' "unknown")"
REQUESTED_TARGET="host"
HOST_RUST_TARGET="$(host_rust_target 2>/dev/null || true)"
# 默认部署 UI 到 nginx：配置 nginx、复制 UI、重启 nginx；--no-deploy-ui 可跳过
DEPLOY_UI_NGINX=""
# --pi-app：配置 Pi App 桌面快捷方式 + 开机自启（小屏）
CONFIGURE_PI_APP=0

# 无构建模式优先使用 Git 跟踪的 release-bin；若本地刚构建，则回退到 target/release
TRACKED_RELEASE_DIR="$SCRIPT_DIR/release-bin"
REQUIRED_BIN_NAME="clawd"
# 交叉编译拉回路径（与 cross-build-upload.sh 一致）
CROSS_TARGET="${RUSTCLAW_CROSS_TARGET:-aarch64-unknown-linux-gnu}"

default_nginx_root() {
  if [[ "$HOST_OS" == "macos" ]]; then
    printf '%s\n' "$HOME/.rustclaw/nginx-ui"
    return
  fi
  printf '%s\n' "/var/www/html/rustclaw"
}

DEPLOY_UI_NGINX="$(default_nginx_root)"
NGINX_SITE_LINK=""

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

nginx_site_link_path() {
  local conf_path="$1"
  if [[ "$HOST_OS" == "macos" ]]; then
    return 0
  fi
  if [[ "$conf_path" == /etc/nginx/sites-available/* ]] && [[ -d "/etc/nginx/sites-enabled" ]]; then
    printf '%s\n' "/etc/nginx/sites-enabled/$(basename "$conf_path")"
  fi
}

NGINX_CONF="$(nginx_conf_path)"
NGINX_CONF_DIR="$(dirname "$NGINX_CONF")"
NGINX_MAIN_CONF="$(nginx_main_conf_path)"
NGINX_SITE_LINK="$(nginx_site_link_path "$NGINX_CONF" || true)"

usage() {
  cat <<'EOF'
Usage:
  bash install-rustclaw-cmd.sh [options]

Options:
  --build          Install deps and build if needed, then install launcher
  --force-build    Force rebuild before install (implies --build)
  --target TARGET  Build/install target triple, or use 'host' (default)
  --user           Install to ~/.local/bin (no sudo)
  --dir <path>     Install to custom directory
  --deploy-ui-nginx [path]   Deploy UI to path (default: auto-detect per OS), configure nginx, reload nginx
  --no-deploy-ui   Skip nginx config and UI deploy (launcher only)
  --pi-app         Configure Pi App: desktop shortcut + autostart on login (RustClaw small screen)
  -h, --help       Show this help

Default: install launcher and deploy UI to nginx (config nginx, copy UI, reload nginx). Default path is auto-detected per OS. Use --no-deploy-ui to skip UI/nginx.
No build unless --build/--force-build; host builds use target/release/clawd, explicit cross targets use target/<target>/release/clawd, then fall back to release-bin/clawd.
Use --build or --force-build when building from source.
Build summary:
  host platform   -> auto-detected from current machine
  selected target -> host by default
  primary output  -> target/release for host, target/<target>/release for explicit cross target

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
BUILD_RELEASE_DIR="$(preferred_release_dir_for_target "$SCRIPT_DIR" "$INSTALL_TARGET")"
HOST_BUILD_RELEASE_DIR="$(preferred_release_dir_for_target "$SCRIPT_DIR" "$HOST_RUST_TARGET")"
FALLBACK_RELEASE_DIR="$(target_release_dir "$SCRIPT_DIR" "")"
CROSS_RELEASE="$(target_release_dir "$SCRIPT_DIR" "$CROSS_TARGET")"
PACKAGE_FLAVOR="$(package_flavor_for_target "$INSTALL_TARGET" 2>/dev/null || printf '%s' "$INSTALL_TARGET")"

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

ensure_protoc() {
  if command -v protoc >/dev/null 2>&1; then
    export PROTOC
    PROTOC="$(command -v protoc)"
    return 0
  fi
  echo "protoc not found. Attempting to install Protocol Buffers compiler..."
  if command -v brew >/dev/null 2>&1; then
    brew install protobuf
  elif command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -qq && sudo apt-get install -y protobuf-compiler
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y protobuf-compiler
  elif command -v yum >/dev/null 2>&1; then
    sudo yum install -y protobuf-compiler
  elif command -v zypper >/dev/null 2>&1; then
    sudo zypper --non-interactive install protobuf
  elif command -v pacman >/dev/null 2>&1; then
    sudo pacman -Sy --noconfirm protobuf
  elif command -v apk >/dev/null 2>&1; then
    sudo apk add protobuf
  else
    echo "Please install protoc first."
    echo "Debian/Ubuntu: sudo apt-get install protobuf-compiler"
    echo "macOS: brew install protobuf"
    exit 1
  fi
  if ! command -v protoc >/dev/null 2>&1; then
    echo "protoc still not found after install attempt."
    exit 1
  fi
  export PROTOC
  PROTOC="$(command -v protoc)"
  echo "protoc ready: $PROTOC"
}

ensure_bindgen_toolchain() {
  local need_install=0
  if ! command -v clang >/dev/null 2>&1; then
    need_install=1
  elif [[ -z "${LIBCLANG_PATH:-}" ]] && ! ldconfig -p 2>/dev/null | grep -q "libclang\.so"; then
    need_install=1
  fi

  if [[ "$need_install" != "1" ]]; then
    return 0
  fi

  echo "clang/libclang not found. Attempting to install bindgen toolchain..."
  if command -v brew >/dev/null 2>&1; then
    brew install llvm
    if [[ -z "${LIBCLANG_PATH:-}" ]]; then
      local llvm_prefix=""
      llvm_prefix="$(brew --prefix llvm 2>/dev/null || true)"
      if [[ -n "$llvm_prefix" && -d "$llvm_prefix/lib" ]]; then
        export LIBCLANG_PATH="$llvm_prefix/lib"
      fi
    fi
  elif command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -qq && sudo apt-get install -y clang libclang-dev
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y clang llvm-devel libclang
  elif command -v yum >/dev/null 2>&1; then
    sudo yum install -y clang llvm-devel libclang
  elif command -v zypper >/dev/null 2>&1; then
    sudo zypper --non-interactive install clang llvm-devel libclang
  elif command -v pacman >/dev/null 2>&1; then
    sudo pacman -Sy --noconfirm clang llvm
  elif command -v apk >/dev/null 2>&1; then
    sudo apk add clang llvm-dev libclang
  else
    echo "Please install clang and libclang first."
    echo "Debian/Ubuntu: sudo apt-get install clang libclang-dev"
    echo "macOS: brew install llvm"
    exit 1
  fi

  if ! command -v clang >/dev/null 2>&1; then
    echo "clang still not found after install attempt."
    exit 1
  fi
  if [[ -z "${LIBCLANG_PATH:-}" ]] && ! ldconfig -p 2>/dev/null | grep -q "libclang\.so"; then
    echo "libclang still not found after install attempt."
    echo "You may need to set LIBCLANG_PATH manually."
    exit 1
  fi
  echo "bindgen toolchain ready."
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
  elif command -v brew >/dev/null 2>&1; then
    brew install node
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

# ----- 确保 nginx 已安装并启动（用于 --deploy-ui-nginx） -----
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
  if command -v systemctl >/dev/null 2>&1 && systemctl list-unit-files >/dev/null 2>&1; then
    sudo nginx -t
    sudo systemctl reload nginx
    echo "Nginx config OK and reloaded via systemctl."
    return
  fi

  if command -v service >/dev/null 2>&1; then
    sudo nginx -t
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
    sudo nginx -t
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
    if nginx -t >/dev/null 2>&1; then
      brew services restart nginx >/dev/null 2>&1
      echo "Nginx config OK and restarted via brew services."
      return
    fi
    echo "Nginx test failed. Check $NGINX_CONF and run: nginx -t"
    exit 1
  fi

  if command -v nginx >/dev/null 2>&1; then
    if sudo nginx -t; then
      sudo nginx -s reload
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
  grep -qE "listen[[:space:]]+.*80[[:space:]]*(default_server)?;" "$conf_path" || return 1
  return 0
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
  if grep -Fq "$include_line" "$main_conf"; then
    return 0
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
  need="$(python3 - "$SCRIPT_DIR" "$force" "$INSTALL_TARGET" "$HOST_RUST_TARGET" <<'PY'
import json
import os
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
mode = sys.argv[2].strip().lower()
install_target = sys.argv[3].strip()
host_target = sys.argv[4].strip()
if not install_target or install_target == host_target:
    release_dir = root / "target" / "release"
else:
    release_dir = root / "target" / install_target / "release"

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
  printf '%s\n' "$need"
}

# 仅构建 release：UI（若有）+ cargo build --workspace --release，不调用其他脚本
do_release_build() {
  [[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
  ensure_cargo
  ensure_protoc
  ensure_bindgen_toolchain
  if [[ "$INSTALL_TARGET" != "$HOST_RUST_TARGET" ]] && command -v rustup >/dev/null 2>&1; then
    rustup target add "$INSTALL_TARGET" >/dev/null 2>&1 || true
  fi
  if [[ -d "$SCRIPT_DIR/UI" ]]; then
    ensure_npm
    if [[ ! -d "$SCRIPT_DIR/UI/node_modules" ]]; then
      echo "Installing UI dependencies..."
      (cd "$SCRIPT_DIR/UI" && npm install)
    fi
    echo "Building UI assets..."
    (cd "$SCRIPT_DIR/UI" && npm run build)
  fi
  echo "Building workspace (release, target=$INSTALL_TARGET, output=$BUILD_RELEASE_DIR)..."
  if [[ "$INSTALL_TARGET" == "$HOST_RUST_TARGET" ]]; then
    (cd "$SCRIPT_DIR" && cargo build --workspace --release)
  else
    (cd "$SCRIPT_DIR" && cargo build --workspace --release --target "$INSTALL_TARGET")
  fi
  if [[ ! -x "$BUILD_RELEASE_DIR/clawd" ]]; then
    echo "Build finished but $BUILD_RELEASE_DIR/clawd missing."
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

echo "Host platform: ${HOST_OS}/${HOST_ARCH} ${HOST_RUST_TARGET:+($HOST_RUST_TARGET)}"
echo "Selected target: $INSTALL_TARGET"
echo "Primary output: $BUILD_RELEASE_DIR"
echo "Flavor tag: $PACKAGE_FLAVOR"

SELECTED_RELEASE_DIR="$(resolve_release_dir)"
REQUIRED_BIN="$SELECTED_RELEASE_DIR/$REQUIRED_BIN_NAME"

if [[ "$DO_BUILD" == "1" ]]; then
  if [[ "$FORCE_BUILD" == "1" ]]; then
    ensure_build "--force-build"
  else
    ensure_build ""
  fi
else
  if [[ "$INSTALL_TARGET" != "$HOST_RUST_TARGET" ]] && [[ ! -f "$REQUIRED_BIN" ]]; then
    if [[ -x "$CROSS_RELEASE/clawd" ]]; then
      echo "Using cross-compiled binaries from $CROSS_RELEASE"
      mkdir -p "$BUILD_RELEASE_DIR"
      for f in "$CROSS_RELEASE"/*; do
        [[ ! -f "$f" || ! -x "$f" ]] && continue
        [[ "$f" == *.rlib || "$f" == *.d ]] && continue
        ln -sf "$f" "$BUILD_RELEASE_DIR/$(basename "$f")" 2>/dev/null || cp -f "$f" "$BUILD_RELEASE_DIR/$(basename "$f")"
      done
      SELECTED_RELEASE_DIR="$(resolve_release_dir)"
      REQUIRED_BIN="$SELECTED_RELEASE_DIR/$REQUIRED_BIN_NAME"
    fi
  fi
  if [[ ! -f "$REQUIRED_BIN" ]]; then
    echo "Error: binary not found: $REQUIRED_BIN"
    echo "Copy your built clawd into $BUILD_RELEASE_DIR/ or release-bin/, or run with --build to build from source."
    echo "To track release executables in git, run: bash scripts/sync-release-bin.sh"
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
  echo "Note: clawcli not found ($CLAWCLI_BIN). Run with --build to build workspace including clawcli."
fi

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
echo "  command -v clawcli && clawcli --help   # terminal chat CLI (if installed)"
echo
echo "Key management:"
echo "  rustclaw -key list"
echo "  rustclaw -key generate user"
echo "  rustclaw -key generate admin"
echo
echo "Try:"
echo "  rustclaw -status"
echo "  rustclaw -start release all                        # 配置通信端后直接启动"
echo "  rustclaw -restart release all                      # 配置通信端后直接重启"
echo "  rustclaw -start release"
echo "  rustclaw -stop"
echo
echo "Tip:"
echo "  bash install-rustclaw-cmd.sh --build     # build from source then install"
echo "  bash install-rustclaw-cmd.sh --force-build   # force rebuild then install"
echo "Uninstall (removes command only, does not touch configs):"
echo "  bash uninstall-rustclaw-cmd.sh [--user|--dir <path>]"
if [[ "$CONFIGURE_PI_APP" == "1" ]]; then
  PI_APP_DIR="$SCRIPT_DIR/pi_app"
  if [[ -d "$PI_APP_DIR" && -x "$PI_APP_DIR/install-desktop.sh" && -x "$PI_APP_DIR/enable-autostart.sh" ]]; then
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
  if path_writable_or_creatable "$DEPLOY_UI_NGINX"; then
    mkdir -p "$DEPLOY_UI_NGINX"
    cp -R "$SCRIPT_DIR/UI/dist/." "$DEPLOY_UI_NGINX/"
    echo "Copied UI to $DEPLOY_UI_NGINX (no sudo)."
  else
    sudo mkdir -p "$DEPLOY_UI_NGINX"
    sudo cp -R "$SCRIPT_DIR/UI/dist/." "$DEPLOY_UI_NGINX/"
    echo "Copied UI to $DEPLOY_UI_NGINX (sudo)."
  fi
  ensure_nginx
  ensure_nginx_site_include "$NGINX_MAIN_CONF" "$NGINX_CONF_DIR"
  PROXY_UPSTREAM="$(resolve_webd_proxy_upstream)"
  NGINX_CONFIG_CHANGED=0
  if nginx_ui_config_matches "$NGINX_CONF" "$DEPLOY_UI_NGINX" "$PROXY_UPSTREAM"; then
    echo "Nginx config already up-to-date, skip configure: $NGINX_CONF"
  elif path_writable_or_creatable "$NGINX_CONF_DIR"; then
    mkdir -p "$NGINX_CONF_DIR"
    cat > "$NGINX_CONF" << NGX
# RustClaw UI: 静态资源由 nginx 托管，/v1 与 /webd 反代到 webd。
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
# RustClaw UI: 静态资源由 nginx 托管，/v1 与 /webd 反代到 webd。
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

    location / {
        try_files \$uri \$uri/ /index.html;
    }
}
NGX
    echo "Wrote nginx config: $NGINX_CONF (sudo)."
    NGINX_CONFIG_CHANGED=1
  fi
  ensure_nginx_site_link "$NGINX_CONF" "$NGINX_SITE_LINK"
  if remove_stale_nginx_ui_entries "$NGINX_CONF" "$NGINX_SITE_LINK"; then
    NGINX_CONFIG_CHANGED=1
  fi
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

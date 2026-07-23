#!/usr/bin/env bash
# 编译 UI（npm run build 默认输出目录），并可显式复制到 nginx 目录。
# 参数：默认仅编译；--deploy / --copy 显式复制；--deploy-if-configured 仅更新已有站点。
# 可选：--path /path/to/nginx/root 指定 nginx 站点根。

set -euo pipefail

# 脚本所在目录 = RustClaw 根目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
UI_DIR="$SCRIPT_DIR/UI"
# npm run build 在 UI 下的默认输出目录（Vite 默认 dist）
DIST_DIR="$UI_DIR/dist"
BUILD_STAMP_FILE="$DIST_DIR/.rustclaw-ui-build-fingerprint"
DEPS_STAMP_FILE="$UI_DIR/node_modules/.rustclaw-ui-dependency-fingerprint"
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

DO_BUILD=""
DO_DEPLOY=""
DEPLOY_IF_CONFIGURED=""
NGINX_ROOT="$NGINX_ROOT_DEFAULT"

ui_deps_healthy() {
  local vite_pkg_bin="$UI_DIR/node_modules/vite/bin/vite.js"
  local vite_cli="$UI_DIR/node_modules/vite/dist/node/cli.js"

  [[ -f "$vite_pkg_bin" ]] || return 1
  [[ -f "$vite_cli" ]] || return 1

  node - "$UI_DIR" <<'NODE'
const fs = require("fs");
const path = require("path");

const uiDir = process.argv[2];
const manifest = JSON.parse(fs.readFileSync(path.join(uiDir, "package.json"), "utf8"));
const dependencies = {
  ...(manifest.dependencies || {}),
  ...(manifest.devDependencies || {}),
};

for (const name of Object.keys(dependencies)) {
  if (!fs.existsSync(path.join(uiDir, "node_modules", name, "package.json"))) {
    process.exit(1);
  }
}
NODE
}

ui_dependency_fingerprint() {
  local package_hash lock_hash
  package_hash="$(hash_file "$UI_DIR/package.json")"
  lock_hash="missing"
  if [[ -f "$UI_DIR/package-lock.json" ]]; then
    lock_hash="$(hash_file "$UI_DIR/package-lock.json")"
  fi
  printf 'package=%s\nlock=%s\n' "$package_hash" "$lock_hash"
}

ensure_ui_deps() {
  local install_reason=""
  if [[ ! -d "$UI_DIR/node_modules" ]]; then
    install_reason="node_modules is missing"
  elif ! ui_deps_healthy; then
    install_reason="installed dependency set is incomplete"
  else
    local current_fingerprint last_fingerprint=""
    current_fingerprint="$(ui_dependency_fingerprint)"
    if [[ -f "$DEPS_STAMP_FILE" ]]; then
      last_fingerprint="$(cat "$DEPS_STAMP_FILE")"
    fi
    if [[ "$current_fingerprint" != "$last_fingerprint" ]]; then
      install_reason="package manifest or lock file changed"
    fi
  fi

  if [[ -n "$install_reason" ]]; then
    echo "Synchronizing UI dependencies: $install_reason."
    (cd "$UI_DIR" && npm install --prefer-offline --no-audit --no-fund)
  fi

  if ! ui_deps_healthy; then
    echo "Error: UI dependencies remain incomplete after npm install." >&2
    return 1
  fi

  ui_dependency_fingerprint > "$DEPS_STAMP_FILE"
}

ui_node_options() {
  local heap_mb="${RUSTCLAW_UI_NODE_MAX_OLD_SPACE_MB:-1536}"
  if [[ ! "$heap_mb" =~ ^[0-9]+$ ]] || (( heap_mb < 512 )); then
    echo "Error: RUSTCLAW_UI_NODE_MAX_OLD_SPACE_MB must be an integer of at least 512." >&2
    return 1
  fi

  local -a retained=()
  local -a existing_options=()
  read -r -a existing_options <<< "${NODE_OPTIONS:-}"
  local skip_next=0
  local token
  for token in "${existing_options[@]}"; do
    if (( skip_next == 1 )); then
      skip_next=0
      continue
    fi
    case "$token" in
      --max-old-space-size|--max_old_space_size)
        skip_next=1
        ;;
      --max-old-space-size=*|--max_old_space_size=*)
        ;;
      *)
        retained+=("$token")
        ;;
    esac
  done
  retained+=("--max-old-space-size=${heap_mb}")
  printf '%s\n' "${retained[*]}"
}

run_ui_build() {
  local node_options
  node_options="$(ui_node_options)"
  echo "UI Node options: $node_options"
  (cd "$UI_DIR" && NODE_OPTIONS="$node_options" npm run build)
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

usage() {
  echo "Usage: $0 [--build] [--deploy|--copy|--deploy-if-configured] [--path DIR]"
  echo "  --build         Only build UI (npm run build, output: UI/dist)."
  echo "  --deploy        Only copy UI/dist and refresh nginx config."
  echo "  --copy          Same as --deploy."
  echo "  --deploy-if-configured"
  echo "                  Build UI, then update nginx only when a RustClaw nginx site already exists."
  echo "  --path DIR      Nginx site root (default: $NGINX_ROOT_DEFAULT)."
  echo "  (no args)       Build UI only. Local deployment does not require nginx."
  echo "  host platform   Auto-detected as ${HOST_OS}/${HOST_ARCH} ${HOST_TARGET:+($HOST_TARGET)}."
  echo "  UI output       Always written to UI/dist."
  echo ""
  echo "Examples:"
  echo "  $0                    # local/default: build UI only"
  echo "  $0 --build            # only build"
  echo "  $0 --deploy           # only copy existing UI/dist"
  echo "  $0 --build --deploy   # cloud/server: build + deploy nginx"
  echo "  $0 --deploy --path /srv/http/rustclaw   # deploy to custom path"
}

hash_file() {
  local target="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$target" | awk '{print $1}'
  else
    shasum -a 256 "$target" | awk '{print $1}'
  fi
}

compute_ui_build_fingerprint() {
  local manifest
  manifest="$(mktemp)"
  trap 'rm -f "$manifest"' RETURN

  local candidate
  local fixed_files=(
    "$UI_DIR/package.json"
    "$UI_DIR/package-lock.json"
    "$UI_DIR/index.html"
    "$UI_DIR/tsconfig.json"
    "$UI_DIR/tsconfig.app.json"
    "$UI_DIR/tsconfig.node.json"
    "$UI_DIR/vite.config.ts"
    "$UI_DIR/vite.config.js"
    "$UI_DIR/tailwind.config.js"
    "$UI_DIR/tailwind.config.ts"
    "$UI_DIR/postcss.config.js"
    "$UI_DIR/postcss.config.cjs"
  )

  for candidate in "${fixed_files[@]}"; do
    if [[ -f "$candidate" ]]; then
      printf '%s  %s\n' "$(hash_file "$candidate")" "${candidate#$UI_DIR/}" >> "$manifest"
    fi
  done

  local dir
  for dir in "$UI_DIR/src" "$UI_DIR/public"; do
    if [[ -d "$dir" ]]; then
      while IFS= read -r candidate; do
        [[ -f "$candidate" ]] || continue
        printf '%s  %s\n' "$(hash_file "$candidate")" "${candidate#$UI_DIR/}" >> "$manifest"
      done < <(find "$dir" -type f | sort)
    fi
  done

  hash_file "$manifest"
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
  if [[ "$include_dir" == "/etc/nginx/sites-enabled" ]]; then
    disable_nginx_sites_available_include "$main_conf"
  fi
  if nginx_main_conf_includes_dir "$main_conf" "$include_dir"; then
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
  local nginx_bin=""
  nginx_bin="$(find_nginx_bin || true)"

  echo "Reloading nginx..."
  if [[ -z "$nginx_bin" ]]; then
    echo "Warning: nginx reload skipped. Please install or reload nginx manually."
    return
  fi

  if systemctl_available; then
    sudo "$nginx_bin" -t
    sudo systemctl reload nginx
    echo "Nginx reloaded via systemctl."
    return
  fi

  if service_available; then
    sudo "$nginx_bin" -t
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
    sudo "$nginx_bin" -t
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
    if [[ -n "$nginx_bin" ]] && "$nginx_bin" -t >/dev/null 2>&1 && brew services restart nginx >/dev/null 2>&1; then
      echo "Nginx restarted via brew services."
      return
    fi
  fi

  if [[ -n "$nginx_bin" ]]; then
    sudo "$nginx_bin" -t
    sudo "$nginx_bin" -s reload
    echo "Nginx reloaded via nginx -s reload."
    return
  fi

  echo "Warning: nginx reload skipped. Please reload nginx manually."
}

NGINX_CONF="$(nginx_conf_path)"
NGINX_CONF_DIR="$(dirname "$NGINX_CONF")"
NGINX_SITE_LINK="$(nginx_site_link_path "$NGINX_CONF" || true)"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --build)
      DO_BUILD=1
      shift
      ;;
    --deploy|--copy)
      DO_DEPLOY=1
      shift
      ;;
    --deploy-if-configured)
      DO_BUILD=1
      DEPLOY_IF_CONFIGURED=1
      shift
      ;;
    --path)
      NGINX_ROOT="${2:?Missing argument for --path}"
      DO_DEPLOY=1
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

# 未指定任何一项时仅构建。Nginx 是云服务器的显式部署选项。
if [[ -z "$DO_BUILD" && -z "$DO_DEPLOY" ]]; then
  DO_BUILD=1
fi

if [[ -n "$DEPLOY_IF_CONFIGURED" ]]; then
  if [[ -f "$NGINX_CONF" || ( -n "$NGINX_SITE_LINK" && -e "$NGINX_SITE_LINK" ) ]]; then
    DO_DEPLOY=1
    echo "Existing RustClaw nginx site detected; UI assets will be deployed after the build."
  else
    echo "No existing RustClaw nginx site detected; keeping local build-only mode."
  fi
fi

if [[ -n "$DO_BUILD" ]]; then
  echo "Host platform: ${HOST_OS}/${HOST_ARCH} ${HOST_TARGET:+($HOST_TARGET)}"
  echo "UI output: $DIST_DIR"
  if [[ ! -d "$UI_DIR" ]]; then
    echo "Error: UI directory not found: $UI_DIR"
    exit 1
  fi
  if ! command -v npm >/dev/null 2>&1; then
    echo "Error: npm not found. Install Node.js/npm first."
    exit 1
  fi
  ensure_ui_deps
  CURRENT_FINGERPRINT="$(compute_ui_build_fingerprint)"
  if [[ -f "$DIST_DIR/index.html" ]] && [[ -f "$BUILD_STAMP_FILE" ]]; then
    LAST_FINGERPRINT="$(tr -d '\r\n' < "$BUILD_STAMP_FILE")"
    if [[ "$CURRENT_FINGERPRINT" == "$LAST_FINGERPRINT" ]]; then
      echo "UI already built for current version, skip build."
    else
      echo "Building UI (output=$DIST_DIR)..."
      run_ui_build
    fi
  else
    echo "Building UI (output=$DIST_DIR)..."
    run_ui_build
  fi
  if [[ ! -d "$DIST_DIR" ]] || [[ ! -f "$DIST_DIR/index.html" ]]; then
    echo "Error: UI build failed (dist missing or no index.html)."
    exit 1
  fi
  printf '%s\n' "$CURRENT_FINGERPRINT" > "$BUILD_STAMP_FILE"
  echo "UI build completed: $DIST_DIR"
fi

if [[ -n "$DO_DEPLOY" ]]; then
  if [[ ! -d "$DIST_DIR" ]] || [[ ! -f "$DIST_DIR/index.html" ]]; then
    echo "Error: UI/dist not built. Run with --build or without --deploy first."
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
  ensure_deployed_ui_readable "$NGINX_ROOT"
  PROXY_UPSTREAM="$(resolve_webd_proxy_upstream)"
  NGINX_MAIN_CONF="$(nginx_main_conf_path)"
  NGINX_INCLUDE_DIR="$(nginx_include_dir_for_conf "$NGINX_CONF" "$NGINX_SITE_LINK")"
  ensure_nginx_site_include "$NGINX_MAIN_CONF" "$NGINX_INCLUDE_DIR"
  NGINX_CONFIG_CHANGED=0
  if nginx_ui_config_matches "$NGINX_CONF" "$NGINX_ROOT" "$PROXY_UPSTREAM"; then
    echo "Nginx config already up-to-date, skip configure: $NGINX_CONF"
  elif nginx_ui_config_is_tls_managed "$NGINX_CONF"; then
    echo "Preserving existing TLS-enabled nginx config: $NGINX_CONF"
    echo "Verify its UI root and proxy upstream manually if the deployment path changed."
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
fi

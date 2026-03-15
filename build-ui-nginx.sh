#!/usr/bin/env bash
# 编译 UI（npm run build 默认输出目录）并复制到 nginx 目录。
# 参数：--build 仅编译；--deploy / --copy 仅复制；默认先编译再复制。
# 可选：--path /path/to/nginx/root 指定 nginx 站点根（默认 /var/www/html/rustclaw）。

set -e

# 脚本所在目录 = RustClaw 根目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UI_DIR="$SCRIPT_DIR/UI"
# npm run build 在 UI 下的默认输出目录（Vite 默认 dist）
DIST_DIR="$UI_DIR/dist"
BUILD_STAMP_FILE="$DIST_DIR/.rustclaw-ui-build-fingerprint"
NGINX_ROOT_DEFAULT="/var/www/html/rustclaw"

DO_BUILD=""
DO_DEPLOY=""
NGINX_ROOT="$NGINX_ROOT_DEFAULT"

usage() {
  echo "Usage: $0 [--build] [--deploy|--copy] [--path DIR]"
  echo "  --build         Only build UI (npm run build, output: UI/dist)."
  echo "  --deploy        Only copy UI/dist to nginx directory."
  echo "  --copy          Same as --deploy."
  echo "  --path DIR      Nginx site root (default: $NGINX_ROOT_DEFAULT)."
  echo "  (no args)       Build then copy (default)."
  echo ""
  echo "Examples:"
  echo "  $0                    # build + copy to default nginx root"
  echo "  $0 --build            # only build"
  echo "  $0 --deploy           # only copy existing UI/dist"
  echo "  $0 --path /srv/http/rustclaw   # build + copy to custom path"
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

# 未指定任何一项时，默认两项都做
if [[ -z "$DO_BUILD" && -z "$DO_DEPLOY" ]]; then
  DO_BUILD=1
  DO_DEPLOY=1
fi

if [[ -n "$DO_BUILD" ]]; then
  if [[ ! -d "$UI_DIR" ]]; then
    echo "Error: UI directory not found: $UI_DIR"
    exit 1
  fi
  if ! command -v npm >/dev/null 2>&1; then
    echo "Error: npm not found. Install Node.js/npm first."
    exit 1
  fi
  if [[ ! -d "$UI_DIR/node_modules" ]]; then
    echo "Installing UI dependencies..."
    (cd "$UI_DIR" && npm install)
  fi
  CURRENT_FINGERPRINT="$(compute_ui_build_fingerprint)"
  if [[ -f "$DIST_DIR/index.html" ]] && [[ -f "$BUILD_STAMP_FILE" ]]; then
    LAST_FINGERPRINT="$(tr -d '\r\n' < "$BUILD_STAMP_FILE")"
    if [[ "$CURRENT_FINGERPRINT" == "$LAST_FINGERPRINT" ]]; then
      echo "UI already built for current version, skip build."
    else
      echo "Building UI (output: $DIST_DIR)..."
      (cd "$UI_DIR" && npm run build)
    fi
  else
    echo "Building UI (output: $DIST_DIR)..."
    (cd "$UI_DIR" && npm run build)
  fi
  if [[ ! -d "$DIST_DIR" ]] || [[ ! -f "$DIST_DIR/index.html" ]]; then
    echo "Error: UI build failed (dist missing or no index.html)."
    exit 1
  fi
  printf '%s\n' "$CURRENT_FINGERPRINT" > "$BUILD_STAMP_FILE"
  echo "UI build done: $DIST_DIR"
fi

if [[ -n "$DO_DEPLOY" ]]; then
  if [[ ! -d "$DIST_DIR" ]] || [[ ! -f "$DIST_DIR/index.html" ]]; then
    echo "Error: UI/dist not built. Run with --build or without --deploy first."
    exit 1
  fi
  if [[ -w "$NGINX_ROOT" ]]; then
    mkdir -p "$NGINX_ROOT"
    cp -r "$DIST_DIR/"* "$NGINX_ROOT/"
    echo "Copied UI to $NGINX_ROOT (no sudo)."
  else
    sudo mkdir -p "$NGINX_ROOT"
    sudo cp -r "$DIST_DIR/"* "$NGINX_ROOT/"
    echo "Copied UI to $NGINX_ROOT (sudo)."
  fi
  echo "Reloading nginx..."
  sudo systemctl reload nginx
  echo "Deploy done."
fi

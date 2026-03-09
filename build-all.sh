#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

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

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi
ensure_cargo

# ----- 确保 npm 已安装（仅在有 UI 目录时需要） -----
ensure_npm() {
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

echo "Syncing skill docs (INTERFACE.md + prompts/skills/*.md)..." # zh: 同步技能文档（INTERFACE.md + prompts/skills/*.md）...
python3 "$SCRIPT_DIR/scripts/sync_skill_docs.py"

PROFILE="${1:-release}"
DO_CLEAN="${2:-0}"

case "$PROFILE" in
  all|release|debug)
    ;;
  *)
    echo "Usage: ./build-all.sh [all|release|debug] [clean]  # default: release" # zh: 用法：./build-all.sh [all|release|debug] [clean]，默认 release
    exit 1
    ;;
esac

# 可通过 SKIP_UI=1 或 第三个参数 no-ui 跳过 UI 构建（避免树莓派上 npm/vite 卡死或 OOM）
SKIP_UI="${SKIP_UI:-0}"
[[ "${3:-}" == "no-ui" ]] && SKIP_UI=1

# 判断 UI 是否已编译：存在 dist 且含 index.html 视为已构建，可跳过
ui_already_built() {
  [[ -f "$SCRIPT_DIR/UI/dist/index.html" ]]
}

if [[ -d "$SCRIPT_DIR/UI" ]] && [[ "$SKIP_UI" != "1" ]]; then
  if ui_already_built; then
    echo "UI already built (UI/dist/index.html exists), skipping UI build." # zh: UI 已编译，跳过。
  else
    ensure_npm
    if [[ ! -d "$SCRIPT_DIR/UI/node_modules" ]]; then
      echo "Installing UI dependencies (may take a while on Pi)..." # zh: 正在安装 UI 依赖（树莓派可能较慢）...
      (cd "$SCRIPT_DIR/UI" && npm install --prefer-offline --no-audit --no-fund 2>&1) || {
        echo "UI npm install failed. Skip UI build. To retry later: cd UI && npm install" # zh: UI 依赖安装失败，跳过。稍后可在 UI 目录执行 npm install
        SKIP_UI=1
      }
    fi
    if [[ "$SKIP_UI" != "1" ]]; then
      echo "Building UI assets (Vite build, may take 1–3 min on Pi)..." # zh: 正在构建 UI 资源（树莓派约 1–3 分钟）...
      (cd "$SCRIPT_DIR/UI" && NODE_OPTIONS="${NODE_OPTIONS:---max-old-space-size=512}" npm run build 2>&1) || {
        echo "UI build failed. Continue without UI. To skip UI next time: SKIP_UI=1 ./build-all.sh or ./build-all.sh all clean no-ui" # zh: UI 构建失败，继续。下次跳过 UI 可加 no-ui 或 SKIP_UI=1
      }
    fi
  fi
elif [[ "$SKIP_UI" == "1" ]]; then
  echo "Skipping UI build (SKIP_UI=1 or no-ui)." # zh: 已跳过 UI 构建
else
  echo "UI directory not found, skip UI build." # zh: 未找到 UI 目录，跳过 UI 构建。
fi

if [[ "$DO_CLEAN" == "clean" ]]; then
  echo "Cleaning previous build artifacts..." # zh: 正在清理历史构建产物...
  cargo clean
fi

BUILD_PROFILES=()
case "$PROFILE" in
  all)
    BUILD_PROFILES=(debug release)
    ;;
  debug|release)
    BUILD_PROFILES=("$PROFILE")
    ;;
esac

echo "Building workspace with profiles: ${BUILD_PROFILES[*]}" # zh: 编译 profile：${BUILD_PROFILES[*]}

# Ensure runtime binaries exist for deployment/start scripts.
# Auto-discover all workspace bin targets to avoid missing newly added skills.
WORKSPACE_METADATA="$(cargo metadata --no-deps --format-version 1)"
export RUSTCLAW_WORKSPACE_METADATA="$WORKSPACE_METADATA"

mapfile -t REQUIRED_BINS < <(
  python3 - <<'PY'
import json
import os
import sys

raw = os.environ.get("RUSTCLAW_WORKSPACE_METADATA", "").strip()
if not raw:
    raise SystemExit(1)
data = json.loads(raw)
workspace_members = set(data.get("workspace_members", []))
bins = set()

for pkg in data.get("packages", []):
    if pkg.get("id") not in workspace_members:
        continue
    for target in pkg.get("targets", []):
        kinds = target.get("kind", [])
        if "bin" in kinds:
            name = (target.get("name") or "").strip()
            if name:
                bins.add(name)

for name in sorted(bins):
    print(name)
PY
)

if [[ "${#REQUIRED_BINS[@]}" -eq 0 ]]; then
  echo "No workspace binary targets discovered via cargo metadata." # zh: 未通过 cargo metadata 发现 workspace 二进制目标。
  exit 1
fi

for build_profile in "${BUILD_PROFILES[@]}"; do
  if [[ "$build_profile" == "release" ]]; then
    cargo build --workspace --release
    OUT_DIR="$SCRIPT_DIR/target/release"
  else
    cargo build --workspace
    OUT_DIR="$SCRIPT_DIR/target/debug"
  fi

  MISSING=0
  for bin in "${REQUIRED_BINS[@]}"; do
    if [[ ! -x "$OUT_DIR/$bin" ]]; then
      echo "Missing binary: $OUT_DIR/$bin" # zh: 缺少二进制：$OUT_DIR/$bin
      MISSING=1
    fi
  done

  if [[ "$MISSING" == "1" ]]; then
    echo "Build finished but required binaries are missing for profile=$build_profile." # zh: 编译结束，但该 profile 关键二进制缺失。
    echo "Try: cargo build --workspace $([[ "$build_profile" == "release" ]] && echo '--release')" # zh: 完整编译工作区：cargo build --workspace [--release]
    exit 1
  fi
done

echo "Build completed." # zh: 编译完成。
if [[ "$PROFILE" == "all" ]]; then
  echo "Output directories: $SCRIPT_DIR/target/debug and $SCRIPT_DIR/target/release" # zh: 输出目录：debug 与 release
else
  echo "Output directory: $SCRIPT_DIR/target/$PROFILE" # zh: 输出目录：$SCRIPT_DIR/target/$PROFILE
fi
echo "Verified binaries: ${REQUIRED_BINS[*]}" # zh: 已校验二进制：${REQUIRED_BINS[*]}

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
	elif command -v brew >/dev/null 2>&1; then
		brew install node
	elif command -v apt-get >/dev/null 2>&1; then
		sudo apt-get update -qq && sudo apt-get install -y nodejs npm
	elif command -v dnf >/dev/null 2>&1; then
		sudo dnf install -y nodejs npm
	elif command -v yum >/dev/null 2>&1; then
		sudo yum install -y nodejs npm
	else
		echo "Please install Node.js and npm first."
		echo "macOS: brew install node"
		echo "Other systems: https://nodejs.org or: curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash"
		exit 1
	fi
	if ! command -v npm >/dev/null 2>&1; then
		echo "npm still not found after install attempt."
		exit 1
	fi
	echo "Node.js/npm ready."
}

echo "Syncing skill docs (INTERFACE.md + prompts/layers/generated/skills/*.md)..." # zh: 同步技能文档（INTERFACE.md + prompts/layers/generated/skills/*.md）...
python3 "$SCRIPT_DIR/scripts/sync_skill_docs.py"

BUILD_PROFILE="release"
DO_CLEAN=0

# 仅保留 release 构建；兼容旧调用中的 release 参数。
# 可通过 SKIP_UI=1 或 no-ui 跳过 UI 构建（避免树莓派上 npm/vite 卡死或 OOM）
SKIP_UI="${SKIP_UI:-0}"
for arg in "$@"; do
	case "$arg" in
	release)
		;;
	clean)
		DO_CLEAN=1
		;;
	no-ui)
		SKIP_UI=1
		;;
	*)
		echo "Usage: ./build-all.sh [release] [clean] [no-ui]  # release only" # zh: 用法：./build-all.sh [release] [clean] [no-ui]，仅构建 release
		exit 1
		;;
	esac
done

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
				echo "UI build failed. Continue without UI. To skip UI next time: SKIP_UI=1 ./build-all.sh or ./build-all.sh clean no-ui" # zh: UI 构建失败，继续。下次跳过 UI 可加 no-ui 或 SKIP_UI=1
			}
		fi
	fi
elif [[ "$SKIP_UI" == "1" ]]; then
	echo "Skipping UI build (SKIP_UI=1 or no-ui)." # zh: 已跳过 UI 构建
else
	echo "UI directory not found, skip UI build." # zh: 未找到 UI 目录，跳过 UI 构建。
fi

if [[ "$DO_CLEAN" == "1" ]]; then
	echo "Cleaning previous build artifacts..." # zh: 正在清理历史构建产物...
	cargo clean
fi

echo "Building workspace with profile: $BUILD_PROFILE" # zh: 编译 profile：$BUILD_PROFILE

# Ensure runtime binaries exist for deployment/start scripts.
# Auto-discover all workspace bin targets to avoid missing newly added skills.
WORKSPACE_METADATA="$(cargo metadata --no-deps --format-version 1)"
export RUSTCLAW_WORKSPACE_METADATA="$WORKSPACE_METADATA"

REQUIRED_BINS=()
while IFS= read -r bin; do
	[[ -n "$bin" ]] && REQUIRED_BINS+=("$bin")
done < <(
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

cargo build --workspace --release
OUT_DIR="$SCRIPT_DIR/target/release"

MISSING=0
for bin in "${REQUIRED_BINS[@]}"; do
	if [[ ! -x "$OUT_DIR/$bin" ]]; then
		echo "Missing binary: $OUT_DIR/$bin" # zh: 缺少二进制：$OUT_DIR/$bin
		MISSING=1
	fi
done

if [[ "$MISSING" == "1" ]]; then
	echo "Build finished but required binaries are missing for profile=$BUILD_PROFILE." # zh: 编译结束，但关键二进制缺失。
	echo "Try: cargo build --workspace --release"                                        # zh: 可尝试完整执行：cargo build --workspace --release
	exit 1
fi

echo "Build completed." # zh: 编译完成。
echo "Output directory: $SCRIPT_DIR/target/$BUILD_PROFILE" # zh: 输出目录：$SCRIPT_DIR/target/$BUILD_PROFILE
echo "Verified binaries: ${REQUIRED_BINS[*]}" # zh: 已校验二进制：${REQUIRED_BINS[*]}

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

PROFILE="${1:-release}"
DO_CLEAN="${2:-0}"

case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./build-all.sh [release|debug] [clean]" # zh: 用法：./build-all.sh [release|debug] [clean]
    exit 1
    ;;
esac

if [[ "$DO_CLEAN" == "clean" ]]; then
  echo "Cleaning previous build artifacts..." # zh: 正在清理历史构建产物...
  cargo clean
fi

echo "Building workspace with profile: $PROFILE" # zh: 使用配置编译整个 workspace：$PROFILE
if [[ "$PROFILE" == "release" ]]; then
  cargo build --workspace --release
  OUT_DIR="$SCRIPT_DIR/target/release"
else
  cargo build --workspace
  OUT_DIR="$SCRIPT_DIR/target/debug"
fi

echo "Build completed." # zh: 编译完成。
echo "Output directory: $OUT_DIR" # zh: 输出目录：$OUT_DIR

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

PROFILE="${1:-all}"
DO_CLEAN="${2:-0}"

case "$PROFILE" in
  all|release|debug)
    ;;
  *)
    echo "Usage: ./build-all.sh [all|release|debug] [clean]" # zh: 用法：./build-all.sh [all|release|debug] [clean]
    exit 1
    ;;
esac

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
    if [[ "$build_profile" == "release" ]]; then
      echo "Try: cargo build -p skill-runner --release" # zh: 可尝试：单独编译 skill-runner（release）。
    else
      echo "Try: cargo build -p skill-runner" # zh: 可尝试：单独编译 skill-runner（debug）。
    fi
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

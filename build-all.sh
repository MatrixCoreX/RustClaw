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

MISSING=0
for bin in "${REQUIRED_BINS[@]}"; do
  if [[ ! -x "$OUT_DIR/$bin" ]]; then
    echo "Missing binary: $OUT_DIR/$bin" # zh: 缺少二进制：$OUT_DIR/$bin
    MISSING=1
  fi
done

if [[ "$MISSING" == "1" ]]; then
  echo "Build finished but required binaries are missing." # zh: 编译结束，但关键二进制缺失。
  if [[ "$PROFILE" == "release" ]]; then
    echo "Try: cargo build -p skill-runner --release" # zh: 可尝试：单独编译 skill-runner（release）。
  else
    echo "Try: cargo build -p skill-runner" # zh: 可尝试：单独编译 skill-runner（debug）。
  fi
  exit 1
fi

# Sync runtime path in config after build.
CONFIG_PATH="$SCRIPT_DIR/configs/config.toml"
SKILL_RUNNER_REL="target/$PROFILE/skill-runner"
if [[ -f "$CONFIG_PATH" ]]; then
  export RUSTCLAW_CONFIG_PATH="$CONFIG_PATH"
  export RUSTCLAW_SKILL_RUNNER_PATH="$SKILL_RUNNER_REL"
  python3 - <<'PY'
import os
import re
from pathlib import Path

cfg_path = Path(os.environ["RUSTCLAW_CONFIG_PATH"])
runner_path = os.environ["RUSTCLAW_SKILL_RUNNER_PATH"].strip()
text = cfg_path.read_text(encoding="utf-8")

section_pat = r"(?ms)^(\[skills\]\n)(.*?)(?=^\[|\Z)"
m = re.search(section_pat, text)
line = f'skill_runner_path = "{runner_path}"'

if not m:
    text = text.rstrip() + f"\n\n[skills]\n{line}\n"
else:
    body = m.group(2)
    if re.search(r'(?m)^skill_runner_path\s*=\s*".*?"\s*$', body):
        body = re.sub(r'(?m)^skill_runner_path\s*=\s*".*?"\s*$', line, body, count=1)
    else:
        body = line + "\n" + body
    text = text[:m.start()] + m.group(1) + body + text[m.end():]

cfg_path.write_text(text, encoding="utf-8")
PY
  echo "Updated config: skills.skill_runner_path = \"$SKILL_RUNNER_REL\"" # zh: 已更新配置 skills.skill_runner_path。
else
  echo "Config file not found, skip path sync: $CONFIG_PATH" # zh: 配置文件不存在，跳过路径回写。
fi

echo "Build completed." # zh: 编译完成。
echo "Output directory: $OUT_DIR" # zh: 输出目录：$OUT_DIR
echo "Verified binaries: ${REQUIRED_BINS[*]}" # zh: 已校验二进制：${REQUIRED_BINS[*]}

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

if [[ -t 1 && -z "${RUSTCLAW_LOG_COLOR:-}" ]]; then
  export RUSTCLAW_LOG_COLOR=1
fi

PROFILE="${1:-${RUSTCLAW_START_PROFILE:-release}}"
case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-feishud.sh [release|debug]" # zh: 用法：./start-feishud.sh [release|debug]
    exit 1
    ;;
esac

CARGO_PROFILE_FLAG=()
if [[ "$PROFILE" == "release" ]]; then
  CARGO_PROFILE_FLAG=(--release)
fi

# Config path: same as feishud default, explicit for scripts
export FEISHU_CONFIG_PATH="${FEISHU_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/feishu.toml}"

python3 - <<'PY'
import sys
import tomllib
from pathlib import Path

path = Path("configs/channels/feishu.toml")
if not path.exists():
    print("configs/channels/feishu.toml not found, skip starting feishud.")  # zh: 配置文件不存在，跳过启动 feishud。
    raise SystemExit(2)
cfg = tomllib.loads(path.read_text(encoding="utf-8"))
feishu = cfg.get("feishu", {}) or {}
if not bool(feishu.get("enabled", False)):
    print("feishu.enabled=false, skip starting feishud.")  # zh: feishu.enabled=false，跳过启动 feishud。
    raise SystemExit(2)
print("Feishu preflight passed.")  # zh: Feishu 预检通过。
PY
code=$?
if [[ "$code" == "2" ]]; then
  exit 0
elif [[ "$code" != "0" ]]; then
  exit "$code"
fi

if pgrep -f 'target/(debug|release)/feishud|cargo run -p feishud' >/dev/null 2>&1; then
  echo "Detected feishud already running on this host. Stop old instance first." # zh: 检测到本机已有 feishud 在运行，请先停止旧实例。
  exit 1
fi

exec cargo run "${CARGO_PROFILE_FLAG[@]}" -p feishud

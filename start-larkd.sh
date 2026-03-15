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
    echo "Usage: ./start-larkd.sh [release|debug]"
    exit 1
    ;;
esac

CARGO_PROFILE_FLAG=()
if [[ "$PROFILE" == "release" ]]; then
  CARGO_PROFILE_FLAG=(--release)
fi

# Config path: Lark international, separate from feishu.toml
export LARK_CONFIG_PATH="${LARK_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/lark.toml}"

if python3 - <<'PY'
import sys
import tomllib
from pathlib import Path

path = Path("configs/channels/lark.toml")
if not path.exists():
    print("configs/channels/lark.toml not found, skip starting larkd.")
    raise SystemExit(2)
cfg = tomllib.loads(path.read_text(encoding="utf-8"))
lark = cfg.get("lark", {}) or {}
if not bool(lark.get("enabled", False)):
    print("lark.enabled=false, skip starting larkd.")
    raise SystemExit(2)
print("Lark (international) preflight passed.")
PY
then
  :
else
  code=$?
  if [[ "$code" == "2" ]]; then
    exit 0
  fi
  exit "$code"
fi

if pgrep -f 'target/(debug|release)/larkd|cargo run -p larkd' >/dev/null 2>&1; then
  echo "Detected larkd already running on this host. Stop old instance first."
  exit 1
fi

exec cargo run "${CARGO_PROFILE_FLAG[@]}" -p larkd

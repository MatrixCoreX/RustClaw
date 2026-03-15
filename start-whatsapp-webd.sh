#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

# Enable colored log tags on interactive terminals unless overridden.
if [[ -t 1 && -z "${RUSTCLAW_LOG_COLOR:-}" ]]; then
  export RUSTCLAW_LOG_COLOR=1
fi

PROFILE="${1:-${RUSTCLAW_START_PROFILE:-release}}"
case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-whatsapp-webd.sh [release|debug]" # zh: 用法：./start-whatsapp-webd.sh [release|debug]
    exit 1
    ;;
esac

CARGO_PROFILE_FLAG=()
if [[ "$PROFILE" == "release" ]]; then
  CARGO_PROFILE_FLAG=(--release)
fi

if pgrep -f 'target/(debug|release)/whatsapp_webd|cargo run -p whatsapp_webd' >/dev/null 2>&1; then
  echo "Detected whatsapp_webd already running on this host. Stop old instance first." # zh: 检测到本机已有 whatsapp_webd 在运行，请先停止旧实例。
  exit 1
fi

if python3 - <<'PY'
import tomllib
from pathlib import Path

cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
for name in ("whatsapp.toml", "whatsapp-web.toml"):
    extra = Path("configs/channels") / name
    if extra.exists():
        cfg.update(tomllib.loads(extra.read_text(encoding="utf-8")))
enabled = bool(cfg.get("whatsapp_web", {}).get("enabled", False))
if not enabled:
    print("whatsapp_web.enabled=false, skip starting whatsapp_webd.")  # zh: whatsapp_web.enabled=false，跳过启动。
    raise SystemExit(2)
print("whatsapp_webd preflight passed.")
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

exec cargo run "${CARGO_PROFILE_FLAG[@]}" -p whatsapp_webd

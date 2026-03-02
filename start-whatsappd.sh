#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

PROFILE="${1:-${RUSTCLAW_START_PROFILE:-debug}}"
case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-whatsappd.sh [release|debug]" # zh: 用法：./start-whatsappd.sh [release|debug]
    exit 1
    ;;
esac

CARGO_PROFILE_FLAG=()
if [[ "$PROFILE" == "release" ]]; then
  CARGO_PROFILE_FLAG=(--release)
fi

if pgrep -f 'target/(debug|release)/whatsappd|cargo run -p whatsappd' >/dev/null 2>&1; then
  echo "Detected whatsappd already running on this host. Stop old instance first." # zh: 检测到本机已有 whatsappd 在运行，请先停止旧实例。
  exit 1
fi

python3 - <<'PY'
import sys
import tomllib
from pathlib import Path

cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
extra = Path("configs/channels/whatsapp.toml")
if extra.exists():
    cfg.update(tomllib.loads(extra.read_text(encoding="utf-8")))
wa = cfg.get("whatsapp", {})
enabled = bool(wa.get("enabled", False))
if not enabled:
    print("whatsapp.enabled=false, skip starting whatsappd.")  # zh: whatsapp.enabled=false，跳过启动。
    raise SystemExit(2)

required = {
    "access_token": str(wa.get("access_token", "") or "").strip(),
    "app_secret": str(wa.get("app_secret", "") or "").strip(),
    "verify_token": str(wa.get("verify_token", "") or "").strip(),
    "phone_number_id": str(wa.get("phone_number_id", "") or "").strip(),
}

for k, v in required.items():
    if not v or v.startswith("REPLACE_ME"):
        print(f"whatsapp.{k} is not configured; cannot start whatsappd.")  # zh: 配置缺失，无法启动 whatsappd。
        raise SystemExit(1)

print("WhatsApp preflight passed.")
PY
code=$?
if [[ "$code" == "2" ]]; then
  exit 0
elif [[ "$code" != "0" ]]; then
  exit "$code"
fi

exec cargo run "${CARGO_PROFILE_FLAG[@]}" -p whatsappd

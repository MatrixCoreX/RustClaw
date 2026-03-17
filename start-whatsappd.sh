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
    echo "Usage: ./start-whatsappd.sh [release|debug]" # zh: 用法：./start-whatsappd.sh [release|debug]
    exit 1
    ;;
esac

BIN_NAME="whatsappd"
BIN_PATH="$SCRIPT_DIR/target/$PROFILE/$BIN_NAME"
if [[ ! -x "$BIN_PATH" ]]; then
  echo "Binary missing: $BIN_PATH"
  echo "Copy built binary to target/$PROFILE/ or run: cargo build -p $BIN_NAME ${PROFILE:+--release}"
  exit 1
fi

if pgrep -f 'target/(debug|release)/whatsappd|cargo run -p whatsappd' >/dev/null 2>&1; then
  echo "Detected whatsappd already running on this host. Stop old instance first." # zh: 检测到本机已有 whatsappd 在运行，请先停止旧实例。
  exit 1
fi

if python3 - <<'PY'
import sys
import tomllib
from pathlib import Path

cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
for name in ("whatsapp.toml", "whatsapp-cloud.toml"):
    extra = Path("configs/channels") / name
    if extra.exists():
        cfg.update(tomllib.loads(extra.read_text(encoding="utf-8")))
wa_legacy = cfg.get("whatsapp", {}) or {}
wa_cloud = cfg.get("whatsapp_cloud", {}) or {}
enabled = bool(wa_cloud.get("enabled", False) or wa_legacy.get("enabled", False))
if not enabled:
    print("whatsapp_cloud.enabled=false and whatsapp.enabled=false, skip starting whatsappd.")  # zh: whatsapp_cloud.enabled=false 且 whatsapp.enabled=false，跳过启动。
    raise SystemExit(2)

required = {
    "access_token": str((wa_cloud.get("access_token") or wa_legacy.get("access_token") or "")).strip(),
    "app_secret": str((wa_cloud.get("app_secret") or wa_legacy.get("app_secret") or "")).strip(),
    "verify_token": str((wa_cloud.get("verify_token") or wa_legacy.get("verify_token") or "")).strip(),
    "phone_number_id": str((wa_cloud.get("phone_number_id") or wa_legacy.get("phone_number_id") or "")).strip(),
}

for k, v in required.items():
    if not v or v.startswith("REPLACE_ME"):
        print(f"whatsapp_cloud.{k}/whatsapp.{k} is not configured; cannot start whatsappd.")  # zh: 配置缺失，无法启动 whatsappd。
        raise SystemExit(1)

print("WhatsApp preflight passed.")
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

exec "$BIN_PATH"

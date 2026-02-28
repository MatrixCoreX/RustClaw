#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

# Preflight 1: avoid local duplicate polling workers.
if pgrep -f 'target/debug/telegramd|cargo run -p telegramd' >/dev/null 2>&1; then
  echo "Detected telegramd already running on this host. Stop old instance first to avoid polling conflicts." # zh: 检测到本机已有 telegramd 在运行。请先停止旧实例，避免轮询冲突。
  echo "You can run: pkill -f 'target/debug/telegramd|cargo run -p telegramd'" # zh: 可执行: pkill -f 'target/debug/telegramd|cargo run -p telegramd'
  exit 1
fi

# Preflight 2: detect Telegram-side conflict before startup.
python3 - <<'PY'
import json
import tomllib
import urllib.parse
import urllib.request
from pathlib import Path
import sys

cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
token = str(cfg.get("telegram", {}).get("bot_token", "") or "").strip()
if not token or token.startswith("REPLACE_ME"):
    print("bot_token is not configured; cannot start telegramd.")  # zh: bot_token 未配置，无法启动 telegramd。
    sys.exit(1)

base = f"https://api.telegram.org/bot{token}"

def post(method: str, data: dict):
    body = urllib.parse.urlencode(data).encode("utf-8")
    req = urllib.request.Request(f"{base}/{method}", data=body)
    with urllib.request.urlopen(req, timeout=12) as resp:
        return json.loads(resp.read().decode("utf-8"))

try:
    webhook = post("getWebhookInfo", {})
except Exception as e:
    print(f"Preflight failed: cannot request Telegram getWebhookInfo: {e}")  # zh: 预检失败：无法请求 Telegram getWebhookInfo: {e}
    sys.exit(1)

if webhook.get("ok") and isinstance(webhook.get("result"), dict):
    url = str(webhook["result"].get("url", "") or "")
    if url:
        print("Detected active webhook for current bot; this conflicts with getUpdates polling.")  # zh: 检测到当前 bot 已配置 webhook，会与 getUpdates 轮询冲突。
        print("Please clear webhook first or switch to webhook mode.")  # zh: 请先清理 webhook 或改为 webhook 模式。
        print(f"Current webhook URL: {url}")  # zh: 当前 webhook URL: {url}
        sys.exit(1)

try:
    check = post("getUpdates", {"timeout": 0, "limit": 1})
except Exception as e:
    print(f"Preflight failed: cannot request Telegram getUpdates: {e}")  # zh: 预检失败：无法请求 Telegram getUpdates: {e}
    sys.exit(1)

if not check.get("ok", False):
    desc = str(check.get("description", "unknown error"))
    low = desc.lower()
    if "terminated by other getupdates" in low:
        print("Detected another instance using getUpdates (possibly on another host/terminal).")  # zh: 检测到另一个实例正在使用 getUpdates（可能在其他机器/终端）。
        print("Stop other instances first, then start local telegramd.")  # zh: 请先停止其它实例，再启动本地 telegramd。
        sys.exit(1)
    if "can't use getupdates method while webhook is active" in low:
        print("Detected webhook/polling conflict. Please remove webhook first.")  # zh: 检测到 webhook 与轮询冲突，请先取消 webhook。
        sys.exit(1)
    print(f"Telegram preflight failed: {desc}")  # zh: Telegram 预检失败: {desc}
    sys.exit(1)

print("Telegram preflight passed.")  # zh: Telegram 预检通过。
PY

exec cargo run -p telegramd

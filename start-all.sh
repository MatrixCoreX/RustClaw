#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

LOG_DIR="$SCRIPT_DIR/logs"
PID_DIR="$SCRIPT_DIR/.pids"
mkdir -p "$LOG_DIR" "$PID_DIR"

# Optional args:
#   ./start-all.sh <vendor(openai|google|anthropic|grok)> [model_override] [release|debug]
PROVIDER_OVERRIDE="${1:-${RUSTCLAW_PROVIDER_OVERRIDE:-}}"
MODEL_OVERRIDE="${2:-${RUSTCLAW_MODEL_OVERRIDE:-}}"
PROFILE="${3:-${RUSTCLAW_START_PROFILE:-debug}}"
case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-all.sh <vendor> [model_override] [release|debug]" # zh: 用法：./start-all.sh <vendor> [model_override] [release|debug]
    exit 1
    ;;
esac
export RUSTCLAW_START_PROFILE="$PROFILE"

if [[ -n "$PROVIDER_OVERRIDE" ]]; then
  export RUSTCLAW_PROVIDER_OVERRIDE="$PROVIDER_OVERRIDE"
  echo "Using preset provider: $RUSTCLAW_PROVIDER_OVERRIDE" # zh: 使用预设 provider: $RUSTCLAW_PROVIDER_OVERRIDE
fi
if [[ -n "$MODEL_OVERRIDE" ]]; then
  export RUSTCLAW_MODEL_OVERRIDE="$MODEL_OVERRIDE"
  echo "Using model override: $RUSTCLAW_MODEL_OVERRIDE" # zh: 使用模型覆盖: $RUSTCLAW_MODEL_OVERRIDE
fi

# Batch start should be non-interactive.
export RUSTCLAW_MODEL_SELECT=0

choose_channel_mode() {
  if [[ ! -t 0 || ! -t 1 ]]; then
    echo "Non-interactive terminal detected; keep current channel enable flags." # zh: 检测到非交互终端，保持当前渠道开关配置不变。
    return 0
  fi

  local primary=""
  local wa_mode=""
  echo "Step 1/3: Select startup channel" # zh: 第 1/3 步：选择启动渠道
  echo "  1) telegram"
  echo "  2) whatsapp"
  while true; do
    read -r -p "> " choice
    case "$choice" in
      1) primary="telegram"; break ;;
      2) primary="whatsapp"; break ;;
      *) echo "Invalid input, choose 1 or 2." ;; # zh: 输入无效，请输入 1 或 2。
    esac
  done

  if [[ "$primary" == "whatsapp" ]]; then
    echo "Select WhatsApp mode:" # zh: 请选择 WhatsApp 模式：
    echo "  1) whatsapp_web"
    echo "  2) whatsapp_cloud"
    while true; do
      read -r -p "> " choice
      case "$choice" in
        1) wa_mode="web"; break ;;
        2) wa_mode="cloud"; break ;;
        *) echo "Invalid input, choose 1 or 2." ;; # zh: 输入无效，请输入 1 或 2。
      esac
    done
  fi

  export RUSTCLAW_START_PRIMARY="$primary"
  export RUSTCLAW_START_WA_MODE="$wa_mode"
  python3 - <<'PY'
import os
from pathlib import Path
import tomllib

primary = os.environ.get("RUSTCLAW_START_PRIMARY", "").strip()
wa_mode = os.environ.get("RUSTCLAW_START_WA_MODE", "").strip()

def set_flag(text: str, section: str, key: str, value: bool) -> str:
    lines = text.splitlines()
    sec = f"[{section}]"
    sec_idx = None
    for i, line in enumerate(lines):
        if line.strip() == sec:
            sec_idx = i
            break
    if sec_idx is None:
        if lines and lines[-1].strip():
            lines.append("")
        lines.append(sec)
        lines.append(f"{key} = {'true' if value else 'false'}")
        return "\n".join(lines) + "\n"
    end_idx = len(lines)
    for j in range(sec_idx + 1, len(lines)):
        s = lines[j].strip()
        if s.startswith("[") and s.endswith("]"):
            end_idx = j
            break
    for j in range(sec_idx + 1, end_idx):
        if lines[j].lstrip().startswith(f"{key}"):
            lines[j] = f"{key} = {'true' if value else 'false'}"
            return "\n".join(lines) + "\n"
    lines.insert(end_idx, f"{key} = {'true' if value else 'false'}")
    return "\n".join(lines) + "\n"

tg_path = Path("configs/channels/telegram.toml")
wa_path = Path("configs/channels/whatsapp.toml")
tg_text = tg_path.read_text(encoding="utf-8") if tg_path.exists() else ""
wa_text = wa_path.read_text(encoding="utf-8") if wa_path.exists() else ""

enable_tg = primary == "telegram"
enable_wa_web = primary == "whatsapp" and wa_mode == "web"
enable_wa_cloud = primary == "whatsapp" and wa_mode == "cloud"

tg_text = set_flag(tg_text, "telegram_bot", "enabled", enable_tg)
wa_text = set_flag(wa_text, "whatsapp_web", "enabled", enable_wa_web)
wa_text = set_flag(wa_text, "whatsapp", "enabled", enable_wa_cloud)
wa_text = set_flag(wa_text, "whatsapp_cloud", "enabled", enable_wa_cloud)

tg_path.parent.mkdir(parents=True, exist_ok=True)
wa_path.parent.mkdir(parents=True, exist_ok=True)
tg_path.write_text(tg_text, encoding="utf-8")
wa_path.write_text(wa_text, encoding="utf-8")
print(f"Applied startup channel selection: primary={primary}, whatsapp_mode={wa_mode or '-'}")
PY
}

choose_channel_mode

# Self-contained startup with selectable profile (default debug for test directory).
CLAWD_BIN="$SCRIPT_DIR/target/$PROFILE/clawd"
TELEGRAMD_BIN="$SCRIPT_DIR/target/$PROFILE/telegramd"
WHATSAPPD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsappd"
WHATSAPP_WEBD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsapp_webd"

echo "Step 2/3: Build check" # zh: 第 2/3 步：检查编译产物
if [[ ! -x "$CLAWD_BIN" || ! -x "$TELEGRAMD_BIN" ]]; then
  echo "Prebuilt binaries missing for profile=$PROFILE, starting foreground build..." # zh: 缺少预编译二进制，先前台编译。
  if [[ -f "$SCRIPT_DIR/Cargo.toml" ]]; then
    cargo build --workspace --release
  else
    echo "Cargo.toml not found in runtime package; cannot compile. Please use a package containing release binaries." # zh: 运行包内未找到 Cargo.toml，无法编译。请使用包含 release 二进制的运行包。
    exit 1
  fi
  echo "Build finished, switching to background startup." # zh: 编译完成，切换为后台启动。
else
  echo "Detected prebuilt binaries under target/$PROFILE; starting directly in background." # zh: 已检测到预编译二进制，直接后台启动。
fi

# Ensure skill-runner binary exists for run_skill tasks.
SKILL_RUNNER_ABS="$SCRIPT_DIR/target/$PROFILE/skill-runner"

if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  echo "skill-runner missing, trying to build: $SKILL_RUNNER_ABS" # zh: 未找到 skill-runner，尝试自动编译。
  if [[ -f "$SCRIPT_DIR/Cargo.toml" ]]; then
    if [[ "$PROFILE" == "release" ]]; then
      cargo build -p skill-runner --release
    else
      cargo build -p skill-runner
    fi
  else
    echo "Cargo.toml not found in runtime package; cannot compile skill-runner." # zh: 运行包内未找到 Cargo.toml，无法编译 skill-runner。
    exit 1
  fi
fi

if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  echo "skill-runner still missing after build: $SKILL_RUNNER_ABS" # zh: 自动编译后仍未找到 skill-runner。
  exit 1
fi

start_clawd() {
  if pgrep -f 'target/(debug|release)/clawd|cargo run -p clawd' >/dev/null 2>&1; then
    echo "clawd is already running, skipping startup." # zh: clawd 已在运行，跳过启动。
    return 0
  fi
  nohup "$CLAWD_BIN" >"$LOG_DIR/clawd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/clawd.pid"
  echo "Starting clawd binary, PID=$pid, log: $LOG_DIR/clawd.log" # zh: clawd 二进制启动中，PID=$pid, 日志: $LOG_DIR/clawd.log
}

start_telegramd() {
  local tg_enabled
  tg_enabled="$(
python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
tg_cfg = Path("configs/channels/telegram.toml")
if tg_cfg.exists():
    cfg.update(tomllib.loads(tg_cfg.read_text(encoding="utf-8")))
print("1" if bool(cfg.get("telegram_bot", {}).get("enabled", True)) else "0")
PY
  )"
  if [[ "$tg_enabled" != "1" ]]; then
    echo "telegram_bot.enabled=false, skipping telegramd startup." # zh: telegram_bot.enabled=false，跳过 telegramd 启动。
    return 0
  fi
  if pgrep -f 'target/(debug|release)/telegramd|cargo run -p telegramd' >/dev/null 2>&1; then
    echo "telegramd is already running, skipping startup." # zh: telegramd 已在运行，跳过启动。
    return 0
  fi
  nohup "$TELEGRAMD_BIN" >"$LOG_DIR/telegramd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/telegramd.pid"
  echo "Starting telegramd binary, PID=$pid, log: $LOG_DIR/telegramd.log" # zh: telegramd 二进制启动中，PID=$pid, 日志: $LOG_DIR/telegramd.log
}

start_whatsapp_webd() {
  local wa_web_enabled
  wa_web_enabled="$(
python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
wa_cfg = Path("configs/channels/whatsapp.toml")
if wa_cfg.exists():
    cfg.update(tomllib.loads(wa_cfg.read_text(encoding="utf-8")))
print("1" if bool(cfg.get("whatsapp_web", {}).get("enabled", False)) else "0")
PY
  )"
  if [[ "$wa_web_enabled" != "1" ]]; then
    echo "whatsapp_web.enabled=false, skipping whatsapp_webd startup." # zh: whatsapp_web.enabled=false，跳过 whatsapp_webd 启动。
    return 0
  fi
  if [[ ! -x "$WHATSAPP_WEBD_BIN" ]]; then
    echo "Binary not found or not executable: $WHATSAPP_WEBD_BIN" # zh: 二进制不存在或不可执行：$WHATSAPP_WEBD_BIN
    return 1
  fi
  if pgrep -f 'target/(debug|release)/whatsapp_webd|cargo run -p whatsapp_webd' >/dev/null 2>&1; then
    echo "whatsapp_webd is already running, skipping startup." # zh: whatsapp_webd 已在运行，跳过启动。
    return 0
  fi
  nohup "$WHATSAPP_WEBD_BIN" >"$LOG_DIR/whatsapp_webd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/whatsapp_webd.pid"
  echo "Starting whatsapp_webd, PID=$pid, log: $LOG_DIR/whatsapp_webd.log" # zh: whatsapp_webd 启动中，PID=$pid, 日志: $LOG_DIR/whatsapp_webd.log
}

start_whatsappd() {
  local wa_enabled
  wa_enabled="$(
python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
wa_cfg = Path("configs/channels/whatsapp.toml")
if wa_cfg.exists():
    cfg.update(tomllib.loads(wa_cfg.read_text(encoding="utf-8")))
print("1" if bool(cfg.get("whatsapp", {}).get("enabled", False)) else "0")
PY
  )"
  if [[ "$wa_enabled" != "1" ]]; then
    echo "whatsapp.enabled=false, skipping whatsappd startup." # zh: whatsapp.enabled=false，跳过 whatsappd 启动。
    return 0
  fi
  if [[ ! -x "$WHATSAPPD_BIN" ]]; then
    echo "Binary not found or not executable: $WHATSAPPD_BIN" # zh: 二进制不存在或不可执行：$WHATSAPPD_BIN
    return 1
  fi
  if pgrep -f 'target/(debug|release)/whatsappd|cargo run -p whatsappd' >/dev/null 2>&1; then
    echo "whatsappd is already running, skipping startup." # zh: whatsappd 已在运行，跳过启动。
    return 0
  fi
  nohup "$WHATSAPPD_BIN" >"$LOG_DIR/whatsappd.log" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_DIR/whatsappd.pid"
  echo "Starting whatsappd binary, PID=$pid, log: $LOG_DIR/whatsappd.log" # zh: whatsappd 二进制启动中，PID=$pid, 日志: $LOG_DIR/whatsappd.log
}

echo "Step 3/3: Start services" # zh: 第 3/3 步：启动服务
start_clawd
start_telegramd
start_whatsapp_webd
start_whatsappd
echo "One-click startup command executed (profile: $PROFILE)." # zh: 一键启动命令已执行（profile: $PROFILE）。

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

print_rustclaw_banner() {
  cat <<'EOF'
######################################################################################
                __!---===[[[  @@@@  ]]]===---!__
           _!@#￥%……&*()_/      <<<<>>>>      \_!@#￥%……&*()_
        _!@#￥%……&*()_/      <<<<  @@  >>>>      \_!@#￥%……&*()_
      !@#￥%……&*()_+       <<<<  @@@@@@  >>>>       +_)(*&……%￥#@!
      !@#￥%……&*()_+=======<<<<==@@@==@@@==>>>>=======+_)(*&……%￥#@!
      !@#￥%……&*()_+       >>>>  @@@@@@  <<<<       +_)(*&……%￥#@!
        !_)(*&……%￥#@!\      >>>>  @@  <<<<      /!@#￥%……&*()_
           !@#￥%……&*()_\      >>>><<<<      /_)(*&……%￥#@!
                --===!!![[[  @@@@  ]]]!!!===--

########   ##    ##   #######   ########      #######   ##         ######    ##      ##
##    ##   ##    ##   ##           ##         ##        ##        ##    ##   ##  ##  ##
########   ##    ##   #######      ##         ##        ##        ########   ##  ##  ##
##   ##    ##    ##        ##      ##         ##        ##        ##    ##   ##  ##  ##
##    ##    ######    #######      ##         #######   ########  ##    ##    ###  ###

########################################################################################
EOF
}

print_rustclaw_banner

LOG_DIR="$SCRIPT_DIR/logs"
PID_DIR="$SCRIPT_DIR/.pids"
mkdir -p "$LOG_DIR" "$PID_DIR"

# Stop any already running RustClaw processes before starting.
if [[ -f "$SCRIPT_DIR/stop-rustclaw.sh" ]]; then
  "$SCRIPT_DIR/stop-rustclaw.sh" || true
fi

# Optional args:
#   ./start-all.sh <vendor(openai|google|anthropic|grok|qwen|custom)> [model_override] [release|debug] [channels]
# channels:
#   telegram | whatsapp_web | both | whatsapp_cloud | all
PROVIDER_OVERRIDE="${1:-${RUSTCLAW_PROVIDER_OVERRIDE:-}}"
MODEL_OVERRIDE="${2:-${RUSTCLAW_MODEL_OVERRIDE:-}}"
PROFILE="${3:-${RUSTCLAW_START_PROFILE:-debug}}"
CHANNELS_ARG="${4:-${RUSTCLAW_START_CHANNELS:-}}"
QUICK_START_ARG="${5:-${RUSTCLAW_QUICK_START:-0}}"
SKIP_SETUP="${RUSTCLAW_SKIP_SETUP:-0}"
ENABLE_UI="${RUSTCLAW_ENABLE_UI:-0}"
UI_FORCE_REBUILD="${RUSTCLAW_UI_FORCE_REBUILD:-0}"
QUICK_START=0
case "$(echo "$QUICK_START_ARG" | tr '[:upper:]' '[:lower:]')" in
  1|true|yes|y|quick|fast)
    QUICK_START=1
    ;;
esac
case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-all.sh <vendor> [model_override] [release|debug] [channels]" # zh: 用法：./start-all.sh <vendor> [model_override] [release|debug] [channels]
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

run_embedded_setup() {
  local do_interactive_setup=1
  if [[ "$SKIP_SETUP" == "1" ]]; then
    do_interactive_setup=0
    echo "Skip interactive setup by RUSTCLAW_SKIP_SETUP=1."
  fi

  local config_path="$SCRIPT_DIR/configs/config.toml"
  if [[ ! -f "$config_path" ]]; then
    echo "Config file not found: $config_path"
    exit 1
  fi

  if [[ "$do_interactive_setup" == "1" && -t 0 && -t 1 ]]; then
    python3 - <<'PY'
import getpass
import json
import os
import re
import sys
import tomllib
import urllib.parse
import urllib.request
from pathlib import Path

cfg_path = Path("configs/config.toml")
text = cfg_path.read_text(encoding="utf-8")
cfg = tomllib.loads(text)
telegram_cfg_path = Path("configs/channels/telegram.toml")
telegram_text = telegram_cfg_path.read_text(encoding="utf-8") if telegram_cfg_path.exists() else ""
telegram_cfg = tomllib.loads(telegram_text) if telegram_text else {}
changed = False
telegram_changed = False
force = str(os.environ.get("RUSTCLAW_SETUP_FORCE", "0")).strip().lower() not in ("0", "false", "no")

try:
    tty_in = open("/dev/tty", "r", encoding="utf-8", errors="ignore")
    tty_out = open("/dev/tty", "w", encoding="utf-8", errors="ignore")
except OSError:
    raise SystemExit(0)

def ask(prompt: str) -> str:
    tty_out.write(prompt)
    tty_out.flush()
    line = tty_in.readline()
    if line == "":
        raise SystemExit(0)
    return line.rstrip("\n")

def is_empty_or_placeholder(v: str) -> bool:
    return (not v) or v.startswith("REPLACE_ME")

def set_key_in_section(src: str, section: str, key: str, value_literal: str) -> str:
    sec_header = f"[{section}]"
    sec_pat = rf"(?ms)^({re.escape(sec_header)}\n)(.*?)(?=^\[|\Z)"
    m = re.search(sec_pat, src)
    key_line_pat = rf"(?m)^{re.escape(key)}\s*=\s*.*$"
    new_line = f"{key} = {value_literal}"
    if m:
        body = m.group(2)
        if re.search(key_line_pat, body):
            body = re.sub(key_line_pat, new_line, body, count=1)
        else:
            body = new_line + "\n" + body
        return src[:m.start()] + m.group(1) + body + src[m.end():]
    insert = f"\n{sec_header}\n{new_line}\n"
    return src.rstrip() + insert + "\n"

def quote_toml_string(s: str) -> str:
    escaped = s.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'

def get_nested(d, *keys, default=None):
    cur = d
    for k in keys:
        if not isinstance(cur, dict) or k not in cur:
            return default
        cur = cur[k]
    return cur

def to_int_or_none(v):
    try:
        if isinstance(v, bool):
            return None
        return int(v)
    except (TypeError, ValueError):
        return None

telegram_enabled = str(os.environ.get("RUSTCLAW_ENABLE_TG", "0")).strip() == "1"
telegram_token = str(get_nested(telegram_cfg, "telegram", "bot_token", default="") or get_nested(cfg, "telegram", "bot_token", default="") or "")
telegram_bot_token = str(get_nested(telegram_cfg, "telegram_bot", "bot_token", default="") or get_nested(cfg, "telegram_bot", "bot_token", default="") or "")
admins = get_nested(telegram_cfg, "telegram", "admins", default=get_nested(cfg, "telegram", "admins", default=[]))
selected_vendor = ""
selected_model = ""

if force or is_empty_or_placeholder(telegram_token) or (telegram_enabled and is_empty_or_placeholder(telegram_bot_token) and is_empty_or_placeholder(telegram_token)):
    token_prompt = "Enter Telegram bot_token: " if telegram_enabled else "Enter Telegram bot_token (empty to skip): "
    token = ask(token_prompt).strip()
    if token:
        while is_empty_or_placeholder(token):
            retry_prompt = "bot_token cannot be empty/placeholder; enter again: " if telegram_enabled else "bot_token cannot be placeholder; enter again (empty to skip): "
            token = ask(retry_prompt).strip()
            if not token:
                break
        if token:
            telegram_text = set_key_in_section(telegram_text, "telegram", "bot_token", quote_toml_string(token))
            telegram_text = set_key_in_section(telegram_text, "telegram_bot", "bot_token", quote_toml_string(token))
            telegram_changed = True
    elif telegram_enabled:
        # telegram is enabled; token is required
        while not token:
            token = ask("Telegram enabled: bot_token is required, please enter: ").strip()
            if token and not is_empty_or_placeholder(token):
                telegram_text = set_key_in_section(telegram_text, "telegram", "bot_token", quote_toml_string(token))
                telegram_text = set_key_in_section(telegram_text, "telegram_bot", "bot_token", quote_toml_string(token))
                telegram_changed = True
                break

if telegram_enabled and not is_empty_or_placeholder(telegram_token) and is_empty_or_placeholder(telegram_bot_token):
    telegram_text = set_key_in_section(telegram_text, "telegram_bot", "bot_token", quote_toml_string(telegram_token))
    telegram_changed = True

if force or (not isinstance(admins, list) or len(admins) == 0):
    admin_raw = ask("Enter admin Telegram user_id (empty to skip): ").strip()
    if admin_raw:
        while not re.fullmatch(r"-?\d+", admin_raw):
            admin_raw = ask("Invalid format, enter numeric user_id (empty to skip): ").strip()
            if not admin_raw:
                break
        if admin_raw:
            telegram_text = set_key_in_section(telegram_text, "telegram", "admins", f"[{admin_raw}]")
            telegram_changed = True

vendors = ["openai", "google", "anthropic", "grok", "qwen", "custom"]
available_vendors = [v for v in vendors if isinstance(get_nested(cfg, "llm", v, default=None), dict)]
if not available_vendors:
    raise SystemExit(0)

print("Select model vendor:")  # always choose on startup
for i, v in enumerate(available_vendors, start=1):
    print(f"  {i}) {v}")
choice = ask("> ").strip()
while not (choice.isdigit() and 1 <= int(choice) <= len(available_vendors)):
    choice = ask("Invalid input, enter option number: ").strip()
selected_vendor = available_vendors[int(choice) - 1]
text = set_key_in_section(text, "llm", "selected_vendor", quote_toml_string(selected_vendor))
changed = True

vendor_cfg = get_nested(cfg, "llm", selected_vendor, default={}) or {}
vendor_models = vendor_cfg.get("models", [])
if not isinstance(vendor_models, list):
    vendor_models = []
vendor_models = [str(m) for m in vendor_models if isinstance(m, str) and m]
default_vendor_model = str(vendor_cfg.get("model", "") or "")
if default_vendor_model and default_vendor_model not in vendor_models:
    vendor_models.insert(0, default_vendor_model)

selected_api_key = str(get_nested(cfg, "llm", selected_vendor, "api_key", default="") or "")
if force or is_empty_or_placeholder(selected_api_key):
    key = getpass.getpass(f"Enter {selected_vendor} api_key (empty to skip remote fetch): ").strip()
    if key:
        while is_empty_or_placeholder(key):
            key = getpass.getpass("api_key cannot be placeholder; enter again (empty to skip remote fetch): ").strip()
            if not key:
                break
        if key:
            text = set_key_in_section(text, f"llm.{selected_vendor}", "api_key", quote_toml_string(key))
            changed = True
            selected_api_key = key

def uniq_keep_order(items):
    out = []
    seen = set()
    for it in items:
        s = str(it).strip()
        if not s or s in seen:
            continue
        seen.add(s)
        out.append(s)
    return out

def fetch_remote_models(vendor: str, base_url: str, api_key: str):
    if not api_key:
        return [], "missing_api_key"
    base = (base_url or "").rstrip("/")
    if not base:
        return [], "missing_base_url"
    headers = {"Accept": "application/json"}
    if vendor == "google":
        url = f"{base}/models?key={urllib.parse.quote(api_key, safe='')}"
    elif vendor == "anthropic":
        url = f"{base}/models"
        headers["x-api-key"] = api_key
        headers["anthropic-version"] = "2023-06-01"
    else:
        url = f"{base}/models"
        headers["Authorization"] = f"Bearer {api_key}"
    req = urllib.request.Request(url, headers=headers, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            body = resp.read().decode("utf-8", errors="ignore")
        data = json.loads(body)
    except Exception as e:
        return [], str(e)

    models = []
    if isinstance(data, dict):
        rows = data.get("data")
        if isinstance(rows, list):
            for item in rows:
                if isinstance(item, dict):
                    models.append(item.get("id") or item.get("name") or "")
        rows2 = data.get("models")
        if isinstance(rows2, list):
            for item in rows2:
                if isinstance(item, dict):
                    name = item.get("name") or item.get("id") or ""
                    if isinstance(name, str) and name.startswith("models/"):
                        name = name.split("/", 1)[1]
                    models.append(name)
    return uniq_keep_order(models), ""

base_url = str(vendor_cfg.get("base_url", "") or "")
remote_models, remote_err = fetch_remote_models(selected_vendor, base_url, selected_api_key)
if remote_models:
    model_options = remote_models
    print(f"Fetched {len(model_options)} models from {selected_vendor} API.")
else:
    model_options = uniq_keep_order(vendor_models)
    if remote_err:
        print(f"Model fetch failed ({selected_vendor}), fallback to preset list: {remote_err}")

if model_options:
    print(f"Select model for {selected_vendor}:")
    for i, m in enumerate(model_options, start=1):
        print(f"  {i}) {m}")
    print("  0) custom input")
    choice = ask("> ").strip()
    while True:
        c = choice.strip().lower()
        if c in ("0", "m", "manual", "custom"):
            selected_model = ask(f"Enter model name for {selected_vendor}: ").strip()
            while not selected_model:
                selected_model = ask("Model name cannot be empty, enter again: ").strip()
            break
        if c.isdigit() and 1 <= int(c) <= len(model_options):
            selected_model = model_options[int(c) - 1]
            break
        choice = ask("Invalid input, enter option number (or 0 for custom): ").strip()
else:
    selected_model = ask(f"Enter model name for {selected_vendor}: ").strip()
    while not selected_model:
        selected_model = ask("Model name cannot be empty, enter again: ").strip()
text = set_key_in_section(text, "llm", "selected_model", quote_toml_string(selected_model))
changed = True

tools_cmd_timeout = to_int_or_none(get_nested(cfg, "tools", "cmd_timeout_seconds", default=None))
if tools_cmd_timeout is None or tools_cmd_timeout <= 0:
    raw = ask("Enter tools.cmd_timeout_seconds (default 10): ").strip()
    timeout_seconds = 10 if not raw else int(raw) if raw.isdigit() and int(raw) > 0 else 10
    text = set_key_in_section(text, "tools", "cmd_timeout_seconds", str(timeout_seconds))
    changed = True

tools_max_cmd_length = to_int_or_none(get_nested(cfg, "tools", "max_cmd_length", default=None))
if tools_max_cmd_length is None or tools_max_cmd_length <= 0:
    raw = ask("Enter tools.max_cmd_length (default 240): ").strip()
    max_cmd_length = 240 if not raw else int(raw) if raw.isdigit() and int(raw) > 0 else 240
    text = set_key_in_section(text, "tools", "max_cmd_length", str(max_cmd_length))
    changed = True

if changed:
    cfg_path.write_text(text, encoding="utf-8")
if telegram_changed:
    telegram_cfg_path.parent.mkdir(parents=True, exist_ok=True)
    telegram_cfg_path.write_text(telegram_text, encoding="utf-8")
if changed:
    print("Configuration updated: configs/config.toml")
if telegram_changed:
    print("Configuration updated: configs/channels/telegram.toml")
PY
  else
    if [[ "$do_interactive_setup" == "0" ]]; then
      echo "Interactive setup prompts are disabled."
    else
      echo "Non-interactive terminal detected; skip interactive setup prompts."
    fi
  fi

  echo "Checking skill/runtime dependencies..."
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found. Please install Rust toolchain first."
    exit 1
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 not found."
    exit 1
  fi
  echo "Syncing skill docs (INTERFACE.md + prompts/skills/*.md)..."
  python3 "$SCRIPT_DIR/scripts/sync_skill_docs.py"

  CONFIG_META="$(
python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
for extra in ("configs/channels/telegram.toml", "configs/channels/whatsapp.toml"):
    p = Path(extra)
    if p.exists():
        cfg.update(tomllib.loads(p.read_text(encoding="utf-8")))
skills = cfg.get("skills", {}) if isinstance(cfg, dict) else {}
skills_list = skills.get("skills_list", [])
if not isinstance(skills_list, list):
    skills_list = []
skills_list = [str(v).strip() for v in skills_list if str(v).strip()]
wa_web_enabled = bool((cfg.get("whatsapp_web", {}) or {}).get("enabled", False))
x_cfg_path = Path("configs/x.toml")
xurl_bin = "xurl"
if x_cfg_path.exists():
    x_cfg = tomllib.loads(x_cfg_path.read_text(encoding="utf-8"))
    xurl_bin = str(x_cfg.get("xurl_bin", "xurl") or "xurl").strip() or "xurl"
print(f"SKILLS_LIST={','.join(skills_list)}")
print(f"WA_WEB_ENABLED={'1' if wa_web_enabled else '0'}")
print(f"XURL_BIN={xurl_bin}")
PY
)"

  local SKILLS_LIST=""
  local WA_WEB_ENABLED=""
  local XURL_BIN=""
  while IFS='=' read -r key value; do
    case "$key" in
      SKILLS_LIST) SKILLS_LIST="$value" ;;
      WA_WEB_ENABLED) WA_WEB_ENABLED="$value" ;;
      XURL_BIN) XURL_BIN="$value" ;;
    esac
  done <<< "$CONFIG_META"

  local profile_flag=()
  local target_dir="target/$PROFILE"
  if [[ "$PROFILE" == "release" ]]; then
    profile_flag=(--release)
  fi

  skill_bin_name() {
    case "$1" in
      x) echo "x-skill" ;;
      system_basic) echo "system-basic-skill" ;;
      http_basic) echo "http-basic-skill" ;;
      git_basic) echo "git-basic-skill" ;;
      install_module) echo "install-module-skill" ;;
      process_basic) echo "process-basic-skill" ;;
      package_manager) echo "package-manager-skill" ;;
      archive_basic) echo "archive-basic-skill" ;;
      db_basic) echo "db-basic-skill" ;;
      docker_basic) echo "docker-basic-skill" ;;
      fs_search) echo "fs-search-skill" ;;
      rss_fetch) echo "rss-fetch-skill" ;;
      image_vision) echo "image-vision-skill" ;;
      image_generate) echo "image-generate-skill" ;;
      image_edit) echo "image-edit-skill" ;;
      audio_transcribe) echo "audio-transcribe-skill" ;;
      audio_synthesize) echo "audio-synthesize-skill" ;;
      health_check) echo "health-check-skill" ;;
      log_analyze) echo "log-analyze-skill" ;;
      service_control) echo "service-control-skill" ;;
      config_guard) echo "config-guard-skill" ;;
      crypto) echo "crypto-skill" ;;
      *) return 1 ;;
    esac
  }

  if [[ -n "${SKILLS_LIST:-}" ]]; then
    IFS=',' read -r -a SKILLS_ARR <<< "$SKILLS_LIST"
    for skill in "${SKILLS_ARR[@]}"; do
      skill="$(echo "$skill" | xargs)"
      [[ -z "$skill" ]] && continue
      if ! bin_name="$(skill_bin_name "$skill")"; then
        echo "Skip unknown skill in skills_list: $skill"
        continue
      fi
      if [[ ! -x "$SCRIPT_DIR/$target_dir/$bin_name" ]]; then
        echo "Building missing skill binary: $bin_name"
        cargo build --bin "$bin_name" "${profile_flag[@]}"
      fi
    done
  fi

  if [[ ",${SKILLS_LIST:-}," == *",x,"* ]]; then
    echo "Checking X skill dependency (xurl)..."
    if ! command -v npm >/dev/null 2>&1; then
      echo "npm not found. Please install npm first."
      exit 1
    fi
    if ! command -v "${XURL_BIN:-xurl}" >/dev/null 2>&1; then
      echo "xurl binary not found (${XURL_BIN:-xurl}), installing @xdevplatform/xurl globally..."
      npm install -g @xdevplatform/xurl
    fi
  fi

  if [[ "${WA_WEB_ENABLED:-0}" == "1" ]]; then
    echo "Checking WhatsApp Web bridge dependencies..."
    if ! command -v node >/dev/null 2>&1; then
      echo "node not found. Please install Node.js 18+."
      exit 1
    fi
    if ! command -v npm >/dev/null 2>&1; then
      echo "npm not found. Please install npm."
      exit 1
    fi
    local bridge_dir="$SCRIPT_DIR/services/wa-web-bridge"
    if [[ -f "$bridge_dir/package.json" && ! -d "$bridge_dir/node_modules" ]]; then
      echo "Installing wa-web-bridge npm dependencies..."
      npm --prefix "$bridge_dir" install
    fi
  fi
}

apply_channel_flags() {
  local enable_tg="$1"
  local enable_wa_web="$2"
  local enable_wa_cloud="$3"
  export RUSTCLAW_ENABLE_TG="$enable_tg"
  export RUSTCLAW_ENABLE_WA_WEB="$enable_wa_web"
  export RUSTCLAW_ENABLE_WA_CLOUD="$enable_wa_cloud"

  python3 - <<'PY'
import os
from pathlib import Path

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

enable_tg = os.environ.get("RUSTCLAW_ENABLE_TG", "0") == "1"
enable_wa_web = os.environ.get("RUSTCLAW_ENABLE_WA_WEB", "0") == "1"
enable_wa_cloud = os.environ.get("RUSTCLAW_ENABLE_WA_CLOUD", "0") == "1"

tg_path = Path("configs/channels/telegram.toml")
wa_path = Path("configs/channels/whatsapp.toml")
tg_text = tg_path.read_text(encoding="utf-8") if tg_path.exists() else ""
wa_text = wa_path.read_text(encoding="utf-8") if wa_path.exists() else ""

tg_text = set_flag(tg_text, "telegram_bot", "enabled", enable_tg)
wa_text = set_flag(wa_text, "whatsapp_web", "enabled", enable_wa_web)
wa_text = set_flag(wa_text, "whatsapp", "enabled", enable_wa_cloud)
wa_text = set_flag(wa_text, "whatsapp_cloud", "enabled", enable_wa_cloud)

tg_path.parent.mkdir(parents=True, exist_ok=True)
wa_path.parent.mkdir(parents=True, exist_ok=True)
tg_path.write_text(tg_text, encoding="utf-8")
wa_path.write_text(wa_text, encoding="utf-8")
print(
    "Applied channel flags: "
    f"telegram={'on' if enable_tg else 'off'}, "
    f"whatsapp_web={'on' if enable_wa_web else 'off'}, "
    f"whatsapp_cloud={'on' if enable_wa_cloud else 'off'}"
)
PY
}

choose_channel_mode() {
  local enable_tg="0"
  local enable_wa_web="0"
  local enable_wa_cloud="0"

  if [[ -n "$CHANNELS_ARG" ]]; then
    case "$CHANNELS_ARG" in
      telegram)
        enable_tg="1"
        ;;
      whatsapp_web)
        enable_wa_web="1"
        ;;
      both)
        enable_tg="1"
        enable_wa_web="1"
        ;;
      whatsapp_cloud)
        enable_wa_cloud="1"
        ;;
      all)
        enable_tg="1"
        enable_wa_web="1"
        enable_wa_cloud="1"
        ;;
      *)
        echo "Invalid channels arg: $CHANNELS_ARG"
        echo "Use one of: telegram | whatsapp_web | both | whatsapp_cloud | all"
        exit 1
        ;;
    esac
    apply_channel_flags "$enable_tg" "$enable_wa_web" "$enable_wa_cloud"
    return 0
  fi

  if [[ ! -t 0 || ! -t 1 ]]; then
    echo "Non-interactive terminal detected; keep current channel enable flags." # zh: 检测到非交互终端，保持当前渠道开关配置不变。
    return 0
  fi

  echo "Step 1/5: Select startup channel(s)" # zh: 第 1/5 步：选择启动渠道（可多选）
  ask_yes_no() {
    local prompt="$1"
    local ans
    while true; do
      read -r -p "$prompt [Y/n] > " ans
      ans="${ans:-y}"
      ans="$(echo "$ans" | tr '[:upper:]' '[:lower:]' | xargs)"
      case "$ans" in
        y|yes) echo "Selected: Y"; return 0 ;;
        n|no) echo "Selected: N"; return 1 ;;
        *) echo "Please input y or n." ;; # zh: 请输入 y 或 n。
      esac
    done
  }

  if ask_yes_no "Enable telegram channel?"; then
    enable_tg="1"
  fi
  if ask_yes_no "Enable whatsapp_web channel?"; then
    enable_wa_web="1"
  fi
  if ask_yes_no "Enable whatsapp_cloud channel?"; then
    enable_wa_cloud="1"
  fi

  if [[ "$enable_tg" != "1" && "$enable_wa_web" != "1" && "$enable_wa_cloud" != "1" ]]; then
    echo "No channel selected, keep current channel flags." # zh: 未选择任何渠道，保持当前渠道配置不变。
    return 0
  fi

  apply_channel_flags "$enable_tg" "$enable_wa_web" "$enable_wa_cloud"
}

if [[ "$QUICK_START" == "1" ]]; then
  echo "Step 1/5: Quick mode enabled; skip channel prompt and keep channel config." # zh: 第 1/5 步：快速模式，跳过渠道提问，使用当前配置。
  if [[ -n "$CHANNELS_ARG" ]]; then
    echo "Quick mode ignores channels argument and keeps existing config."
  fi
else
  choose_channel_mode
fi

echo "Step 2/5: Service selection skipped; startup follows enable flags." # zh: 第 2/5 步：跳过服务选择，按 enabled 配置自动启动。

choose_ui_mode() {
  if [[ "${RUSTCLAW_ENABLE_UI:-}" == "1" ]]; then
    ENABLE_UI=1
    return 0
  fi
  if [[ "${RUSTCLAW_ENABLE_UI:-}" == "0" && -n "${RUSTCLAW_ENABLE_UI:-}" ]]; then
    ENABLE_UI=0
    unset RUSTCLAW_UI_DIST || true
    return 0
  fi
  if [[ ! -t 0 || ! -t 1 ]]; then
    ENABLE_UI=0
    unset RUSTCLAW_UI_DIST || true
    return 0
  fi

  local ans
  while true; do
    read -r -p "Enable Web UI for clawd? [Y/n] > " ans
    ans="${ans:-y}"
    ans="$(echo "$ans" | tr '[:upper:]' '[:lower:]' | xargs)"
    case "$ans" in
      y|yes) ENABLE_UI=1; echo "Selected: Y"; break ;;
      n|no) ENABLE_UI=0; unset RUSTCLAW_UI_DIST || true; echo "Selected: N"; break ;;
      *) echo "Please input y or n." ;;
    esac
  done
}

ui_assets_need_build() {
  if [[ "$ENABLE_UI" != "1" ]]; then
    return 1
  fi
  if [[ "$UI_FORCE_REBUILD" == "1" ]]; then
    echo "forced"
    return 0
  fi
  local ui_dir="$SCRIPT_DIR/UI"
  if [[ ! -d "$ui_dir" ]]; then
    echo "missing_ui_dir"
    return 0
  fi
  if [[ ! -f "$ui_dir/dist/index.html" ]]; then
    echo "missing_dist"
    return 0
  fi
  local reason
  reason="$(python3 - <<'PY'
import os
from pathlib import Path

ui = Path("UI")
dist = ui / "dist"
if not ui.exists():
    print("missing_ui_dir")
    raise SystemExit(0)
if not dist.exists():
    print("missing_dist")
    raise SystemExit(0)

scan_paths = [
    ui / "src",
    ui / "public",
    ui / "index.html",
    ui / "package.json",
    ui / "package-lock.json",
    ui / "vite.config.ts",
    ui / "vite.config.js",
    ui / "tsconfig.json",
]

def latest_mtime(paths):
    latest = 0.0
    for p in paths:
        if not p.exists():
            continue
        if p.is_file():
            latest = max(latest, p.stat().st_mtime)
            continue
        for root, _, files in os.walk(p):
            for name in files:
                fp = Path(root) / name
                try:
                    latest = max(latest, fp.stat().st_mtime)
                except OSError:
                    pass
    return latest

src_latest = latest_mtime(scan_paths)
dist_latest = latest_mtime([dist])
if src_latest > dist_latest:
    print("stale_dist")
PY
)"
  if [[ -n "${reason// }" ]]; then
    echo "$reason"
    return 0
  fi
  return 1
}

build_ui_if_needed() {
  if [[ "$ENABLE_UI" != "1" ]]; then
    return 0
  fi
  local reason
  if ! reason="$(ui_assets_need_build)"; then
    export RUSTCLAW_UI_DIST="$SCRIPT_DIR/UI/dist"
    echo "UI assets are up-to-date: $RUSTCLAW_UI_DIST"
    return 0
  fi
  echo "UI build required: ${reason:-unknown_reason}"
  if ! command -v npm >/dev/null 2>&1; then
    echo "npm is required for UI build. Please install Node.js/npm first."
    exit 1
  fi
  if [[ ! -d "$SCRIPT_DIR/UI/node_modules" ]]; then
    echo "Installing UI dependencies..."
    (cd "$SCRIPT_DIR/UI" && npm install)
  fi
  echo "Building UI assets..."
  (cd "$SCRIPT_DIR/UI" && npm run build)
  export RUSTCLAW_UI_DIST="$SCRIPT_DIR/UI/dist"
  echo "UI assets ready: $RUSTCLAW_UI_DIST"
}

if [[ "$QUICK_START" == "1" ]]; then
  echo "Quick mode: skip UI prompt."
else
  choose_ui_mode
fi

echo "Step 3/5: Setup and dependency check" # zh: 第 3/5 步：执行初始化与依赖检查
run_embedded_setup

# Self-contained startup with selectable profile (default debug for test directory).
CLAWD_BIN="$SCRIPT_DIR/target/$PROFILE/clawd"
TELEGRAMD_BIN="$SCRIPT_DIR/target/$PROFILE/telegramd"
WHATSAPPD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsappd"
WHATSAPP_WEBD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsapp_webd"

echo "Step 4/5: Build check" # zh: 第 4/5 步：检查编译产物
if [[ ! -x "$CLAWD_BIN" || ! -x "$TELEGRAMD_BIN" ]]; then
  echo "Prebuilt binaries missing for profile=$PROFILE, starting foreground build..." # zh: 缺少预编译二进制，先前台编译。
  if [[ -f "$SCRIPT_DIR/Cargo.toml" ]]; then
    if [[ "$PROFILE" == "release" ]]; then
      cargo build --workspace --release
    else
      cargo build --workspace
    fi
  else
    echo "Cargo.toml not found in runtime package; cannot compile. Please use a package containing release binaries." # zh: 运行包内未找到 Cargo.toml，无法编译。请使用包含 release 二进制的运行包。
    exit 1
  fi
  echo "Build finished, switching to background startup." # zh: 编译完成，切换为后台启动。
else
  echo "Detected prebuilt binaries under target/$PROFILE; starting directly in background." # zh: 已检测到预编译二进制，直接后台启动。
fi

# Optional UI build and stale check for clawd static assets.
echo "Step 4.5/5: UI build check" # zh: 第 4.5/5 步：检查 UI 资源是否需要构建
build_ui_if_needed

# Ensure skill-runner binary exists for run_skill tasks.
SKILL_RUNNER_ABS="$SCRIPT_DIR/target/$PROFILE/skill-runner"
if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  ALT_PROFILE="debug"
  if [[ "$PROFILE" == "debug" ]]; then
    ALT_PROFILE="release"
  fi
  ALT_RUNNER="$SCRIPT_DIR/target/$ALT_PROFILE/skill-runner"
  if [[ -x "$ALT_RUNNER" ]]; then
    echo "skill-runner missing in $PROFILE, fallback to $ALT_PROFILE: $ALT_RUNNER" # zh: 当前 profile 未找到 skill-runner，回退到另一 profile。
    SKILL_RUNNER_ABS="$ALT_RUNNER"
  fi
fi

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

echo "Step 5/5: Start services" # zh: 第 5/5 步：启动服务
start_clawd
start_telegramd
start_whatsapp_webd
start_whatsappd
echo "One-click startup command executed (profile: $PROFILE)." # zh: 一键启动命令已执行（profile: $PROFILE）。

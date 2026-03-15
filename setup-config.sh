#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

CONFIG_PATH="$SCRIPT_DIR/configs/config.toml"

if [[ ! -f "$CONFIG_PATH" ]]; then
  echo "Config file not found: $CONFIG_PATH" # zh: 未找到配置文件: $CONFIG_PATH
  exit 1
fi

if [[ ! -t 0 || ! -t 1 ]]; then
  echo "This script must run in an interactive terminal." # zh: 该脚本需要交互终端运行。
  exit 1
fi

python3 - <<'PY'
import getpass
import os
import re
import sys
import tomllib
from pathlib import Path

cfg_path = Path("configs/config.toml")
text = cfg_path.read_text(encoding="utf-8")
cfg = tomllib.loads(text)
telegram_cfg_path = Path("configs/channels/telegram.toml")
telegram_text = telegram_cfg_path.read_text(encoding="utf-8") if telegram_cfg_path.exists() else ""
telegram_cfg = tomllib.loads(telegram_text) if telegram_text else {}
changed = False
telegram_changed = False
force = str(os.environ.get("RUSTCLAW_SETUP_FORCE", "1")).strip().lower() not in ("0", "false", "no")

try:
    tty_in = open("/dev/tty", "r", encoding="utf-8", errors="ignore")
    tty_out = open("/dev/tty", "w", encoding="utf-8", errors="ignore")
except OSError:
    print("No interactive terminal detected; cannot prompt for configuration.", file=sys.stderr)  # zh: 未检测到可交互终端，无法进行配置输入。
    raise SystemExit(1)

def ask(prompt: str) -> str:
    tty_out.write(prompt)
    tty_out.flush()
    line = tty_in.readline()
    if line == "":
        raise SystemExit("Failed to read input (EOF). Please run ./setup-config.sh directly in a local terminal.")  # zh: 读取输入失败（EOF）。请在本地终端直接运行 ./setup-config.sh
    return line.rstrip("\n")

def is_empty_or_placeholder(v: str) -> bool:
    return (not v) or v.startswith("REPLACE_ME")

def section_name(path: str) -> str:
    return f"[{path}]"

def set_key_in_section(src: str, section: str, key: str, value_literal: str) -> str:
    sec_header = section_name(section)
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

telegram_token = str(get_nested(telegram_cfg, "telegram", "bot_token", default="") or get_nested(cfg, "telegram", "bot_token", default="") or "")
admins = get_nested(telegram_cfg, "telegram", "admins", default=get_nested(cfg, "telegram", "admins", default=[]))
selected_vendor = str(get_nested(cfg, "llm", "selected_vendor", default="") or "")
selected_model = str(get_nested(cfg, "llm", "selected_model", default="") or "")

if force or is_empty_or_placeholder(telegram_token):
    token = ask("Enter Telegram bot_token: ").strip()  # zh: 请输入 Telegram bot_token:
    while is_empty_or_placeholder(token):
        token = ask("bot_token cannot be empty or placeholder; enter again: ").strip()  # zh: bot_token 不能为空且不能是占位值，请重新输入:
    telegram_text = set_key_in_section(telegram_text, "telegram", "bot_token", quote_toml_string(token))
    telegram_changed = True

if force or (not isinstance(admins, list) or len(admins) == 0):
    admin_raw = ask("Enter admin Telegram user_id (number): ").strip()  # zh: 请输入管理员 Telegram user_id（数字）:
    while (not admin_raw) or (not re.fullmatch(r"-?\d+", admin_raw)):
        admin_raw = ask("Invalid format, enter numeric user_id: ").strip()  # zh: 格式不正确，请输入数字 user_id:
    telegram_text = set_key_in_section(telegram_text, "telegram", "admins", f"[{admin_raw}]")
    telegram_changed = True

vendors = ["openai", "google", "anthropic", "grok"]
available_vendors = [v for v in vendors if isinstance(get_nested(cfg, "llm", v, default=None), dict)]
if not available_vendors:
    print("No selectable vendors found under [llm]. Please check config.", file=sys.stderr)  # zh: [llm] 下未发现可选厂商，请检查配置。
    raise SystemExit(1)
if force or (not selected_vendor):
    print("Select model vendor:")  # zh: 请选择模型厂商:
    for i, v in enumerate(available_vendors, start=1):
        print(f"  {i}) {v}")
    choice = ask("> ").strip()
    while not (choice.isdigit() and 1 <= int(choice) <= len(available_vendors)):
        choice = ask("Invalid input, enter the option number: ").strip()  # zh: 输入无效，请输入序号:
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

if force or (not selected_model):
    if vendor_models:
        print(f"Select model for {selected_vendor}:")  # zh: 请选择 {selected_vendor} 的模型:
        for i, m in enumerate(vendor_models, start=1):
            print(f"  {i}) {m}")
        choice = ask("> ").strip()
        while not (choice.isdigit() and 1 <= int(choice) <= len(vendor_models)):
            choice = ask("Invalid input, enter the option number: ").strip()  # zh: 输入无效，请输入序号:
        selected_model = vendor_models[int(choice) - 1]
    else:
        selected_model = ask(f"Enter model name for {selected_vendor}: ").strip()  # zh: 请输入 {selected_vendor} 的模型名:
        while not selected_model:
            selected_model = ask("Model name cannot be empty, enter again: ").strip()  # zh: 模型名不能为空，请重新输入:
    text = set_key_in_section(text, "llm", "selected_model", quote_toml_string(selected_model))
    changed = True

selected_api_key = str(get_nested(cfg, "llm", selected_vendor, "api_key", default="") or "")
if force or is_empty_or_placeholder(selected_api_key):
    key = getpass.getpass(f"Enter {selected_vendor} api_key (input hidden): ").strip()  # zh: 请输入 {selected_vendor} 的 api_key（输入不回显）:
    while is_empty_or_placeholder(key):
        key = getpass.getpass("api_key cannot be empty or placeholder; enter again: ").strip()  # zh: api_key 不能为空且不能是占位值，请重新输入:
    text = set_key_in_section(text, f"llm.{selected_vendor}", "api_key", quote_toml_string(key))
    changed = True

tools_cmd_timeout = to_int_or_none(get_nested(cfg, "tools", "cmd_timeout_seconds", default=None))
tools_max_cmd_length = to_int_or_none(get_nested(cfg, "tools", "max_cmd_length", default=None))

if tools_cmd_timeout is None or tools_cmd_timeout <= 0:
    print("Configuration: tools.cmd_timeout_seconds")  # zh: 配置说明：tools.cmd_timeout_seconds
    print("  Meaning: timeout in seconds for a single run_cmd execution.")  # zh: 含义：run_cmd 工具单次命令执行的超时时间（秒）。
    print("  Purpose: prevent long hangs (e.g. network stalls/dead loops).")  # zh: 作用：防止命令长时间卡住（例如网络阻塞/死循环）。
    print("  Recommended: 10~60; leave empty to use default 10.")  # zh: 建议：10~60；留空使用默认 10。
    raw = ask("Enter cmd_timeout_seconds (optional): ").strip()  # zh: 请输入 cmd_timeout_seconds（可留空）:
    if not raw:
        timeout_seconds = 10
    else:
        while (not raw.isdigit()) or int(raw) <= 0:
            raw = ask("Enter a positive integer (or empty for default 10): ").strip()  # zh: 请输入正整数（可留空使用默认 10）:
            if not raw:
                break
        timeout_seconds = 10 if not raw else int(raw)
    text = set_key_in_section(text, "tools", "cmd_timeout_seconds", str(timeout_seconds))
    changed = True

if tools_max_cmd_length is None or tools_max_cmd_length <= 0:
    print("Configuration: tools.max_cmd_length")  # zh: 配置说明：tools.max_cmd_length
    print("  Meaning: maximum allowed command length for run_cmd (characters).")  # zh: 含义：run_cmd 工具允许的最大命令长度（字符数）。
    print("  Purpose: limit overly long commands and reduce misuse/parser risk.")  # zh: 作用：限制过长命令，降低误操作与解析风险。
    print("  Recommended: 240~2000; leave empty to use default 240.")  # zh: 建议：240~2000；留空使用默认 240。
    raw = ask("Enter max_cmd_length (optional): ").strip()  # zh: 请输入 max_cmd_length（可留空）:
    if not raw:
        max_cmd_length = 240
    else:
        while (not raw.isdigit()) or int(raw) <= 0:
            raw = ask("Enter a positive integer (or empty for default 240): ").strip()  # zh: 请输入正整数（可留空使用默认 240）:
            if not raw:
                break
        max_cmd_length = 240 if not raw else int(raw)
    text = set_key_in_section(text, "tools", "max_cmd_length", str(max_cmd_length))
    changed = True

if changed:
    cfg_path.write_text(text, encoding="utf-8")
if telegram_changed:
    telegram_cfg_path.parent.mkdir(parents=True, exist_ok=True)
    telegram_cfg_path.write_text(telegram_text, encoding="utf-8")
if changed or telegram_changed:
    if changed:
        print("Configuration updated: configs/config.toml")  # zh: 配置已更新: configs/config.toml
    if telegram_changed:
        print("Configuration updated: configs/channels/telegram.toml")  # zh: 配置已更新: configs/channels/telegram.toml
else:
    print("Configuration is already complete; no changes needed.")  # zh: 配置已完整，无需修改。
PY

echo "Checking skill/runtime dependencies..." # zh: 检查技能与运行时依赖...

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found. Please install Rust toolchain first." # zh: 未找到 cargo，请先安装 Rust 工具链。
  exit 1
fi

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

while IFS='=' read -r key value; do
  case "$key" in
    SKILLS_LIST) SKILLS_LIST="$value" ;;
    WA_WEB_ENABLED) WA_WEB_ENABLED="$value" ;;
    XURL_BIN) XURL_BIN="$value" ;;
  esac
done <<< "$CONFIG_META"

SKILL_RUNNER_ABS="$SCRIPT_DIR/target/release/skill-runner"
BUILD_RELEASE=1
if [[ ! -x "$SKILL_RUNNER_ABS" && -x "$SCRIPT_DIR/target/debug/skill-runner" ]]; then
  SKILL_RUNNER_ABS="$SCRIPT_DIR/target/debug/skill-runner"
  BUILD_RELEASE=0
fi
CARGO_PROFILE_FLAG=()
TARGET_DIR="target/debug"
if [[ "$BUILD_RELEASE" == "1" ]]; then
  CARGO_PROFILE_FLAG=(--release)
  TARGET_DIR="target/release"
fi

if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  echo "skill-runner missing, building..." # zh: 未找到 skill-runner，开始编译...
  cargo build -p skill-runner "${CARGO_PROFILE_FLAG[@]}"
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
      echo "Skip unknown skill in skills_list: $skill" # zh: skills_list 中存在未知技能，已跳过
      continue
    fi
    if [[ ! -x "$SCRIPT_DIR/$TARGET_DIR/$bin_name" ]]; then
      echo "Building missing skill binary: $bin_name" # zh: 正在编译缺失的技能二进制
      cargo build --bin "$bin_name" "${CARGO_PROFILE_FLAG[@]}"
    fi
  done
fi

if [[ ",${SKILLS_LIST:-}," == *",x,"* ]]; then
  echo "Checking X skill dependency (xurl)..." # zh: 检查 X 技能依赖（xurl）...
  if ! command -v npm >/dev/null 2>&1; then
    echo "npm not found. Please install npm first." # zh: 未找到 npm，请先安装 npm
    exit 1
  fi
  if ! command -v "${XURL_BIN:-xurl}" >/dev/null 2>&1; then
    echo "xurl binary not found (${XURL_BIN:-xurl}), installing @xdevplatform/xurl globally..." # zh: 未找到 xurl 命令，开始全局安装 @xdevplatform/xurl
    npm install -g @xdevplatform/xurl
  fi
  if ! "${XURL_BIN:-xurl}" --version >/dev/null 2>&1; then
    echo "xurl verification failed: ${XURL_BIN:-xurl} --version" # zh: xurl 安装校验失败：无法执行 --version
    exit 1
  fi
fi

if [[ "${WA_WEB_ENABLED:-0}" == "1" ]]; then
  echo "Checking WhatsApp Web bridge dependencies..." # zh: 检查 WhatsApp Web bridge 依赖...
  if ! command -v node >/dev/null 2>&1; then
    echo "node not found. Please install Node.js 18+." # zh: 未找到 node，请先安装 Node.js 18+
    exit 1
  fi
  if ! command -v npm >/dev/null 2>&1; then
    echo "npm not found. Please install npm." # zh: 未找到 npm，请先安装 npm
    exit 1
  fi
  BRIDGE_DIR="$SCRIPT_DIR/services/wa-web-bridge"
  if [[ ! -f "$BRIDGE_DIR/package.json" ]]; then
    echo "wa-web-bridge package.json not found: $BRIDGE_DIR/package.json" # zh: 未找到 wa-web-bridge 的 package.json
    exit 1
  fi
  if [[ ! -d "$BRIDGE_DIR/node_modules" ]]; then
    echo "Installing wa-web-bridge npm dependencies..." # zh: 安装 wa-web-bridge npm 依赖...
    npm --prefix "$BRIDGE_DIR" install
  fi
fi

echo "Done." # zh: 完成。

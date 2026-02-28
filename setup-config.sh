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
import re
import sys
import tomllib
from pathlib import Path

cfg_path = Path("configs/config.toml")
text = cfg_path.read_text(encoding="utf-8")
cfg = tomllib.loads(text)
changed = False

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

telegram_token = str(get_nested(cfg, "telegram", "bot_token", default="") or "")
admins = get_nested(cfg, "telegram", "admins", default=[])
selected_vendor = str(get_nested(cfg, "llm", "selected_vendor", default="") or "")
selected_model = str(get_nested(cfg, "llm", "selected_model", default="") or "")

if is_empty_or_placeholder(telegram_token):
    token = ask("Enter Telegram bot_token: ").strip()  # zh: 请输入 Telegram bot_token:
    while is_empty_or_placeholder(token):
        token = ask("bot_token cannot be empty or placeholder; enter again: ").strip()  # zh: bot_token 不能为空且不能是占位值，请重新输入:
    text = set_key_in_section(text, "telegram", "bot_token", quote_toml_string(token))
    changed = True

if not isinstance(admins, list) or len(admins) == 0:
    admin_raw = ask("Enter admin Telegram user_id (number): ").strip()  # zh: 请输入管理员 Telegram user_id（数字）:
    while (not admin_raw) or (not re.fullmatch(r"-?\d+", admin_raw)):
        admin_raw = ask("Invalid format, enter numeric user_id: ").strip()  # zh: 格式不正确，请输入数字 user_id:
    text = set_key_in_section(text, "telegram", "admins", f"[{admin_raw}]")
    changed = True

vendors = ["openai", "google", "anthropic"]
available_vendors = [v for v in vendors if isinstance(get_nested(cfg, "llm", v, default=None), dict)]
if not selected_vendor:
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

if not selected_model:
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
if is_empty_or_placeholder(selected_api_key):
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
    print("Configuration updated: configs/config.toml")  # zh: 配置已更新: configs/config.toml
else:
    print("Configuration is already complete; no changes needed.")  # zh: 配置已完整，无需修改。
PY

echo "Done." # zh: 完成。

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/component_start/common.sh"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/scripts/shell_compat.sh"
configure_platform_command_path
configure_python3_with_tomllib
component_start_init "$SCRIPT_DIR" "${1:-}" "./component_start/start-clawd.sh"
PROFILE="$COMPONENT_PROFILE"

LOG_DIR="$SCRIPT_DIR/logs"
LOG_FILE="$LOG_DIR/clawd.log"
mkdir -p "$LOG_DIR"

# If launched from an interactive terminal, mirror output to logs/clawd.log.
# For non-interactive callers (e.g. start-all.sh with nohup redirection),
# keep caller-managed redirection to avoid duplicate log lines.
if [[ -t 1 ]]; then
  if [[ -z "${APP_LOG_COLOR:-}" ]]; then
    export APP_LOG_COLOR=1
  fi
  # Terminal side keeps ANSI colors; log file side strips them via sed (use $'...' so \x1b is ESC).
  exec > >(tee >(sed $'s/\x1b\\[[0-9;]*m//g' >> "$LOG_FILE")) 2>&1
  echo "Logging to: $LOG_FILE" # zh: 日志输出到：$LOG_FILE
fi

# Ensure clawd binary exists.
CLAWD_BIN="$(component_require_binary clawd)"

# Ensure skill-runner binary exists before starting clawd.
SKILL_RUNNER_ABS="$(component_require_binary skill-runner)"

# First startup policy:
# - if llm.selected_vendor/selected_model is empty, MUST select interactively and persist
# - if both not empty, start directly with default settings
CURRENT_SELECTION="$(
python3 - <<'PY'
import os
import tomllib
from pathlib import Path

cfg_path = Path(os.environ["APP_CONFIG_PATH"])
cfg = tomllib.loads(cfg_path.read_text(encoding="utf-8"))
llm = cfg.get("llm", {})
vendor = str(llm.get("selected_vendor", "") or "")
model = str(llm.get("selected_model", "") or "")
print(f"{vendor}|{model}")
PY
)"
IFS='|' read -r CURRENT_VENDOR CURRENT_MODEL <<<"$CURRENT_SELECTION"
NEED_FIRST_SELECT=0
if [[ -z "${CURRENT_VENDOR}" || -z "${CURRENT_MODEL}" ]]; then
  NEED_FIRST_SELECT=1
fi

if [[ "$NEED_FIRST_SELECT" == "1" ]]; then
  if [[ ! -t 0 || ! -t 1 || "${APP_MODEL_SELECT:-1}" == "0" ]]; then
    echo "First startup requires interactive provider/model selection." # zh: 首次启动需要交互选择模型厂商与模型。
    exit 1
  fi
  echo "First startup: select provider and model..." # zh: 首次启动：请选择模型厂商与模型...
  PROVIDER_ROWS="$(
      python3 - <<'PY'
import os
import tomllib
from pathlib import Path

cfg_path = Path(os.environ["APP_CONFIG_PATH"])
cfg = tomllib.loads(cfg_path.read_text(encoding="utf-8"))
llm = cfg.get("llm", {})
vendors = ["openai", "google", "anthropic", "grok", "deepseek", "qwen", "minimax", "mimo", "custom"]
rows = []
for vendor in vendors:
    section = llm.get(vendor)
    if not isinstance(section, dict):
        continue
    models = section.get("models") or []
    current = str(section.get("model", "-"))
    if not models:
        models = [current]
    for model in models:
        marker = " (default)" if model == current else ""
        rows.append((vendor, str(model), marker))

if not rows:
    print("")
    raise SystemExit(0)

for i, (vendor, model, marker) in enumerate(rows, start=1):
    print(f"{i}|{vendor}|{model}|{marker}")
PY
  )"

  if [[ -z "$PROVIDER_ROWS" ]]; then
    echo "No selectable models detected in config. Please check llm.<vendor>.models." # zh: 配置中未检测到可选模型，请检查 llm.<vendor>.models。
    exit 1
  fi

  array_from_string_lines PROVIDERS "$PROVIDER_ROWS"
  for row in "${PROVIDERS[@]}"; do
    IFS='|' read -r idx vendor model marker <<<"$row"
    echo "  ${idx}) ${vendor} | ${model}${marker}"
  done

  while true; do
    read -r -p "> " choice
    if [[ -n "${choice}" ]] && [[ "${choice}" =~ ^[0-9]+$ ]] && (( choice >= 1 && choice <= ${#PROVIDERS[@]} )); then
      selected="${PROVIDERS[$((choice - 1))]}"
      IFS='|' read -r _ CHOSEN_VENDOR CHOSEN_MODEL _ <<<"$selected"
      break
    fi
    echo "Invalid input, please enter a valid number." # zh: 输入无效，请输入正确序号。
  done

  echo "Selected: ${CHOSEN_VENDOR} | ${CHOSEN_MODEL}" # zh: 已选择: ${CHOSEN_VENDOR} | ${CHOSEN_MODEL}

  python3 - "$CHOSEN_VENDOR" "$CHOSEN_MODEL" <<'PY'
import os
import re
import sys
from pathlib import Path

cfg_path = Path(os.environ["APP_CONFIG_PATH"])
text = cfg_path.read_text(encoding="utf-8")
vendor = sys.argv[1]
model = sys.argv[2]
if not vendor or not model:
    raise SystemExit(0)

def set_or_insert_key(src: str, key: str, value: str) -> str:
    pattern = rf'(?m)^{re.escape(key)}\s*=\s*".*?"\s*$'
    repl = f'{key} = "{value}"'
    if re.search(pattern, src):
        return re.sub(pattern, repl, src, count=1)

    llm_start = src.find("[llm]")
    if llm_start == -1:
        return src.rstrip() + f'\n\n[llm]\n{repl}\n'

    next_section = src.find("\n[", llm_start + 1)
    if next_section == -1:
        next_section = len(src)
    return src[:next_section] + "\n" + repl + src[next_section:]

text = set_or_insert_key(text, "selected_vendor", vendor)
text = set_or_insert_key(text, "selected_model", model)
cfg_path.write_text(text, encoding="utf-8")
PY
fi

ACTIVE_VENDOR="${CHOSEN_VENDOR:-$CURRENT_VENDOR}"
ACTIVE_MODEL="${CHOSEN_MODEL:-$CURRENT_MODEL}"

if [[ -n "${ACTIVE_VENDOR}" ]]; then
  ENV_API_KEY="$(component_vendor_api_key_from_env "$ACTIVE_VENDOR")"
  if [[ -z "${ENV_API_KEY}" ]] && ! component_model_uses_hosted_relay_enrollment \
    "$APP_CONFIG_PATH" "$ACTIVE_VENDOR" "$ACTIVE_MODEL"; then
    echo "The API key for the current vendor (${ACTIVE_VENDOR}) is missing from the runtime environment." >&2
    echo "Set the vendor API-key environment variable in ${APP_RUNTIME_ENV_SCRIPT:-$HOME/runtime_env_filled.sh}, then start the service again." >&2
    exit 1
  fi
fi

if ! "$SCRIPT_DIR/component_start/start-whisper-server.sh" "$PROFILE"; then
  echo "Local audio transcription is unavailable; clawd will continue to start." >&2
fi

component_exec_binary clawd "$CLAWD_BIN"

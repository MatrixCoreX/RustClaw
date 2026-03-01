#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

LOG_DIR="$SCRIPT_DIR/logs"
LOG_FILE="$LOG_DIR/clawd.log"
mkdir -p "$LOG_DIR"

# If launched from an interactive terminal, mirror output to logs/clawd.log.
# For non-interactive callers (e.g. start-all.sh with nohup redirection),
# keep caller-managed redirection to avoid duplicate log lines.
if [[ -t 1 ]]; then
  exec > >(tee -a "$LOG_FILE") 2>&1
  echo "Logging to: $LOG_FILE" # zh: 日志输出到：$LOG_FILE
fi

PROFILE="${1:-${RUSTCLAW_START_PROFILE:-debug}}"
case "$PROFILE" in
  release|debug)
    ;;
  *)
    echo "Usage: ./start-clawd.sh [release|debug]" # zh: 用法：./start-clawd.sh [release|debug]
    exit 1
    ;;
esac

CARGO_PROFILE_FLAG=()
if [[ "$PROFILE" == "release" ]]; then
  CARGO_PROFILE_FLAG=(--release)
fi

# Ensure skill-runner binary exists before starting clawd.
SKILL_RUNNER_PATH="$(
python3 - <<'PY'
import tomllib
from pathlib import Path

cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
skills = cfg.get("skills", {})
print(str(skills.get("skill_runner_path", "target/debug/skill-runner") or "target/debug/skill-runner"))
PY
)"

if [[ "$SKILL_RUNNER_PATH" = /* ]]; then
  SKILL_RUNNER_ABS="$SKILL_RUNNER_PATH"
else
  SKILL_RUNNER_ABS="$SCRIPT_DIR/$SKILL_RUNNER_PATH"
fi

if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  echo "skill-runner missing, trying to build: $SKILL_RUNNER_ABS" # zh: 未找到 skill-runner，尝试自动编译。
  BUILD_SKILL_RELEASE=0
  if [[ "$SKILL_RUNNER_PATH" == *"/release/"* || "$SKILL_RUNNER_PATH" == *"target/release/"* ]]; then
    BUILD_SKILL_RELEASE=1
  fi
  if [[ "$BUILD_SKILL_RELEASE" == "1" ]]; then
    cargo build -p skill-runner --release
  else
    cargo build -p skill-runner
  fi
fi

if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
  echo "skill-runner still missing after build: $SKILL_RUNNER_ABS" # zh: 自动编译后仍未找到 skill-runner。
  echo "Try: ./build-all.sh release" # zh: 可尝试：./build-all.sh release
  echo "Or:  ./build-all.sh debug"   # zh: 可尝试：./build-all.sh debug
  exit 1
fi

# First startup policy:
# - if llm.selected_vendor/selected_model is empty, MUST select interactively and persist
# - if both not empty, start directly with default settings
CURRENT_SELECTION="$(
python3 - <<'PY'
import tomllib
from pathlib import Path

cfg_path = Path("configs/config.toml")
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
  if [[ -n "${RUSTCLAW_PROVIDER_OVERRIDE:-}" && -n "${RUSTCLAW_MODEL_OVERRIDE:-}" ]]; then
    CHOSEN_VENDOR="${RUSTCLAW_PROVIDER_OVERRIDE}"
    CHOSEN_MODEL="${RUSTCLAW_MODEL_OVERRIDE}"
  else
    if [[ ! -t 0 || ! -t 1 || "${RUSTCLAW_MODEL_SELECT:-1}" == "0" ]]; then
      echo "First startup requires interactive provider/model selection (or provide both RUSTCLAW_PROVIDER_OVERRIDE and RUSTCLAW_MODEL_OVERRIDE)." # zh: 首次启动需要交互选择模型厂商与模型（或同时提供 RUSTCLAW_PROVIDER_OVERRIDE 与 RUSTCLAW_MODEL_OVERRIDE）。
      exit 1
    fi
    echo "First startup: select provider and model..." # zh: 首次启动：请选择模型厂商与模型...
    PROVIDER_ROWS="$(
      python3 - <<'PY'
import tomllib
from pathlib import Path

cfg_path = Path("configs/config.toml")
cfg = tomllib.loads(cfg_path.read_text(encoding="utf-8"))
llm = cfg.get("llm", {})
vendors = ["openai", "google", "anthropic", "grok"]
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

    mapfile -t PROVIDERS <<<"$PROVIDER_ROWS"
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
  fi

  export RUSTCLAW_PROVIDER_OVERRIDE="${CHOSEN_VENDOR}"
  export RUSTCLAW_MODEL_OVERRIDE="${CHOSEN_MODEL}"
  echo "Selected: ${CHOSEN_VENDOR} | ${CHOSEN_MODEL}" # zh: 已选择: ${CHOSEN_VENDOR} | ${CHOSEN_MODEL}

  python3 - <<'PY'
import os
import re
from pathlib import Path

cfg_path = Path("configs/config.toml")
text = cfg_path.read_text(encoding="utf-8")
vendor = os.environ.get("RUSTCLAW_PROVIDER_OVERRIDE", "")
model = os.environ.get("RUSTCLAW_MODEL_OVERRIDE", "")
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

ACTIVE_VENDOR="${RUSTCLAW_PROVIDER_OVERRIDE:-$CURRENT_VENDOR}"
ACTIVE_MODEL="${RUSTCLAW_MODEL_OVERRIDE:-$CURRENT_MODEL}"

if [[ -n "${ACTIVE_VENDOR}" ]]; then
  CURRENT_API_KEY="$(
RUSTCLAW_ACTIVE_VENDOR="${ACTIVE_VENDOR}" python3 - <<'PY'
import os
import tomllib
from pathlib import Path

vendor = os.environ.get("RUSTCLAW_ACTIVE_VENDOR", "")
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
llm = cfg.get("llm", {})
section = llm.get(vendor, {})
if isinstance(section, dict):
    print(str(section.get("api_key", "") or ""))
else:
    print("")
PY
  )"

  if [[ -z "${CURRENT_API_KEY}" || "${CURRENT_API_KEY}" == REPLACE_ME* ]]; then
    if [[ ! -t 0 || ! -t 1 || "${RUSTCLAW_MODEL_SELECT:-1}" == "0" ]]; then
      echo "The api_key for current vendor (${ACTIVE_VENDOR}) is empty. Interactive input is required before startup." # zh: 当前厂商(${ACTIVE_VENDOR})的 api_key 为空，必须交互填写后才能启动。
      exit 1
    fi

    while true; do
      read -r -s -p "Enter ${ACTIVE_VENDOR} api_key: " INPUT_API_KEY # zh: 请输入 ${ACTIVE_VENDOR} 的 api_key:
      echo
      if [[ -n "${INPUT_API_KEY}" && "${INPUT_API_KEY}" != REPLACE_ME* ]]; then
        break
      fi
      echo "api_key cannot be empty and cannot be a REPLACE_ME placeholder." # zh: api_key 不能为空，且不能是 REPLACE_ME 占位值。
    done

    export RUSTCLAW_INPUT_API_KEY="${INPUT_API_KEY}"
    export RUSTCLAW_ACTIVE_VENDOR="${ACTIVE_VENDOR}"
    python3 - <<'PY'
import os
import re
from pathlib import Path

vendor = os.environ.get("RUSTCLAW_ACTIVE_VENDOR", "")
api_key = os.environ.get("RUSTCLAW_INPUT_API_KEY", "")
if not vendor or not api_key:
    raise SystemExit(0)

cfg_path = Path("configs/config.toml")
text = cfg_path.read_text(encoding="utf-8")

section_pat = rf"(?ms)^(\[llm\.{re.escape(vendor)}\]\n)(.*?)(?=^\[|\Z)"
m = re.search(section_pat, text)
if not m:
    text = text.rstrip() + f'\n\n[llm.{vendor}]\napi_key = "{api_key}"\n'
else:
    body = m.group(2)
    if re.search(r'(?m)^api_key\s*=\s*".*?"\s*$', body):
        body = re.sub(r'(?m)^api_key\s*=\s*".*?"\s*$', f'api_key = "{api_key}"', body, count=1)
    else:
        body = f'api_key = "{api_key}"\n' + body
    text = text[:m.start()] + m.group(1) + body + text[m.end():]

cfg_path.write_text(text, encoding="utf-8")
PY
    echo "Wrote ${ACTIVE_VENDOR} api_key into config file." # zh: 已写入 ${ACTIVE_VENDOR} 的 api_key 到配置文件。
  fi
fi

exec cargo run "${CARGO_PROFILE_FLAG[@]}" -p clawd

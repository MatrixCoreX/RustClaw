#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/scripts/version_info.sh"
print_rustclaw_version "$SCRIPT_DIR"

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

if [[ -z "${RUSTCLAW_SKIP_BANNER:-}" ]]; then
	print_rustclaw_banner
fi

LOG_DIR="$SCRIPT_DIR/logs"
PID_DIR="$SCRIPT_DIR/.pids"
mkdir -p "$LOG_DIR" "$PID_DIR"

# Optional args:
#   ./start-all.sh <vendor(openai|google|anthropic|grok|deepseek|qwen|minimax|custom)> [model] [release] [channels]
# channels:
#   telegram | whatsapp_web | both | whatsapp_cloud | all
SELECTED_VENDOR_ARG="${1:-}"
SELECTED_MODEL_ARG="${2:-}"
PROFILE="${3:-${RUSTCLAW_START_PROFILE:-release}}"
CHANNELS_ARG="${4:-${RUSTCLAW_START_CHANNELS:-}}"
ENABLE_UI="${RUSTCLAW_ENABLE_UI:-0}"
UI_FORCE_REBUILD="${RUSTCLAW_UI_FORCE_REBUILD:-0}"
case "$PROFILE" in
release) ;;
*)
	echo "Usage: ./start-all.sh <vendor> [model] [release] [channels]" # zh: 用法：./start-all.sh <vendor> [model] [release] [channels]
	exit 1
	;;
esac
export RUSTCLAW_START_PROFILE="$PROFILE"

if [[ -n "$SELECTED_VENDOR_ARG" ]]; then
	python3 - "$SCRIPT_DIR/configs/config.toml" "$SELECTED_VENDOR_ARG" "$SELECTED_MODEL_ARG" <<'PY'
import re
import sys
import tomllib
from pathlib import Path

config_path = Path(sys.argv[1])
vendor = sys.argv[2].strip().lower()
model_arg = sys.argv[3].strip()

raw = config_path.read_text(encoding="utf-8")
cfg = tomllib.loads(raw)
llm = cfg.get("llm") or {}
section = llm.get(vendor)
if not isinstance(section, dict):
    print(f"Error: 配置中未找到 [llm.{vendor}]", file=sys.stderr)
    sys.exit(1)

model = model_arg or str(section.get("model") or "").strip()
if not model:
    print(f"Error: [llm.{vendor}] 的 model 为空", file=sys.stderr)
    sys.exit(1)

llm_section_re = re.compile(r"^\[llm\]\s*$", re.MULTILINE)
match = llm_section_re.search(raw)
if not match:
    print("Error: 未找到 [llm] 段", file=sys.stderr)
    sys.exit(1)

start = match.end()
next_section = re.search(r"\n\s*\[", raw[start:])
end = start + next_section.start() if next_section else len(raw)
section_text = raw[start:end]

def replace_or_insert(text: str, key: str, value: str) -> str:
    pat = re.compile(rf'(?m)^(\s*{re.escape(key)}\s*=\s*)".*?"(\s*(?:#.*)?)$')
    repl = rf'\g<1>"{value}"\g<2>'
    if pat.search(text):
        return pat.sub(repl, text, count=1)
    stripped = text.rstrip("\n")
    prefix = "" if stripped.endswith("\n") or not stripped else "\n"
    return f"{stripped}{prefix}{key} = \"{value}\"\n"

new_section = replace_or_insert(section_text, "selected_vendor", vendor)
new_section = replace_or_insert(new_section, "selected_model", model)
config_path.write_text(raw[:start] + new_section + raw[end:], encoding="utf-8")
print(f"Using config-selected provider/model: {vendor} | {model}")
PY
fi

# Batch start should be non-interactive.
export RUSTCLAW_MODEL_SELECT=0

run_embedded_setup() {
	local config_path="$SCRIPT_DIR/configs/config.toml"
	if [[ ! -f "$config_path" ]]; then
		echo "Config file not found: $config_path"
		exit 1
	fi

	echo "Startup setup prompts are disabled; manage these settings in UI or config files." # zh: 启动阶段不再弹出配置问题，这些设置请在 UI 或配置文件中完成。

	echo "Checking skill/runtime dependencies..."
	if ! command -v python3 >/dev/null 2>&1; then
		echo "python3 not found."
		exit 1
	fi
	# 不再在启动/保存配置时自动执行 sync_skill_docs，避免部署包（无 crates/skills）误删 prompts 文件

	CONFIG_META="$(
		python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
for extra in ("configs/channels/telegram.toml", "configs/channels/whatsapp-web.toml", "configs/channels/whatsapp-cloud.toml"):
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
	done <<<"$CONFIG_META"

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
		IFS=',' read -r -a SKILLS_ARR <<<"$SKILLS_LIST"
		for skill in "${SKILLS_ARR[@]}"; do
			skill="$(echo "$skill" | xargs)"
			[[ -z "$skill" ]] && continue
			if ! bin_name="$(skill_bin_name "$skill")"; then
				continue
			fi
			if [[ ! -x "$SCRIPT_DIR/$target_dir/$bin_name" ]]; then
				echo "Skill binary missing: $SCRIPT_DIR/$target_dir/$bin_name (run: cargo build -p <pkg> ${PROFILE:+--release})"
				exit 1
			fi
		done
	fi

	if [[ ",${SKILLS_LIST:-}," == *",x,"* ]]; then
		echo "Checking X skill dependency (xurl)..."
		if ! command -v npm >/dev/null 2>&1; then
			echo "npm not found."
			echo "Suggested install: sudo apt-get install -y npm"
			echo "Continue startup without auto-install." # zh: 仅提示缺失，继续启动流程。
		fi
		if command -v npm >/dev/null 2>&1 && ! command -v "${XURL_BIN:-xurl}" >/dev/null 2>&1; then
			echo "xurl binary not found (${XURL_BIN:-xurl})."
			echo "Suggested install: sudo npm install -g @xdevplatform/xurl"
			echo "Continue startup without auto-install." # zh: 仅提示缺失，继续启动流程。
		fi
	fi

	if [[ "${WA_WEB_ENABLED:-0}" == "1" ]]; then
		echo "Checking WhatsApp Web bridge dependencies..."
		if ! command -v node >/dev/null 2>&1; then
			echo "node not found."
			echo "Suggested install: sudo apt-get install -y nodejs"
			echo "Continue startup without auto-install." # zh: 仅提示缺失，继续启动流程。
		fi
		if ! command -v npm >/dev/null 2>&1; then
			echo "npm not found."
			echo "Suggested install: sudo apt-get install -y npm"
			echo "Continue startup without auto-install." # zh: 仅提示缺失，继续启动流程。
		fi
		local bridge_dir="$SCRIPT_DIR/services/wa-web-bridge"
		if command -v node >/dev/null 2>&1 && command -v npm >/dev/null 2>&1 && [[ -f "$bridge_dir/package.json" && ! -d "$bridge_dir/node_modules" ]]; then
			echo "WhatsApp Web bridge dependencies missing: $bridge_dir/node_modules"
			echo "Suggested install: cd \"$bridge_dir\" && npm install"
			echo "Continue startup without auto-install." # zh: 仅提示缺失，继续启动流程。
		fi
	fi
}

refresh_channel_flags() {
	eval "$(
		python3 - <<'PY'
import tomllib
from pathlib import Path

channels = [
    ("CHANNEL_WEBD", Path("configs/channels/webd.toml"), ("webd", "enabled"), False),
    ("CHANNEL_TG", Path("configs/channels/telegram.toml"), ("telegram_bot", "enabled"), True),
    ("CHANNEL_WA_WEB", Path("configs/channels/whatsapp-web.toml"), ("whatsapp_web", "enabled"), False),
    ("CHANNEL_WA_CLOUD", Path("configs/channels/whatsapp-cloud.toml"), ("whatsapp", "enabled"), False),
    ("CHANNEL_WECHAT", Path("configs/channels/wechat.toml"), ("wechat", "enabled"), False),
    ("CHANNEL_FEISHU", Path("configs/channels/feishu.toml"), ("feishu", "enabled"), False),
    ("CHANNEL_LARK", Path("configs/channels/lark.toml"), ("lark", "enabled"), False),
]

for var_name, path, key_path, default in channels:
    enabled = default
    if path.exists():
        try:
            cfg = tomllib.loads(path.read_text(encoding="utf-8"))
            cur = cfg
            for key in key_path:
                if not isinstance(cur, dict):
                    cur = None
                    break
                cur = cur.get(key)
            if isinstance(cur, bool):
                enabled = cur
        except Exception:
            pass
    print(f'{var_name}={"1" if enabled else "0"}')
PY
	)"
}

print_channel_flags_summary() {
	local title="${1:-Current communication endpoints}"
	refresh_channel_flags
	echo "$title"
	printf '  [%s] webd            -> configs/channels/webd.toml\n' "$([[ "$CHANNEL_WEBD" == "1" ]] && echo x || echo ' ')"
	printf '  [%s] telegram        -> configs/channels/telegram.toml\n' "$([[ "$CHANNEL_TG" == "1" ]] && echo x || echo ' ')"
	printf '  [%s] whatsapp_web    -> configs/channels/whatsapp-web.toml\n' "$([[ "$CHANNEL_WA_WEB" == "1" ]] && echo x || echo ' ')"
	printf '  [%s] whatsapp_cloud  -> configs/channels/whatsapp-cloud.toml\n' "$([[ "$CHANNEL_WA_CLOUD" == "1" ]] && echo x || echo ' ')"
	printf '  [%s] wechat          -> configs/channels/wechat.toml\n' "$([[ "$CHANNEL_WECHAT" == "1" ]] && echo x || echo ' ')"
	printf '  [%s] feishu          -> configs/channels/feishu.toml\n' "$([[ "$CHANNEL_FEISHU" == "1" ]] && echo x || echo ' ')"
	printf '  [%s] lark            -> configs/channels/lark.toml\n' "$([[ "$CHANNEL_LARK" == "1" ]] && echo x || echo ' ')"
}

apply_channel_flags() {
	local enable_webd="$1"
	local enable_tg="$2"
	local enable_wa_web="$3"
	local enable_wa_cloud="$4"
	local enable_wechat="$5"
	local enable_feishu="$6"
	local enable_lark="$7"
	export RUSTCLAW_ENABLE_WEBD="$enable_webd"
	export RUSTCLAW_ENABLE_TG="$enable_tg"
	export RUSTCLAW_ENABLE_WA_WEB="$enable_wa_web"
	export RUSTCLAW_ENABLE_WA_CLOUD="$enable_wa_cloud"
	export RUSTCLAW_ENABLE_WECHAT="$enable_wechat"
	export RUSTCLAW_ENABLE_FEISHU="$enable_feishu"
	export RUSTCLAW_ENABLE_LARK="$enable_lark"

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

def write_flag(path_str: str, updates):
    path = Path(path_str)
    text = path.read_text(encoding="utf-8") if path.exists() else ""
    for section, key, value in updates:
        text = set_flag(text, section, key, value)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")

enable_webd = os.environ.get("RUSTCLAW_ENABLE_WEBD", "0") == "1"
enable_tg = os.environ.get("RUSTCLAW_ENABLE_TG", "0") == "1"
enable_wa_web = os.environ.get("RUSTCLAW_ENABLE_WA_WEB", "0") == "1"
enable_wa_cloud = os.environ.get("RUSTCLAW_ENABLE_WA_CLOUD", "0") == "1"
enable_wechat = os.environ.get("RUSTCLAW_ENABLE_WECHAT", "0") == "1"
enable_feishu = os.environ.get("RUSTCLAW_ENABLE_FEISHU", "0") == "1"
enable_lark = os.environ.get("RUSTCLAW_ENABLE_LARK", "0") == "1"

write_flag("configs/channels/webd.toml", [("webd", "enabled", enable_webd)])
write_flag("configs/channels/telegram.toml", [("telegram_bot", "enabled", enable_tg)])
write_flag("configs/channels/whatsapp-web.toml", [("whatsapp_web", "enabled", enable_wa_web)])
write_flag(
    "configs/channels/whatsapp-cloud.toml",
    [("whatsapp", "enabled", enable_wa_cloud), ("whatsapp_cloud", "enabled", enable_wa_cloud)],
)
write_flag("configs/channels/wechat.toml", [("wechat", "enabled", enable_wechat)])
write_flag("configs/channels/feishu.toml", [("feishu", "enabled", enable_feishu)])
write_flag("configs/channels/lark.toml", [("lark", "enabled", enable_lark)])

print(
    "Applied channel flags: "
    f"webd={'on' if enable_webd else 'off'}, "
    f"telegram={'on' if enable_tg else 'off'}, "
    f"whatsapp_web={'on' if enable_wa_web else 'off'}, "
    f"whatsapp_cloud={'on' if enable_wa_cloud else 'off'}, "
    f"wechat={'on' if enable_wechat else 'off'}, "
    f"feishu={'on' if enable_feishu else 'off'}, "
    f"lark={'on' if enable_lark else 'off'}"
)
PY
}

ask_yes_no_default() {
	local prompt="$1"
	local default="$2"
	local ans normalized
	local suffix="[y/N]"
	[[ "$default" == "1" ]] && suffix="[Y/n]"
	while true; do
		read -r -p "$prompt $suffix > " ans
		ans="$(echo "$ans" | tr '[:upper:]' '[:lower:]' | xargs)"
		if [[ -z "$ans" ]]; then
			[[ "$default" == "1" ]] && echo "Selected: Y" || echo "Selected: N"
			[[ "$default" == "1" ]]
			return
		fi
		case "$ans" in
		y | yes)
			echo "Selected: Y"
			return 0
			;;
		n | no)
			echo "Selected: N"
			return 1
			;;
		*) echo "Please input y or n." ;;
		esac
	done
}

choose_channel_mode_picker() {
	local labels=("webd" "telegram" "whatsapp_web" "whatsapp_cloud" "wechat" "feishu" "lark")
	local descs=(
		"Web HTTP gateway"
		"Telegram bot"
		"WhatsApp Web QR mode"
		"WhatsApp Cloud webhook"
		"WeChat iLink"
		"Feishu"
		"Lark"
	)
	local states=("$1" "$2" "$3" "$4" "$5" "$6" "$7")
	local cursor=0
	local count="${#labels[@]}"
	local key=""
	local key2=""
	local key3=""
	local idx=""
	local i=""
	local mark=""
	local pointer=""

	while true; do
		printf '\033[2J\033[H'
		echo "Step 1/5: Select communication endpoints"
		echo "Use Up/Down (or j/k) to move, Space to toggle, Enter to confirm, c to save and exit."
		echo "You can also press 1-7 to toggle an item directly."
		echo
		for ((i = 0; i < count; i++)); do
			mark=" "
			[[ "${states[$i]}" == "1" ]] && mark="x"
			pointer="  "
			[[ "$i" -eq "$cursor" ]] && pointer="> "
			printf "%s[%s] %d. %-16s %s\n" "$pointer" "$mark" "$((i + 1))" "${labels[$i]}" "${descs[$i]}"
		done

		IFS= read -rsn1 key
		case "$key" in
		"")
			break
			;;
		" ")
			if [[ "${states[$cursor]}" == "1" ]]; then
				states[$cursor]="0"
			else
				states[$cursor]="1"
			fi
			;;
		[jJ])
			cursor=$(((cursor + 1) % count))
			;;
		[kK])
			cursor=$(((cursor - 1 + count) % count))
			;;
		[cCqQ])
			CHANNEL_PICK_WEBD="${states[0]}"
			CHANNEL_PICK_TG="${states[1]}"
			CHANNEL_PICK_WA_WEB="${states[2]}"
			CHANNEL_PICK_WA_CLOUD="${states[3]}"
			CHANNEL_PICK_WECHAT="${states[4]}"
			CHANNEL_PICK_FEISHU="${states[5]}"
			CHANNEL_PICK_LARK="${states[6]}"
			printf '\033[2J\033[H'
			return 2
			;;
		$'\x1b')
			IFS= read -rsn1 -t 0.05 key2 || true
			if [[ "$key2" == "[" ]]; then
				IFS= read -rsn1 -t 0.05 key3 || true
				case "$key3" in
				A) cursor=$(((cursor - 1 + count) % count)) ;;
				B) cursor=$(((cursor + 1) % count)) ;;
				esac
			fi
			;;
		[1-7])
			idx=$((10#$key - 1))
			if [[ "$idx" -ge 0 && "$idx" -lt "$count" ]]; then
				cursor="$idx"
				if [[ "${states[$idx]}" == "1" ]]; then
					states[$idx]="0"
				else
					states[$idx]="1"
				fi
			fi
			;;
		esac
	done

	printf '\033[2J\033[H'
	CHANNEL_PICK_WEBD="${states[0]}"
	CHANNEL_PICK_TG="${states[1]}"
	CHANNEL_PICK_WA_WEB="${states[2]}"
	CHANNEL_PICK_WA_CLOUD="${states[3]}"
	CHANNEL_PICK_WECHAT="${states[4]}"
	CHANNEL_PICK_FEISHU="${states[5]}"
	CHANNEL_PICK_LARK="${states[6]}"
	return 0
}

choose_channel_mode() {
	refresh_channel_flags
	print_channel_flags_summary "Step 1/5: Current communication endpoints (enabled comes from config files)"
	local enable_webd="$CHANNEL_WEBD"
	local enable_tg="$CHANNEL_TG"
	local enable_wa_web="$CHANNEL_WA_WEB"
	local enable_wa_cloud="$CHANNEL_WA_CLOUD"
	local enable_wechat="$CHANNEL_WECHAT"
	local enable_feishu="$CHANNEL_FEISHU"
	local enable_lark="$CHANNEL_LARK"

	if [[ -n "$CHANNELS_ARG" ]]; then
		case "$CHANNELS_ARG" in
		telegram)
			enable_tg="1"
			enable_wa_web="0"
			enable_wa_cloud="0"
			;;
		whatsapp_web)
			enable_tg="0"
			enable_wa_web="1"
			enable_wa_cloud="0"
			;;
		both)
			enable_tg="1"
			enable_wa_web="1"
			enable_wa_cloud="0"
			;;
		whatsapp_cloud)
			enable_tg="0"
			enable_wa_web="0"
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
		apply_channel_flags "$enable_webd" "$enable_tg" "$enable_wa_web" "$enable_wa_cloud" "$enable_wechat" "$enable_feishu" "$enable_lark"
		return 0
	fi

	if [[ ! -t 0 || ! -t 1 ]]; then
		echo "Non-interactive terminal detected; keep current channel enable flags." # zh: 检测到非交互终端，保持当前渠道开关配置不变。
		return 0
	fi

	echo "Select which communication endpoints to enable (only edits *.enabled)." # zh: 请选择要启用的通信端，仅修改配置文件中的 enabled。
	local picker_status=0
	choose_channel_mode_picker "$enable_webd" "$enable_tg" "$enable_wa_web" "$enable_wa_cloud" "$enable_wechat" "$enable_feishu" "$enable_lark" || picker_status=$?
	if [[ "$picker_status" == "0" ]]; then
		enable_webd="${CHANNEL_PICK_WEBD:-$enable_webd}"
		enable_tg="${CHANNEL_PICK_TG:-$enable_tg}"
		enable_wa_web="${CHANNEL_PICK_WA_WEB:-$enable_wa_web}"
		enable_wa_cloud="${CHANNEL_PICK_WA_CLOUD:-$enable_wa_cloud}"
		enable_wechat="${CHANNEL_PICK_WECHAT:-$enable_wechat}"
		enable_feishu="${CHANNEL_PICK_FEISHU:-$enable_feishu}"
		enable_lark="${CHANNEL_PICK_LARK:-$enable_lark}"
	elif [[ "$picker_status" == "2" ]]; then
		enable_webd="${CHANNEL_PICK_WEBD:-$enable_webd}"
		enable_tg="${CHANNEL_PICK_TG:-$enable_tg}"
		enable_wa_web="${CHANNEL_PICK_WA_WEB:-$enable_wa_web}"
		enable_wa_cloud="${CHANNEL_PICK_WA_CLOUD:-$enable_wa_cloud}"
		enable_wechat="${CHANNEL_PICK_WECHAT:-$enable_wechat}"
		enable_feishu="${CHANNEL_PICK_FEISHU:-$enable_feishu}"
		enable_lark="${CHANNEL_PICK_LARK:-$enable_lark}"
		echo "Channel selection exited early; writing current enable flags back to config." # zh: 已提前退出通信端选择，当前勾选结果会回写到配置文件。
	else
		if ask_yes_no_default "Enable webd channel?" "$enable_webd"; then enable_webd="1"; else enable_webd="0"; fi
		if ask_yes_no_default "Enable telegram channel?" "$enable_tg"; then enable_tg="1"; else enable_tg="0"; fi
		if ask_yes_no_default "Enable whatsapp_web channel?" "$enable_wa_web"; then enable_wa_web="1"; else enable_wa_web="0"; fi
		if ask_yes_no_default "Enable whatsapp_cloud channel?" "$enable_wa_cloud"; then enable_wa_cloud="1"; else enable_wa_cloud="0"; fi
		if ask_yes_no_default "Enable wechat channel?" "$enable_wechat"; then enable_wechat="1"; else enable_wechat="0"; fi
		if ask_yes_no_default "Enable feishu channel?" "$enable_feishu"; then enable_feishu="1"; else enable_feishu="0"; fi
		if ask_yes_no_default "Enable lark channel?" "$enable_lark"; then enable_lark="1"; else enable_lark="0"; fi
	fi

	apply_channel_flags "$enable_webd" "$enable_tg" "$enable_wa_web" "$enable_wa_cloud" "$enable_wechat" "$enable_feishu" "$enable_lark"
}

choose_channel_mode

echo "Step 2/5: Service selection skipped; startup follows enable flags." # zh: 第 2/5 步：跳过服务选择，按 enabled 配置自动启动。

choose_ui_mode() {
	if [[ "${RUSTCLAW_ENABLE_UI:-}" == "1" ]]; then
		ENABLE_UI=1
		return 0
	fi
	ENABLE_UI=0
	unset RUSTCLAW_UI_DIST || true
	return 0
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
	reason="$(
		python3 - <<'PY'
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
	if [[ -n "${reason// /}" ]]; then
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
	echo "Build UI first: cd UI && npm install && npm run build  (or start without --with-ui)"
	exit 1
}

choose_ui_mode
if [[ "$ENABLE_UI" == "1" ]]; then
	echo "Web UI startup enabled via --with-ui; continuing with release startup." # zh: 已通过 --with-ui 启用 Web UI，继续执行 release 启动。
else
	echo "Web UI prompt skipped; continuing with release startup." # zh: 跳过 Web UI 交互，继续执行 release 启动。
fi

echo "Step 3/5: Setup and dependency check" # zh: 第 3/5 步：执行初始化与依赖检查
run_embedded_setup

# Self-contained startup with release profile.
CLAWD_BIN="$SCRIPT_DIR/target/$PROFILE/clawd"
WEBD_BIN="$SCRIPT_DIR/target/$PROFILE/webd"
TELEGRAMD_BIN="$SCRIPT_DIR/target/$PROFILE/telegramd"
WHATSAPPD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsappd"
WHATSAPP_WEBD_BIN="$SCRIPT_DIR/target/$PROFILE/whatsapp_webd"
WECHATD_BIN="$SCRIPT_DIR/target/$PROFILE/wechatd"
FEISHUD_BIN="$SCRIPT_DIR/target/$PROFILE/feishud"
LARKD_BIN="$SCRIPT_DIR/target/$PROFILE/larkd"

echo "Step 4/5: Build check" # zh: 第 4/5 步：检查编译产物
if [[ ! -x "$CLAWD_BIN" ]]; then
	echo "Prebuilt binaries missing for profile=$PROFILE." # zh: 缺少预编译二进制
	echo "Required: $CLAWD_BIN"
	echo "Copy your built binaries to target/$PROFILE/ or run: cargo build --workspace --release"
	exit 1
fi
echo "Detected prebuilt binaries under target/$PROFILE; starting directly in background." # zh: 已检测到预编译二进制，直接后台启动。

# Optional UI build and stale check for clawd static assets.
echo "Step 4.5/5: UI build check" # zh: 第 4.5/5 步：检查 UI 资源是否需要构建
build_ui_if_needed

# Ensure skill-runner binary exists for run_skill tasks.
SKILL_RUNNER_ABS="$SCRIPT_DIR/target/$PROFILE/skill-runner"
if [[ ! -x "$SKILL_RUNNER_ABS" ]]; then
	echo "skill-runner missing: $SKILL_RUNNER_ABS" # zh: 未找到 skill-runner
	echo "Copy your built skill-runner to target/$PROFILE/ or run: cargo build -p skill-runner --release"
	exit 1
fi

start_clawd() {
	if pgrep -f 'target/release/clawd|cargo run -p clawd' >/dev/null 2>&1; then
		echo "clawd is already running, skipping startup." # zh: clawd 已在运行，跳过启动。
		return 0
	fi
	nohup "$CLAWD_BIN" >"$LOG_DIR/clawd.log" 2>&1 &
	local pid=$!
	echo "$pid" >"$PID_DIR/clawd.pid"
	echo "Starting clawd binary, PID=$pid, log: $LOG_DIR/clawd.log" # zh: clawd 二进制启动中，PID=$pid, 日志: $LOG_DIR/clawd.log
}

start_webd() {
	local webd_enabled
	webd_enabled="$(
		python3 - <<'PY'
import tomllib
from pathlib import Path
cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
webd_cfg = Path("configs/channels/webd.toml")
if webd_cfg.exists():
    cfg.update(tomllib.loads(webd_cfg.read_text(encoding="utf-8")))
print("1" if bool(cfg.get("webd", {}).get("enabled", False)) else "0")
PY
	)"
	if [[ "$webd_enabled" != "1" ]]; then
		echo "webd.enabled=false, skipping webd startup." # zh: webd.enabled=false，跳过 webd 启动。
		return 0
	fi
	if [[ ! -x "$WEBD_BIN" ]]; then
		echo "Binary not found or not executable: $WEBD_BIN" # zh: 二进制不存在或不可执行：$WEBD_BIN
		return 1
	fi
	if pgrep -f 'target/release/webd|cargo run -p webd' >/dev/null 2>&1; then
		echo "webd is already running, skipping startup." # zh: webd 已在运行，跳过启动。
		return 0
	fi
	nohup "$WEBD_BIN" >"$LOG_DIR/webd.log" 2>&1 &
	local pid=$!
	echo "$pid" >"$PID_DIR/webd.pid"
	echo "Starting webd binary, PID=$pid, log: $LOG_DIR/webd.log" # zh: webd 二进制启动中，PID=$pid, 日志: $LOG_DIR/webd.log
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
	if pgrep -f 'target/release/telegramd|cargo run -p telegramd' >/dev/null 2>&1; then
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
wa_cfg = Path("configs/channels/whatsapp-web.toml")
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
	if pgrep -f 'target/release/whatsapp_webd|cargo run -p whatsapp_webd' >/dev/null 2>&1; then
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
wa_cfg = Path("configs/channels/whatsapp-cloud.toml")
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
	if pgrep -f 'target/release/whatsappd|cargo run -p whatsappd' >/dev/null 2>&1; then
		echo "whatsappd is already running, skipping startup." # zh: whatsappd 已在运行，跳过启动。
		return 0
	fi
	nohup "$WHATSAPPD_BIN" >"$LOG_DIR/whatsappd.log" 2>&1 &
	local pid=$!
	echo "$pid" >"$PID_DIR/whatsappd.pid"
	echo "Starting whatsappd binary, PID=$pid, log: $LOG_DIR/whatsappd.log" # zh: whatsappd 二进制启动中，PID=$pid, 日志: $LOG_DIR/whatsappd.log
}

start_feishud() {
	local feishu_enabled
	feishu_enabled="$(
		python3 - <<'PY'
import tomllib
from pathlib import Path
path = Path("configs/channels/feishu.toml")
if not path.exists():
    print("0")
    raise SystemExit(0)
cfg = tomllib.loads(path.read_text(encoding="utf-8"))
feishu = cfg.get("feishu", {}) or {}
print("1" if bool(feishu.get("enabled", False)) else "0")
PY
	)"
	if [[ "$feishu_enabled" != "1" ]]; then
		echo "feishu.enabled=false, skipping feishud startup." # zh: feishu.enabled=false，跳过 feishud 启动。
		return 0
	fi
	if [[ ! -x "$FEISHUD_BIN" ]]; then
		echo "Binary not found or not executable: $FEISHUD_BIN" # zh: 二进制不存在或不可执行：$FEISHUD_BIN
		return 0
	fi
	if pgrep -f 'target/release/feishud|cargo run -p feishud' >/dev/null 2>&1; then
		echo "feishud is already running, skipping startup." # zh: feishud 已在运行，跳过启动。
		return 0
	fi
	export FEISHU_CONFIG_PATH="${FEISHU_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/feishu.toml}"
	nohup "$FEISHUD_BIN" >"$LOG_DIR/feishud.log" 2>&1 &
	local pid=$!
	echo "$pid" >"$PID_DIR/feishud.pid"
	echo "Starting feishud binary, PID=$pid, log: $LOG_DIR/feishud.log" # zh: feishud 二进制启动中，PID=$pid, 日志: $LOG_DIR/feishud.log
}

start_wechatd() {
	local wechat_enabled
	wechat_enabled="$(
		python3 - <<'PY'
import tomllib
from pathlib import Path
path = Path("configs/channels/wechat.toml")
if not path.exists():
    print("0")
    raise SystemExit(0)
cfg = tomllib.loads(path.read_text(encoding="utf-8"))
wechat = cfg.get("wechat", {}) or {}
print("1" if bool(wechat.get("enabled", False)) else "0")
PY
	)"
	if [[ "$wechat_enabled" != "1" ]]; then
		echo "wechat.enabled=false, skipping wechatd startup."
		return 0
	fi
	if [[ ! -x "$WECHATD_BIN" ]]; then
		echo "Binary not found or not executable: $WECHATD_BIN"
		return 1
	fi
	if pgrep -f 'target/release/wechatd|cargo run -p wechatd' >/dev/null 2>&1; then
		echo "wechatd is already running, skipping startup."
		return 0
	fi
	export WECHAT_CONFIG_PATH="${WECHAT_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/wechat.toml}"
	nohup "$WECHATD_BIN" >"$LOG_DIR/wechatd.log" 2>&1 &
	local pid=$!
	echo "$pid" >"$PID_DIR/wechatd.pid"
	echo "Starting wechatd binary, PID=$pid, log: $LOG_DIR/wechatd.log"
}

start_larkd() {
	local lark_enabled
	lark_enabled="$(
		python3 - <<'PY'
import tomllib
from pathlib import Path
path = Path("configs/channels/lark.toml")
if not path.exists():
    print("0")
    raise SystemExit(0)
cfg = tomllib.loads(path.read_text(encoding="utf-8"))
lark = cfg.get("lark", {}) or {}
print("1" if bool(lark.get("enabled", False)) else "0")
PY
	)"
	if [[ "$lark_enabled" != "1" ]]; then
		echo "lark.enabled=false, skipping larkd startup."
		return 0
	fi
	if [[ ! -x "$LARKD_BIN" ]]; then
		echo "Binary not found or not executable: $LARKD_BIN"
		return 0
	fi
	if pgrep -f 'target/release/larkd|cargo run -p larkd' >/dev/null 2>&1; then
		echo "larkd is already running, skipping startup."
		return 0
	fi
	export LARK_CONFIG_PATH="${LARK_CONFIG_PATH:-$SCRIPT_DIR/configs/channels/lark.toml}"
	nohup "$LARKD_BIN" >"$LOG_DIR/larkd.log" 2>&1 &
	local pid=$!
	echo "$pid" >"$PID_DIR/larkd.pid"
	echo "Starting larkd binary, PID=$pid, log: $LOG_DIR/larkd.log"
}

echo "Step 5/5: Start services" # zh: 第 5/5 步：启动服务
start_clawd
start_webd
start_telegramd
start_whatsapp_webd
start_whatsappd
start_wechatd
start_feishud
start_larkd
echo "Startup finished (profile: $PROFILE)." # zh: 启动完成（profile: $PROFILE）。
echo "Next: configure LLM provider/model, API keys, and communication channels in the UI." # zh: 下一步：请在 UI 中配置大模型厂商/模型、API Key、通信端等设置。
echo "You can also edit config files directly under: $SCRIPT_DIR/configs/" # zh: 也可以直接修改配置目录：$SCRIPT_DIR/configs/
echo "Common files: configs/config.toml and configs/channels/*.toml" # zh: 常用配置文件：configs/config.toml 和 configs/channels/*.toml

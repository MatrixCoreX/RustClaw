#!/usr/bin/env bash
# 一键切换主模型为 qwen 或 minimax，仅修改当前目录下 configs/config.toml 中 [llm] 的 selected_vendor / selected_model，
# 模型名使用配置文件里该厂商已有的 model 配置，不改动其他内容。
set -euo pipefail

ROOT_DIR="$(pwd)"
CONFIG_PATH="$ROOT_DIR/configs/config.toml"

usage() {
    echo "Usage: $0 <qwen|minimax>"
    echo "  使用配置文件现有 [llm.qwen] / [llm.minimax] 的 model，切换主模型为该厂商。"
    exit 1
}

if [[ $# -lt 1 ]]; then
    usage
fi

VENDOR="${1,,}"
case "$VENDOR" in
    qwen|minimax) ;;
    *) echo "Error: 仅支持 qwen 或 minimax"; usage ;;
esac

if [[ ! -f "$CONFIG_PATH" ]]; then
    echo "Error: 配置文件不存在: $CONFIG_PATH"
    exit 1
fi

python3 - "$CONFIG_PATH" "$VENDOR" <<'PY'
import re
import sys
import tomllib
from pathlib import Path

def main():
    config_path = Path(sys.argv[1])
    vendor = sys.argv[2].strip().lower()
    if vendor not in ("qwen", "minimax"):
        print("Error: 仅支持 qwen 或 minimax", file=sys.stderr)
        sys.exit(1)

    raw = config_path.read_text(encoding="utf-8")
    cfg = tomllib.loads(raw)

    llm = cfg.get("llm") or {}
    if isinstance(llm, dict) and vendor in llm and isinstance(llm[vendor], dict):
        model = (llm[vendor].get("model") or "").strip()
    else:
        print(f"Error: 配置中未找到 [llm.{vendor}] 或 model", file=sys.stderr)
        sys.exit(1)
    if not model:
        print(f"Error: [llm.{vendor}] 的 model 为空", file=sys.stderr)
        sys.exit(1)

    # 只处理 [llm] 段（第一个 [llm] 到下一个 [ 或文件末尾），仅改 selected_model / selected_vendor 两行
    llm_section_re = re.compile(r"^\[llm\]\s*$", re.MULTILINE)
    match = llm_section_re.search(raw)
    if not match:
        print("Error: 未找到 [llm] 段", file=sys.stderr)
        sys.exit(1)
    start = match.end()
    next_section = re.search(r"\n\s*\[", raw[start:])
    end = start + next_section.start() if next_section else len(raw)
    section = raw[start:end]

    def replace_in_section(text, key, new_val):
        # 匹配 key = "..." 或 key = '...'
        pat = re.compile(
            r"^(\s*" + re.escape(key) + r"\s*=\s*)([\"'])[^\"']*\2(\s*(?:#.*)?)$",
            re.MULTILINE,
        )
        return pat.sub(rf'\g<1>"{new_val}"\g<3>', text, count=1)

    new_section = replace_in_section(section, "selected_model", model)
    new_section = replace_in_section(new_section, "selected_vendor", vendor)
    if new_section == section:
        print("Warning: 在 [llm] 段内未匹配到 selected_model/selected_vendor，未修改文件", file=sys.stderr)
        sys.exit(1)

    new_raw = raw[:start] + new_section + raw[end:]
    config_path.write_text(new_raw, encoding="utf-8")
    print(f"已切换主模型: selected_vendor={vendor}, selected_model={model}")

if __name__ == "__main__":
    main()
PY

echo "完成。重启 clawd 后生效。"

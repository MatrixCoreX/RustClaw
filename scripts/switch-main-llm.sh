#!/usr/bin/env bash
# Switch the primary LLM vendor by updating [llm].selected_vendor / selected_model
# in configs/config.toml. The model is read from the selected vendor's existing
# [llm.<vendor>].model entry; no other fields are modified.
set -euo pipefail

ROOT_DIR="$(pwd)"
CONFIG_PATH="$ROOT_DIR/configs/config.toml"

usage() {
    echo "Usage: $0 <qwen|minimax|mimo>"
    echo "  Switch to the current model configured under [llm.<vendor>]."
    exit 1
}

if [[ $# -lt 1 ]]; then
    usage
fi

VENDOR="${1,,}"
case "$VENDOR" in
    qwen|minimax|mimo) ;;
    *) echo "Error: supported vendors: qwen, minimax, mimo"; usage ;;
esac

if [[ ! -f "$CONFIG_PATH" ]]; then
    echo "Error: config file not found: $CONFIG_PATH"
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
    if vendor not in ("qwen", "minimax", "mimo"):
        print("Error: supported vendors: qwen, minimax, mimo", file=sys.stderr)
        sys.exit(1)

    raw = config_path.read_text(encoding="utf-8")
    cfg = tomllib.loads(raw)

    llm = cfg.get("llm") or {}
    if isinstance(llm, dict) and vendor in llm and isinstance(llm[vendor], dict):
        model = (llm[vendor].get("model") or "").strip()
    else:
        print(f"Error: [llm.{vendor}] or model not found in config", file=sys.stderr)
        sys.exit(1)
    if not model:
        print(f"Error: [llm.{vendor}].model is empty", file=sys.stderr)
        sys.exit(1)

    # Only update selected_model / selected_vendor inside the top-level [llm] section.
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
        # Match key = "..." or key = '...'.
        pat = re.compile(
            r"^(\s*" + re.escape(key) + r"\s*=\s*)([\"'])[^\"']*\2(\s*(?:#.*)?)$",
            re.MULTILINE,
        )
        return pat.sub(rf'\g<1>"{new_val}"\g<3>', text, count=1)

    new_section = replace_in_section(section, "selected_model", model)
    new_section = replace_in_section(new_section, "selected_vendor", vendor)
    if new_section == section:
        current_vendor = str(llm.get("selected_vendor") or "").strip().lower()
        current_model = str(llm.get("selected_model") or "").strip()
        if current_vendor == vendor and current_model == model:
            print(f"Primary LLM already selected: selected_vendor={vendor}, selected_model={model}")
            return
        print("Warning: selected_model/selected_vendor not found in [llm]; config was not modified", file=sys.stderr)
        sys.exit(1)

    new_raw = raw[:start] + new_section + raw[end:]
    config_path.write_text(new_raw, encoding="utf-8")
    print(f"Primary LLM switched: selected_vendor={vendor}, selected_model={model}")

if __name__ == "__main__":
    main()
PY

echo "Done. Restart clawd to apply the change."

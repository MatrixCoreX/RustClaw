#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Enable colored log tags on interactive terminals unless overridden.
if [[ -t 1 && -z "${RUSTCLAW_LOG_COLOR:-}" ]]; then
  export RUSTCLAW_LOG_COLOR=1
fi

python3 - <<'PY'
import tomllib
from pathlib import Path

cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
adapters = cfg.get("adapters", {})
enabled = []
for name, conf in adapters.items():
    if isinstance(conf, dict) and bool(conf.get("enabled", False)):
        enabled.append(name)

if not enabled:
    print("no future adapters enabled, skip.")  # zh: 未启用 future adapters，占位跳过。
else:
    print("future adapters enabled but not implemented:", ", ".join(enabled))  # zh: 已启用 future adapters，但当前仅占位未实现。
PY

exit 0

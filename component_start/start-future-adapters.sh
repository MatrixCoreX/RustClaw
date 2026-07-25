#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/component_start/common.sh"
component_start_init "$SCRIPT_DIR" release "./component_start/start-future-adapters.sh"

python3 - <<'PY'
import os
import tomllib
from pathlib import Path

cfg = tomllib.loads(Path(os.environ["RUSTCLAW_CONFIG_PATH"]).read_text(encoding="utf-8"))
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

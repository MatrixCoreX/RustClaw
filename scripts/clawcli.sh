#!/usr/bin/env bash
# 调用已编译的 clawcli（与 crates/clawcli 产出的可执行名一致：clawcli）。
#
# 解析顺序（命中即用）:
#   1) 环境变量 CLAWCLI 指向可执行文件
#   2) 仓库根下 target/release/clawcli
#   3) 仓库根下 target/debug/clawcli
#   4) PATH 中的 clawcli
#
# 若未设置 RUSTCLAW_WORKSPACE，则自动设为本仓库根（含 configs/config.toml），
# 便于从数据库读取 admin key（与 clawcli 行为一致）。
#
# 用法：与直接运行 clawcli 相同，例如：
#   scripts/clawcli.sh --help
#   scripts/clawcli.sh health
#   RUSTCLAW_BASE_URL=http://127.0.0.1:9000 scripts/clawcli.sh chat
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [[ -z "${RUSTCLAW_WORKSPACE:-}" && -f "$REPO_ROOT/configs/config.toml" ]]; then
  export RUSTCLAW_WORKSPACE="$REPO_ROOT"
fi

resolve_clawcli() {
  if [[ -n "${CLAWCLI:-}" && -x "$CLAWCLI" ]]; then
    printf '%s' "$CLAWCLI"
    return 0
  fi
  local rel="$REPO_ROOT/target/release/clawcli"
  if [[ -x "$rel" ]]; then
    printf '%s' "$rel"
    return 0
  fi
  rel="$REPO_ROOT/target/debug/clawcli"
  if [[ -x "$rel" ]]; then
    printf '%s' "$rel"
    return 0
  fi
  if command -v clawcli >/dev/null 2>&1; then
    command -v clawcli
    return 0
  fi
  return 1
}

BIN="$(resolve_clawcli)" || {
  echo "clawcli: 未找到可执行文件。请先: cargo build -p clawcli --release" >&2
  echo "  或设置 CLAWCLI=/path/to/clawcli" >&2
  exit 127
}

exec "$BIN" "$@"

#!/usr/bin/env bash
# 注册一个稳定的小屏启动入口，避免桌面/自启动直接写死仓库目录

set -euo pipefail

PI_APP_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
STATE_DIR="${HOME}/.config/rustclaw-small-screen"
STATE_FILE="${STATE_DIR}/active-pi-app-dir"
BIN_DIR="${HOME}/.local/bin"
WRAPPER="${BIN_DIR}/rustclaw-small-screen-launcher"
ICON_DIR="${HOME}/.local/share/icons"
ICON_LINK="${ICON_DIR}/rustclaw-small-screen.png"

mkdir -p "$STATE_DIR" "$BIN_DIR" "$ICON_DIR"
printf '%s\n' "$PI_APP_DIR" > "$STATE_FILE"

cat > "$WRAPPER" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

STATE_FILE="${HOME}/.config/rustclaw-small-screen/active-pi-app-dir"
if [[ ! -f "$STATE_FILE" ]]; then
  echo "未找到 RustClaw 小屏目录配置: $STATE_FILE" >&2
  exit 1
fi

PI_APP_DIR="$(tr -d '\r' < "$STATE_FILE" | head -n 1)"
if [[ -z "${PI_APP_DIR}" ]]; then
  echo "RustClaw 小屏目录配置为空: $STATE_FILE" >&2
  exit 1
fi

LAUNCHER="${PI_APP_DIR}/run-small-screen-launcher.sh"
if [[ ! -x "$LAUNCHER" ]]; then
  echo "启动器不存在或不可执行: $LAUNCHER" >&2
  exit 1
fi

exec "$LAUNCHER" "$@"
EOF

chmod +x "$WRAPPER"

if [[ -f "${PI_APP_DIR}/longxia.png" ]]; then
  ln -sfn "${PI_APP_DIR}/longxia.png" "$ICON_LINK"
fi

echo "已注册稳定启动入口: $WRAPPER"
echo "当前小屏目录: $PI_APP_DIR"
echo "目录状态文件: $STATE_FILE"

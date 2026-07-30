#!/usr/bin/env bash
# 在桌面创建产品小屏快捷方式，双击即可启动（pi_app 版）。

set -e
PI_APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ROOT="$(cd "$PI_APP_DIR/.." && pwd)"
# shellcheck source=/dev/null
source "$APP_ROOT/scripts/product_identity.sh"
REGISTER="${PI_APP_DIR}/register-launcher.sh"
WRAPPER="${HOME}/.local/bin/agent-small-screen-launcher"
ICON="${HOME}/.local/share/icons/agent-small-screen.png"
PRODUCT_DISPLAY_NAME="$APP_DISPLAY_NAME"
DESKTOP_FILE="${HOME}/Desktop/agent-small-screen.desktop"

"$REGISTER"
mkdir -p "$(dirname "$DESKTOP_FILE")"
cat > "$DESKTOP_FILE" << EOF
[Desktop Entry]
Type=Application
Name=${PRODUCT_DISPLAY_NAME}
Comment=480×320 小屏状态（Python），请求 /v1/health
Exec=${WRAPPER}
Path=${HOME}
TryExec=${WRAPPER}
Icon=${ICON}
Terminal=false
Categories=Utility;
StartupNotify=true
EOF
chmod +x "$DESKTOP_FILE"
chmod +x "$WRAPPER"
echo "已创建桌面快捷方式: $DESKTOP_FILE"
echo "双击桌面上的 ${PRODUCT_DISPLAY_NAME} 图标即可启动小屏。"

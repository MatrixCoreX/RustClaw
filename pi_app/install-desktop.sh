#!/usr/bin/env bash
# 在桌面创建「RustClaw 小屏」快捷方式，双击即可启动（pi_app 版）
# 在 pi_app 目录下执行。图标写入 ~/Desktop/RustClaw.desktop

set -e
PI_APP_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
REGISTER="${PI_APP_DIR}/register-launcher.sh"
WRAPPER="${HOME}/.local/bin/rustclaw-small-screen-launcher"
ICON="${HOME}/.local/share/icons/rustclaw-small-screen.png"
DESKTOP_FILE="${HOME}/Desktop/RustClaw.desktop"

"$REGISTER"
mkdir -p "$(dirname "$DESKTOP_FILE")"
cat > "$DESKTOP_FILE" << EOF
[Desktop Entry]
Type=Application
Name=RustClaw
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
echo "双击桌面上的 RustClaw 图标即可启动小屏。"

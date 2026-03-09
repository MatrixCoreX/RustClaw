#!/usr/bin/env bash
# 将 RustClaw 小屏监控设为当前用户登录后自动启动（XDG Autostart）
# 在 pi_app 目录下执行。自启动项写入 ~/.config/autostart/
# 取消自启动：运行本目录下 disable-autostart.sh

set -e
PI_APP_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
LAUNCHER="${PI_APP_DIR}/run-small-screen-launcher.sh"
AUTOSTART_DIR="${HOME}/.config/autostart"
DESKTOP_FILE="${AUTOSTART_DIR}/rustclaw-small-screen.desktop"

mkdir -p "$AUTOSTART_DIR"
cat > "$DESKTOP_FILE" << EOF
[Desktop Entry]
Type=Application
Name=RustClaw Small Screen
Comment=RustClaw 小屏监控开机自启动
Exec=${LAUNCHER}
Path=${PI_APP_DIR}
Terminal=false
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=2
StartupNotify=false
EOF
chmod +x "$LAUNCHER"
echo "已启用开机自启动: $DESKTOP_FILE"
echo "取消自启动: $PI_APP_DIR/disable-autostart.sh"

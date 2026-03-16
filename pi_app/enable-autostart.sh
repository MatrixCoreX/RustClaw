#!/usr/bin/env bash
# 将 RustClaw 小屏监控设为当前用户登录后自动启动
# 在 pi_app 目录下执行。
# 同时写入 XDG autostart 与 LXDE-pi autostart，兼容树莓派桌面。
# 取消自启动：运行本目录下 disable-autostart.sh

set -e
PI_APP_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
REGISTER="${PI_APP_DIR}/register-launcher.sh"
WRAPPER="${HOME}/.local/bin/rustclaw-small-screen-launcher"
AUTOSTART_DIR="${HOME}/.config/autostart"
DESKTOP_FILE="${AUTOSTART_DIR}/rustclaw-small-screen.desktop"
LXDE_AUTOSTART="${HOME}/.config/lxsession/LXDE-pi/autostart"

"$REGISTER"
mkdir -p "$AUTOSTART_DIR"
cat > "$DESKTOP_FILE" << EOF
[Desktop Entry]
Type=Application
Name=RustClaw Small Screen
Comment=RustClaw 小屏监控开机自启动
Exec=${WRAPPER}
Path=${HOME}
TryExec=${WRAPPER}
Terminal=false
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=5
StartupNotify=false
EOF
chmod +x "$WRAPPER"

# 树莓派常用 LXDE：自启动由 lxsession 读 ~/.config/lxsession/LXDE-pi/autostart
mkdir -p "$(dirname "$LXDE_AUTOSTART")"
MARKER="# RustClaw small screen"
tmp="$(mktemp)"
if [[ -f "$LXDE_AUTOSTART" ]]; then
  grep -v "run-small-screen-launcher.sh" "$LXDE_AUTOSTART" | grep -v "rustclaw-small-screen-launcher" | grep -v "# RustClaw small screen" > "$tmp" || true
else
  : > "$tmp"
fi
{
  cat "$tmp"
  printf '\n%s\n@%s\n' "$MARKER" "$WRAPPER"
} > "$LXDE_AUTOSTART"
rm -f "$tmp"

echo "已启用开机自启动:"
echo "  - XDG: $DESKTOP_FILE"
echo "  - LXDE-pi: $LXDE_AUTOSTART"
echo "取消自启动: $PI_APP_DIR/disable-autostart.sh"

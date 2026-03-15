#!/usr/bin/env bash
# 取消 RustClaw 小屏监控的开机自启动（删除 XDG 与 LXDE-pi 两处）

DESKTOP_FILE="${HOME}/.config/autostart/rustclaw-small-screen.desktop"
LXDE_AUTOSTART="${HOME}/.config/lxsession/LXDE-pi/autostart"

removed=0
if [[ -f "$DESKTOP_FILE" ]]; then
  rm -f "$DESKTOP_FILE"
  echo "已移除: $DESKTOP_FILE"
  removed=1
fi

if [[ -f "$LXDE_AUTOSTART" ]]; then
  if grep -q "run-small-screen-launcher.sh" "$LXDE_AUTOSTART" 2>/dev/null; then
    tmp=$(mktemp)
    grep -v "run-small-screen-launcher.sh" "$LXDE_AUTOSTART" | grep -v "# RustClaw small screen" > "$tmp" || true
    mv -f "$tmp" "$LXDE_AUTOSTART"
    echo "已从 LXDE-pi 自启动中移除: $LXDE_AUTOSTART"
    removed=1
  fi
fi

if [[ $removed -eq 1 ]]; then
  echo "已取消开机自启动。"
else
  echo "未找到自启动项。"
fi

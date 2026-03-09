#!/usr/bin/env bash
# 取消 RustClaw 小屏监控的开机自启动（删除 ~/.config/autostart 中的 desktop 项）

DESKTOP_FILE="${HOME}/.config/autostart/rustclaw-small-screen.desktop"
if [[ -f "$DESKTOP_FILE" ]]; then
  rm -f "$DESKTOP_FILE"
  echo "已取消开机自启动。"
else
  echo "未找到自启动项: $DESKTOP_FILE"
fi

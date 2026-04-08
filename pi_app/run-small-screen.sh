#!/bin/bash
# 前台直接启动小屏（pi_app 版），适合终端调试

PI_APP_DIR="$(dirname "$(readlink -f "$0")")"
cd "$PI_APP_DIR" || exit 1

if [[ -z "${DISPLAY}" ]]; then
  echo "未设置 DISPLAY，图形小屏无法启动。"
  echo "可选：本机桌面下执行 export DISPLAY=:0 后再运行本脚本；"
  echo "      或无桌面时用网页版： $PI_APP_DIR/open-small-screen.sh"
  exit 1
fi

# 小屏运行时尽量关闭 X 层空闲熄屏/DPMS，避免长时间无人操作后画面看似“卡住”。
if command -v xset >/dev/null 2>&1; then
  xset s off >/dev/null 2>&1 || true
  xset -dpms >/dev/null 2>&1 || true
  xset s noblank >/dev/null 2>&1 || true
fi

exec /usr/bin/env python3 "${PI_APP_DIR}/rustclaw_small_screen.py"

#!/bin/bash
# 供桌面图标或 XDG 自启动调用：设置图形环境后启动小屏（pi_app 版）
# 桌面点击时往往没有 DISPLAY/PATH，这里统一补全

# 桌面/自启动时 PATH 可能只有 /usr/bin，先保证能找到 python3
export PATH="/usr/bin:/usr/local/bin:${PATH:-/usr/bin:/bin}"

# 未设置 DISPLAY 时用 :0（本机默认显示器），并设置 X 鉴权（开机自启动时常见）
if [[ -z "${DISPLAY}" ]]; then
  export DISPLAY=:0
  export XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}"
  # 开机自启动时桌面可能尚未就绪，先等 8 秒再检测
  sleep 8
  # 再轮询直到 Tk 能连上 X（最多约 30 秒）
  wait_sec=0
  while [ "$wait_sec" -lt 30 ]; do
    if /usr/bin/env python3 -c "
import os
os.environ.setdefault('DISPLAY', os.environ.get('DISPLAY', ':0'))
import tkinter as tk
try:
    r = tk.Tk()
    r.destroy()
except Exception:
    exit(1)
" 2>/dev/null; then
      break
    fi
    sleep 1
    wait_sec=$((wait_sec + 1))
  done
else
  export XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}"
fi

# 若小屏进程已在运行，直接退出，避免重复启动
if pgrep -f "rustclaw_small_screen\.py" >/dev/null 2>&1; then
  exit 0
fi

PI_APP_DIR="$(dirname "$(readlink -f "$0")")"
cd "$PI_APP_DIR" || exit 1

LOG="$HOME/.rustclaw-small-screen.log"
if ! /usr/bin/env python3 "${PI_APP_DIR}/rustclaw_small_screen.py" >>"$LOG" 2>&1; then
  echo "RustClaw 小屏启动失败，详见: $LOG" >>"$LOG"
  notify-send "RustClaw 小屏" "启动失败，请查看 $LOG" 2>/dev/null || true
  exit 1
fi

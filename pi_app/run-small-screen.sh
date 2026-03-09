#!/bin/bash
# 前台直接启动小屏（pi_app 版），适合终端调试

PI_APP_DIR="$(dirname "$(readlink -f "$0")")"
cd "$PI_APP_DIR" || exit 1
exec /usr/bin/env python3 "${PI_APP_DIR}/rustclaw_small_screen.py"

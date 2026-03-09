# RustClaw 树莓派 / Pi 桌面小程序

本目录为树莓派等小屏设备上的 RustClaw 桌面监控应用，包含 Python 小屏程序、启动脚本、桌面快捷方式与开机自启动配置。

## 目录结构

```
pi_app/
├── rustclaw_small_screen.py   # 主程序（480×320 全屏，/v1/health + 技能/加密货币/NNI 等）
├── run-small-screen-launcher.sh   # 桌面/自启动用启动脚本（补全 DISPLAY/PATH）
├── run-small-screen.sh            # 终端前台启动（调试用）
├── open-small-screen.sh           # 用浏览器打开网页版小屏页（small-screen.html）
├── enable-autostart.sh            # 启用开机自启动
├── disable-autostart.sh           # 取消开机自启动
├── install-desktop.sh             # 在桌面创建「RustClaw」快捷方式
├── assets/                        # 资源（如 lobster.gif 等）
├── image/                         # NNI 页图库图片
├── RustClaw480X320.png            # 启动闪屏图
├── longxia.png                    # 桌面图标
└── README.md                      # 本说明
```

## 路径说明

| 用途 | 路径 |
|------|------|
| **桌面快捷方式** | 运行 `./install-desktop.sh` 后生成 `~/Desktop/RustClaw.desktop`，双击即启动小屏 |
| **开机自启动** | 运行 `./enable-autostart.sh` 后写入 `~/.config/autostart/rustclaw-small-screen.desktop`，登录后自动启动 |
| **自启动取消** | 运行 `./disable-autostart.sh` 或删除 `~/.config/autostart/rustclaw-small-screen.desktop` |
| **启动日志** | 启动失败时错误信息写入 `~/.rustclaw-small-screen.log` |
| **用户配置** | 语言/主题/小程序专用 key 保存在 pi_app 目录下 `.rustclaw_small_screen_lang`、`.rustclaw_small_screen_theme`、`.rustclaw_small_screen_key` |

## 使用方式

1. **终端启动（调试）**
   ```bash
   cd /path/to/RustClaw/pi_app
   ./run-small-screen.sh
   ```

2. **桌面图标启动**  
   先执行一次：`./install-desktop.sh`，之后双击桌面上的「RustClaw」图标即可。

3. **开机自启动**  
   执行：`./enable-autostart.sh`。取消则执行：`./disable-autostart.sh`。

4. **网页版小屏**（需先启动 clawd）  
   `./open-small-screen.sh` 会用 Chromium 全屏打开 `http://127.0.0.1:8787/small-screen.html`。

## 依赖

- Python 3 + tkinter
- 图形环境（DISPLAY，桌面或 `export DISPLAY=:0`）
- 小屏程序请求 `http://127.0.0.1:8787/v1/health`，需先启动 clawd
- 首次启动时，Python 小程序会自动生成并写入一把本机专用 `user` key 到数据库，同时保存到 `pi_app/.rustclaw_small_screen_key`

## 与 scripts/ 的关系

小屏相关逻辑已集中到 `pi_app/`。仓库内 `scripts/` 下仍可能保留旧脚本或符号链接，以兼容已有用法；新部署请以 `pi_app/` 为准。

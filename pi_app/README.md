# Agent Runtime 树莓派 / Pi 桌面小程序

本目录为树莓派等小屏设备上的 Agent Runtime 桌面监控应用，包含 Python 小屏程序、启动脚本、桌面快捷方式与开机自启动配置。

## 目录结构

```
pi_app/
├── agent_small_screen.py          # 主程序（480×320 全屏，运行状态、消息、行情、Bancor、NNI 等）
├── run-small-screen-launcher.sh   # 桌面/自启动用启动脚本（补全 DISPLAY/PATH）
├── run-small-screen.sh            # 终端前台启动（调试用）
├── open-small-screen.sh           # 用浏览器打开网页版小屏页（small-screen.html）
├── enable-autostart.sh            # 启用开机自启动
├── disable-autostart.sh           # 取消开机自启动
├── install-desktop.sh             # 在桌面创建「Agent Runtime」快捷方式
├── assets/                        # 资源（如 lobster.gif 等）
├── image/                         # NNI 页图库图片
├── app-splash-480x320.png         # 默认启动闪屏图；实际文件名由产品身份配置选择
├── longxia.png                    # 桌面图标
└── README.md                      # 本说明
```

## 路径说明

| 用途 | 路径 |
|------|------|
| **桌面快捷方式** | 运行 `./install-desktop.sh` 后按产品身份配置生成桌面入口，双击即启动小屏 |
| **开机自启动** | 运行 `./enable-autostart.sh` 后写入 XDG `~/.config/autostart/` 与树莓派 LXDE `~/.config/lxsession/LXDE-pi/autostart`，登录后自动启动 |
| **自启动取消** | 运行 `./disable-autostart.sh` 会同时移除上述两处 |
| **启动日志** | 启动失败时错误信息写入 `~/.agent-runtime-small-screen.log` |
| **用户配置** | 语言、主题、页面显示开关和备用的小程序专用 key 保存在 pi_app 目录下 `.agent_small_screen_config.json`；运行库中的 admin key 不写入该文件 |

## 使用方式

1. **终端启动（调试）**
   ```bash
   cd /path/to/agent-runtime/pi_app
   ./run-small-screen.sh
   ```

2. **桌面图标启动**  
   先执行一次：`./install-desktop.sh`，之后双击按产品身份配置生成的桌面图标即可。

3. **开机自启动**  
   执行：`./enable-autostart.sh`。取消则执行：`./disable-autostart.sh`。

4. **网页版小屏**（需先启动 clawd）  
   `./open-small-screen.sh` 会用 Chromium 全屏打开 `http://127.0.0.1:8787/small-screen.html`。

## 依赖

- Python 3 + tkinter
- 图形环境（DISPLAY，桌面或 `export DISPLAY=:0`）
- 小屏程序请求 `http://127.0.0.1:8787/v1/health`，需先启动 clawd
- Python 小程序优先直接读取运行库中已启用的 `admin` key，以本机管理界面的完整权限读取状态；不会把 admin key 复制到设置文件
- 运行库中没有可用 admin key 时，才会生成并注册一把本机专用 `user` key，保存到权限为 `0600` 的 `pi_app/.agent_small_screen_config.json`
- 请求返回 401 时会重新读取当前 admin key 并重试一次，以适配运行期间的密钥轮换

## 自启动后进程在但窗口不出现

若开机自启动后能在后台看到 `agent_small_screen.py` 进程，但屏幕没有窗口：

- 小屏已做「启动后窗口置前」处理，先**重启一次**再观察。
- 仍不出现时，在本机桌面开终端执行：`cd pi_app && ./run-small-screen.sh`，看是否能正常弹窗；若可以，多半是自启动时的图形环境差异，可改用「登录后手动点桌面图标」或把 `run-small-screen.sh` 加到「首选项 → 默认应用 → 自启动」并适当增加延迟。

## 与 scripts/ 的关系

小屏相关逻辑已集中到 `pi_app/`。仓库内 `scripts/` 下仍可能保留旧脚本或符号链接，以兼容已有用法；新部署请以 `pi_app/` 为准。

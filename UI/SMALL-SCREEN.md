# 小屏 480×320 只读状态页

与主 UI 使用**同一接口** `GET /v1/health`，仅做展示、无操作。

## 显示内容

| 项     | 说明           |
|--------|----------------|
| 状态   | 在线 / 错误    |
| 版本   | clawd version  |
| 运行时长 | uptime_seconds |
| 队列   | queue_length   |
| 执行中 | running_length |
| Worker | worker_state   |
| 内存 RSS | memory_rss_bytes |
| 服务   | TG/WA 是否 healthy（✓/✗） |

每 15 秒自动刷新一次。

## 使用方式

1. **先启动 clawd**（监听 8787，并启用 UI 静态资源）。
2. 在小屏设备浏览器中打开：
   - **本机访问**（推荐）：`http://127.0.0.1:8787/small-screen.html`
3. 或在本机用脚本全屏打开小屏页：
   ```bash
   cd /path/to/RustClaw/pi_app && ./open-small-screen.sh
   ```

## 文件位置

- 页面：`UI/dist/small-screen.html`（clawd 直接提供）
- 源拷贝：`UI/public/small-screen.html`（`npm run build` 时会复制到 dist）
- 启动脚本：`pi_app/open-small-screen.sh`（网页版）；Python 小屏见 `pi_app/README.md`

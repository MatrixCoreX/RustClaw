# RustClaw

<img src="./RustClaw.png" width="420" />

英文版：`README.md`

RustClaw 是一个本地运行的 Rust Agent Runtime，面向 Telegram、WhatsApp、飞书/Lark 和浏览器 UI 的日常使用场景。它把任务路由、技能执行、记忆、调度、多模态能力，以及基于 `user_key` 的身份体系整合到一套可部署系统里。

## 这个项目能做什么

RustClaw 主要用于：

- 通过多个通道和 Agent 对话
- 调用内置技能处理文件、HTTP、服务、图片、加密货币等任务
- 用 `user_key` 管理用户身份和权限
- 通过本地 UI 做监控和日常管理
- 支持记忆能力和可恢复的多步骤任务

## 快速开始

### 1. 前置条件

```bash
rustup default stable
python3 --version
```

建议使用 Python `3.11+`。

### 2. 安装 `rustclaw` 命令

```bash
# 标准安装
bash install-rustclaw-cmd.sh

# 无 sudo 环境安装
bash install-rustclaw-cmd.sh --user
```

安装后检查：

```bash
command -v rustclaw
rustclaw -h
rustclaw -status
```

### 3. 配置模型和通道

主配置文件：`configs/config.toml`

通道配置：

- `configs/channels/telegram.toml`
- `configs/channels/whatsapp.toml`
- `configs/channels/feishu.toml`

常见拆分配置：

- `configs/image.toml`
- `configs/audio.toml`
- `configs/crypto.toml`

### 4. 启动 RustClaw

```bash
# 带 UI 的完整启动
rustclaw -start --vendor openai --model gpt-4.1 --profile release --channels all --with-ui --quick

# 其它厂商示例
rustclaw -start --vendor qwen --model qwen-max-latest --profile release --channels all --quick

# 简化启动
rustclaw -start release all
```

### 5. 日常操作

```bash
rustclaw -status
rustclaw -logs clawd 200 --follow
rustclaw -health
rustclaw -stop
```

传统脚本仍可使用：

```bash
./start-all.sh
./stop-rustclaw.sh
```

## Key、身份和权限

RustClaw 使用 `user_key` 作为跨 UI 和消息通道的主身份标识。

- 权限按 `user_key` 解析
- 会话按 `channel + external_chat_id` 解析
- UI 与 Telegram、WhatsApp 使用同一套鉴权模型
- 当鉴权表为空时，`clawd` 可以自动生成首个 admin key

Key 管理：

```bash
rustclaw -key list
rustclaw -key generate user
rustclaw -key generate admin
scripts/auth-key.sh list
```

## 本地 UI 和 API

监听地址在 `configs/config.toml` 中配置，常见是 `127.0.0.1:8787` 或 `0.0.0.0:8787`。

UI：

- 地址：`http://127.0.0.1:8787/`
- 静态目录：`UI/dist`
- 可用 `RUSTCLAW_UI_DIST` 覆盖 UI 目录
- 浏览器会本地保存合法 `user_key`，并通过 `X-RustClaw-Key` 发送给后端

常用 API：

- `GET /v1/health`
- `POST /v1/tasks`
- `GET /v1/tasks/{task_id}`
- `POST /v1/tasks/cancel`
- `GET /v1/auth/me`
- `POST /v1/auth/channel/bind`
- `GET/POST /v1/auth/crypto-credentials`

示例：

```bash
curl http://127.0.0.1:8787/v1/health \
  -H "X-RustClaw-Key: rk-xxxx"

curl -X POST http://127.0.0.1:8787/v1/tasks \
  -H "Content-Type: application/json" \
  -H "X-RustClaw-Key: rk-xxxx" \
  -d '{"user_id":1,"chat_id":1,"user_key":"rk-xxxx","channel":"ui","external_user_id":"local-ui","external_chat_id":"local-ui","kind":"ask","payload":{"text":"hello","agent_mode":true}}'
```

## Telegram 常用命令

- `/start`
- `/help`
- `/agent on|off`
- `/status`
- `/cancel`
- `/skills`
- `/run <skill> <args>`
- `/sendfile <path>`
- `/voicemode show|voice|text|both|reset`
- `/openclaw config show|vendors|set <vendor> <model>`
- `/cryptoapi show`
- `/cryptoapi set binance <api_key> <api_secret>`
- `/cryptoapi set okx <api_key> <api_secret> <passphrase>`

## Crypto 凭据

交易所凭据按 `user_key` 存储，不是所有用户共用一套全局密钥。

- 当前实盘支持 `binance`、`okx`
- 风控配置在 `configs/crypto.toml`
- 凭据存放在 `exchange_api_credentials`
- 每个 `user_key` 都有自己的交易所凭据记录

你可以通过以下方式管理凭据：

- `GET/POST /v1/auth/crypto-credentials`
- Telegram `/cryptoapi ...` 命令
- `scripts/import-crypto-credentials.sh` 迁移旧配置

## 内置技能

常见内置技能包括：

- `archive_basic`
- `audio_synthesize`
- `audio_transcribe`
- `chat`
- `config_guard`
- `crypto`
- `db_basic`
- `docker_basic`
- `fs_search`
- `git_basic`
- `health_check`
- `http_basic`
- `image_edit`
- `image_generate`
- `image_vision`
- `log_analyze`
- `package_manager`
- `process_basic`
- `rss_fetch`
- `service_control`
- `system_basic`
- `x`

## 重要目录和文件

- `configs/config.toml`：主运行配置
- `configs/channels/*.toml`：通道配置
- `configs/image.toml`：图片技能配置
- `configs/audio.toml`：音频技能配置
- `configs/crypto.toml`：交易和风控配置
- `configs/i18n/*.toml`：文本资源
- `prompts/`：提示词模板
- `migrations/`：数据库迁移
- `UI/`：浏览器 UI
- `pi_app/`：桌面小程序和小屏监控
- `systemd/`：服务模板
- `crates/clawd`：API、路由、队列、记忆、调度
- `crates/skills/*`：技能实现

## 小屏桌面程序

小屏桌面程序位于 `pi_app/`。

```bash
cd pi_app && ./run-small-screen.sh
cd pi_app && ./install-desktop.sh
cd pi_app && ./enable-autostart.sh
cd pi_app && ./open-small-screen.sh
```

它依赖 `GET /v1/health`，所以需要先启动 `clawd`。

## 说明

- 生产部署前请检查并脱敏配置
- 如果 `/usr/local/bin` 不可写，请使用 `bash install-rustclaw-cmd.sh --user`
- 如果 `~/.local/bin` 不在 `PATH` 中，请加入 `export PATH="$HOME/.local/bin:$PATH"`
- 需要做 systemd 部署时，可从 `systemd/` 模板开始

## 许可证

本项目使用非商用、源码可见许可。

- 英文法律文本：`LICENSE`
- 中文参考翻译：`LICENSE.zh-CN.md`

# RustClaw

<img src="./RustClaw.png" width="420" />

RustClaw 是一个基于 Rust 的本地 Agent 运行栈。系统以 `clawd`（任务网关与执行编排）为核心，通过 Telegram / WhatsApp 适配器接入多通道消息，并支持技能执行、调度、记忆和多模态能力。

## 近期变更（相对旧版本）

- 新增大量技能模块（运维、日志、配置守护、服务控制、加密交易、多媒体等）。
- 引入 WhatsApp 双通道适配（Cloud API + Web Bridge）与统一通道路由设计。
- `clawd` 支持在同端口提供本地监控 UI（`UI/dist`）。
- 部分旧脚本已删除或被替换，不再维护：
  - `rollback.sh`
  - `setup-config.sh`
  - `script.py`
- 建议使用当前启动与打包脚本作为标准入口（见下方“脚本说明”）。

## 核心架构

- `crates/clawd`：HTTP API、任务队列、路由、调度、记忆、执行适配。
- `crates/claw-core`：共享配置、类型与错误模型。
- `crates/skill-runner`：技能进程宿主，负责调用各技能二进制。
- 消息适配器：
  - `crates/telegramd`
  - `crates/whatsappd`（Cloud API）
  - `crates/whatsapp_webd` + `services/wa-web-bridge`（WhatsApp Web）
- 技能实现：`crates/skills/*`
- 配置目录：`configs/`
- 数据与迁移：`data/`、`migrations/`

## 当前技能清单（workspace）

- `x`
- `system_basic`
- `http_basic`
- `git_basic`
- `install_module`
- `process_basic`
- `package_manager`
- `archive_basic`
- `db_basic`
- `docker_basic`
- `fs_search`
- `rss_fetch`
- `image_vision`
- `image_generate`
- `image_edit`
- `audio_transcribe`
- `audio_synthesize`
- `health_check`
- `log_analyze`
- `service_control`
- `config_guard`
- `crypto`

## API 与本地 UI

默认监听地址在 `configs/config.toml` 中配置（通常为 `127.0.0.1:8787`）。

- `GET /v1/health`：服务健康、队列与进程状态
- `POST /v1/tasks`：提交任务（`ask` / `run_skill`）
- `GET /v1/tasks/{task_id}`：查询任务结果
- `POST /v1/tasks/cancel`：按会话范围取消任务

示例：

```bash
curl http://127.0.0.1:8787/v1/health
curl -X POST http://127.0.0.1:8787/v1/tasks \
  -H "Content-Type: application/json" \
  -d '{"user_id":1,"chat_id":1,"kind":"ask","payload":{"text":"hello","agent_mode":true}}'
```

本地监控 UI：

- 地址：`http://127.0.0.1:8787/`
- 默认静态目录：`UI/dist`
- 可通过环境变量覆盖：`RUSTCLAW_UI_DIST`

## 快速开始

1) 安装 Rust 工具链

```bash
rustup default stable
```

2) 构建

```bash
./build-all.sh release
```

3) 启动核心服务（推荐）

```bash
./start-all.sh
```

4) 仅使用二进制启动（可选）

```bash
./start-all-bin.sh release
```

5) 按需启动适配器

```bash
./start-telegramd.sh
./start-whatsappd.sh
./start-whatsapp-webd.sh
./start-wa-web-bridge.sh
```

6) 查看日志

```bash
./check-logs.sh -n 120
```

## Telegram 常用命令

- `/start`, `/help`
- `/agent on|off`
- `/status`
- `/cancel`
- `/skills`
- `/run <skill> <args>`
- `/sendfile <path>`
- `/voicemode show|voice|text|both|reset`（admin）
- `/openclaw config show|vendors|set <vendor> <model>`（admin）

## 多媒体供应商支持（概览）

- `image_generate`：原生 `openai`、`google`；可选兼容模式 `anthropic`、`grok`
- `image_edit`：原生 `openai`、`google`；可选兼容模式 `anthropic`、`grok`
- `image_vision`：原生 `openai`、`google`、`anthropic`
- `audio_synthesize`：原生 `openai`、`google`；可选兼容模式 `anthropic`、`grok`
- `audio_transcribe`：原生 `openai`、`google`；可选兼容模式 `anthropic`、`grok`

兼容开关位于 `configs/config.toml`，默认均为 `false`：

- `image_generation.allow_compat_adapters`
- `image_edit.allow_compat_adapters`
- `audio_synthesize.allow_compat_adapters`
- `audio_transcribe.allow_compat_adapters`

## Crypto 技能（行情 + 洞察 + 交易防护）

`crypto` 支持：

- 行情：`quote`、`multi_quote`、`candles`、`indicator`
- 洞察：`onchain`（资讯由 `rss_fetch` 提供）
- 交易：`trade_preview`、`trade_submit`、`order_status`、`cancel_order`、`positions`

默认安全行为：

- 当 `crypto.require_explicit_send=true` 时，`trade_submit` 必须包含 `confirm=true`。
- 风控项可配置：`max_notional_usd`、`allowed_symbols`、`allowed_exchanges`、`blocked_actions`。
- 默认执行模式为 `cextest`（向后兼容别名：`paper`，写入 `data/crypto-paper-orders.jsonl`）。
- 独立配置文件：`configs/crypto.toml`。
- 支持实盘交易所：`binance`、`okx`（需在 `configs/crypto.toml` 中启用并完成配置）。

## 脚本说明（当前推荐入口）

- `build-all.sh`：构建 workspace 二进制（支持 profile 选择与校验）
- `start-all.sh`：一键启动（优先预编译二进制，缺失时回退源码启动）
- `start-all-bin.sh`：仅使用预编译二进制启动
- `start-clawd.sh`：启动 `clawd`
- `start-clawd-ui.sh`：构建 `UI/dist` 并启动 `clawd`
- `start-telegramd.sh`：启动 `telegramd`
- `start-whatsappd.sh`：启动 `whatsappd`
- `start-whatsapp-webd.sh`：启动 `whatsapp_webd`
- `start-wa-web-bridge.sh`：启动 WhatsApp Web bridge
- `start-future-adapters.sh`：启动未来适配器占位进程
- `stop-rustclaw.sh`：停止核心守护进程并清理 PID 文件
- `check-logs.sh`：查看/跟踪日志
- `simulate-telegramd.sh`：本地模拟 Telegram 向 `clawd` 的提交与轮询
- `package-release.sh`：构建发布包产物
- `copy_rustclaw_safe.sh`：安全复制项目用于部署/分发

## 目录参考

- `configs/config.toml`：主运行配置
- `configs/channels/*.toml`：通道配置文件
- `configs/command_intent/*.toml`：意图路由规则
- `configs/i18n/*.toml`：多语言文本资源
- `prompts/`：提示词模板
- `migrations/`：数据库迁移
- `systemd/`：服务部署模板
- `USAGE.md`：团队协作与上手补充文档

## 备注

- 生产部署前请检查并脱敏配置。
- 做服务化部署时，优先参考 `systemd/` 下的单元模板。

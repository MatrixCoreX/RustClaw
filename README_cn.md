# RustClaw

<img src="./RustClaw.png" width="420" />

RustClaw 是一个基于 Rust 的本地 Agent 运行时栈。它由 `clawd`（任务网关与执行编排）驱动，通过 Telegram / WhatsApp 适配器接入多通道消息，支持技能执行、调度、记忆和多模态能力。

## 核心架构

- `crates/clawd`：HTTP API、任务队列、路由、调度、记忆、执行适配器。
- `crates/claw-core`：共享配置、类型和错误模型。
- `crates/skill-runner`：技能进程宿主，负责调用技能二进制。
- 消息适配器：
  - `crates/telegramd`
  - `crates/whatsappd`（Cloud API）
  - `crates/whatsapp_webd` + `services/wa-web-bridge`（WhatsApp Web）
- 技能实现：`crates/skills/*`
- 配置：`configs/`
- 数据与迁移：`data/`、`migrations/`

## 技能参考（详细）

可以通过 Telegram `/run <skill> <json-args>` 或 Agent 自动路由触发技能。

- `archive_basic`：归档工作流；用于压缩、解压和归档内容清单，适合备份或部署包场景。
- `audio_synthesize`：文本转语音；把文本生成为可投递的语音文件。
- `audio_transcribe`：语音转文本；将音频内容转换为可读文字。
- `config_guard`：安全配置修改；强调最小变更、配置校验和敏感信息保护输出。
- `crypto`：行情、洞察和交易防护；支持 `quote`、`multi_quote`、`candles`、`indicator`、`onchain`、`trade_preview`、`trade_submit`、`order_status`、`cancel_order`、`positions`。
- `db_basic`：数据库基础操作；执行查询和受控的数据变更。
- `docker_basic`：容器运维；用于查看、日志、启动、停止、重启、镜像与 compose 诊断。
- `fs_search`：文件系统检索；递归搜索文件、路径过滤和快速定位任务。
- `git_basic`：仓库操作；支持状态、差异、分支、提交、拉取和合并辅助。
- `health_check`：健康诊断；汇总关键检查结果并给出下一步建议。
- `http_basic`：HTTP/API 探测；用于 GET/POST 调试和 webhook/API 联调验证。
- `image_edit`：图片编辑；按指令修改已有图片。
- `image_generate`：文生图；根据描述生成图片。
- `image_vision`：视觉理解；场景描述、OCR 抽取和差异对比。
- `install_module`：模块安装辅助；按生态识别并安装依赖模块。
- `log_analyze`：日志诊断；提炼关键错误、证据、可能原因和后续检查项。
- `package_manager`：包管理生命周期；按生态执行安装、升级、卸载和列表操作。
- `process_basic`：进程生命周期；查找、停止、重启并返回状态结果。
- `rss_fetch`：RSS 资讯抓取；按分类和来源层级抽取最新条目。
- `service_control`：服务控制；执行状态查看、启动、停止、重启并做后置状态检查。
- `system_basic`：系统基础巡检；系统信息、资源、网络和基础命令诊断。
- `x`：X 平台流程；草拟、改写并在确认后发布。

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

桌面小程序 / 小屏监控：

- 目录：`pi_app/`
- Python 桌面小程序前台启动：`cd pi_app && ./run-small-screen.sh`
- 安装桌面快捷方式：`cd pi_app && ./install-desktop.sh`
- 启用登录后自启动：`cd pi_app && ./enable-autostart.sh`
- 浏览器全屏打开网页版小屏：`cd pi_app && ./open-small-screen.sh`
- 桌面小程序读取 `GET /v1/health`，因此需要先启动 `clawd`
- 首次启动时，Python 小程序会自动生成一把本机专用 `user` key，并保存到 `pi_app/.rustclaw_small_screen_key`

## 快速开始（推荐使用 `rustclaw` 命令）

1) 前置条件

```bash
rustup default stable
python3 --version   # 建议 3.11+
```

2) 安装统一命令

```bash
# 标准安装（尝试安装到 /usr/local/bin，支持自动回退）
bash install-rustclaw-cmd.sh

# 无 sudo 环境（macOS/Linux/树莓派系统）
bash install-rustclaw-cmd.sh --user
```

安装后检查：

```bash
command -v rustclaw
rustclaw -h
rustclaw -status
```

Key 管理：

```bash
rustclaw -key list
rustclaw -key generate user
rustclaw -key generate admin
```

3) 构建并启动

```bash
# 使用 start-all 的完整能力启动
rustclaw -start --vendor openai --model gpt-4.1 --profile release --channels all --with-ui --quick
rustclaw -start --vendor qwen --model qwen-max-latest --profile release --channels all --quick
rustclaw -start --vendor custom --model custom-model --profile release --channels all --quick

# 简化启动
rustclaw -start release all
```

4) 日常运维

```bash
rustclaw -status
rustclaw -logs clawd 200 --follow
rustclaw -health
rustclaw -stop
```

5) 传统脚本模式（仍支持）

```bash
./start-all.sh
./stop-rustclaw.sh
```

## Telegram 常用命令

- `/start`, `/help`
- `/agent on|off`
- `/status`
- `/cancel`
- `/skills`
- `/run <skill> <args>`
- `/sendfile <path>`
- `/voicemode show|voice|text|both|reset`（管理员）
- `/openclaw config show|vendors|set <vendor> <model>`（管理员）

## 技能行为说明

- 多媒体兼容开关已拆分到独立配置文件，默认值均为 `false`：
  - `image_generation.allow_compat_adapters`
  - `image_edit.allow_compat_adapters`
  - `audio_synthesize.allow_compat_adapters`
  - `audio_transcribe.allow_compat_adapters`
- 配置拆分与优先级：
  - `configs/config.toml`：全局基础配置
  - `configs/image.toml`：图片技能配置（`image_edit` / `image_generation` / `image_vision`）
  - `configs/audio.toml`：语音技能配置（`audio_synthesize` / `audio_transcribe`）
  - 运行时同名键优先级：`config.toml` 显式值优先，拆分文件作为默认补充。
- 原生适配模型路由：
  - `configs/image.toml` / `configs/audio.toml` 中的 `native_models` 用来控制哪些 Qwen 模型在 `auto` 模式下优先走原生适配。
  - 如果模型不在 `native_models` 中，且允许 compat，则 RustClaw 会优先走 compat 适配。
- 图片/语音模型选择优先级：
  - `request.model > default_model > <vendor>_models[0] > models[0] > llm.<vendor>.model`
  - 原生/兼容通道选择独立于模型优先级，由 `native_models` 和 `adapter_mode` 决定。
- Crypto 安全默认策略：
  - 当 `crypto.require_explicit_send=true` 时，`trade_submit` 需要包含 `confirm=true`。
  - 主要风控字段：`max_notional_usd`、`allowed_symbols`、`allowed_exchanges`、`blocked_actions`。
  - 默认执行交易所是 `binance`，实盘支持 `binance` 与 `okx`。

## 脚本参考（推荐入口）

- `rustclaw`：统一运行命令（`-start/-stop/-restart/-status/-logs/-health/-build/-h`）
- `install-rustclaw-cmd.sh`：安装 `rustclaw` 命令（跨平台参数：`--user`、`--dir`、`--force-build`）
- `build-all.sh`：构建 workspace 二进制（支持 profile 选择和结果校验）
- `start-all.sh`：一键启动（优先预编译二进制，缺失时回退源码构建）
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
- `simulate-telegramd.sh`：本地模拟 Telegram 到 `clawd` 的提交与轮询流程
- `package-release.sh`：构建发布包产物
- `copy_rustclaw_safe.sh`：安全复制项目用于部署/分发
- `pi_app/run-small-screen.sh`：前台启动 Python 桌面小程序，适合调试
- `pi_app/run-small-screen-launcher.sh`：桌面图标/自启动使用的启动器，会补全图形环境变量
- `pi_app/install-desktop.sh`：创建 `~/Desktop/RustClaw.desktop`
- `pi_app/enable-autostart.sh`：启用桌面小程序开机自启动
- `pi_app/disable-autostart.sh`：取消桌面小程序开机自启动
- `pi_app/open-small-screen.sh`：全屏打开网页版小屏

## 跨平台说明

- 目标平台：Linux、Ubuntu、Debian/Raspberry Pi OS、macOS。
- 若 `/usr/local/bin` 无写权限，建议使用：
  - `bash install-rustclaw-cmd.sh --user`
- 若 `~/.local/bin` 不在 `PATH` 中，请加入：
  - `export PATH="$HOME/.local/bin:$PATH"`
- 启动脚本依赖 Python TOML 解析（`tomllib`），建议使用 Python `3.11+`。

## 目录参考

- `configs/config.toml`：主运行配置
- `configs/image.toml`：图片技能配置（默认 + 厂商备选）
- `configs/audio.toml`：语音技能配置（默认 + 厂商备选）
- `configs/channels/*.toml`：通道配置文件
- `configs/command_intent/*.toml`：意图路由规则
- `configs/i18n/*.toml`：国际化文本资源
- `prompts/`：提示词模板
- `migrations/`：数据库迁移
- `pi_app/`：桌面小程序 / 树莓派小屏监控
- `systemd/`：服务部署模板
- `USAGE.md`：团队协作和上手补充文档

## 备注

- 生产部署前请检查并脱敏配置。
- 进行服务化部署时，优先参考 `systemd/` 下的单元模板。

## 许可证

本项目采用“源码可见、禁止商业使用”的许可条款：

- 英文法律文本：`LICENSE`
- 中文说明版本：`LICENSE.zh-CN.md`

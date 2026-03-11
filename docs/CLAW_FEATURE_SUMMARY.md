# Claw 功能总结（简短版）

## 定位

RustClaw 是本地 Agent 运行时：**clawd** 做任务网关与执行编排，多通道接入（Telegram / WhatsApp / UI），支持技能调用、调度、记忆、多模态，身份以 **user_key** 为主。

## 核心组件

| 组件 | 作用 |
|------|------|
| **clawd** | HTTP API、任务队列、意图路由、调度、记忆、Agent 执行（拆步→执行→resume） |
| **claw-core** | 共享配置、类型、错误模型 |
| **skill-runner** | 技能进程宿主，按请求拉起各 skill |
| **telegramd / whatsappd / wa-web-bridge** | 各通道 Bot 与消息转发 |
| **crates/skills/*** | 各技能实现 |

## 身份与通道

- 主身份：**user_key**（admin/user）。Telegram / WhatsApp / UI 绑定到 key，不各自维护永久 ID。
- 会话：按 `channel + external_chat_id` 区分；凭证、权限按 **user_key**。
- Key 管理：`rustclaw -key list/generate`、`scripts/auth-key.sh`；鉴权表为空时可自动生成首个 admin key。

## 任务与 Agent

- **ask**：用户文本 → 意图解析（Chat / Act / ChatAct / AskClarify）→ LLM 或 Agent 执行。
- **Agent（Act）**：单轮规划拆成多步（工具/技能/回复），顺序执行；**不再向用户发送拆分步骤列表**，直接执行并返回最终结果与进度。
- **中断与继续**：执行失败会保存 `resume_context`；用户说「继续」「为什么失败」等由 LLM 分类为 resume/defer/abandon，resume 只执行剩余步骤。
- **run_skill**：直接调用指定技能，可带 user_key、exchange_credentials 等 context。

## 技能一览

- **系统与运维**：system_basic、process_basic、service_control、health_check、config_guard、docker_basic、package_manager、install_module
- **文件与搜索**：archive_basic、fs_search、git_basic、log_analyze
- **网络与数据**：http_basic、db_basic、rss_fetch
- **多模态**：image_vision、image_generate、image_edit、audio_transcribe、audio_synthesize
- **对话与交易**：chat、crypto、x（X/Twitter）

## 主要 API（需 X-RustClaw-Key）

- `GET /v1/health` — 健康与队列
- `POST /v1/tasks` — 提交 ask / run_skill
- `GET /v1/tasks/{id}` — 查询结果
- `POST /v1/tasks/cancel` — 按会话取消
- `POST /v1/auth/ui-key/verify`、`GET /v1/auth/me`、channel bind/resolve
- `GET/POST /v1/auth/crypto-credentials` — 用户交易所 API 凭证

## 配置与启动

- 主配置：`configs/config.toml`（数据库、LLM 默认 vendor/model、限流等）。
- 技能/图片/音频等：`configs/crypto.toml`、`configs/image.toml`、`configs/audio.toml` 等。
- 启动：`clawd` 提供 API；按需启动 `telegramd`、`start-whatsappd`、`start-wa-web-bridge` 等；本地监控 UI 默认 `http://<host>:8787/`（静态目录 `UI/dist`）。

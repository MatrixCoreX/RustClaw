# Rust Claw（树莓派优先 / 默认 1GB）终极开发规格说明（给 Cursor/Codex）

## 项目目标
用 Rust 重写一个轻量版 “claw”。  
优先适配树莓派本机系统（Raspberry Pi OS / Ubuntu for Pi）。  
默认以 1GB 内存为最低基线设计与测试。  
实际也支持更大内存（2/4/8GB）通过配置提升吞吐。  
不包含网页抓取与浏览器自动化。  
支持 Telegram 作为主要交互入口。  
支持连接多种大模型（优先 OpenAI-compatible 接口）。  
保证长期稳定运行，避免 OOM，避免资源无界增长。  

## 核心原则
核心服务常驻、轻量、可控。  
所有外部请求必须有 timeout、重试、熔断、限流。  
默认并发极低（针对 1GB），通过配置扩展（针对大内存）。  
技能必须隔离（外部进程/沙箱），避免单个技能拖垮主进程。  
队列、缓存、日志、数据库都必须“有上限 + 可清理”。  
Telegram 渠道只做收发、鉴权、转发，不做重逻辑。  

---

## 运行环境与资源策略（重要：1GB 优先）
### 目标平台
默认以树莓派本机系统为第一优先（Raspberry Pi OS / Ubuntu for Pi 均可）。  
默认以 1GB 内存机型为最低基线进行设计与测试。  
在更大内存（2/4/8GB）设备上允许通过配置提升并发与缓存，但不改变架构。  

### “1GB 优先”的设计约束
所有模块必须在 1GB 设备上长期运行不崩溃。  
默认配置必须满足：空闲常驻内存低、峰值可控、无无限增长。  
默认并发必须极低（worker=1，LLM 并发=1，技能并发=1）。  
任何可能产生大内存峰值的功能必须默认关闭或严格限制。  
所有请求必须有 timeout，所有任务必须有 task_timeout。  
所有队列、缓存、日志必须有上限与淘汰策略。  

### 面向“大内存”的扩展方式
支持通过 config.toml 调整 worker 并发、LLM 并发、技能并发、缓存上限。  
支持通过 config.toml 开启可选的内存缓存（默认关闭或小配额）。  
支持通过 config.toml 调整队列批处理参数与轮询间隔。  
扩展只允许“加资源提升吞吐”，不允许依赖大内存才能正确运行。  

### 运行时自适应（可选但推荐）
启动时读取系统内存总量（/proc/meminfo）。  
根据内存档位选择默认 profile（1g/2g/4g/8g）。  
profile 只影响并发与缓存上限，不影响功能正确性。  

建议档位：  
1g：worker=1，llm_max_concurrency=1，skill_max_concurrency=1，cache=off，queue_limit=小。  
2g：worker=1-2，llm=1-2，skill=1-2，cache=小。  
4g：worker=2-4，llm=2-4，skill=2-4，cache=中。  
8g：worker=4-8，llm=4-8，skill=4-8，cache=大。  

### systemd 资源限制（必须提供默认值）
默认启用 MemoryMax（1GB 机型下建议 300-450MB，按实际压测调整）。  
默认启用 TasksMax 限制子进程数量，避免 fork 风暴。  
默认启用 Restart=always 与 RestartSec=2。  
允许在大内存机器上通过覆盖配置放宽 MemoryMax。  

### 持久化与增长控制（必须）
SQLite 必须设置 busy_timeout 并避免长事务。  
tasks 表必须有清理策略（例如保留最近 N 天或最近 N 条）。  
audit_logs 必须有滚动与清理策略。  
日志文件必须滚动（按大小/按天），并限制最大保留量。  

---

## 进程划分
### 1）clawd（核心守护进程）
提供本机 HTTP API（优先 Unix Domain Socket，其次 127.0.0.1）。  
负责：配置加载、鉴权二次校验、任务队列、状态机、限流、审计、LLM 路由、结果聚合。  
负责：与 SQLite 交互（用户、任务、审计、配置快照）。  
负责：调用 skill-runner 执行技能并收集输出。  

### 2）telegramd（Telegram 渠道进程）
使用 Telegram Bot API（Rust：teloxide）。  
负责：收消息、解析命令、最小鉴权（allowlist + admin）。  
负责：把请求转为任务提交到 clawd。  
负责：轮询或长轮询任务结果并回复用户。  
不在 telegramd 内做复杂逻辑，不做 LLM 调用，不跑技能。  

### 3）skill-runner（技能执行器）
以“外部进程协议”运行每个技能。  
输入从 stdin 读一行 JSON。  
输出向 stdout 写一行 JSON。  
技能崩溃不影响 clawd。  
必须支持：超时终止、最大并发限制、失败原因回传。  

---

## 目录结构（Cargo Workspace）
workspace 根目录：  
- crates/claw-core（共用类型、配置、错误、工具）  
- crates/clawd（核心守护进程）  
- crates/telegramd（Telegram 渠道）  
- crates/skill-runner（技能执行器）  
- crates/skills/*（内置技能集合）  
- configs/config.toml（示例配置）  
- migrations（SQLite 初始化）  
- systemd（服务文件）  
- README.md  

---

## 技术栈建议
异步：tokio。  
HTTP Server：axum。  
HTTP Client：reqwest。  
序列化：serde + serde_json。  
配置：config + toml。  
日志：tracing + tracing-subscriber。  
SQLite：优先 rusqlite（更轻）或 sqlx（更现代但更重）。  
UUID：uuid。  
时间：time 或 chrono（尽量轻量）。  

---

## 数据库设计（SQLite）
### 表 1：users
user_id（telegram id，PRIMARY KEY）。  
role（admin/user）。  
is_allowed（bool）。  
created_at。  
last_seen。  

### 表 2：tasks
task_id（uuid，PRIMARY KEY）。  
user_id。  
chat_id。  
message_id（可选）。  
kind（ask/run_skill/admin）。  
payload_json（TEXT）。  
status（queued/running/succeeded/failed/canceled/timeout）。  
result_json（TEXT，可空）。  
error_text（TEXT，可空）。  
created_at。  
updated_at。  

### 表 3：audit_logs
id（INTEGER PRIMARY KEY AUTOINCREMENT）。  
ts。  
user_id（可空）。  
action（submit_task/run_llm/run_skill/auth_fail/limit_hit/timeout/…）。  
detail_json（TEXT，可空）。  
error_text（TEXT，可空）。  

### 数据清理策略（必须实现）
tasks：保留最近 N 天或最近 N 条（可配置），定时清理。  
audit_logs：按天或按条数清理（可配置）。  

---

## clawd 对外 API（本机）
统一返回 JSON：  
- ok（bool）  
- data（object，可空）  
- error（string，可空）  

### POST /v1/tasks
提交任务。  
请求：user_id、chat_id、kind、payload。  
返回：task_id。  

### GET /v1/tasks/{task_id}
查询任务状态与结果。  
返回：status、result_json、error_text。  

### GET /v1/health
健康检查。  
返回：version、queue_length、worker_state、uptime、(可选)memory_rss、(可选)telegramd_healthy、(可选)telegramd_process_count。  

### GET /v1/config
返回当前加载的配置概要（隐藏敏感 key）。  

---

## Telegram 命令设计
必须支持最小集合：  
/start 输出帮助与权限提示。  
/help 输出命令说明。  
/ask <text> 使用默认模型回答。  
/ask --model <name> <text> 指定模型回答。  
/skills 列出可用技能。  
/run <skill_name> <args> 运行技能。  
/status 查看队列与系统状态。  
/admin allow <user_id>（仅管理员）加入 allowlist。  
/admin deny <user_id>（仅管理员）移出 allowlist。  

---

## 权限模型
默认拒绝所有用户（deny by default）。  
仅 allowlist 用户可用。  
管理员列表从 config.toml 加载。  
telegramd 必须先做 allowlist 判断，拒绝直接回复“未授权”。  
clawd 必须做二次校验，避免被绕过。  
所有 auth_fail 写入 audit_logs。  

---

## 多模型接入（LLM Provider 层）
### 总体策略
优先实现 OpenAI-compatible Provider：  
通过 base_url + api_key + model 实现多供应商切换。  
后续可扩展 Anthropic/Gemini/Bedrock 等专用 Provider（非本阶段必需）。  

### 统一接口（Trait）
定义 LlmProvider trait：  
输入统一为 LlmRequest：  
- messages（role/content）  
- temperature（可选）  
- max_tokens（可选）  
- stream（默认 false，本阶段可不做流式）  
- metadata（user_id/chat_id/task_id，用于审计与追踪）  

输出统一为 LlmResponse：  
- text  
- usage（可选）  
- finish_reason（可选）  
- raw（可选，调试用，默认关闭）  

### 路由与容错（必须实现）
实现 LlmRouter：  
按模型名选择 provider 配置。  
支持 priority 与 fallback 列表。  
失败条件包含：timeout、5xx、429、解析失败。  
每次失败写 audit_logs（包含 provider、model、错误类型）。  

### 限流与并发（必须）
每个 provider/model 配置 max_concurrency。  
每个 user 配置每分钟请求上限（rpm）。  
全局也有并发与 rpm 上限。  
超限必须返回友好错误，并写 audit_logs（limit_hit）。  

---

## 配置文件（config.toml）要求
### server
listen（默认 127.0.0.1:port 或 unix_socket_path）。  
request_timeout_seconds。  

### telegram
bot_token。  
admins（数组，telegram user_id）。  

### database
sqlite_path。  
busy_timeout_ms。  

### worker
concurrency（默认 1）。  
task_timeout_seconds（默认 60 或更保守）。  
poll_interval_ms。  
queue_limit（最大排队数，默认小）。  

### llm.providers（数组）
每项包含：  
name。  
type = "openai_compat"。  
base_url。  
api_key。  
model。  
priority。  
timeout_seconds。  
max_concurrency。  

### skills
skills_dir 或 skills_list（示例技能注册方式）。  
skill_timeout_seconds。  
skill_max_concurrency。  

### logging
level。  
file_path（可选）。  
rotate（按大小/按天）。  
max_size_mb。  
max_files。  

### profiles（可选）
profile_auto（true/false）。  
profile_override（"1g"/"2g"/"4g"/"8g"）。  
每个 profile 可覆盖 worker/llm/skills/cache 参数。  

---

## 技能系统（外部进程协议）
### 输入 JSON（stdin，一行）
request_id。  
user_id。  
chat_id。  
skill_name。  
args（string 或 json）。  
context（可选：语言、时区、来源命令）。  

### 输出 JSON（stdout，一行）
request_id。  
status（"ok"|"error"）。  
text（给用户的回复）。  
buttons（可选：inline keyboard 定义）。  
extra（可选：结构化数据）。  
error_text（可选）。  

### 超时与资源限制（必须）
clawd 调用 skill-runner 必须设置超时。  
超时后 kill 子进程并标记 timeout。  
默认 skill_max_concurrency=1。  
必要时支持 systemd/cgroup 限制技能子进程资源（可选）。  

### X skill（新增约束）
用途：向 X 发帖（tweet.create）。  
技能名：`x`。  
参数：`text`（必填）、`dry_run`（可选）、`send`（可选）。  
默认安全策略：未显式 `send=true` 时仅预览，不真实发帖。  
环境变量：  
- `X_API_TOKEN`：真实发帖必需。  
- `X_API_BASE_URL`：默认 `https://api.x.com/2`。  
- `X_REQUIRE_EXPLICIT_SEND`：默认开启（推荐保持开启）。  
- `X_MAX_TEXT_CHARS`：默认 280。  
返回要求：失败时必须回传 `error_text`，便于 clawd/telegramd 展示真实错误。  

---

## 运行与部署（systemd）
提供：clawd.service、telegramd.service。  
Restart=always。  
RestartSec=2。  
TasksMax（限制进程/线程数量）。  
MemoryMax（1GB 默认 300-450MB，可覆盖）。  
WorkingDirectory 与 ExecStart 指向 release 二进制。  
环境变量仅用于注入 token（也可用 config）。  

---

## 里程碑（按顺序交付）
### Milestone 1：基础跑通
clawd 启动。  
SQLite 初始化 + migrations。  
telegramd 能接收 /start 并回消息。  
allowlist 生效（默认拒绝）。  

### Milestone 2：任务队列
telegramd 把 /ask 转 task 提交 clawd。  
clawd worker 从 tasks 表取任务执行并写回结果。  
telegramd 轮询结果并回复。  

### Milestone 3：LLM OpenAI-compatible
实现 openai_compat provider。  
支持 base_url + api_key + model。  
实现 timeout、重试、fallback。  
实现错误分类（429/5xx/timeout/parse）。  

### Milestone 4：技能系统
实现 skill-runner。  
实现内置技能执行与扩展。  
实现 /skills 与 /run。  
实现超时终止与错误回传。  

### Milestone 5：稳定性强化
实现全局与用户限流。  
实现日志滚动与限速。  
实现 /status 输出队列长度与运行状态。  
audit_logs 覆盖关键路径。  
实现 tasks/audit_logs 清理任务。  

---

## 性能与内存约束（必须达标）
默认 profile=1g：  
clawd 空闲常驻内存目标 < 120MB。  
telegramd 空闲常驻内存目标 < 80MB。  
worker=1。  
llm_max_concurrency=1。  
skill_max_concurrency=1。  

所有请求默认 timeout 30s（可配置）。  
队列长度、日志量、数据库体积必须可控，不允许无限增长。  

---

## 测试要求
单元测试覆盖 LlmRouter fallback 行为。  
集成测试模拟：提交任务 -> 执行 -> 查询结果。  
错误测试覆盖：timeout、429、provider 解析失败、技能崩溃。  

---

## 交付物
完整可编译的 Cargo workspace。  
示例 config.toml（包含 1g 默认 profile 与大内存 profile 示例）。  
SQLite migrations 文件。  
systemd 服务文件。  
README（安装、运行、升级、排错、资源配置建议）。  

---

## Image 三模块（新增）
新增三类技能并遵循现有 skill-runner 一行 JSON 协议：  
- image_vision：看图理解、结构化提取、多图对比、截图要点。  
- image_generate：prompt 生成图片并写入默认输出目录。  
- image_edit：基于原图 + 指令进行改图、扩图、换风格、增删元素。  

统一约束：  
- 入参支持 vendor/model/timeout_seconds 覆盖。  
- 统一返回 status/text/extra（provider/model/outputs）。  
- 默认输出目录来自 config.toml 中 image_* 段。  
- 具备能力检测与降级（不支持动作时明确报错或 fallback）。  
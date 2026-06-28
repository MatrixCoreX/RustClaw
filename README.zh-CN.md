# RustClaw

<img src="./RustClaw.png" width="420" />

英文版：`README.md`

RustClaw 是一个以 `clawd` 为核心的本地 Rust Agent Runtime。它把多通道接入、任务执行、技能路由、记忆、调度、浏览器 UI，以及基于 `user_key` 的身份体系整合到一套可部署系统里。

## 项目概览

RustClaw 面向“消息端或浏览器里就能完成日常使用和管理”的场景，而不是只给命令行使用者。

当前仓库的主要能力包括：

- 多通道接入：Telegram、微信、飞书、Lark、WhatsApp Cloud、WhatsApp Web、浏览器 UI，以及可选的 `webd`
- 由 `clawd` 提供任务运行时、HTTP API、路由、记忆和调度
- 共享技能调度层，支持进程内 builtin、external adapter，以及通过 `skill-runner` 拉起的 runner 子进程
- 覆盖系统、文件、网络、图片、语音、视频、音乐、加密货币、知识库、自动化等场景的 builtin、external 与 runner 技能
- 本地浏览器控制台位于 `UI/`，其中包含独立的 NNI 设备签名页面
- 树莓派/小屏桌面程序位于 `pi_app/`

## Agent Loop 架构

RustClaw 主自然语言路径在 `semantic_route_authority = "agent_loop_default"` 下默认使用接近 Codex / Claude 的 agent loop。Boundary layer 负责把本轮绑定到身份与会话状态，构建结构化路由信号，并应用 locator、契约、安全、确认、dry-run、预算、能力和证据护栏；之后把普通语义决策交给 agent loop：回复、调用能力、按证据合成、修复、继续或停止。缺少必要信息时，由 boundary/finalizer 澄清路径处理。意图归一化器只是初始结构化提示，不是最终语义权威。旧 pre-agent 路由现在只是显式 `semantic_route_authority = "legacy"` / rollback 兼容路径，不再是普通 chat fallback 的默认路线。

### 请求与 Agent Loop 流程

```mermaid
flowchart TD
    A[通道 / UI / API 请求] --> B[POST /v1/tasks]
    B --> C[持久化任务并入队]
    C --> D[返回 task_id<br/>调用方可轮询]
    D --> E0[worker_once 恢复 tick<br/>stale running + due checkpoint]
    E0 --> E1[认领下一个 queued 任务]
    E1 --> E{任务类型}
    E -->|run_skill| RS[直接 run_skill 路径<br/>只接受显式 skill_name]
    E -->|ask| F[解析身份 + 会话 + 活跃任务]
    F --> G[意图归一化<br/>仅结构化提示]
    G --> H[Ask 上下文包<br/>记忆 + 附件 + 最近执行]
    H --> I[Boundary 护栏<br/>locator + contract + safety + budget + compat]
    I -->|agent-loop 权威| J[Agent-loop 语义权威]
    I -->|调度 / 澄清 / 机器事实快路径 / 回滚| K[Boundary-owned 收尾路径]
    J --> L{循环轮次}
    L -->|确定性观测契约| M[运行时构建观测计划]
    L -->|一般工作| N[Planner LLM<br/>优先 call_capability]
    M --> O[CapabilityResolver]
    N --> O
    O --> P[PlanVerifier<br/>permission_decision + 风险 + 效果 + 契约]
    P --> Q{已验证步骤}
    Q -->|respond| R[终端回复]
    Q -->|synthesize_answer| S[基于证据合成]
    Q -->|call_tool / call_skill| QP[Pre-tool hooks + adapter preflight<br/>policy_decision + contract args]
    QP -->|long-tail async_start| AS[Async media/job adapter<br/>pending_async_job + poll/cancel contract + checkpoint]
    AS --> ASP[进度机器回复<br/>checkpoint_id + poll_ref + next_check_after + can_poll/can_cancel]
    QP -->|call_tool| T[工具执行]
    QP -->|call_skill| U[共享技能调度]
    RS --> RSG[不经过 normalizer / planner / resolver 选择<br/>不做 PlanVerifier 语义选择]
    RSG --> U
    T --> V[观测结果]
    U --> V
    S --> V
    V --> W[证据覆盖 + 答案形状检查]
    W -->|修复 / 缺证据| WR[RepairEnvelope<br/>issue codes + attempt ledger]
    WR --> J
    W -->|完成| X[观测输出收尾]
    R --> Y[用户可见消息组装]
    K --> Y
    X --> Y
    Y --> Z[输出契约护栏 + 任务结果]
    Z --> AA[通道交付]
    Z --> AB[Journal + 会话更新]
    AB --> AD[Task event stream<br/>状态迁移 + checkpoint + 工具生命周期 + coding checkpoint/evidence]
    ASP --> AA
    ASP --> AB
    ASP --> AD
    AD --> AE[CLI / UI watch + report]
    Z -. 可选 .-> AC[后台记忆刷新]
```

- `POST /v1/tasks`：通道守护进程、浏览器 UI 和 HTTP 调用者都收敛到同一套持久化任务队列。
- `task_id polling`：API/通道请求的等待超时只影响调用方等多久；后台任务仍可通过 `GET /v1/tasks/{task_id}` 查询，除非 worker 生命周期逻辑已经把它标记为终态。
- `worker_once recovery tick`：worker 认领新 queued 任务前，会先检查 stale running、受保护 paused checkpoint、到期恢复任务、async poll 结果和结果投影。
- `Task kind`：`kind=ask` 进入可使用 agent 的自然语言路径；`kind=run_skill` 绕过 intent normalizer、planner loop、capability resolver 和 plan verifier，只把显式提供的 `payload.skill_name` 交给共享 skill dispatcher / 协议执行。两种 task kind 都会把结果写回原始 `task_id`，调用方仍可通过 task 查询 API 查看最终状态。

### Ask 与 Run Skill 边界

这里需要明确区分，因为 `run_skill` 是 API 层任务类型，不是自然语言路由捷径。

直接技能任务的关键点：

- `kind=run_skill` 不运行 intent normalizer，也不进入 planner / agent loop；调用方已经提供了 `payload.skill_name` 和参数。
- `kind=run_skill` 接受显式技能名后，仍使用共享 skill dispatcher 和技能协议。
- `kind=run_skill` 仍创建和更新普通 task row，因此最终状态和结果仍可通过 `task_id` 查询。

| 问题 | `kind=ask` | `kind=run_skill` |
| --- | --- | --- |
| 是否运行 intent normalizer？ | 是，作为结构化提示和兼容输入。 | 否。调用方已经提供目标技能。 |
| 是否进入 planner / agent loop？ | 普通自然语言工作默认进入；boundary-owned 澄清、调度和机器事实快路径可不经过 planner round 直接收尾。 | 否。不会让 planner 选择技能或 action。 |
| 是否用 `CapabilityResolver` / `PlanVerifier` 做语义选择？ | 是，planner step 会先解析和验证再执行。 | 否，不做语义选择；显式 skill call 仍走调度和协议校验。 |
| 是否使用共享 skill dispatcher？ | 是，planner 选择 `call_skill` 或 capability 解析到 skill 时使用。 | 是。把 `payload.skill_name` 派发到同一套 builtin / external / runner 技能协议。 |
| 结果是否能用 `task_id` 查询？ | 是。 | 是。直接技能结果保存到原始 task row，可通过 `GET /v1/tasks/{task_id}` 或 `clawcli get` 读取。 |

操作上：用户给自然语言请求时使用 `kind=ask`，让 RustClaw 自己判断回答、澄清、规划或执行。API 调用方已经知道明确技能和参数时使用 `kind=run_skill`，只把 RustClaw 当作任务队列、鉴权、生命周期和结果投影层来运行该技能。

- `Intent normalizer`：产出结构化提示和兼容 trace 字段；对普通 eligible 工作，它不是最终语义权威。
- `Boundary guards`：绑定身份/会话状态，并基于机器字段应用 locator、contract、safety、budget、confirmation、dry-run 和兼容检查。该层应保持轻量，不能继续增加按语言维护的短语逻辑。
- `Agent-loop 语义权威`：普通自然语言工作默认进入循环，由 planner/runtime 决定回复、调用能力、执行工具或技能、按证据合成、修复或停止。
- `CapabilityResolver / PlanVerifier`：把 `call_capability` 解析到当前 tool 或 skill 实现，再检查可见性、必填参数、allowed action、risk/effect、confirmation 和输出契约。
- `permission_decision`：verifier 和 preflight blocker 输出 `allowed`、`needs_confirmation`、`denied_by_policy`、`dry_run_required`、`external_provider_blocked`、`risk_level`、`action_effect`、registry dedup/idempotency 等机器字段。UI、API、finalizer 和 i18n 应消费这些字段渲染说明，而不是解析 runtime prose。
- `Async job start`：长尾工具可以先发布包含 `checkpoint_id`、`poll_ref`、`next_check_after`、`can_poll`、`can_cancel` 的机器回复，同时任务仍可通过 checkpoint 轮询恢复。媒体技能通过 registry capability 暴露这类形状，例如 `image.generate` / `image.poll` / `image.cancel`、`audio.synthesize` / `audio.poll` / `audio.cancel`、`video.generate` / `video.poll` / `video.cancel` 和 `music.generate` / `music.poll` / `music.cancel`。
- `Evidence coverage`：工具、技能和合成输出都会成为循环内观测；缺证据或可恢复失败会带着压缩的已尝试方法历史回到循环。
- `RepairEnvelope`：repair 是有边界的循环内恢复。运行时提供 `repair_source`、`issue_codes`、`missing_evidence`、`permission_decision`、`provider_status`、`attempt_fingerprint`、`side_effect_fingerprint`、`checkpoint_id`、`next_recovery_kind` 等机器字段；planner/finalizer 可以据此重新规划、澄清、转后台等待或结构化失败，而不是解析本地化 prose。
- `Observed-output finalizer`：只有答案形状与证据契约满足后，才发布有观测依据的结果。
- `Output-contract guard`：保存结果前规范最终文本、`messages` 数组、文件 token、标量/严格输出形状和通道交付一致性。
- `Journal + session update`：任务状态、观测事实和活跃会话锚点在收尾后持久化；后台记忆任务是可选、非阻塞的。
- `Task event stream`：journal trace 事件暴露机器可读进度，例如 `task_transition`、`checkpoint_created`、`tool_started`、`tool_step`、`tool_finished`、`coding_checkpoint`、`coding_evidence`、`provider_call`、`agent_hook`、`subagent` 和 `task_final`。CLI 与 UI 直接渲染这些字段，包括 `evidence_ref`、`checkpoint_ref`、`checkpoint_kind`、`pending_async_job_id`、coding 计数、验证命令计数/token、验证状态/失败类别 token、未验证风险 token 和 step 时间字段，不读取原始日志或本地化文本来判断状态。

### Planner、LLM 与 Capability 流程

```mermaid
flowchart TD
    A[用户本轮输入] --> B[Normalizer prompt]
    B --> C[LLM: 结构化路由提示]
    C --> D[解析 schema 字段]
    D --> E[Ask 上下文包]
    E --> F[Boundary 护栏<br/>只消费机器字段]
    F -->|agent-loop 权威| G[Agent-loop context]
    F -->|兼容 / 调度 / 机器事实快路径| H[兼容收尾路径]
    G --> I{轮次来源}
    I -->|运行时可证明计划| J[确定性计划]
    I -->|runtime async command contract| JA[确定性 async job 计划<br/>start / poll / cancel]
    I -->|需要推理| K[LLM: planner round]
    K --> L[Plan JSON steps]
    J --> M[CapabilityResolver]
    JA --> M
    L --> M
    N[技能注册表<br/>planner_capabilities] --> M
    O[生成的 INTERFACE prompts] --> K
    M --> P[PlanVerifier<br/>schema + permission_decision + effect]
    P --> Q{Step}
    Q -->|call_capability| R[解析后的 tool 或 skill]
    Q -->|call_tool / call_skill| QA[Pre-tool hooks + adapter preflight]
    R --> QA
    QA -->|runtime async marker| AR[允许 async_start + 移除内部 marker]
    QA -->|subagent tool| SS[有界只读子代理 batch<br/>role/config + aggregation]
    QA -->|call_tool| S[Tool executor]
    QA -->|call_skill| T[Skill dispatcher]
    AR --> T
    T --> U{Skill kind}
    U -->|builtin| V[进程内 builtin]
    U -->|external| W[External adapter]
    U -->|runner| X[skill-runner 子进程]
    X --> Y[具体技能二进制<br/>单行 JSON 协议]
    SS --> Z
    S --> Z[Observation]
    V --> Z
    W --> Z
    Y --> Z
    Z --> ZEV[Task journal event<br/>工具生命周期 + coding checkpoint/evidence + evidence refs]
    Q -->|synthesize_answer| ZA[LLM: 基于证据合成]
    Q -->|respond| ZB[终端回复]
    ZA --> ZC[Evidence coverage]
    ZEV --> ZC
    ZC -->|需要修复| ZR[RepairEnvelope<br/>bounded recovery signal]
    ZR --> G
    ZC -->|完成| ZD[Observed-output finalizer]
    ZB --> ZE[Output-contract guard]
    ZD --> ZE
    H --> ZE
    ZE --> ZF[持久化结果 + 交付]
```

- `Normalizer prompt`：让 LLM 阅读本轮用户输入并输出 schema 字段。运行时把这些字段当作提示和契约消费，而不是匹配用户短语。
- `Planner prompt`：只在循环轮次需要模型推理时构建；窄范围观测契约可直接使用运行时构建的计划。
- `call_capability`：推荐的 planner action，把 tool/skill 选择放到 registry metadata 与 resolver policy 后面。
- `Generated INTERFACE prompts`：来自 `crates/skills/*/INTERFACE.md`、`external_skills/*/INTERFACE.md` 和 `prompts/layers/generated/skills/*`；新增技能应改这些契约，不改 `clawd` 主流程分支。
- `Command payload contract repair`：声明了 command payload 的任务会按需要归一到 `RawCommandOutput` 或 `CommandOutputSummary` 机器契约，包括上游提示误标成 service-status 的情况。
- `PlanVerifier`：执行前阻断不可用能力、缺必填字段、不安全 mutation，以及不符合输出/证据形状的计划。拒绝路径应携带稳定机器字段，不写固定用户可见回复模板。
- `Pre-tool hooks + adapter preflight`：循环执行和有边界的恢复重试都必须经过同一套 hook、contract-argument、command-policy 与结构化错误检查，之后才允许真正执行有副作用的 adapter。
- `Task journal event`：executor observation 会投影为稳定的 `tool_started`、`tool_step`、`tool_finished`，以及可选 `coding_checkpoint` / `coding_evidence` 事件，带 step refs、evidence refs、artifact 计数、coding 计数、checkpoint kind、验证命令计数/token、验证状态/失败类别 token、验证风险 token、时间字段和 failure attribution，供 CLI/UI 进度视图使用。
- `subagent tool`：planner 授权的子代理必须显式、只读。单个 child run 或有界 `children` batch 都通过 role/config 校验、timeout/cancellation policy 字段、optional/required failure 隔离，以及只包含机器字段的聚合（`child_results`、`finding_refs`、`evidence_refs`）记录；不会授予写入或外部发布权限。
- `Skill dispatcher`：直接 `run_skill` 和 planner skill call 复用同一调度层。直接 `run_skill` 不让 normalizer / planner 选择技能，只派发显式的 `payload.skill_name`。Builtin 在进程内运行，external 走 adapter，runner 才启动 `skill-runner` 和具体二进制。
- `Skill process protocol`：runner 技能通过 stdin/stdout 交换单行 JSON；运行时需要判断时，技能应在 `extra` 返回稳定机器字段。
- `synthesize_answer`：在循环内需要自然语言合成时调度，不是每个任务固定最后再调用一次 LLM。
- `RepairEnvelope`：verifier、executor、permission、provider 和 checkpoint recovery 路径会把结构化 repair context 暴露给下一轮循环；用户可见 fallback prose 应来自 i18n、finalizer、UI 或模型，不应来自 runtime 模板。
- `Compatibility finalization`：保留给调度、澄清、机器事实快路径，以及显式 legacy/rollback 兼容，不是普通语义决策路径。

### 权限平面与命令策略

权限平面是结构化执行边界，不是第二套语义路由器。来自 `configs/skills_registry.toml` 的 registry metadata、contract matrix policy 和 verifier 状态会投影到 `permission_decision`，让 UI/API/finalizer 能解释发生了什么，而不需要 runtime 写死自然语言回复。

- `risk_level`、`requires_confirmation`、`once_per_task`、`idempotent`、`dedup_scope` 优先来自 registry 与 planner capability metadata。
- `action_effect` 从结构化 skill/action 参数和 contract metadata 派生，不从用户语言短语里判断。
- `run_cmd` 会在 `command_policy` 下输出 `policy_authority`、`literal_command_token`、`command_arg_present`、`unresolved_runtime_template_present` 和命令 effect 标记。
- 显式用户命令用 `_clawd_literal_command` 表达；否则 `run_cmd` 作为 planner 结构化命令参数处理，继续受 contract 与媒体产物 blocker 约束。

## 自然语言契约边界

RustClaw 的原则是：自然语言理解交给 LLM，运行时只消费结构化契约。意图归一化器和规划器可以阅读用户表达、示例、技能文档和多语言提示词，但进入 Rust 运行时前，语义必须已经落到稳定字段里。

运行时允许依赖的确定性输入包括：

- schema enum，例如 `semantic_kind = "package_manager_detection"`
- action name，例如 `read_field`、`validate_config`、`transform_data`
- registry metadata 与 `planner_capabilities`
- `TaskContract` / `OutputContract`、结构化 locator、明确的 `field_path`
- JSON/TOML/YAML 字段路径、文件扩展名、工具结构化输出、exit code、error kind、risk/effect metadata
- `permission_decision` 与 `command_policy` 机器字段

运行时不要为了某个中文、英文或其他语言样例通过而新增短语表、固定问法分支或 `prompt.contains(...)`。如果新的自然语言表达没有被理解，应优先改 normalizer/planner schema、registry capability metadata、`INTERFACE.md`、生成技能提示词或必要的 vendor prompt patch，让 LLM 在不同语言下输出同一套结构化契约。本地门禁是：

```bash
python3 scripts/check_no_nl_hardmatch.py
```

## 记忆系统

RustClaw 记忆分为短期对话记录、结构化用户偏好、长期事实卡和检索索引。目标是让记忆能帮助当前任务，同时避免旧助手输出变成新的隐藏指令。

### 写入路径

`ask` 任务收尾后，RustClaw 可以持久化：

- `memories` 短期记录：按 `user_key`、`user_id`、`chat_id`、角色、类型、显著性和安全标记分组
- `user_preferences` 用户偏好：例如 `response_language`、`response_style`、`response_format`、`agent_display_name`
- `memory_facts` 长期事实卡：包含来源、置信度、作用域、状态、冲突组、过期和 supersede 信息

偏好和事实写入走结构化 memory intent contract。LLM 输出 `memory_actions`，例如 `upsert`、`delete`、`expire`、`noop`；运行时再校验 action enum、kind、scope、confidence、source evidence、TTL 和 safety 字段后才写入数据库。运行时不会通过匹配某一句自然语言来决定 durable preference。

长期摘要刷新仍作为兜底摘要路径存在，但优先把可复用知识写成事实卡。事实卡保留 `fact_key`、`fact_value`、`fact_text`、`source_ref`、`source_memory_ids_json`、`reason`、`confidence`、`expires_at_ts`、`conflict_group` 和 `status`。同一冲突组的新 active fact 会 supersede 旧 fact；过期或删除的 fact 不再进入召回。

### 召回与使用策略

记忆召回会先构造成结构化上下文，再按当前阶段套用 memory use policy：

- route：默认只给最小上下文，包括 active preferences、相关 facts 和 knowledge docs；不把旧助手结果塞进新任务
- follow-up route：当会话状态显示用户正在延续之前任务时，可以加入 recent events、assistant results、similar triggers、unfinished goals 和 snippets
- planner：可使用 unfinished goals、preferences、facts 和 knowledge docs，默认避开 fallback long-term summaries 和旧助手结果
- chat：使用稳定 preferences 与 facts；只有当前会话状态相关时才带有限 recent context
- skill：`_memory` 会按技能 registry 的 `memory_policy` 裁剪；没有显式策略的技能使用安全默认配置

例如 `photo_organize` 技能声明了自己的 memory policy：允许 preferences、relevant facts 和 knowledge docs，但排除 long-term summaries、recent events、assistant results、similar triggers、unfinished goals 和 raw recent snippets。

### 检索索引

混合召回使用 `memory_retrieval_index` 和可选 FTS。索引行会记录 `source_kind`、`source_ref`、memory kind、metadata、salience、success state 和 embedding metadata：

- `embedding_model`
- `embedding_dims`
- `embedding_version`

默认 provider 是离线可用的 `local-hash-v1`。如果配置了不可用或不支持的 embedding provider，运行时会回退到 local hash。只有索引行的 embedding metadata 与当前 provider spec 匹配时才使用 cosine scoring；不匹配时会回退到词法、显著性、时间和成功状态评分。可以在 `configs/memory.toml` 设置 `reindex_on_startup = true`，或从空索引启动，来重建短期记录、偏好、事实卡和知识库快照的检索索引。

### 用户控制

浏览器控制台包含 Memory 页面。它会展示当前身份下的数量、偏好、事实卡和最近记录。用户可以：

- 删除某条偏好、事实或最近记忆
- 把事实卡标记为过期
- 清空当前身份下的最近记录、偏好、事实或全部记忆
- 通过 `configs/memory.toml` 开启或关闭长期记忆

对应 HTTP API：

```text
GET    /v1/memory
GET    /v1/memory/recent
GET    /v1/memory/preferences
GET    /v1/memory/facts
DELETE /v1/memory/:id
POST   /v1/memory/:id/expire
POST   /v1/memory/clear
POST   /v1/memory/settings
```

带 safety 标记的 recent records 默认不会在 UI 中展示。事实卡的 reason、source、conflict group 等细节放在二级详情视图，而不是默认暴露原始 JSON。

### 追踪与排障

Task journal summary 和 trace 会记录 `memory_trace`。它包含 stage、use policy、召回 source refs、纳入原因和字符预算，但不复制原始记忆文本，便于排查“为什么这次任务用了记忆”，同时降低敏感内容泄露风险。

常用代码和配置入口：

- `configs/memory.toml`
- `crates/clawd/src/memory/intent.rs`
- `crates/clawd/src/memory/apply.rs`
- `crates/clawd/src/memory/facts.rs`
- `crates/clawd/src/memory/use_policy.rs`
- `crates/clawd/src/memory/retrieval.rs`
- `crates/clawd/src/memory/indexing.rs`
- `crates/clawd/src/memory/api.rs`

### 后台、恢复与记忆流程

```mermaid
flowchart TD
    A[POST /v1/tasks] --> B[(tasks row<br/>status=queued)]
    B --> C[返回 task_id]
    C --> D[调用方轮询<br/>GET /v1/tasks/:id]
    B --> E[worker_once tick]
    E --> F[Runtime recovery pass]
    F --> G{running task state}
    G -->|普通 stale running| H[status=timeout<br/>machine error_text]
    G -->|paused waiting/background| I[保持 running<br/>读取 checkpoint]
    G -->|无恢复工作| J[认领下一个 queued 任务]
    I --> K{checkpoint due?}
    K -->|未到期| D
    K -->|到期| L[claim_due_paused_checkpoint_task<br/>resume lease]
    L --> M[Seed LoopState<br/>budget + observations + side effects]
    M --> N[record resume_work_item]
    N --> O[record resume_executor]
    O --> P{resume_entrypoint}
    P -->|next_planner_round| Q[plan run_seeded_agent_loop]
    P -->|poll_async_job| R[plan poll_async_job]
    P -->|await_user_input| S[state=needs_user]
    P -->|verify_and_finalize| T[plan verify_and_finalize]
    Q --> U[handoff + dispatch claim]
    R --> U
    T --> U
    U --> V{concrete executor}
    V -->|seeded loop| W[run_agent_with_tools_seeded]
    V -->|async poll| X[poll adapter result]
    V -->|finalize| Y[verify/finalize projection]
    W --> Z[dispatch result]
    X --> Z
    Y --> Z
    Z --> ZA{projection}
    ZA -->|reschedule| I
    ZA -->|terminal async poll| ZAP[Terminal async projection<br/>final_result_json + 必要时补机器回复]
    ZAP --> ZB
    ZA -->|terminal success/failure| ZB[Persist result_json/status]
    J --> ZC[Heartbeat + process ask/run_skill]
    ZC --> ZD{agent loop outcome}
    ZD -->|soft budget/provider wait/async job| ZE[task_lifecycle<br/>waiting/background + task_checkpoint + repair_signal]
    ZE --> D
    ZE --> ZEE[checkpoint_created event<br/>checkpoint_ref + pending_async_job_id]
    ZD -->|needs user| S
    ZD -->|complete| ZB
    ZC --> ZCE[tool_started / tool_finished / coding_checkpoint / coding_evidence events]
    ZB --> ZF[通道交付 + 会话更新]
    ZB --> ZG[Task journal trace + event_stream]
    ZG --> ZW[CLI / UI watch + report]
    ZB -. 可选 .-> ZH[结构化 memory intent extractor]
    ZH --> ZI[运行时校验<br/>enum + scope + confidence + safety]
    ZI --> ZJ[(user_preferences)]
    ZI --> ZK[(memory_facts)]
    ZB -. 可选 .-> ZL[(memories)]
    ZB -. 可选 .-> ZM[(long_term_memories)]
    ZJ --> ZN[(memory_retrieval_index)]
    ZK --> ZN
    ZL --> ZN
    ZM --> ZN
```

关键生命周期细节：

- 前台 HTTP/通道等待时间默认较短。调用方停止等待后应继续轮询同一个 `task_id`，不要重新创建重复任务，也不要把后台任务误判为失败。
- `task_lifecycle` 是机器可读的状态投影。查询 API 暴露 `state`、`db_status`、`can_poll`、`can_cancel`、`checkpoint_id`、`resume_due`、`resume_wait_seconds` 和 heartbeat 字段，供 UI 渲染。
- 状态来源：`crates/clawd/src/task_lifecycle.rs` 负责生命周期投影，`repo::get_task_query_record()` 会把该投影挂到 `GET /v1/tasks/{task_id}`。UI、CLI 和通道应渲染这些结构化字段，不从 `text` 或 `error_text` 推断状态。
- `clawcli get` 和 `clawcli watch` 渲染 lifecycle 机器字段；`clawcli cancel-task <task_id>` 使用直接 task-id 取消 API，`clawcli cancel-index` 只保留给 active-list index 兼容。
- `clawcli resume-task <task_id>` 会把已有 checkpoint 标记为到期恢复；`clawcli pause-task <task_id> --pause-seconds N` 只延迟已有 waiting/background checkpoint，不会重启没有 checkpoint 的任务。
- `clawcli submit --detach` 快速返回 `task_id`；`clawcli submit --wait` 轮询到终态；`--json` 保持 submit/watch 输出适合脚本消费。
- `clawcli active` 默认打印紧凑任务表，也支持 `--json`；`clawcli events <task_id>` 支持 `--jsonl` 和 `--event-type`、`--checkpoint-id`、`--policy-decision`、`--subagent-id`、`--async-job-id` 等机器过滤器。
- task event stream 包含状态迁移、checkpoint、工具生命周期、coding checkpoint/evidence、provider、hook、subagent 和 final 事件。`clawcli events/watch`、`clawcli report` 与浏览器任务详情会渲染 `evidence_ref`、`checkpoint_ref`、`checkpoint_kind`、`pending_async_job_id`、`step_ref`、`changed_file_count`、`test_count`、`verification_command_count`、`verification_command`、`verification_commands`、`verification_status`、`verification_failure_kinds`、`unverified_risk`、`started_at`、`finished_at` 等机器字段；原始 event JSON 放在二级详情。
- `clawcli run-skill <skill_name> --args-json '{...}'` 提交显式 `kind=run_skill` 任务，不走自然语言路由；加 `--wait` 可轮询同一个 `task_id`。
- `clawcli skills` 读取 registry-backed 技能元数据；`clawcli capabilities` 读取扁平化 `/v1/capabilities` 机器端点。脚本消费时请加 `--json`。
- 普通 stale `running` 任务会变成 `timeout`；处于 `waiting` 或 `background` 的 paused checkpoint 仍保留 `running`，以便恢复逻辑按 checkpoint id 认领。
- async 长尾工具应启动外部 job、写入 `pending_async_job`、建立 checkpoint，并先发布包含 `checkpoint_id`、`poll_ref`、`next_check_after` 的 accepted 机器回复；当 provider 或 dry-run adapter 支持时，poll 和 cancel 也应作为结构化 capability 暴露。后续由 worker recovery 通过 `poll_async_job` 继续轮询。
- terminal async poll projection 会保留已有 ask 可见回复；如果 ask 任务只有机器 executor 输出，则补一个包含 `checkpoint_id`、`poll_ref`、`task_id` 和 `final_result_json` 的机器 JSON 回复。
- seeded resume 会恢复 checkpoint 中的预算计数、observations、artifact refs、repair budget 字段和已完成 side-effect fingerprints，再重新进入 agent loop。
- runtime recovery 和 projection 只移动 `status_code`、`message_key`、`executor_state`、`resume_directive`、`job_id`、artifact refs 等机器字段。用户可见 prose 由 finalizer、i18n、UI 或模型渲染。
- Lease/heartbeat 模型见 `docs/task_lifecycle_lease_model.md`；当前 runtime 使用 `tasks.updated_at` 与 checkpoint `resume_executor` 机器字段，新的数据库 lease columns 会等到 multi-worker claim 真正需要时再加入。

## 主要组件

- `crates/clawd`：核心运行时、HTTP API、任务队列、路由、记忆、鉴权、调度
- `crates/skill-runner`：启动 runner 技能二进制；`clawd` 会先解析 registry kind / `runner_name` 再调用它
- `crates/clawcli`：面向 `clawd` 的终端 CLI
- `crates/webd`：可选的反向代理和登录会话桥接层
- `crates/telegramd`、`crates/wechatd`、`crates/feishud`、`crates/larkd`、`crates/whatsappd`、`crates/whatsapp_webd`：通道守护进程
- `services/wa-web-bridge`：WhatsApp Web 通道使用的本地 Node bridge
- `crates/skills/*`：技能实现及其 `INTERFACE.md`
- `external_skills/*`：外部提交技能及其必须提供的 `INTERFACE.md`
- `UI/`：基于 Vite + React 的本地控制台
- `pi_app/`：小屏桌面程序和启动脚本

## 快速开始

### 1. 前置条件

```bash
rustup default stable
python3 --version
```

必须有 `python3`。如果你要构建或部署前端 UI，还需要 `npm`。

### 2. 安装启动命令

推荐方式：

```bash
# 仅安装启动器，不部署 nginx/UI
bash install-rustclaw-cmd.sh --user --no-deploy-ui

# 从源码构建后再安装
bash install-rustclaw-cmd.sh --build --user --no-deploy-ui

# 安装启动器，并按脚本默认行为把 UI 部署到 nginx
bash install-rustclaw-cmd.sh --build --user
```

说明：

- `install-rustclaw-cmd.sh` 会安装 `rustclaw` 启动器
- 如果仓库里已经构建出 `clawcli`，安装脚本也会一并安装它
- 默认情况下，安装脚本会部署 `UI/dist` 到 nginx、写入 nginx 配置并尝试重载 nginx；如果只想装命令，不想碰 UI/nginx，请显式传 `--no-deploy-ui`
- 支持 `--target <triple>`、`--dir <path>`、`--deploy-ui-nginx [path]`、`--pi-app`；其中 `--pi-app` 只会在树莓派上配置小屏桌面程序和登录自启动，普通电脑会自动跳过
- 如果未传 `--build`，脚本会优先复用现有二进制；找不到时才提示你构建或同步 `release-bin`

安装后检查：

```bash
command -v rustclaw
rustclaw -h
rustclaw -status
```

### 3. 配置运行时和通道

主配置：

- `configs/config.toml`
- `configs/skills_registry.toml`

常见拆分配置：

- `configs/image.toml`
- `configs/audio.toml`
- `configs/crypto.toml`
- `configs/memory.toml`

当前实际存在的通道配置文件：

- `configs/channels/telegram.toml`
- `configs/channels/wechat.toml`
- `configs/channels/feishu.toml`
- `configs/channels/lark.toml`
- `configs/channels/whatsapp.toml`
- `configs/channels/whatsapp-web.toml`
- `configs/channels/whatsapp-cloud.toml`
- `configs/channels/webd.toml`

### 4. 从源码构建

```bash
# 完整 release 构建：先同步技能文档，再构建工作区，并在未跳过时执行 UI 构建/部署脚本
./build-all.sh

# 跳过 UI 构建
./build-all.sh no-ui

# 清理后重建
./build-all.sh clean

# 指定主 target
./build-all.sh --target aarch64-unknown-linux-gnu

# 树莓派交叉编译：默认 64 位 Raspberry Pi OS
./cross-build-pi.sh

# 32 位 Raspberry Pi OS
./cross-build-pi.sh --target pi32

# 一次构建多个 target
./build-all.sh --target host --extra-target aarch64-unknown-linux-gnu
```

`build-all.sh` 的当前行为：

- 开始前先执行 `scripts/sync_skill_docs.py`
- 默认构建 `release`，并自动发现工作区里的二进制目标后校验产物是否齐全
- 若存在 `UI/` 且未传 `no-ui`，会调用 `build-ui-nginx.sh`，也就是走“构建 UI + 部署到 nginx”的默认流程
- `--target host` 输出到 `target/release`，交叉编译输出到 `target/<triple>/release`
- `cross-build-pi.sh` 会先准备 Raspberry Pi 目标的 linker / `cc` / bindgen 参数，再调用现有构建流程；默认跳过 UI 构建，避免交叉编译时被前端构建阻塞

如果你只想临时本地编译某个 Rust 目标，仍然可以直接用 `cargo build --workspace --release`，但它不会覆盖 `build-all.sh` 里的同步、UI 构建和产物校验逻辑。

### 5. 启动 RustClaw

使用启动器的示例：

```bash
# 最简启动：等价于 release + channels=all + quick 模式
rustclaw start -q

# 指定厂商/模型启动
rustclaw -start --vendor openai --model gpt-5 --profile release --channels all --quick --skip-setup

# 启动时要求检查并带上 UI
rustclaw -start release all --with-ui
```

当前启动链路与脚本语义：

- `rustclaw -start ...` 最终调用的是 `start-all.sh`
- `start-all.sh` 当前按 `configs/channels/*.toml` 里的 `enabled` 开关决定启动哪些服务
- 如果传了 `telegram | whatsapp_web | both | whatsapp_cloud | all`，脚本会把 Telegram / WhatsApp 相关通道的 `enabled` 值写回配置文件
- 这里的 `all` 是启动器里的快捷通道组合，不等于强制打开 `webd`、`wechat`、`feishu`、`lark` 等所有通道；这些仍以各自配置文件里的 `enabled` 为准
- `--with-ui` 不会自动帮你开发模式起前端，而是要求 `UI/dist` 已存在且没有过期；缺失时会提示你先执行 `cd UI && npm install && npm run build`
- `start-all.sh` 不再在启动阶段自动执行 `sync_skill_docs.py`

脚本方式依然可用：

```bash
./start-all.sh
./stop-rustclaw.sh
```

如果你想按服务精细控制，也可以直接用单服务脚本：

```bash
./component_start/start-clawd.sh
./component_start/start-telegramd.sh
./component_start/start-wechatd.sh
./component_start/start-feishud.sh
./component_start/start-larkd.sh
./component_start/start-whatsappd.sh
./component_start/start-whatsapp-webd.sh
./component_start/start-wa-web-bridge.sh
./component_start/start-clawd-ui.sh
```

单独启动 `clawd` 时：

- `./component_start/start-clawd.sh` 会检查 `target/release/clawd` 和 `target/release/skill-runner`
- 如果 `configs/config.toml` 里还没有 `selected_vendor` / `selected_model`，会在首次启动时要求交互选择
- 若当前厂商的 `api_key` 为空或还是 `REPLACE_ME...`，也会要求在终端里补齐后再启动

### 6. 日常运维命令

```bash
rustclaw -status
rustclaw -logs clawd 200 --follow
rustclaw -health
rustclaw -stop
rustclaw -key list
```

## 身份与访问控制

RustClaw 使用 `user_key` 作为跨 UI 和消息通道的主身份标识。

- 权限按 `user_key` 解析
- 会话按 `channel + external_chat_id` 解析
- 浏览器 UI 通过 `X-RustClaw-Key` 传递身份
- 当鉴权表为空时，`clawd` 可以引导生成首个管理员 key

常用 key 管理命令：

```bash
rustclaw -key list
rustclaw -key generate user
rustclaw -key generate admin
rustclaw -key add rk-xxxx admin
rustclaw -key disable rk-xxxx
```

## UI、API 与 `webd`

主 API 仍由 `clawd` 提供；而脚本当前默认更推荐的对外方式是：

- `clawd` 提供内部 API
- `webd` 作为浏览器访问层/反向代理桥接
- nginx 托管 `UI/dist`，并把 `/v1`、`/webd` 反代到 `webd`

在默认配置里，`configs/config.toml` 中的 `clawd` 监听通常是 `0.0.0.0:8787`，`webd` 默认监听常见为 `0.0.0.0:8788`；部署脚本会从 `configs/channels/webd.toml` 推导反代上游地址。

常用接口（请求时带上当前 UI/user key 的 `X-RustClaw-Key`）：

- `GET /v1/health`
- `POST /v1/tasks`
- `GET /v1/tasks/{task_id}`
- `POST /v1/tasks/cancel`
- `POST /v1/services/{service}/{action}`：浏览器控制台服务启动/停止/重启；失败时返回 `error_code`、`status_code`、`message_key`、`service`、`action` 等机器字段
- `GET /v1/auth/me`
- `POST /v1/auth/channel/bind`
- `GET/POST /v1/auth/crypto-credentials`：按当前 `X-RustClaw-Key` 作用域读取或覆盖当前 key 自己的交易所凭据
- `GET /v1/nni/device/status`：返回 NNI helper 状态、支持的操作，以及是否检测到设备签名芯片
- `POST /v1/nni/device/action`：执行 `pubkey`、`sign_timestamp`、`tng_device_pubkey`、`tng_device_cert`、`tng_signer_cert` 或 `tng_root_cert`

快速示例：

```bash
curl http://127.0.0.1:8787/v1/health \
  -H "X-RustClaw-Key: rk-xxxx"

curl -X POST http://127.0.0.1:8787/v1/tasks \
  -H "Content-Type: application/json" \
  -H "X-RustClaw-Key: rk-xxxx" \
  -d '{"user_id":1,"chat_id":1,"user_key":"rk-xxxx","channel":"ui","external_user_id":"local-ui","external_chat_id":"local-ui","kind":"ask","payload":{"text":"hello","agent_mode":true}}'
```

## NL 回归快捷入口

面向长尾闭环链路的常用入口：

- `bash scripts/nl_tests/run_suite.sh ops_closed_loop`
- `bash scripts/nl_tests/run_suite.sh long_tail_flows`
- `bash scripts/nl_tests/run_suite.sh ops_http_repair`

其中 `ops_http_repair` 是专门盯 `ops_http_repair_then_validate_{zh,en}` 的双语回归入口，日志写到 `scripts/nl_suite_logs/ops_http_repair/<timestamp>/`。

UI 相关说明：

- 源码位于 `UI/`
- 构建产物位于 `UI/dist`
- `build-ui-nginx.sh` 默认会执行“构建 UI + 复制到 nginx + 校验/写入 nginx 配置”
- `deploy-ui-nginx.sh` 更偏向“部署已有 `UI/dist`”，可选 `--build`
- `install-rustclaw-cmd.sh` 默认也会执行 UI/nginx 部署，除非传 `--no-deploy-ui`
- 浏览器 UI 里有独立的 `NNI` 导航分类，对应后端 `/v1/nni/device/*`；没有签名芯片的设备会返回 `signature_chip_present=false`，并在 UI 上显示明确的缺失签名芯片状态
- 服务控制提示基于后端机器码（`error_code` / `message_key`）渲染，不解析后端英文错误字符串
- `webd` 可以作为 `clawd` 前面的反向代理和登录会话桥接层

## 技能体系

RustClaw 当前内置的技能已经比较完整，按类别可大致分为：

- 系统与运维：`system_basic`、`process_basic`、`service_control`、`health_check`、`log_analyze`、`task_control`
- 文件与开发工具：`run_cmd`、`fs_basic`、`config_basic`、`config_edit`、`config_guard`、`archive_basic`、`fs_search`、`git_basic`、`package_manager`、`install_module`、`docker_basic`、`db_basic`
- 网络与内容处理：`http_basic`、`rss_fetch`、`browser_web`、`doc_parse`、`transform`、`web_search_extract`
- 多模态与媒体生成：`image_generate`（`image.generate` / `image.poll` / `image.cancel`）、`image_edit`、`image_vision`、`audio_transcribe`、`audio_synthesize`（`audio.synthesize` / `audio.poll` / `audio.cancel`）、`video_generate`（`video.generate` / `video.poll` / `video.cancel`）、`music_generate`（`music.generate` / `music.poll` / `music.cancel`）
- 工作流与发布类：`schedule`、`extension_manager`、`photo_organize`、`invest_copy`、`x`
- 业务与知识类：`crypto`、`stock`、`weather`、`map_merchant`、`kb`

如果要回答“某个 skill 怎么配置、怎么绑定、缺什么前置条件”，优先看：`prompts/references/skill_setup_guide.zh-CN.md`。

技能发现与运行主要由这些位置驱动：

- `configs/skills_registry.toml`
- `configs/config.toml` 里的 `[skills]`
- `crates/skills/*/INTERFACE.md`
- `external_skills/*/INTERFACE.md`
- `prompts/layers/generated/skills/*.md`

Planner 的技能选择必须由 registry、capability metadata 与 interface/prompt 驱动。一个技能完成注册、开启、补齐 `INTERFACE.md`、执行 `python3 scripts/sync_skill_docs.py`，并在需要给 planner 使用时在 `configs/skills_registry.toml` 声明 `planner_capabilities` 后，planner 应该通过 registry metadata 与生成的 skill prompt 学会何时使用它。不要为了让某个新自然语言样例通过，就在 `clawd` 里新增按技能名分支的选择逻辑。若选择准确率不够，优先改 registry capability metadata、`INTERFACE.md`、生成提示词或必要的 vendor patch；Rust 代码只保留协议校验、resolver/verifier 边界、权限/安全边界、runner 派发、输出合同校验和确定性的跨平台执行兼容。

技能接入入口：

- 内置和普通 `runner` 技能：`skill_develop/README.md`
- 外部技能示例：`external_skills/example/README.md`
- 技能配置和前置条件参考：`prompts/references/skill_setup_guide.zh-CN.md`

### 本地 STT：whisper.cpp

`audio_transcribe` 可以通过 `custom` OpenAI-compatible provider 接本地 whisper.cpp 服务。建议使用专用本地端口，例如 `8178`，避免和 `clawd`、UI 或其他组件端口冲突。

先把多语言模型下载到被 git 忽略的本地模型目录。脚本会按设备内存自动选择 `tiny` / `base` / `small` / `medium`，只有显式传 `--model large-v3` 时才会下载大模型。

```bash
MODEL_PATH="$(bash scripts/download-whisper-model.sh --print-path-only)"
data/vendor/whisper.cpp/build/bin/whisper-server -m "$MODEL_PATH" \
  --host 127.0.0.1 --port 8178 \
  --request-path /v1 --inference-path /audio/transcriptions \
  --convert --language auto
```

中文语音要选多语言 Whisper 模型，例如 `ggml-small.bin`、`ggml-medium.bin` 或 `ggml-large-v3.bin`；不要用 `.en` 结尾的英文专用模型。

```toml
[audio_transcribe]
default_vendor = "custom"
adapter_mode = "compat"
allow_compat_adapters = true
default_model = "local-whisper"
custom_models = ["local-whisper", "whisper-1"]

[audio_transcribe.providers.custom]
base_url = "http://127.0.0.1:8178/v1"
api_key = ""
model = "local-whisper"
timeout_seconds = 120
```

空 `api_key` 只允许本机 `custom` provider（`localhost`、`127.0.0.1`、`::1`）。如果是远端 custom provider，仍然必须配置真实 key。

## 目录说明

- `configs/`：运行时、通道、模型、记忆、技能配置
- `crates/`：Rust 服务、守护进程、CLI 和技能实现
- `external_skills/`：外部提交技能与示例脚手架
- `prompts/`：提示词分层和自动生成的技能提示词
- `scripts/`：安装、回归、维护、技能调用辅助脚本
- `services/`：非 Rust 辅助服务，例如 WhatsApp Web bridge
- `UI/`：浏览器控制台项目
- `pi_app/`：桌面小屏程序
- `docker/`：Docker 相关配置和入口
- `systemd/`：服务模板

## Pi App 小屏程序

小屏桌面程序位于 `pi_app/`。

```bash
cd pi_app && ./run-small-screen.sh
cd pi_app && ./install-desktop.sh
cd pi_app && ./enable-autostart.sh
cd pi_app && ./open-small-screen.sh
```

它会读取 `clawd` 的健康状态，所以需要先启动后端。

Pi App 也包含后端和浏览器 UI 使用的 NNI 设备签名 helper。`pi_app/signature.py` 在硬件和 `cryptoauthlib` 可用时支持读取 Slot 0 公钥、时间戳签名，以及读取 TNG 设备 / signer / root 证书；详细说明见 `pi_app/TNG_SERVER_GUIDE.md`。没有这类芯片的设备也是有效部署，会被显示为“缺失签名芯片”状态。

## 开发说明

- 如果你是源码开发者，`build-all.sh` 是最贴近当前仓库脚本行为的统一构建入口
- 如果你是部署或体验使用者，`install-rustclaw-cmd.sh` 是更直接的入口，因为它会同时处理启动器安装和可选的 UI/nginx 部署
- 如果你只想更新 UI 静态站点，优先看 `build-ui-nginx.sh` 和 `deploy-ui-nginx.sh`
- 如果你在做技能接入，记得显式执行 `python3 scripts/sync_skill_docs.py`，不要依赖启动脚本帮你同步
- 各类回归和辅助脚本主要集中在 `scripts/`
- 如果要跑本地 `ops_closed_loop` 闭环回归，执行 `bash scripts/regression_ops_closed_loop.sh`

## 许可证

本项目使用非商用、源码可见许可。

- 英文法律文本：`LICENSE`
- 中文参考翻译：`LICENSE.zh-CN.md`

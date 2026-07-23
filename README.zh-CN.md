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

RustClaw 主自然语言路径默认使用接近 Codex / Claude 的 agent loop。第一次 planner 调用前，front door 只负责物化文本、语音转写与附件，绑定 task/session 身份，并构造机器拥有的 `TurnBoundaryEnvelope`，其中包含显式 API 字段、locator、权限/预算 profile 和安全上下文。它不会调用语义路由模型，也不会提前决定普通请求该回复、澄清还是执行。每个普通 `ask` 都进入 agent loop，由 planner 做语义决策，并可回复、调用能力、按证据合成、修复、继续、checkpoint 或停止。可恢复失败通过 `RepairEnvelope` 机器字段、attempt history 和 checkpoint state 回到循环，不解析用户语言短语。旧 intent normalizer、contract-repair judge、pre-agent 语义路由开关和 route-selected rollback 路径已经物理删除。

### 请求与 Agent Loop 流程

```mermaid
flowchart TD
    A[通道 / UI / API 请求] --> B[POST /v1/tasks]
    B --> BA[认证后的任务执行策略<br/>服务端拥有的机器 envelope]
    BA --> C[持久化任务并入队]
    C --> D[返回 task_id<br/>调用方可轮询]
    D --> E0[worker_once 恢复 tick<br/>stale running + due checkpoint]
    E0 --> E1[认领下一个 queued 任务]
    E1 --> E{任务类型}
    E -->|run_skill| RS[直接 run_skill 路径<br/>只接受显式 skill_name]
    E -->|ask| F[物化文本/语音/附件]
    F --> G[TurnBoundaryEnvelope<br/>身份 + 显式机器事实 + 安全/预算 profile]
    G --> H[Ask 上下文包<br/>记忆 provenance + 最近执行 + goal/journal refs]
    H --> J[Agent loop<br/>普通语义权威]
    J --> L{循环轮次}
    L --> N[Planner LLM<br/>回复 / 澄清 / call_capability / 合成]
    N --> O
    O --> P[PlanVerifier<br/>permission_decision + 风险 + 效果 + 契约]
    P --> Q{已验证步骤}
    Q -->|respond| R[结构化终端回复<br/>自由文本、精确列表、模型对象<br/>或观察对象投影]
    Q -->|synthesize_answer| S[基于证据合成]
    Q -->|call_tool / call_skill| QP[Pre-tool hooks + adapter preflight<br/>policy_decision + contract args]
    QP --> MG{非幂等变更?}
    RS --> RSG[不经过 planner / resolver 选择<br/>不做 verifier 语义选择]
    RSG --> MG
    MG -->|是| MI[调用前持久化 intent + 确定性 idempotency key<br/>再持久化 attempt]
    MG -->|否| EX{执行 adapter}
    MI --> EX
    EX -->|long-tail async_start| AS[Async media/job adapter<br/>pending_async_job + poll/cancel contract + checkpoint]
    AS --> ASP[进度机器回复<br/>checkpoint_id + poll_ref + next_check_after + can_poll/can_cancel]
    EX -->|call_tool| T[工具执行]
    EX -->|call_skill / 直接 run_skill| U[共享技能调度]
    T --> MR{Mutation receipt 状态}
    U --> MR
    MR -->|非变更| V[观测结果]
    MR -->|收到 receipt| MC[在精确 worker claim 下<br/>持久化 receipt + verification + commit]
    MR -->|timeout / crash 不确定| MU[Reconciliation checkpoint<br/>不从 prose 猜测]
    MC --> V
    MU --> ASP
    S --> V
    V --> W[证据覆盖 + 答案形状检查]
    W -->|修复 / 缺证据| WR[RepairEnvelope<br/>issue codes + attempt ledger]
    WR --> J
    W -->|本轮已观测| BD{BudgetDecision<br/>progress + deadline + policy + hard ceilings}
    BD -->|continue| J
    BD -->|finish| X[观测输出收尾]
    BD -->|checkpoint_requeue / waiting / needs_user| BC[持久化 TaskBudgetSlice + task_checkpoint<br/>释放精确 claim]
    BC --> ASP
    BD -->|terminal| X
    R --> Y[用户可见消息组装]
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
- `认证后的任务执行策略`：`clawcli` 默认继续使用配置中的 approval/sandbox 策略，只有当前仍启用的管理员密钥显式请求全局 `--yolo` 才会进入 YOLO；其他通信 adapter 使用管理员密钥认证时默认进入 YOLO。服务端会删除调用方提供的策略 envelope，在认证后重新签发机器合同，并在每次使用时再次确认管理员密钥仍有效。YOLO 表示 `approval_policy=never` 与 `sandbox_mode=danger_full`，不会绕过 registry allow/deny、参数 schema、路径校验、外部发布控制、取消、预算、脱敏和审计。
- `task_id polling`：API/通道请求的等待超时只影响调用方等多久；后台任务仍可通过 `GET /v1/tasks/{task_id}` 查询，除非 worker 生命周期逻辑已经把它标记为终态。
- `worker_once recovery tick`：worker 认领新 queued 任务前，会先检查 stale running、受保护 paused checkpoint、到期恢复任务、async poll 结果和结果投影。
- `Task kind`：`kind=ask` 进入 planner-owned 自然语言路径；`kind=run_skill` 绕过 planner loop、能力选择和 plan verifier，只把显式提供的 `payload.skill_name` 交给共享 skill dispatcher / 协议执行。两种 task kind 都会把结果写回原始 `task_id`，调用方仍可通过 task 查询 API 查看最终状态。

### Ask 与 Run Skill 边界

这里需要明确区分，因为 `run_skill` 是 API 层任务类型，不是自然语言路由捷径。

直接技能任务的关键点：

- `kind=run_skill` 不进入 planner / agent loop；调用方已经提供了 `payload.skill_name` 和参数。
- `kind=run_skill` 接受显式技能名后，仍使用共享 skill dispatcher 和技能协议。
- `kind=run_skill` 仍创建和更新普通 task row，因此最终状态和结果仍可通过 `task_id` 查询。

| 问题 | `kind=ask` | `kind=run_skill` |
| --- | --- | --- |
| planner 前是否有语义 LLM/router？ | 否。front door 只构造 `TurnBoundaryEnvelope` 和上下文引用。 | 否。调用方已经提供目标技能。 |
| 是否进入 planner / agent loop？ | 是，每个普通自然语言任务都会进入。 | 否。不会让 planner 选择技能或 action。 |
| 是否把 `CapabilityResolver` / `PlanVerifier` 当作语义选择器？ | 否。普通语义选择由 planner 负责；resolver/verifier 只解析和校验 planner step，再允许执行。 | 否。直接技能任务绕过语义选择；显式 skill call 仍走调度和协议校验。 |
| 是否使用共享 skill dispatcher？ | 是，planner 选择 `call_skill` 或 capability 解析到 skill 时使用。 | 是。把 `payload.skill_name` 派发到同一套 builtin / external / runner 技能协议。 |
| 结果是否能用 `task_id` 查询？ | 是。 | 是。直接技能结果保存到原始 task row，可通过 `GET /v1/tasks/{task_id}` 或 `clawcli get` 读取。 |

操作上：用户给自然语言请求时使用 `kind=ask`，让 RustClaw 自己判断回答、澄清、规划或执行。API 调用方已经知道明确技能和参数时使用 `kind=run_skill`，只把 RustClaw 当作任务队列、鉴权、生命周期和结果投影层来运行该技能。

- `Planner-owned front door`：物化文本/语音/附件，并根据 task 身份、显式 API 字段、结构化 locator facts 和安全/预算 profile 构造 `TurnBoundaryEnvelope`。它不做语义 LLM 调用，也没有普通 respond/clarify/execute 分支。
- `Agent-loop 语义权威`：每个普通自然语言任务都会进入循环，由 planner 决定回复、澄清、调用能力、执行工具或技能、按证据合成、修复、checkpoint 或停止。
- `CapabilityResolver / PlanVerifier`：把 `call_capability` 解析到当前 tool 或 skill 实现，再检查可见性、必填参数、allowed action、risk/effect、confirmation 和输出契约。
- `permission_decision`：verifier 和 preflight blocker 输出 `allowed`、`needs_confirmation`、`denied_by_policy`、`dry_run_required`、`external_provider_blocked`、`risk_level`、`action_effect`、registry dedup/idempotency 等机器字段。UI、API、finalizer 和 i18n 应消费这些字段渲染说明，而不是解析 runtime prose。
- `Side-effect outbox`：planner-owned 执行与显式 `kind=run_skill` 的非幂等变更都会在调用前依次持久化 `intent_recorded` 和 `attempt_started`。由 task 与规范 action fingerprint 推导的确定性 key 会进入 runner context、外部 HTTP `Idempotency-Key`，或受支持本地 adapter 的环境变量。Receipt、verification、reconciliation 和 commit 都由精确 worker claim 隔离；已有 receipt 的状态禁止重放原动作，timeout/crash 结果不确定时建立 `mutation_reconciliation` checkpoint，并且只接受 fingerprint 绑定的结构化 `applied|not_applied|still_unknown` resume constraint。
- `Async job start`：长尾工具可以先发布包含 `checkpoint_id`、`poll_ref`、`next_check_after`、`can_poll`、`can_cancel` 的机器回复，同时任务仍可通过 checkpoint 轮询恢复。媒体技能通过 registry capability 暴露这类形状，例如 `image.generate` / `image.poll` / `image.cancel`、`audio.synthesize` / `audio.poll` / `audio.cancel`、`video.generate` / `video.poll` / `video.cancel` 和 `music.generate` / `music.poll` / `music.cancel`。
- `Capability result observation`：每个成功的 `CapabilityResultEnvelope` 都会投影成一条有界、脱敏的通用机器 observation，返回下一轮 planner。领域专用投影可以压缩常用证据，但未知能力或新安装能力无需新增 runtime 分支，也能保留 provider、artifact、异步任务、effect 和 verification 等结构化字段。
- `Evidence coverage`：工具、技能和合成输出都会成为循环内观测；缺证据或可恢复失败会带着压缩的已尝试方法历史回到循环。
- `TaskBudgetSlice / BudgetDecision`：交互任务使用可恢复的软墙钟切片和结构化进度，不再把普通 `max_rounds` 或 `max_tool_calls` 当完成阈值。每次模型/工具结果被观测后，runtime 根据 verifier 通过的计划事实、evidence/artifact 进度、continuation、policy、取消、deadline 和管理员硬上限，选择 `continue`、`finish`、`checkpoint_requeue`、`waiting`、`needs_user` 或 `terminal`。Profile timeout class 会在软切片边界前约束 planner provider 调用和 agent-loop tool/MCP 调用；变更动作超时后进入 reconciliation，而不是盲目重放。模型可以请求续跑，但不能提高成本、权限、时间或资源上限。
- `RepairEnvelope`：repair 是有边界的循环内恢复。运行时提供 `repair_source`、`issue_codes`、`missing_evidence`、`permission_decision`、`provider_status`、`attempt_fingerprint`、`side_effect_fingerprint`、`checkpoint_id`、`next_recovery_kind` 等机器字段；planner/finalizer 可以据此重新规划、澄清、转后台等待或结构化失败，而不是解析本地化 prose。
- `Observed-output finalizer`：只有答案形状与证据契约满足后，才发布有观测依据的结果。
- `Output-contract guard`：保存结果前规范最终文本、`messages` 数组、文件 token、标量/严格输出形状和通道交付一致性。
- `Journal + session update`：任务状态、观测事实和活跃会话锚点在收尾后持久化；后台记忆任务是可选、非阻塞的。
- `Task event stream`：journal trace 事件暴露机器可读进度，例如 `task_goal`、`context_budget`、`context_compaction`、`budget_decision`、`task_transition`、`checkpoint_created`、工具/coding/provider/hook/subagent 事件、`agent_team_started`、`subagent_finished`、`agent_team_aggregated` 和 `task_final`。Provider/context 投影保留 `prompt_truncation_count`、`prompt_bytes_before_max` 等机器指标。CLI 与 UI 直接渲染预算 profile/decision、continuation index、累计模型/工具/token/成本/耗时、soft-slice 状态、checkpoint、验证和 team 进度等机器字段，不读取原始日志或本地化文本来判断状态。Coding 事件是不可变快照：续跑任务会追加更高版本投影，消费者选择最新投影，同时保留之前的红测证据作为历史。

### Planner、LLM 与 Capability 流程

详细流程见：[Agent Loop 与规划](docs/architecture/01-agent-loop.zh-CN.md)。

- `TurnBoundaryEnvelope`：根据已认证 task/session 状态、附件、显式 API 字段、locator 和 policy profile 确定性构建。它只是 planner 上下文，不是语义 route 结果。
- `Planner prompt`：是普通 `ask` 的第一次语义 LLM 调用。resume 和 async-poll executor 可以恢复已经准入的机器 checkpoint，但不能引入新的 planner 前语义决策路径。
- `call_capability`：推荐的 planner action，把 tool/skill 选择放到 registry metadata 与 resolver policy 后面。
- `CapabilityResultEnvelope`：通过有界、脱敏的通用投影返回下一轮 planner。Skill 专用投影只是可选优化，新能力无需专用 Rust 分支也能让结构化结果继续进入循环。
- `respond`：是原生终止格式化 action，不是能力模拟器。普通回答使用模型生成的 `free_text`；严格列表使用 item 数组和精确计数。模型自行生成的命名字段或 JSON 使用 `object`，其中每个 `value_json` 都是一份完整序列化 JSON 值；畸形值只进入有界结构化 repair，不会被静默强制转换。当请求值已经存在于成功的 `CapabilityResultEnvelope` observation 中时，使用 `observed_object`，模型只提交输出字段名、精确 capability token 和语言无关的点路径，runtime 直接复制原始 JSON 值，不再要求模型转录大型嵌套机器数据；缺失、失败或非法引用都会被拒绝。Provider 省略当前 shape 不使用的 payload 时，只规范化为空值/零；冗余的模型生成 object content 只有在解析后的 JSON 与命名字段完全一致时才接受。Runtime 拥有的 provider/config/permission、领域解析/归一化/校验/预演、dry-run、artifact/job、checkpoint、diff、verification、repair 和 rewind 字段必须先有对应 capability observation。低层环境事实只能辅助这次调用，不能替代拥有结果的已披露领域能力。Runtime 只物化终态机器 payload，不解析多语言用户文本，也不追加固定 prose；单个标量、标识符、标题、token 或路径仍使用 `free_text`。
- `Generated INTERFACE prompts`：来自 `crates/skills/*/INTERFACE.md`、`optional_skills/*/INTERFACE.md`、`external_skills/*/INTERFACE.md` 和 `prompts/layers/generated/skills/*`；新增技能应改这些契约，不改 `clawd` 主流程分支。
- `Exact machine output`：planner 使用 `response_shape=strict` 和经过验证的 `structured_field_selector`（例如 `command_output`）提出精确输出要求；runtime 只从 `CapabilityResultEnvelope` 投影该字段。自由回答和一句话回答继续由模型合成。
- `PlanVerifier`：执行前阻断不可用能力、缺必填字段、不安全 mutation，以及不符合输出/证据形状的计划。拒绝路径应携带稳定机器字段，不写固定用户可见回复模板。
- `Pre-tool hooks + adapter preflight`：循环执行和有边界的恢复重试都必须经过同一套 hook、contract-argument、command-policy 与结构化错误检查，之后才允许真正执行有副作用的 adapter。
- `Task journal event`：executor observation 会投影为稳定的 `tool_started`、`tool_step`、`tool_finished`，以及可选 `coding_checkpoint` / `coding_evidence` 事件，带 step refs、evidence refs、artifact 计数、coding 计数、checkpoint kind、验证命令计数/token、验证状态/失败类别 token、验证风险 token、时间字段和 failure attribution，供 CLI/UI 进度视图使用。
- `subagent tool`：planner 授权的 child 工作必须显式。Explorer 保持有界只读；隔离 writer/tester 只能在自己的 task-scoped Git worktree 中工作并返回 patch/evidence refs。父任务会检查冲突、dirty-parent 状态和 precondition hash 后再接纳或拒绝 patch。Child 不得直接写父 worktree，也不得外部发布。
- `Skill dispatcher`：直接 `run_skill` 和 planner skill call 复用同一调度层。直接 `run_skill` 不让 planner 选择技能，只派发显式的 `payload.skill_name`。Builtin 在进程内运行，external 走 adapter，runner 才启动 `skill-runner` 和具体二进制。
- `Skill process protocol`：runner 技能通过 stdin/stdout 交换单行 JSON；运行时需要判断时，技能应在 `extra` 返回稳定机器字段。
- `synthesize_answer`：在循环内需要自然语言合成时调度，不是每个任务固定最后再调用一次 LLM。
- `RepairEnvelope`：verifier、executor、permission、provider 和 checkpoint recovery 路径会把结构化 repair context 暴露给下一轮循环；用户可见 fallback prose 应来自 i18n、finalizer、UI 或模型，不应来自 runtime 模板。
- `Output-contract finalization`：只保留精确机器字段与 artifact 传输的确定性边界，其余回答发布模型基于证据的合成结果；它不选择技能，也不渲染领域专属 prose。

### 权限平面与命令策略

权限平面是结构化执行边界，不是第二套语义路由器。来自 `configs/skills_registry.toml` 的 registry metadata、面向非能力输出形态的 bundled evidence policy，以及 verifier 状态会投影到 `permission_decision`，让 UI/API/finalizer 能解释发生了什么，而不需要 runtime 写死自然语言回复。普通 registry capability family 由 planner `call_capability` 和 resolver metadata 选择，不由历史 route marker 或兼容 hint 选择。

- `risk_level`、`requires_confirmation`、`once_per_task`、`idempotent`、`dedup_scope` 优先来自 registry 与 planner capability metadata。
- `action_effect` 从结构化 skill/action 参数和 contract metadata 派生，不从用户语言短语里判断。
- `run_cmd` 会在 `command_policy` 下输出 `policy_authority`、`literal_command_token`、`command_arg_present`、`unresolved_runtime_template_present` 和命令 effect 标记。
- 显式用户命令用 `_clawd_literal_command` 表达；否则 `run_cmd` 作为 planner 结构化命令参数处理，继续受 contract 与媒体产物 blocker 约束。
- 有风险的本地代码或文件变更能力应在 registry metadata 中声明 isolation profile。`local_temp_workspace` 用于一次性预览、dry-run 和可通过 artifact refs 清理的生成产物；`local_worktree` 用于明确写入当前工作区的开发任务，必须通过 task evidence、changed-file refs 和 verification commands 展示。UI 和 CLI 渲染 `permission_decision.steps[].sandbox`、`workspace_scope` 与 `registry_policy` 机器字段，不从本地化文本里推断权限状态。
- 确认决策使用闭合机器协议 `approve_once|always_for_scope|deny`。只有 registry 声明的本地工作区变更能力才能提供 `always_for_scope`，且作用域必须精确绑定 capability、effect 和资源；`run_cmd`、网络访问、外部发布、凭据访问、包安装和提权操作不得使用持久作用域授权。授权由 `clawd` 使用 HMAC 签名，绑定已认证 actor 与 channel/chat 会话，最长一小时失效，并由服务端负责存储、匹配、列出和撤销；CLI 或 UI 的本地状态本身不产生授权。

权限与策略决策流程：


详细流程见：[安全与执行](docs/architecture/02-security-execution.zh-CN.md)。

### 沙箱与跨平台执行

`sandbox_mode` 定义权限范围，`sandbox_backend` 定义实现该范围的平台后端；两者互不替代。默认 `sandbox_backend = "auto"` 在 Linux 选择 Bubblewrap，在 macOS 选择 Seatbelt。受限模式下后端缺失、平台不匹配或远程执行器未配置时一律结构化拒绝，不会静默降级为无沙箱执行。只有管理员明确配置 `sandbox_mode = "danger_full"`，或由后端认证通过的任务级 YOLO 策略时，才直接启动进程。


详细流程见：[安全与执行](docs/architecture/02-security-execution.zh-CN.md)。

`clawcli --yolo <创建任务的命令>` 只负责请求该模式，且必须使用当前仍启用的管理员密钥；CLI 默认仍使用配置中的安全策略。其他通信 adapter 使用管理员密钥认证时默认采用 YOLO，因此通道管理员密钥必须按完整执行凭据保护。后端是唯一策略权威，request payload、浏览器状态、planner 输出和用户措辞都不能授予此模式。

| 后端 | 主机 | 选择方式 | 当前合同 |
| --- | --- | --- | --- |
| Bubblewrap | Linux | `auto` 或 `bubblewrap` | 文件写入范围、PID/IPC/UTS namespace、可选网络 namespace、前台或持久任务生命周期；缺失时 fail closed。 |
| Seatbelt | macOS | `auto` 或 `macos_seatbelt` | 全局只读、限定路径写入、可选网络和进程策略；缺失时 fail closed。 |
| Remote container | 任意 | 显式 `remote_container` | 目前只定义执行器合同；配置完成前返回 `sandbox_remote_backend_not_configured`，不会成为自动回退。 |
| Direct | 任意 | 显式 `sandbox_mode = "danger_full"` 或认证通过的任务级 YOLO | 不声明沙箱保护；这是管理员选择的绕过方式，不是后端不可用时的回退。 |

诊断会报告请求/解析后的后端、平台、可用性、fail-closed 状态、原因码，以及 filesystem/network/process/credential/resource/environment 控制级别。服务发现同样由平台 adapter 管理：Linux 可使用 systemd/SysV；macOS 使用 Homebrew services、launchd 或进程观测。不兼容的显式 manager 会返回 `unsupported_platform`，不会在 macOS 启动 Linux 命令。开发与发布脚本统一使用 `scripts/shell_compat.sh`，不依赖 GNU 专有的文件/日期参数或 Bash 4 专有集合语法。完整合同见 [`docs/cross_platform_contract.md`](docs/cross_platform_contract.md)。

可信生命周期 Hook 配置在 `configs/agent_guard.toml`，默认保持关闭。管理员可通过 `GET /v1/admin/hooks/status` 或浏览器“模型”页面查看安全状态投影。该机器合同展示 setup state、启用/有效/无效数量、信任与 hash 就绪状态，以及全部支持的 stage；不会回传 handler 参数、endpoint URL、环境变量引用或凭据。浏览器只提供刷新，并把 `redacted_config` 放在手动展开的第二层详情中。启用或信任 Hook 仍必须经过受审查的仓库配置，UI 不能把未审查脚本直接变成执行边界。

## 自然语言契约边界

RustClaw 的原则是：自然语言理解交给 LLM，运行时只消费结构化契约。Planner 可以阅读用户表达、示例、技能文档和多语言提示词，但在运行时执行前必须把理解落到结构化 action。Planner 前的 front door 只能通过 `TurnBoundaryEnvelope` 提供已经认证的机器事实，不能推断普通语义意图。

运行时允许依赖的确定性输入包括：

- evidence-policy 答案形状字段，例如 `final_answer_shape = "summary_with_evidence"` 和 `final_answer_shape_class = "grounded_summary"`
- planner 输出的 capability ref，例如 `capability_ref = "package.detect_manager"` 或 `call_capability("package.detect_manager")`
- action name，例如 `read_field`、`validate_config`、`transform_data`
- registry metadata 与 `planner_capabilities`
- `EvidencePolicyContext` / `OutputContract`、结构化 locator、明确的 `field_path`
- JSON/TOML/YAML 字段路径、文件扩展名、工具结构化输出、exit code、error kind、risk/effect metadata
- `permission_decision` 与 `command_policy` 机器字段

运行时不要为了某个中文、英文或其他语言样例通过而新增短语表、固定问法分支或 `prompt.contains(...)`。如果新的自然语言表达没有被理解，应优先改 registry capability metadata、`INTERFACE.md`、生成技能提示词、planner schema 或必要的 vendor prompt patch，让 LLM 在不同语言下输出同一套结构化 action。天气、网页、图片、照片、发布、包管理、Docker、RSS、行情等普通技能必须走 registry capability metadata。历史 route/semantic 字段只允许由隔离的旧日志读取器展示，不得选择 capability、修改 contract 或决定最终答案形态。本地门禁是：

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

记忆召回会先构造成结构化上下文，再按当前消费者套用 memory use policy：

- planner：可使用 unfinished goals、preferences、relevant facts 和 knowledge docs，但排除 fallback long-term summaries、assistant results、similar triggers 和 raw recent snippets
- chat：使用稳定 preferences 与 facts；只有当前会话状态相关时才带有限 recent context
- skill：`_memory` 会按技能 registry 的 `memory_policy` 裁剪；没有显式策略的技能使用安全默认配置

例如 `photo_organize` 技能声明了自己的 memory policy：允许 preferences、relevant facts 和 knowledge docs，但排除 long-term summaries、recent events、assistant results、similar triggers、unfinished goals 和 raw recent snippets。

### 检索索引

混合召回使用 `memory_retrieval_index` 和可选 FTS。索引行会记录 `source_kind`、`source_ref`、memory kind、metadata、salience、success state 和 embedding metadata：

- `embedding_model`
- `embedding_dims`
- `embedding_version`

默认 provider 是离线可用的 `local-hash-v1`。如果配置了不可用或不支持的 embedding provider，运行时会回退到 local hash。只有索引行的 embedding metadata 与当前 provider spec 匹配时才使用 cosine scoring；不匹配时会回退到词法、显著性、时间和成功状态评分。可以在 `configs/memory.toml` 设置 `reindex_on_startup = true`，或从空索引启动，来重建短期记录、偏好、事实卡和知识库快照的检索索引。

### 知识库设计流程

`kb` 技能是用户管理文档知识库的路径。它和其他普通能力一样接入：`ask` 任务由 agent loop 规划 `call_capability("kb.*")`，直接 API 调用可以用 `kind=run_skill` 加 `skill_name=kb`。运行时不会在 planner 前按用户自然语言特殊判断知识库意图；它只解析和校验 registry capability metadata，然后通过同一套 runner skill 协议派发。


详细流程见：[任务状态与上下文](docs/architecture/03-task-state-context.zh-CN.md)。

关键边界：

- `kb.ingest` 是本地知识库写入能力；registry policy 把它标成 medium risk、once per task，并通过 local process adapter 走 async-preferred。
- `kb.search`、`kb.list_namespaces`、`kb.stats` 是 observe-mode 能力，返回 `namespace`、`hits`、`names`、`document_count`、`chunk_count` 等结构化机器字段。
- `data/kb/by_user/...` 下的 namespace 快照保留兼容文档索引；ingest 还会把 chunks 同步到统一 `memory_retrieval_index`，以 `kb_doc` / `knowledge_doc` 形式参与后续召回，启动重建索引时也可以从快照恢复这些行。
- KB 行按 `user_key` 和 workspace 文件作用域管理，不绑定单个 chat thread。后续只有 memory use policy 允许时才进入 planner/chat/skill 上下文，且当前用户输入始终优先。

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

Task journal summary 和 trace 会记录 `memory_trace`。它包含 stage、use policy、召回 source refs、纳入原因和字符预算，但不复制原始记忆文本，便于排查“为什么这次任务用了记忆”，同时降低敏感内容泄露风险。浏览器教学模式的 trace 面板、`clawcli llm-trace` 和 `/v1/debug/tasks/{task_id}` 还会在编号 LLM 调用上方展示紧凑的 `flow_summary`，包含 stage、module、retry、verifier、finalizer、provider-error 等机器计数，并把结构化 memory/KB 策略、`model_catalog_trace`、`model_catalog_trace.readiness` 和 `resume_trace` 放在原始请求/响应细节旁边。教学模式里，当前选中的对话轮次会展示 task id、状态、LLM 调用次数、stage 数、verifier/finalizer 次数、目标/上下文/team/coding/checkpoint 事件时间线、模型/厂商能力决策、当前模型 readiness 决策、后台续跑/checkpoint 决策，并基于 `flow_stage`、`flow_node`、`code_module`、`code_entrypoint` 和调用编号生成 agent 过程时间线。

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

详细流程见：[任务状态与上下文](docs/architecture/03-task-state-context.zh-CN.md)。

关键生命周期细节：

- 前台 HTTP/通道等待时间默认较短。调用方停止等待后应继续轮询同一个 `task_id`，不要重新创建重复任务，也不要把后台任务误判为失败。
- `task_lifecycle` 是机器可读的状态投影。查询 API 暴露 `state`、`db_status`、`can_poll`、`can_cancel`、`checkpoint_id`、`resume_due`、`resume_wait_seconds` 和 heartbeat 字段，供 UI 渲染。
- 状态来源：`crates/clawd/src/task_lifecycle.rs` 负责生命周期投影，`repo::get_task_query_record()` 会把该投影挂到 `GET /v1/tasks/{task_id}`。UI、CLI 和通道应渲染这些结构化字段，不从 `text` 或 `error_text` 推断状态。
- `clawcli get` 和 `clawcli watch` 渲染 lifecycle 机器字段；`clawcli cancel-task <task_id>` 使用直接 task-id 取消 API，`clawcli cancel-index` 只保留给 active-list index 兼容。
- `clawcli resume-task <task_id>` 会把已有 checkpoint 标记为到期恢复；`clawcli pause-task <task_id> --pause-seconds N` 只延迟已有 waiting/background checkpoint，不会重启没有 checkpoint 的任务。
- `clawcli submit --detach` 快速返回 `task_id`；`clawcli submit --wait` 轮询到终态；`--json` 保持 submit/watch 输出适合脚本消费。
- `clawcli --yolo submit|exec|code|chat|run-skill ...` 为新任务请求 `approval_policy=never` 与 `sandbox_mode=danger_full`，后端只接受当前仍启用的管理员密钥。这是高风险模式：会取消本地确认和进程沙箱隔离，但 registry、schema、外部发布、取消、预算、脱敏与审计控制仍然有效。
- `clawcli exec` 是面向 CI/脚本的执行入口：提交或恢复 ask 任务，默认等待，返回稳定 exit class/code，支持 `--profile quick|coding|release-gate|long-tail`，可在后台 checkpoint 停下，非 JSON 输出会用 `exec_compact_*` 机器行展示预算、代码变更、验证、resume 与残余风险；artifact 目录会写 `summary.json`、`task.json`、`events.jsonl`、`verification.json`、`diff_summary.json`、`llm_summary.json`、`resume.json` 和 `index.json`。`clawcli code` 是 `exec --profile coding` 的简写。
- `clawcli active` 默认打印紧凑任务表，也支持 `--json`；`clawcli events <task_id>` 支持 `--jsonl` 和 `--event-type`、`--checkpoint-id`、`--policy-decision`、`--subagent-id`、`--async-job-id` 等机器过滤器。
- `clawcli tui --user-id <id> --chat-id <id>` 是同一 task API 上的终端控制台；加 `--once` 可输出单次 snapshot，加 `--task-id <task_id>` 可展示 selected task 的 `selected_progress` 和 `selected_summary`。
- `clawcli session list/show/resume/archive/delete/fork` 会维护本地 session navigation store，只保存 `session_id`、`task_ids`、`active_goal_id`、`workspace_root`、checkpoint、event sequence、archive status 和 fork source 等 operator metadata，不作为自然语言路由来源。
- `clawcli goal start/status/pause/resume/edit/clear` 管理结构化长任务 goal contract，包含 `objective`、`done_conditions`、`verification_commands`、constraints、checkpoint resume 字段和脱敏 control response。
- 交互式 chat 中，`/continue` 从持久化线程状态恢复当前 background/checkpoint 任务，不需要复制 task id；`/approve` 只批准当前待确认动作一次，`/approve-scope` 只批准服务端给出的当前会话、精确 capability/resource 作用域，`/deny` 关闭待确认请求。`clawcli permission grants` 列出服务端 scope grant，`clawcli permission revoke <grant_id>` 立即撤销指定授权；浏览器 Tasks 页面使用同一套结构化选择和撤销 API。

```bash
clawcli session list --user-id 1 --chat-id 1 --json
clawcli session show task-123 --json
clawcli session resume task-123 "continue from the checkpoint" --json
clawcli session archive task-123 --json
clawcli session fork task-123 task-123.fork --json
clawcli session delete task-123 --json
```

```bash
clawcli goal start "make the focused change" --objective "ship the fix" --done tests_pass --verify "cargo test -p clawcli" --json
clawcli goal status task-123 --json
clawcli goal pause task-123 --pause-seconds 3600
clawcli goal resume task-123 --checkpoint-id ckpt-123 --message "continue from checkpoint"
clawcli goal edit task-123 --objective "updated goal" --done tests_pass --goal-status background
clawcli goal clear task-123
```

- task event stream 包含 goal、context budget/compaction、task-budget decision、状态迁移、checkpoint、工具/coding/provider/hook/subagent 和 final 事件。CLI、report 和浏览器任务详情直接渲染 `decision`、`profile`、`continuation_index`、累计模型/工具/token/成本/耗时、`soft_slice_exhausted`、`resumable`、checkpoint、验证证据和 team 进度等稳定机器字段；原始 event JSON 放在二级详情。
- `clawcli run-skill <skill_name> --args-json '{...}'` 提交显式 `kind=run_skill` 任务，不走自然语言路由；加 `--wait` 可轮询同一个 `task_id`。
- `clawcli skills` 读取 registry-backed 技能元数据；`clawcli capabilities` 读取扁平化 `/v1/capabilities` 机器端点。脚本消费时请加 `--json`。
- `clawcli llm-trace <task_id> [--raw] [--limit N]` 读取 task debug endpoint，并用 `llm_call_ref=LLM#1..N` 输出每轮 LLM 调用编号、flow/code attribution、provider/model/status、usage token，以及可选 raw request/response 字段。
- `clawcli replay export/run/diff` 使用脱敏的 recorded-only bundle 调试和 CI 对比，不调用 live 模型或工具；`replay run --coverage` 查看记录覆盖，`replay run --view llm|tools|checkpoints|summary` 只看指定类型证据，`replay diff` 输出 `route_changed`、`plan_changed`、`permission_changed`、`final_status_changed` 等分类 token。
- 普通 stale `running` 任务会变成 `timeout`；处于 `waiting` 或 `background` 的 paused checkpoint 仍保留 `running`，以便恢复逻辑按 checkpoint id 认领。
- async 长尾工具应启动外部 job、写入 `pending_async_job`、建立 checkpoint，并先发布包含 `checkpoint_id`、`poll_ref`、`next_check_after` 的 accepted 机器回复；当 provider 或 dry-run adapter 支持时，poll 和 cancel 也应作为结构化 capability 暴露。后续由 worker recovery 通过 `poll_async_job` 继续轮询。
- terminal async poll projection 会保留已有 ask 可见回复；如果 ask 任务只有机器 executor 输出，则补一个包含 `checkpoint_id`、`poll_ref`、`task_id` 和 `final_result_json` 的机器 JSON 回复。
- seeded resume 会恢复持久化的 `TaskBudgetSlice`、累计模型/工具/token/成本/耗时、continuation index、observations、artifact refs、repair 状态和已完成 side-effect fingerprints，再重新进入 agent loop。健康任务只要持续产生结构化进度，就可以跨越旧 4-round/12-tool 数值；真正停止由重复/停滞、取消、policy 或管理员硬上限决定。
- runtime recovery 和 projection 只移动 `status_code`、`message_key`、`executor_state`、`resume_directive`、`job_id`、artifact refs 等机器字段。用户可见 prose 由 finalizer、i18n、UI 或模型渲染。
- Lease/heartbeat 模型见 `docs/task_lifecycle_lease_model.md`；foreground 与 resume-executor 的每次写入都由 task row 的精确 `(lease_owner, claim_attempt)` 隔离。heartbeat 只能续租当前 claim，checkpoint recovery 会推进 generation，旧 worker 不能再发布 claimed process event 或覆盖终态结果。

CLI 生命周期及其持久化教学证据见[任务状态与上下文](docs/architecture/03-task-state-context.zh-CN.md)和[编码与可观测性](docs/architecture/04-coding-observability.zh-CN.md)。

<!-- ai-learning-exclude:start -->
## 详细架构指南

GitHub README 不支持真正的页内分页。详细流程图按顺序维护为独立页面，让每页只聚焦一个主题；AI 学习页面直接渲染同一组 Markdown 源文件，不再维护第二份内容：

1. [Agent Loop 与规划](docs/architecture/01-agent-loop.zh-CN.md)
2. [安全与执行](docs/architecture/02-security-execution.zh-CN.md)
3. [任务状态与上下文](docs/architecture/03-task-state-context.zh-CN.md)
4. [编码与可观测性](docs/architecture/04-coding-observability.zh-CN.md)
5. [技能、多媒体与模型](docs/architecture/05-skills-media-models.zh-CN.md)
6. [发布验证](docs/architecture/06-release-validation.zh-CN.md)

可从[架构索引](docs/architecture/README.md)选择语言并使用上一页/下一页导航。
<!-- ai-learning-exclude:end -->

## 主要组件

- `crates/clawd`：核心运行时、HTTP API、任务队列、路由、记忆、鉴权、调度
- `crates/skill-runner`：启动 runner 技能二进制；`clawd` 会先解析 registry kind / `runner_name` 再调用它
- `crates/clawcli`：面向 `clawd` 的终端 CLI
- `crates/webd`：可选的反向代理和登录会话桥接层
- `crates/telegramd`、`crates/wechatd`、`crates/feishud`、`crates/larkd`、`crates/whatsappd`、`crates/whatsapp_webd`：通道守护进程
- `services/wa-web-bridge`：WhatsApp Web 通道使用的本地 Node bridge
- `crates/skills/*`：固定/核心内建技能实现及其 `INTERFACE.md`
- `optional_skills/*`：由 Skill Store 按需编译和安装的内建技能
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
# 本地安装：只安装启动器，不配置 nginx
bash install-rustclaw-cmd.sh --user

# 从源码构建后再安装
bash install-rustclaw-cmd.sh --build --user

# 仅云服务器：显式把 UI 部署到 nginx
bash install-rustclaw-cmd.sh --build --user --deploy-ui-nginx
```

说明：

- `install-rustclaw-cmd.sh` 会安装 `rustclaw` 启动器
- 如果仓库里已经构建出 `clawcli`，安装脚本也会一并安装它
- 本地默认安装不会安装、配置或重载 nginx；使用 `--with-ui` 启动时由 `clawd` 直接托管 `UI/dist`
- 云服务器需要 nginx 时显式传 `--deploy-ui-nginx [path]`；`--no-deploy-ui` 仅保留为兼容空操作
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
- 若存在 `UI/` 且未传 `no-ui`，会调用 `build-ui-nginx.sh`；该脚本默认只构建 `UI/dist`，除非显式要求部署，否则不会修改 nginx
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
- `--with-ui` 要求 `UI/dist` 已存在且没有过期，并由 `clawd` 在 API 监听地址直接提供页面，通常可打开 `http://127.0.0.1:8787/`；不需要 nginx
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

主 API 仍由 `clawd` 提供；部署方式按环境拆分：

- 本地机器：`clawd` 同时提供 `UI/dist` 和 `/v1`，不需要 nginx
- 云服务器：可由 nginx 托管 `UI/dist`，并把 `/v1`、`/webd` 反代到 `webd`
- `webd` 在启用时提供密码登录和会话桥接；本地直连 UI 可以使用 RustClaw key
- 通过域名打开 UI 时，登录页默认沿用当前 origin，不再附加 `:8787` 或 `:8788`；只有本地直连时才推导服务端口
- 导航栏中的 `AI 学习` 页面直接读取随 UI 打包的 README，按一级主题分页，并把 Mermaid 流程图渲染为可缩放、可全屏查看的图形；切换 UI 语言时会选择对应语言的 README。

在默认配置里，`configs/config.toml` 中的 `clawd` 监听通常是 `0.0.0.0:8787`，`webd` 默认监听常见为 `0.0.0.0:8788`；部署脚本会从 `configs/channels/webd.toml` 推导反代上游地址。

常用接口（请求时带上当前 UI/user key 的 `X-RustClaw-Key`）：

- `GET /v1/health`
- `POST /v1/tasks`
- `GET /v1/tasks/{task_id}`
- `POST /v1/tasks/cancel`
- `POST /v1/tasks/cancel-by-task-id`
- `POST /v1/tasks/cancel-one`：按 active-list index 取消的兼容接口
- `POST /v1/services/{service}/{action}`：浏览器控制台服务启动/停止/重启；失败时返回 `error_code`、`status_code`、`message_key`、`service`、`action` 等机器字段
- `GET /v1/auth/me`
- `POST /v1/auth/channel/bind`
- `GET/POST /v1/auth/crypto-credentials`：按当前 `X-RustClaw-Key` 作用域读取或覆盖当前 key 自己的交易所凭据
- `GET /v1/models/catalog`：返回不含密钥的模型/厂商能力目录，供 UI Models 页面和教学模式 `model_catalog_trace` 使用
- `GET /v1/nni/device/status`：返回 NNI helper 状态、支持的操作，以及是否检测到设备签名芯片
- `POST /v1/nni/device/action`：执行 `pubkey`、`sign_timestamp`、`tng_device_pubkey`、`tng_device_cert`、`tng_signer_cert` 或 `tng_root_cert`

快速示例：

```bash
curl http://127.0.0.1:8787/v1/health \
  -H "X-RustClaw-Key: rk-xxxx"

curl -X POST http://127.0.0.1:8787/v1/tasks \
  -H "Content-Type: application/json" \
  -H "X-RustClaw-Key: rk-xxxx" \
  -d '{"user_id":1,"chat_id":1,"user_key":"rk-xxxx","channel":"ui","external_user_id":"local-ui","external_chat_id":"local-ui","kind":"ask","payload":{"text":"hello"}}'
```

## 模型能力目录与中文 Provider 验证

模型能力目录是配置派生的机器事实，不是运行时临时猜测。它从 `configs/config.toml` 的 LLM provider 表，以及 `configs/image.toml`、`configs/audio.toml`、`configs/video.toml`、`configs/music.toml` 的多模态模型配置生成，输出不含密钥的能力字段：文本、图片/视频/音频输入、图片/语音/视频/音乐生成、是否需要 async、是否支持 dry-run、timeout、context window、`credential_state`、当前激活文本 provider 和配置来源。`credential_state` 是机器 token（`configured_inline`、`configured_env` 或 `missing`），不会包含密钥值。`clawcli models catalog` 会渲染 `model_catalog_summary` 和 `model_catalog_entry` 机器行，让脚本不用解析 prose 就能读取 selected provider/model、entry count、modalities 和 capability flags。`clawcli models readiness` 会为当前选中的 provider/model 渲染 compact 的 `model_readiness_summary`，task debug/教学 trace 和 `clawcli llm-trace` 也会用 `model_catalog_trace.readiness` 暴露同一组选中模型投影，包含 `selected_entry_status`、`credential_state`、`ready`、文本/图片/语音/视频/音乐能力、`async_required` 和 `dry_run`，同样只从 catalog 派生，不探测密钥值、provider 日志或自然语言说明。


模型目录、readiness 与 provider 验证流程见[技能、多媒体与模型](docs/architecture/05-skills-media-models.zh-CN.md)和[发布验证](docs/architecture/06-release-validation.zh-CN.md)。

MiniMax M3/M2.7、MiMo、Qwen 和 DeepSeek 的中文 provider 元数据由 `scripts/check_chinese_model_catalog.py` 守住；它的 `--self-test` 会覆盖 TOML 和 env-file 缺失、读取失败、坏 UTF-8、语法错误等结构化 finding，并在 agent parity gate 中写入 `chinese_model_catalog_self_test.txt`，之后 gate 才信任配置派生的元数据。`scripts/nl_tests/run_chinese_provider_smoke_matrix.sh --dry-run` 可只验证 case 与凭据状态，不调用 provider；它会把 `check_chinese_provider_smoke_matrix.py --self-test`、`check_chinese_provider_smoke_summary.py --self-test` 和生成 summary 的主检查结果写入 `chinese_provider_smoke.txt`。需要 live 验证时，必须确保当前运行中的 `clawd` 已按对应 provider/config 启动，runner 的 `RUSTCLAW_PROVIDER_OVERRIDE` 只用于元数据和同环境启动 wrapper，不会重写已经运行的进程。如果当前账号只购买/启用了一部分 provider，用 `--live-providers minimax` 或其他机器 token CSV 明确当前验收范围，范围外 provider 会记录为 `provider_not_in_live_scope`，不再被当成代码未完成；默认 live scope 是 MiniMax，只有明确需要完整账号验收时才使用 `--live-providers all`。

Agent parity gate 会传递 `CHINESE_PROVIDER_ENV_FILE` 或默认的 `../runtime_env_filled.sh` 给中文 provider catalog 与 smoke preflight，并且只记录 env-file 状态/来源和无密钥凭据元数据，不记录 env-file 路径或密钥值。它的 `gate_summary.env` 会把 gate artifact 位置记录为可搬移的 `out_dir_ref`，而不是本机绝对 `out_dir` 路径；wrapped run log 也会打印 `run_dir_ref` / `run_log_ref` / artifact refs，而不是本机绝对路径。中文 provider smoke metadata，包括 `case_coverage.json`，只记录可搬移路径引用，例如 repo-relative 路径、`out_dir/...` 或 `external_path`；validator 会拒绝 `case_file`、`output_file`、`run_dir` 中的本机绝对路径。它会先运行 `scripts/check_no_runtime_hard_reply.py --self-test` 再运行 baseline 扫描，写入 `runtime_hard_reply_baseline.txt`，并记录 `runtime_hard_reply_baseline=1`；该 artifact 必须包含 `RUNTIME_HARD_REPLY_ALL_SCAN` 和 `new=0`，防止新增生产 Rust 句子型回复悄悄变成固定用户回复模板。它也会运行 `scripts/check_no_policy_boundary_hard_reply.py --self-test`，写入 `policy_boundary_hard_reply.txt`，并记录 `policy_boundary_hard_reply=1`；该 artifact 必须包含 `POLICY_BOUNDARY_HARD_REPLY_SELF_TEST ok` 和 `POLICY_BOUNDARY_HARD_REPLY_CHECK ok`，防止 UserResponseContract、policy boundary、最终回复和确定性恢复合同重新堆固定 prose 回复规则。它还会运行 `scripts/check_repair_no_user_text_fields.py --self-test`，写入 `repair_no_user_text_fields.txt`，并记录 `repair_no_user_text_fields=1`；该 artifact 必须包含 `REPAIR_USER_TEXT_FIELD_CHECK ok`，保证 loop repair、answer verifier、recovery 和确定性 resume 路径继续使用 `extra`、`status_code`、`message_key`、`RepairEnvelope` 和结构化 evidence 等机器字段，而不是把用户可见 `text/error_text` 当协议读取。它还会运行 `scripts/check_policy_decision_tokens.py --self-test`，写入 `policy_decision_tokens.txt`，并记录 `policy_decision_tokens=1`；该 artifact 必须包含 `POLICY_DECISION_TOKEN_SELF_TEST ok` 和 `POLICY_DECISION_TOKEN_CHECK ok`，保证 permission、确认和后台等待决策继续集中由 `PolicyDecision` 机器 token 枚举生成，而不是散落 JSON 字符串。它还会运行 `scripts/check_registry_policy_contracts.py --self-test`、`scripts/check_skill_registry_aliases.py --self-test` 和 `scripts/check_long_tail_skill_contracts.py --self-test`，写入 `registry_policy_contracts.txt`、`skill_registry_aliases.txt` 和 `long_tail_skill_contracts.txt`；这些 artifact 记录 `registry_policy_contracts=1`、`skill_registry_aliases=1` 和 `long_tail_skill_contracts=1`，保证 planner capability policy 元数据、语言无关 registry alias、async/dry-run/poll/cancel 长尾合同继续作为 release-gated 机器事实。它会运行 `scripts/check_no_agent_mode_payload.py` 并写入 `no_agent_mode_payload.txt`，防止旧 channel/UI agent-mode 布尔开关重新成为关闭默认 agent loop 的隐形入口。它还会先对 route-authority legacy-key guard、legacy route boundary guard、pre-planner removal guard、NL hard-match scanner 和 historical hardcoded-language scanner 运行 self-test，再运行主检查，并写入 `agent_loop_static_contracts.txt`，确保旧 pre-route 语义路由和固定自然语言捷径不会回到生产路径，同时在 artifact 被信任前证明这些守卫的拒绝路径有效。它还会先运行 `scripts/check_evidence_extractor_contracts.py --self-test` 再运行主检查，并写入 `evidence_extractor_contracts.txt`，要求结构化 extractor 声明稳定机器 evidence 字段，拒绝新增未登记的严格 `text_legacy` evidence 路径，并在 artifact 被信任前证明这些拒绝路径有效。

Agent parity gate 还会运行 `scripts/check_agent_loop_guard_final_scope.py --self-test`，写入 `agent_loop_guard_final_scope.txt`，并记录 `agent_loop_guard_final_scope=1`；该 artifact 必须包含 `AGENT_LOOP_GUARD_FINAL_SCOPE_SELF_TEST ok` 和 `AGENT_LOOP_GUARD_FINAL_SCOPE_CHECK findings=0`，保证 answer-verifier evidence 与 registry idempotency 边界保持终态 `all` scope，而不是隐藏的旧 route 选择式回滚开关。

`agent_loop_static_contracts.txt` 还会包含 frontdoor boundary dispatch 守卫：`scripts/check_frontdoor_boundary_dispatch.py --self-test` 和主检查必须输出 `AGENT_LOOP_STATIC_SELF_TEST check_frontdoor_boundary_dispatch.py` 与 `FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings=0`，确保 ask front door 只保留 schedule/resume/边界准备入口，不重新决定普通请求该直答、澄清还是执行。

同一 gate 还会写入 `planner_runtime_boundary_contracts.txt` 并记录 `planner_runtime_boundary_contracts=1`。该 artifact 必须包含 `PLANNER_RUNTIME_BOUNDARY_CHECK findings=0`、`CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0`、`ROUTE_REASON_MARKER_FACADE_SELF_TEST ok`、`ROUTE_REASON_MARKER_FACADE_CHECK findings=0`、`FINALIZER_ARCHITECTURE_SELF_TEST ok`、`FINALIZER_ARCHITECTURE_CHECK findings=0`、`zero_domain_hits=0` 和 `registry_dependencies=0`，把 planner-owned runtime、loop-only repair、机器 route marker 和 zero-domain finalizer 纳入 release gate。

同一 gate 还会写入 `agent_architecture_boundary_contracts.txt` 并记录 `agent_architecture_boundary_contracts=1`。该 artifact 必须包含 `BOUNDARY_ENVELOPE_SCHEMA_CHECK findings=0`、`PLANNER_PRE_LLM_DETERMINISTIC_FAST_PATH_CHECK strict_tests=false findings=0`、`CAPABILITY_RESOLVER_REGISTRY_ONLY_CHECK findings=0`、`FINALIZER_BOUNDARY_CHECK ok` 和 `EVIDENCE_POLICY_FACADE_BOUNDARY_CHECK strict=false findings=0`，保证 machine-only boundary schema、planner、resolver、finalizer 和 evidence-policy facade 不会悄悄退回旧的前置语义路由或静态兼容路径；`PLANNER_RUNTIME_BOUNDARY_CHECK findings=0` 另行证明已删除的 intent-normalizer 与 contract-repair prompt/schema/helper 没有回归。

同一 gate 还会写入 `deterministic_boundary_inventory_contracts.txt` 并记录 `deterministic_boundary_inventory_contracts=1`。该 artifact 必须包含 `ANSWER_VERIFIER_BOUNDARY_CHECK ok`、`OBSERVED_OUTPUT_BOUNDARY_CHECK ok`、`DETERMINISTIC_DECISION_INVENTORY_CHECK ok`、`REPAIR_BOUNDARY_INVENTORY_CHECK ok` 和 `REPAIR_BOUNDARY_INVENTORY_COVERAGE_CHECK required=... missing=0`，保证 answer verifier、observed-output、确定性 branch inventory 和 repair inventory 不会继续膨胀成隐藏路由器、固定 prose 渲染器或未登记兼容路径。

同一 gate 还会写入 `maintainability_skill_contracts.txt` 并记录 `maintainability_skill_contracts=1`。该 artifact 必须包含 `LONG_FILE_CHECK ok`、`OK: all ... registry skills have a generated layered prompt body` 和 `REGISTRY_PARITY mode=all ... differences=0`，保证长文件上限、生成式分层 skill prompt 和主 registry/docker registry 元数据一致性都作为 release gate 证据。

同一 gate 还会写入 `agent_parity_gate_inventory_contracts.txt` 并记录 `agent_parity_gate_inventory_contracts=1`。该 artifact 必须包含 `AGENT_PARITY_GATE_INVENTORY_SELF_TEST ok`、`AGENT_PARITY_GATE_INVENTORY_CHECK ok`、`NL_TEST_CHECKER_INVENTORY_SELF_TEST ok` 与 `NL_TEST_CHECKER_INVENTORY_CHECK ok`，保证 top-level `scripts/check_*.py` 和 nested `scripts/nl_tests/check_*.py` 守卫要么进入 release runner，要么因 live-run-only 兼容原因显式豁免。

同一 gate 也会写入 `task_lifecycle_contracts.txt` 并记录 `task_lifecycle_contracts=1`。该 artifact 来自 `scripts/check_task_lifecycle_contracts.py --self-test` 和主检查，必须包含 `TASK_LIFECYCLE_CONTRACT_SELF_TEST ok` 与 `TASK_LIFECYCLE_CONTRACT_CHECK findings=0`。它把后台执行、checkpoint/resume、resume executor lease、seeded agent-loop resume、async poll/cancel projection 以及 CLI/UI task lifecycle 展示都固定在机器字段上，而不是本地化 `text/error_text`。

同一 gate 也会写入 `task_event_context_team_contracts.txt` 并记录 `task_event_context_team_contracts=1`。该 artifact 来自 `scripts/check_task_event_context_team_contracts.py --self-test` 和主检查，必须包含 `TASK_EVENT_CONTEXT_TEAM_CONTRACT_SELF_TEST ok` 与 `TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings=0`。它保证 `task_goal`、`context_budget`、`context_compaction`、provider prompt-budget metrics、coding evidence，以及 subagent/team lifecycle events 继续作为结构化 event-stream 字段提供给 CLI、UI、教学模式和 replay 工具；允许写入的 child 仍必须隔离在 task-scoped worktree，并由父任务接纳。

同一 gate 也会写入 `clawcli_exec_replay_contracts.txt` 并记录 `clawcli_exec_replay_contracts=1`。该 artifact 来自 `scripts/check_clawcli_exec_replay_contracts.py --self-test` 和主检查，必须包含 `CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok` 与 `CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0`。它把 `clawcli exec`/`clawcli code` 的 CI artifacts（`summary.json`、`task.json`、`events.jsonl`、`verification.json`、`diff_summary.json`、`llm_summary.json`、`resume.json`、`index.json`）、`exec_compact_*` 输出，以及 `clawcli replay export/run/diff` 的 recorded_only coverage/view/diff class 行为都固定在机器字段合同上。

同一 gate 也会写入 `clawcli_session_tui_contracts.txt` 并记录 `clawcli_session_tui_contracts=1`。该 artifact 来自 `scripts/check_clawcli_session_tui_contracts.py --self-test` 和主检查，必须包含 `CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok` 与 `CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0`。它把 `clawcli session list/show/resume/archive/delete/fork`、本地 session store metadata、`clawcli tui` selected task snapshot、`selected_progress`、`selected_summary`、operator key tokens，以及 TUI report/review/subagents/permission 投影都固定在机器字段合同上。

同一 gate 也会写入 `clawcli_goal_contracts.txt` 并记录 `clawcli_goal_contracts=1`。该 artifact 来自 `scripts/check_clawcli_goal_contracts.py --self-test` 和主检查，必须包含 `CLAWCLI_GOAL_CONTRACT_SELF_TEST ok` 与 `CLAWCLI_GOAL_CONTRACT_CHECK findings=0`。它把 `clawcli goal start/status/pause/resume/edit/clear`、`done_conditions`、`verification_commands`、constraints、resume/checkpoint control summary 和敏感字段脱敏都固定在结构化机器合同上。

同一 gate 也会写入 `clawcli_llm_trace_contracts.txt` 并记录 `clawcli_llm_trace_contracts=1`。该 artifact 来自 `scripts/check_clawcli_llm_trace_contracts.py --self-test` 和主检查，必须包含 `CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok` 与 `CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0`。它把 `clawcli llm-trace`、`llm_call_ref=LLM#1..N`、flow/code attribution、provider/model/status/usage token、raw request/response 字段、`llm_trace_model_readiness` 和 UI 教学 trace helper 都固定在机器字段合同上。

同一 gate 也会写入 `clawcli_models_catalog_contracts.txt` 并记录 `clawcli_models_catalog_contracts=1`。该 artifact 来自 `scripts/check_clawcli_models_catalog_contracts.py --self-test` 和主检查，必须包含 `CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok` 与 `CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0`。它把 `clawcli models catalog`、`model_catalog_summary`、`model_catalog_entry`、provider filter、`credential_state`、active text provider、modalities、capability flags、async/dry-run metadata 和 UI model catalog display 都固定在无密钥机器字段合同上。

同一 gate 也会写入 `clawcli_models_readiness_contracts.txt` 并记录 `clawcli_models_readiness_contracts=1`。该 artifact 来自 `scripts/check_clawcli_models_readiness_contracts.py --self-test` 和主检查，必须包含 `CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok` 与 `CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0`。它把 `clawcli models readiness`、`clawcli llm-trace`、`model_readiness_summary`、`model_catalog_trace.readiness`、selected provider/model 匹配、`selected_entry_status`、`credential_state`、`ready`、capability flags、async/dry-run metadata、UI teaching trace tokens 和缺失选中条目的行为都固定在无密钥机器字段合同上。

Agent parity gate 还会先运行 `scripts/nl_tests/check_secret_scan_contract.py --self-test` 并写入 `secret_scan_contract_self_test.txt`；该 artifact 必须包含 `SECRET_SCAN_CONTRACT_SELF_TEST ok`，证明 forbidden secret field 和 secret-like value 的拒绝路径有效，之后才信任主 JSON 结果。随后它运行 `scripts/nl_tests/check_secret_scan_contract.py --json` 并写入 `secret_scan_contract.json`，把禁用密钥字段、非 object JSON artifact 和疑似密钥值的检查固定成机器合同，而不是靠人工约定；同时运行 `scripts/nl_tests/check_suite_wrapper_contract.py` 并写入 `suite_wrapper_contract.json`，保证长任务回放和教学追踪依赖的 wrapped-suite 恢复产物保持稳定。它还会运行 `scripts/nl_tests/check_runner_path_ref_contract.py` 并写入 `runner_path_ref_contract.json`，保证 full/manual/multi-turn/client-like/provider A/B/dynamic/regression runner 的 console log 继续使用可搬移 path-ref，而不是本机绝对路径。它还会运行 `check_suite_wrapper_contract.py --self-test`、`check_runner_path_ref_contract.py --self-test` 和 `check_compact_coverage.py --self-test`，并写入 `nl_suite_checker_self_tests.txt`；该 artifact 必须包含 `SUITE_WRAPPER_CONTRACT_SELF_TEST ok`、`RUNNER_PATH_REF_CONTRACT_SELF_TEST ok` 与 `COMPACT_COVERAGE_SELF_TEST ok`，证明这些 NL-suite checker 能拒绝坏 snippet、坏 path ref、compact tag 缺失、不安全媒体 dry-run 行和禁止 live 发布行，之后才信任它们的 JSON 报告。它还会运行 `scripts/nl_tests/check_suite_artifact_contract.py --self-test`、`scripts/nl_tests/print_llm_raw_trace.py --self-test` 和 `scripts/nl_tests/summarize_rollout_metrics.py --self-test`，并写入 `suite_artifact_contract_self_test.txt`、`llm_raw_trace_runner_contract.txt`、`rollout_metrics_contract.txt`，证明 checker 会拒绝 report 缺失、不可读取、JSON 损坏、顶层不是 object、基础 report 字段错误、未完成自证、summary 不一致、嵌套 agent parity contract 不一致、中文 live provider scope 非法、env-file state/source 非法、gate summary path ref 不安全、中文 provider smoke path ref 不安全、rollout metrics source/output ref 不安全或意外带入嵌套 agent parity contract 的 report，之后才把它作为 release artifact 信任。当通过 `scripts/nl_tests/run_suite.sh agent_parity_gate` 启动时，`suite_artifact_contract.json` 还会验证嵌套的 `agent_parity_gate/` artifacts，并记录 `agent_parity_gate_contract.checked=true`，证明 runtime hard-reply、policy-boundary hard-reply、repair no-user-text、policy-decision-token、final-scope、registry-policy、registry-alias、long-tail-skill、clawcli exec/replay、clawcli session/TUI、clawcli goal、clawcli LLM trace、agent-loop static、no-agent-mode、evidence-extractor、secret scan self-test、secret、wrapper、runner path-ref、NL-suite checker self-tests、suite-artifact self-test、raw LLM trace 和 rollout metrics path 合同都参与了该 wrapped run；如果嵌套 gate summary 缺失，checker 会返回结构化 `agent_parity_gate_summary_missing` finding，而不是 traceback。最终 report 写入会使用 `--validate-contract-report-content` 和 `--require-contract-report-content-checked`，要求既有 report 为 `ok=true`、无 findings、与当前 summary 和嵌套合同计数一致，并且已经标记 `contract_report_content_checked=true`。`gate_summary.env` 必须包含 `live_metrics=0|1`，`chinese_provider_live_providers` 必须是 `all` 或已知中文 provider 机器 token 的 CSV，env-file state/source 也必须保持在允许的机器 token 集合内；`metrics=1` 只表示 metrics gate 没有被禁用，`live_metrics=1` 才表示提供了 run directory 且 `run_metrics.*` 已生成并以可搬移 source/output refs 通过内容校验，checker 不会从 `metrics` 推断 live metrics。NL/live NL runner 会保留 `logs/model_io.log` offset、`task_id`、`PRINT_LLM_TRACE` 和 `LLM#1..N` 原始字段回放合同。

同一个 raw LLM trace artifact 现在会同时运行 `print_llm_raw_trace.py --self-test`、`check_llm_raw_trace_runner_contract.py --self-test` 和 checker 主检查。`llm_raw_trace_runner_contract.txt` 必须包含 `LLM_RAW_TRACE_RUNNER_CONTRACT_SELF_TEST ok` 与 `LLM_RAW_TRACE_RUNNER_CONTRACT ok`，证明原始字段打印 helper 和 runner wiring checker 都通过后，才信任 NL/live NL 的 `LLM#1..N` trace 输出。

## NL 回归快捷入口

代码还在快速推进时，优先跑最小受影响 NL 集；阶段收口或 release gate 时再扩大覆盖：

1. 静态 compact 覆盖：`python3 scripts/nl_tests/check_compact_coverage.py --report`，只检查源控 case 覆盖基础技能、route/lifecycle 分类和媒体 dry-run，不调用 provider。
2. 受影响小集合：针对本次修改路径挑 10-30 条。
3. 典型聚合集：一个阶段完成后跑压缩代表性覆盖。
4. Canary：改变默认 authority 或删除旧 gate 前跑 500 条 client-like。
5. Safe aggregate：先跑 compact 等价覆盖；只有高风险删除 gate 或发布硬化才跑完整 2100+。

实际 NL 建议通过 `bash scripts/nl_tests/run_all_nl_with_server.sh` 运行。该入口
默认使用随机 loopback 端口、隔离的 task/audit 数据库和不会向外发送消息的
`ui` channel，结束后自动删除临时状态。只有显式传入 `--reuse-server` 才会
复用正在运行的开发服务器。可用 `--suite <name>` 或 `--category <name>` 选择
最小受影响范围；除非明确关闭，仍会打印带编号的原始 `LLM#1..N` 请求/返回字段。

当前不再用固定七天等待作为普通开发删除门槛。删除兼容路径前，应使用受影响 compact live NL、release-gate 等价覆盖、loop-boundary/replay 无 unexplained mismatch，以及静态门禁。Contract repair 清理必须通过 `python3 scripts/check_contract_repair_loop_observation_boundary.py`；planner/output-contract 清理应通过 `python3 scripts/check_planner_runtime_boundary.py`、`python3 scripts/check_route_reason_marker_facade.py` 和 `python3 scripts/check_finalizer_architecture.py`；repair 清理应通过 `python3 scripts/check_repair_boundary_inventory_coverage.py` 和 `python3 scripts/check_repair_no_user_text_fields.py`。

面向长尾闭环链路的常用入口：

- `bash scripts/nl_tests/run_suite.sh ops_closed_loop`
- `bash scripts/nl_tests/run_suite.sh long_tail_flows`
- `bash scripts/nl_tests/run_suite.sh ops_http_repair`

其中 `ops_http_repair` 是专门盯 `ops_http_repair_then_validate_{zh,en}` 的双语回归入口，日志写到 `scripts/nl_suite_logs/ops_http_repair/<timestamp>/`。

UI 相关说明：

- 源码位于 `UI/`
- 构建产物位于 `UI/dist`
- `build-ui-nginx.sh` 默认只构建 `UI/dist`；只有显式传 `--deploy` 才会配置 nginx
- `build-ui-nginx.sh --deploy-if-configured` 只更新机器上已存在的 RustClaw nginx 站点，本地更新不会因此写系统配置
- `deploy-ui-nginx.sh` 更偏向“部署已有 `UI/dist`”，可选 `--build`
- `install-rustclaw-cmd.sh` 默认走本地无 nginx 安装；云服务器使用 `--deploy-ui-nginx`
- 管理员打开首页时会自动检查源码版本和当前平台可用的 GitHub Release，并显示运行版本与最新 Release 标签
- 只有真实构建或部署任务才显示进度；完整编译、仅 UI、仅 clawd 成功完成后会自动刷新一次页面
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
- `optional_skills/*/INTERFACE.md`
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
- 如果只想重建本地 UI，使用 `build-ui-nginx.sh`；只有 nginx 托管的服务器才使用 `deploy-ui-nginx.sh`
- 如果你在做技能接入，记得显式执行 `python3 scripts/sync_skill_docs.py`，不要依赖启动脚本帮你同步
- 各类回归和辅助脚本主要集中在 `scripts/`
- 如果要跑本地 `ops_closed_loop` 闭环回归，执行 `bash scripts/regression_ops_closed_loop.sh`

## 许可证

本项目使用非商用、源码可见许可。

- 英文法律文本：`LICENSE`
- 中文参考翻译：`LICENSE.zh-CN.md`

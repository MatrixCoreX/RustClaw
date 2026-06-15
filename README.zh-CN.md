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
- 覆盖系统、文件、网络、图片、语音、加密货币、知识库、自动化等场景的 builtin、external 与 runner 技能
- 本地浏览器控制台位于 `UI/`，其中包含独立的 NNI 设备签名页面
- 树莓派/小屏桌面程序位于 `pi_app/`

## Agent Loop 架构

RustClaw 主自然语言路径正在迁移到接近 Codex / Claude 的 agent loop 架构。Boundary Layer 负责把本轮绑定到身份与会话状态，产出结构化路由信号，并处理 locator、契约、安全、确认、dry-run 与预算护栏；当 `semantic_route_authority = "agent_loop_default"` 选中 eligible class 时，普通低风险语义决策交给 agent loop。对这些 class，意图归一化器只是初始结构化提示，不再是最终语义权威。旧 pre-agent 路由仍作为直接聊天、澄清、调度、高风险、交付和非 eligible case 的兼容/回滚路径，直到 release-deprecation gate 完成。

### 运行时流程

```mermaid
flowchart TD
    A[用户输入] --> B[通道 / API 入口]
    B --> B1[任务队列<br/>POST /v1/tasks]
    B1 --> B2[worker_once / 处理任务]
    B2 --> B3{任务类型}
    B3 -->|run_skill| RS[直接技能任务<br/>绕过归一化 / 规划器]
    B3 -->|ask| C0{调度直达文本?}
    C0 -->|是| SD0[调度直达文本收尾<br/>早于归一化]
    C0 -->|否| C[会话快照与本地表面信号]
    C --> D[绑定 / 恢复 / 活跃任务上下文]
    D --> E[意图归一化 LLM]
    E --> ER{需要契约修复?}
    ER -->|可选| RJ[Contract-repair judge LLM<br/>基于 schema 的语义修复]
    ER -->|否| EC[构建 ask 上下文包<br/>记忆 + 附件 + 最近执行]
    RJ --> EC
    CM[任务契约矩阵<br/>semantic kind + required evidence + allowed action + response shape] --> E2
    CM --> DG
    CM --> VF
    CM --> EV
    CM --> Q
    EC --> E2[Boundary 路由后策略<br/>locator + 契约矩阵护栏]
    E2 -->|调度直达| SD[调度直达收尾]
    E2 -->|恢复讨论| RD[恢复讨论提示词]
    E2 -->|恢复执行| H
    E2 -->|运行时已有标量/直答证据| RDC[有证据支撑的直接候选<br/>不额外调用 LLM]
    E2 -->|普通 ask| SA{semantic_route_authority}
    SA -->|agent_loop_default + eligible class| H[Agent-loop 语义权威<br/>decision envelope + runtime context]
    SA -->|legacy / shadow / non-eligible| F{Boundary 分发<br/>兼容路径}
    F -->|Clarify| G[澄清问句]
    F -->|DirectAnswer| DG[直接回答候选 / 可选预检<br/>运行时证据 + 契约检查]
    DG -->|有证据候选| VP
    DG -->|直接回答| CH[构建直接回答聊天上下文与提示词]
    DG -->|澄清| G
    DG -->|提升为执行| H
    CH --> CR[聊天回复 LLM]
    F -->|PlannerExecute fallback| H
    SK[技能注册表 + 生成技能文档<br/>configs/skills_registry.toml] --> RV
    H --> I[规划器 / 运行时循环]
    I --> ID{窄范围确定性<br/>观测契约?}
    ID -->|是| JD[运行时构建观测计划<br/>不调用规划 LLM]
    ID -->|否| PL[规划 LLM 轮次<br/>推荐 call_capability]
    JD --> RV[CapabilityResolver<br/>能力 / 兼容动作归一]
    PL --> RV
    RV --> VF[PlanVerifier + 契约动作门禁<br/>schema + allowed action + 风险/效果]
    VF --> L{已验证动作}
    L -->|respond| M[直接回复]
    L -->|synthesize_answer| SS[基于证据的合成 LLM]
    L -->|call_tool| N[工具执行<br/>虚拟工具调度]
    L -->|call_skill| N0[execution_adapters::run_skill<br/>共享技能入口]
    N0 --> N1[run_skill_with_runner<br/>技能调度]
    RS --> N1
    N1 -->|builtin| N1B[进程内 builtin 技能]
    N1 -->|external| N1E[External skill adapter]
    N1 -->|runner| N2[skill-runner 子进程]
    N2 --> N3[具体技能二进制]
    N1B --> SR[技能结果]
    N1E --> SR
    N3 --> SR
    SR -->|planner call_skill| P
    SR -->|直接 run_skill| RSK[run_skill 收尾<br/>任务结果 + journal]
    N --> P[循环内观测<br/>失败分类]
    SS --> P
    P --> EV[证据覆盖校验<br/>required evidence + answer shape]
    EV -->|缺证据 / 修复| I
    EV -->|证据足够| OF[观测输出收尾<br/>直接答案或合成]
    M --> VP[用户可见消息组装<br/>契约约束正文 + 可选脱敏消息]
    CR --> VP
    G --> VP
    OF --> VP
    RDC --> VP
    SD --> VP
    RD --> RDL[恢复讨论 LLM]
    RDL --> VP
    VP --> Q[最终交付 / 输出契约护栏<br/>shape + delivery consistency]
    Q --> R[收尾结果<br/>text + messages]
    SD0 --> R
    RSK --> R
    R --> S[通道发送<br/>单条或多条消息]
    R --> T[更新会话 / 任务日志<br/>持久化观测事实]
    R -. 后台 .-> U[长期记忆刷新]
    R -. 可选 .-> V[记忆偏好 LLM fallback]
```

- **会话快照与本地表面信号**：把每一轮话绑定到当前对话，并在路由前抽取有限的本地事实；这是 boundary context，**不是**单独的「轮次分类」LLM 阶段。
- **意图归一化 LLM**：产出 `first_layer_decision`、`needs_clarify`、`output_contract` 以及可选的 `turn_type` / `target_task_policy` 等字段；运行时再派生 `ask_mode` 和仅用于日志的 route label。当 schema 修复标记出可疑语义契约时，可选的 contract-repair judge LLM 会先细化结构化契约再分发。在 `agent_loop_default` 下，eligible 的低风险 planner-execute class 会把这些字段视为初始提示，由 loop decision envelope 承担普通语义决策。
- **任务队列**：HTTP 调用通过 `POST /v1/tasks` 入队；各通道守护进程也复用同一 worker 任务路径。
- **任务类型**：`kind=ask` 进入归一化 / 路由后策略 / ask 分发流程；`kind=run_skill` 不跑 LLM 路由，直接通过共享技能调度路径执行指定技能。
- **ask 上下文包**：归一化后、ask 分发前统一构建；提供聊天上下文、执行提示词上下文、附件、持久记忆，以及路由后 locator 策略需要的最近执行上下文。
- **Boundary 路由后策略**：ask 上下文包可用之后、分发之前处理 locator 解析、缺 locator 澄清和契约护栏；它消费机器字段和安全状态，不应继续膨胀成按语言维护的语义路由器。
- **任务契约矩阵**：把 semantic kind、allowed action、required evidence 和 response shape 收敛到同一份共享契约，由路由后护栏、直接回答预检、计划校验、证据覆盖校验和最终交付共同使用。
- **调度 / 恢复支路**：调度器触发的直达文本任务可在归一化前收尾；普通调度直达请求可在路由后、进入规划器前完成收尾；恢复讨论走恢复提示词；恢复执行回到正常执行运行时。
- **semantic_route_authority**：当前 live config 使用 `agent_loop_default`。对于结构化字段读取、精确路径/名称列表、绑定路径摘要、最近产物判断、标量计数等 eligible 低风险 class，普通语义权威进入 agent loop。`legacy` 仍是热回滚 token；`shadow` 和 `agent_loop_canary` 是灰度模式。
- **FirstLayerDecision / boundary 分发**：对没有被 agent-loop authority 选中的 case，兼容路径仍处理 `Clarify / DirectAnswer / PlannerExecute`。`AskMode` 是代码分发类型；`AskClarify`、`Chat`、`Act`、`ChatAct` 这类 route label 只从 `AskMode` 派生用于日志和 journal，不再作为第二套路由状态存储。
- **直接回答候选 / 可选预检**：正常聊天答案发送前，运行时可在候选答案与当前运行时事实匹配时直接复用；否则可先跑轻量契约 / advice-only 检查。纯聊天仍保持 `DirectAnswer`，但如果发现需要工具证据，会提升到 `PlannerExecute`；如果发现缺少唯一关键参数，会转成一次澄清。
- **聊天回复 LLM**：只处理确认后的 `DirectAnswer`；纯聊天不进入执行规划器循环。
- **规划器 / 运行时循环**：被 agent-loop authority 选中的 case 与 `PlannerExecute` fallback 都进入多轮执行。大多数轮次会调用规划 LLM；窄范围结构化观测契约可在当前轮由运行时构建确定性观测计划，但仍走同一套循环、观测、护栏与收尾路径。规划步骤类型为 `think`、`call_capability`、`call_tool`、`call_skill`、`synthesize_answer`、`respond`（当前**没有** `delegate` 类型；子任务前缀多用于日志与追踪，而非独立的子循环委派）。`call_capability` 是推荐的能力级规划动作；`call_tool` / `call_skill` 保留为兼容直达动作。
- **Agent-loop runtime context**：复用 ask 上下文包、boundary snapshot、契约、可见能力、memory policy 输出和解析后的提示词，避免记忆压过最新用户指令。
- **技能注册表 + 生成技能文档**：规划器可见技能与 capability metadata 来自运行时 skill views 与生成接口文档，主要由 `configs/skills_registry.toml`、`crates/skills/*/INTERFACE.md`、`external_skills/*/INTERFACE.md`、`prompts/layers/generated/skills/*` 提供。新增规划器可见技能应声明 `planner_capabilities`，而不是新增特定语言的规划分支。
- **CapabilityResolver / PlanVerifier**：能力级动作会先解析到具体 tool 或 skill，再进入执行。Verifier 和契约动作门禁会在真实执行前检查能力可见性、allowed action、必填参数、风险/效果边界、确认要求和 mutation 后验证。
- **call_skill / 直接 run_skill**：规划器的 `call_skill` 先进入 `execution_adapters::run_skill`，再与直接任务路径一起收敛到 `run_skill_with_runner`。共享调度层会做策略与技能开关检查，再按 registry kind 分发：builtin 在进程内运行，external 走 external adapter，runner 才会拉起 `skill-runner` 与具体技能二进制。
- **循环内观测、证据覆盖与观测输出收尾**：工具、技能与合成步骤输出作为循环内证据；证据校验器会在发布前检查 required evidence 和 answer shape。可恢复失败会带着已尝试方法的压缩历史回到规划器；终止型失败会用已观测事实收尾；如果计划只完成观测，也可以通过运行时结构化直答完成交付，只有运行时无法安全格式化时才走观测答案合成。
- **用户可见消息组装**：纯聊天可以保持单条回答。执行、澄清、重试和技能路径可以附加脱敏后的进度/状态 `messages`，并与最终交付正文分离。严格输出、标量和文件 token 契约仍约束最终正文；除非用户明确要求且输出契约允许，否则不能暴露原始 prompt、堆栈、完整工具 JSON 或密钥。
- **最终交付 / 输出契约护栏**：在保存结果前规范文件 token、`messages`、精确标量/严格输出形状与交付一致性。
- **收尾结果**：可同时包含 `text` 和 `messages` 数组；通道适配器在有多条可发布消息时会分别发送。

### LLM 请求流程

```mermaid
flowchart TD
    A[当前用户输入] --> B[构建归一化提示词]
    B --> C[LLM 请求1<br/>意图归一化]
    C --> D[解析 JSON]
    D --> E{结构化结果}
    E --> Er{需要语义契约修复?}
    Er -->|是| Ej[可选 contract-repair judge LLM]
    Er -->|否| Ec[构建 ask 上下文包<br/>记忆 + 附件 + 最近执行]
    Ej --> Ec
    CM[任务契约矩阵<br/>allowed actions + required evidence + response shape] --> E2
    CM --> G0
    CM --> Kv
    CM --> Ev
    CM --> R
    Ec --> E2[Boundary 路由后策略<br/>locator + 契约矩阵护栏]
    E2 -->|调度直达| Fs[调度直达收尾<br/>证据足够时不进规划器]
    E2 -->|恢复讨论| Fr[恢复讨论提示词]
    E2 -->|恢复执行| H
    E2 -->|运行时已有标量/直答证据| Gd[有证据支撑的直接候选<br/>不额外调用 LLM]
    E2 -->|普通 ask| As{semantic_route_authority}
    As -->|agent_loop_default + eligible class| H[构建 agent-loop runtime context]
    As -->|legacy / shadow / non-eligible| Bd{Boundary 分发<br/>兼容路径}
    Bd -->|first_layer_decision=clarify| F[澄清问句]
    Bd -->|first_layer_decision=direct_answer| G0[直接回答候选 / 可选预检<br/>运行时证据 + 契约检查]
    G0 -->|有证据候选| VP
    G0 -->|直接回答| G[构建直接回答聊天提示词]
    G0 -->|澄清| F
    G0 -->|提升为执行| H
    Bd -->|first_layer_decision=planner_execute| H
    SK[技能注册表 + 生成技能文档<br/>planner capabilities] --> H
    SK --> Kr
    G --> Ic[后续 LLM 请求<br/>聊天回复]
    Fr --> Ir[后续 LLM 请求<br/>恢复讨论]
    H --> H0{窄范围确定性<br/>观测契约?}
    H0 -->|是| Jd[运行时构建观测计划<br/>不调用规划 LLM]
    H0 -->|否| Ip[后续 LLM 请求+<br/>每轮规划]
    Ip --> J[解析规划步骤]
    J --> Kr[CapabilityResolver<br/>call_capability -> 具体动作]
    Jd --> Kr
    Kr --> Kv[PlanVerifier + 契约动作门禁<br/>schema + allowed action + 风险/效果]
    Kv --> K{已验证步骤类型}
    K -->|respond| L[回复正文]
    K -->|call_tool| M[执行工具<br/>虚拟工具调度]
    K -->|call_skill| Ma[execution_adapters::run_skill<br/>共享技能入口]
    Ma --> Ms[run_skill_with_runner<br/>技能调度]
    Ms -->|builtin| Msb[进程内 builtin 技能]
    Ms -->|external| Mse[External skill adapter]
    Ms -->|runner| Msr[skill-runner 子进程]
    Msr --> Msbinary[具体技能二进制]
    K -->|synthesize_answer| N[按证据引用的合成 LLM]
    M --> O[记录循环内观测<br/>失败 / 进度状态]
    Msb --> O
    Mse --> O
    Msbinary --> O
    N --> O
    O --> Ev[证据覆盖校验<br/>required evidence + answer shape]
    Ev --> P{是否再规划一轮?}
    P -->|是 / 缺证据 / 修复| H
    P -->|否 / 证据足够| Q[观测输出收尾<br/>必要时直答或合成]
    L --> VP[用户可见消息组装<br/>契约约束正文 + 可选脱敏消息]
    Q --> VP
    Ic --> VP
    Ir --> VP
    F --> VP
    Fs --> VP
    Gd --> VP
    VP --> R[最终交付 / 输出契约护栏]
    R --> S[收尾 / 用户可见回复]
    S -. 可选后台 .-> T[长期摘要 LLM]
    S -. 可选后台 .-> U[记忆偏好抽取 LLM]
```

- **LLM 请求1 / 意图归一化**：只做结构化理解，不产出最终答案。在 `agent_loop_default` 下，它的 route 字段对 eligible 普通语义 class 是初始提示，不是最终语义权威。如果 schema 归一化发现无法安全确定性修复的语义契约，可选的 contract-repair judge LLM 会在 ask 上下文包分发前运行。
- 本图只覆盖常规 `kind=ask` 的 LLM 路径。`kind=run_skill` 和调度器触发的直达文本 ask 不发生归一化 / 规划器 LLM 请求，会走各自的直接任务路径收尾。
- **构建聊天提示词 / agent-loop runtime context**：把模式、会话态、boundary snapshot、工作上下文、可见能力与输出约定拼进后续请求；只有当前循环轮次确实调用规划 LLM 时，才需要构建完整规划提示词。
- **任务契约矩阵**：把同一套 semantic kind、allowed action、required evidence 和 response shape 同时用于路由后策略、直接回答预检、计划校验、证据覆盖校验、最终交付以及生成式 NL 评测。
- **技能注册表 + 生成技能文档**：规划提示词与 resolver 映射从已启用技能视图、生成接口文档和 `planner_capabilities` 构建，技能能力增长应由数据/契约驱动。
- **DirectAnswer 候选 / 预检**：**DirectAnswer** 在发送聊天回复前可能复用运行时证据支撑的直接候选，或先跑一次轻量预检 LLM。确认纯回答时才进入聊天回复并收尾；发现缺少必要信息时转澄清；发现需要真实工具/工作区/系统证据时提升到 `PlannerExecute`。
- **semantic_route_authority**：当前配置为 `agent_loop_default`，会选择任意 eligible 低风险迁移 class，而不是只依赖单个 canary token。非 eligible 或高风险 case 仍保留兼容分发，直到各自 deletion gate 通过。
- **PlannerExecute / agent loop**：通常按循环进行**一轮或多轮**规划 LLM；窄范围确定性观测契约可以在当前轮跳过规划 LLM，改由运行时生成观测步骤。规划 JSON 只包含 `{think, call_capability, call_tool, call_skill, synthesize_answer, respond}`（**没有** `clarify`、`delegate` 步骤类型）。优先使用 `call_capability`；`call_tool` 和 `call_skill` 保留为兼容直达动作。`AskMode` 的收尾样式负责控制执行结果是直接返回还是经过聊天包装。
- **CapabilityResolver / PlanVerifier**：`call_capability` 会在执行前归一到当前具体 tool/skill 实现。Verifier 和契约动作门禁会在 executor 前阻断不可用能力、契约不允许的动作、缺必填字段、风险预算越界和不安全 mutation 计划。
- **执行工具或技能**：跑真实能力，避免模型假装已执行。技能执行使用共享调度层；只有 runner 技能会启动 `skill-runner`。
- **synthesize_answer**：当规划里包含该步骤时会**额外**触发合成 LLM；可与执行交错，**不一定**是「全部规划结束后的固定第三次 LLM」。
- **证据覆盖 / 观测输出收尾**：观测必须满足契约里的 required evidence 和 answer shape 后才能发布。如果计划在观测步骤后没有终端 `respond`，运行时仍可发布结构化直答，或走观测答案合成路径。可恢复失败会作为已尝试方法证据交给后续规划轮，而不是藏在 shell fallback 里。
- **用户可见消息组装**：纯聊天回复可以不附加额外进度块；澄清与执行路径可以附加脱敏的进度/状态消息，但最终正文仍受输出契约约束，不应暴露原始工具 JSON、prompt、堆栈或密钥，除非用户明确要求且契约允许。
- **最终交付 / 输出契约护栏**：在最终任务持久化前执行交付规范化与输出契约验证。
- **收尾**：保存用户可见结果后，还可能启动后台记忆任务，包括长期摘要刷新，以及受 `configs/memory.toml` 控制的可选偏好抽取。

## 自然语言契约边界

RustClaw 的原则是：自然语言理解交给 LLM，运行时只消费结构化契约。意图归一化器和规划器可以阅读用户表达、示例、技能文档和多语言提示词，但进入 Rust 运行时前，语义必须已经落到稳定字段里。

运行时允许依赖的确定性输入包括：

- schema enum，例如 `semantic_kind = "package_manager_detection"`
- action name，例如 `read_field`、`validate_config`、`transform_data`
- registry metadata 与 `planner_capabilities`
- `TaskContract` / `OutputContract`、结构化 locator、明确的 `field_path`
- JSON/TOML/YAML 字段路径、文件扩展名、工具结构化输出、exit code、error kind、risk/effect metadata

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

### 记忆流程图

```mermaid
flowchart TD
    User[用户请求] --> Ingress[通道 / UI / POST /v1/tasks]
    Ingress --> Identity[解析身份<br/>user_key + user_id + chat_id]
    Identity --> Session[(conversation_states<br/>别名 + 活跃任务锚点)]
    Identity --> Worker[worker_once]
    Worker --> Kind{任务类型}
    Kind -->|run_skill| DirectSkill[直接 run_skill 路径]
    Kind -->|ask| Snapshot[会话快照与本地表面信号]
    Session --> Snapshot
    Snapshot --> Normalizer[意图归一化]
    Normalizer --> Bundle[Ask 上下文包]
    Bundle --> Recall[结构化记忆召回]
    Index[(memory_retrieval_index)] --> Recall
    Stores[(memories<br/>user_preferences<br/>memory_facts<br/>long_term_memories)] --> Index
    Recall --> Safety[安全 / 过期 / 状态过滤]
    Safety --> Policy[Memory use policy<br/>route / planner / chat / skill]
    Policy --> RouteCtx[路由记忆上下文]
    Policy --> PlannerCtx[规划器记忆上下文]
    Policy --> ChatCtx[聊天记忆上下文]
    Policy --> SkillArgs[技能 _memory 参数]
    SkillArgs --> SkillPolicy[Registry memory_policy 裁剪]
    RouteCtx --> PostRoute[路由后策略<br/>contract + locator 护栏]
    PlannerCtx --> Runtime[规划器 / 运行时循环]
    ChatCtx --> Chat[直接聊天回答]
    SkillPolicy --> SkillRuntime[共享技能调度<br/>builtin / external / runner]
    PostRoute --> Runtime
    PostRoute --> Chat
    DirectSkill --> SkillRuntime
    SkillRuntime --> Runtime
    Runtime --> Visible
    Chat --> Visible
    Visible --> Finalize[任务收尾 + journal]
    Finalize --> RecentWrite[短期写入过滤]
    RecentWrite --> Memories[(memories)]
    Finalize -. 可选 .-> MemIntent[结构化记忆意图提取]
    MemIntent --> Validate[运行时 enum / scope / confidence / safety 校验]
    Validate --> Prefs[(user_preferences)]
    Validate --> Facts[(memory_facts)]
    Finalize -. 可选 .-> Summary[长期摘要刷新]
    Summary --> LongTerm[(long_term_memories)]
    Facts --> Conflict[冲突组覆盖 / 过期]
    Conflict --> Facts
    Memories --> Reindex[Index 更新 / reindex_on_startup]
    Prefs --> Reindex
    Facts --> Reindex
    LongTerm --> Reindex
    Reindex --> Index
    Policy --> Trace[Task journal memory_trace]
    Runtime --> Trace
```

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
- `webd` 可以作为 `clawd` 前面的反向代理和登录会话桥接层

## 技能体系

RustClaw 当前内置的技能已经比较完整，按类别可大致分为：

- 系统与运维：`system_basic`、`process_basic`、`service_control`、`health_check`、`log_analyze`、`task_control`
- 文件与开发工具：`run_cmd`、`fs_basic`、`config_basic`、`config_edit`、`config_guard`、`archive_basic`、`fs_search`、`git_basic`、`package_manager`、`install_module`、`docker_basic`、`db_basic`
- 网络与内容处理：`http_basic`、`rss_fetch`、`browser_web`、`doc_parse`、`transform`、`web_search_extract`
- 多模态：`image_generate`、`image_edit`、`image_vision`、`audio_transcribe`、`audio_synthesize`
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

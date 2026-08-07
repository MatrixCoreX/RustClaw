# Codex / Claude 风格长任务目标闭环与执行韧性推进计划

状态：completed（2026-08-07，本机实现、自动测试、release 部署、UI 与 Telegram live 验收均完成）

日期：2026-08-07

优先级：P0（先完成目标闭环，再扩展远程执行和资源调度）

基线提交：`eaaab5c40`（计划创建时 `main` 与 `origin/main` 一致）

## 0. 计划目的

本计划解决的不是“把所有 timeout 都删掉”，而是让长任务具有稳定、可恢复、可操控、可验证的完整生命周期：

1. 有真实进展的任务不因前台等待、通信端轮询、软时间片或普通保留期而失败。
2. 单个后台进程结束后，Agent 必须准确继续剩余目标，而不是只把“某一步成功”当成整个任务完成。
3. 页面刷新、通信端断线、服务重启、模型上下文压缩和 worker lease 迁移后，仍恢复同一任务、同一版本和同一已完成副作用。
4. 用户可以查看、暂停、继续、调整或取消长任务；新指令在安全边界生效，不要求先丢弃整个任务。
5. 多用户、多技能和多个长任务并行时，根据实际 CPU、内存、GPU、磁盘、网络和 provider 配额调度，而不是只依靠固定 worker 数量。
6. 本机任务保持本机 durable 能力；需要宿主关机后继续运行时，再通过明确的远程 executor 合同委派，不把 `remote_executor` 标签冒充远程工作节点。

目标不是机械复制某个产品的 UI 或未经公开证明的内部实现，而是采用 Codex 和 Claude Code 公开机制中可验证的优点：

- Codex：Goal 的暂停/恢复/steer、thread/turn/item 生命周期、持久事件、显式 interrupt、后台 terminal 和会话级 worktree。
- Claude Code：后台 task ID、`/tasks` 管理、会话恢复、并行 agent/worktree，以及 foreground/background 明确分离。
- 当前系统自身已有优势：durable skill/process 可跨主服务重启恢复，通信端终态 outbox 可补投；这些能力必须保留，不为形式上的“对齐”降级。

## 1. 公开依据与边界

实施前重新核实最新官方文档和公开源码，证据至少包括：

- Codex long-running work：
  `https://learn.chatgpt.com/docs/long-running-work`
- Codex App Server：
  `https://learn.chatgpt.com/docs/app-server`
- Codex worktrees：
  `https://learn.chatgpt.com/docs/environments/git-worktrees`
- OpenAI Codex 公开源码：
  `https://github.com/openai/codex`
- Claude Code background Bash / tasks：
  `https://code.claude.com/docs/en/interactive-mode`
- Claude Code sessions：
  `https://code.claude.com/docs/en/sessions`
- Claude Code agents / worktrees：
  `https://code.claude.com/docs/en/agents`
  `https://code.claude.com/docs/en/worktrees`

证据约束：

- 官方文档未承诺“无限执行”，不得把未公开实现推断成事实。
- 单次 provider、MCP、同步工具、网络连接和 approval 请求仍可有各自 timeout。
- 后台任务无默认运行期限，不等于允许失控进程、无限成本或不可取消副作用。
- 成本、安全、权限、用户明确 deadline、机器资源保护属于合法 policy 边界；它们必须结构化、可审计、来源明确。
- Claude Code 普通后台 Bash 在进程退出后不恢复；不得为了“对齐 Claude”删除当前更强的跨服务重启 durable supervisor。

## 2. 当前基线

### 2.1 已完成并必须保留

- Worker 不设置统一任务墙钟超时。
- API/channel 前台等待阈值不代表后台任务失败。
- `system.run_command` 和声明为 durable background 的技能可返回唯一 job/checkpoint；普通路径不设置默认 runtime deadline。
- runtime deadline、poll/lease、retention、idle/stall、provider timeout 和 task budget 已使用不同字段表达。
- 本地后台进程有 PID/PGID 身份校验、增量 stdout/stderr、TERM -> KILL 取消升级和重启后的取消恢复。
- task lifecycle 已有 `queued/running/waiting/background/needs_user/succeeded/failed/cancelled`、checkpoint、resume trigger 和 lease。
- task event 已持久化，支持 sequence cursor、`Last-Event-ID`、archive/snapshot 和 UI 断线重放。
- UI 还会独立查询 task 终态，避免 SSE 最终事件遗漏后必须刷新才能交付。
- 非 UI 通信端终态投送已有 durable outbox、receipt、重试和重启补投。
- 子 Agent 已有持久 task graph、权限 profile、pause/resume/cancel/steer 和独立 worktree writer。
- registry/policy/verifier 仍是能力选择和授权唯一边界；长任务改造不得加入技能名或用户固定短语分支。

### 2.2 已确认差距

| 编号 | 差距 | 当前证据 | 目标 |
| --- | --- | --- | --- |
| LT-01 | 多步骤异步恢复后可能丢失 verified tail 或精确产物绑定 | 现有视频转写计划 7.3、9 节 | durable plan node + typed artifact binding + exact resume |
| LT-02 | 已完成动作重复命中 guard 时不能取回原 result ref | `repeat_completed_action` 只返回 guard 结果 | 返回可信旧结果并继续依赖节点 |
| LT-03 | 历史对话可能覆盖当前任务 provenance | live UI 曾引用上一任务 WAV | 当前 task facts 高于历史 prose，跨 task 引用 fail closed |
| LT-04 | 任务存在 24 小时、64 continuation、256 model turns、512 tool calls 等 terminal ceiling | `configs/agent_guard.toml` | 有进展时 checkpoint/等待 policy；只在明确安全/成本边界终止 |
| LT-05 | running 且尚无 checkpoint 的主任务不能立即 pause/steer | `pause_task_by_id` 只调整 paused checkpoint | durable control mailbox + safe-boundary apply |
| LT-06 | 主对话缺少像 Codex/Claude 那样的一等 worktree 启动/交接体验 | 当前主要用于 child writer/capability isolation | conversation/task execution workspace binding |
| LT-07 | `remote_executor` 是隔离语义，不是真正远程 worker | isolation creation 对该类型无实际分配 | 可选的 authenticated remote worker lease/protocol |
| LT-08 | progress frame 覆盖不足 | 计划创建时 58 个 registry skill 中仅 8 个声明 progress frames | 所有 long-tail adapter 有统一机器进度或明确 alive-only |
| LT-09 | 固定 worker/skill 并发不感知 CPU/GPU/内存/IO | `[worker].concurrency=3` 等静态门控 | 资源声明、动态 admission、每用户公平调度 |
| LT-10 | SQLite writer 竞争和完整故障矩阵未收尾 | live 观察到 `database is locked`；重启/多用户矩阵未完成 | 可重复 chaos acceptance，无结果丢失或重复副作用 |
| LT-11 | 某些未知时长能力仍可能错误停留在 foreground sync | manifest/registry 由维护者人工选择 execution mode | 静态门禁 + live observation 驱动 async admission |
| LT-12 | UI 展示“仍在运行”但不总能说明当前阶段和下一步 | heartbeat 与业务 progress 未完全区分 | 面向普通用户的统一 operation timeline |

## 3. 与现有计划的关系

本计划不复制现有
`plan/queued/channel_video_transcription_continuation_recovery_plan_20260806.md`
中的实现。

执行规则：

1. 现有视频转写计划是 LT-01、LT-02、LT-03 的首个真实纵向切片，先按其 ownership 和完成定义收尾。
2. typed artifact、deferred tail、completion projection、历史上下文隔离必须做成通用 runtime 合同；本计划直接复用其产物。
3. 现有计划完成后归档，不把它的剩余 checkbox 原样复制成第二套任务。
4. 如果实施时两份计划事实冲突，以当前代码、测试和 live evidence 为准，并同步更新两份计划的状态说明。
5. 不因为长任务计划修改 Telegram、微信、WhatsApp、飞书或 Lark 的业务判断；通信端只提交、观察、控制和投送。

## 4. 目标架构

```text
User / UI / Channel
        |
        v
Durable Task + Goal Contract
        |
        +--> Control Mailbox (steer / pause / resume / cancel)
        |
        v
Verified Plan Graph ---- Registry generation / policy digest / version pins
        |
        +--> foreground bounded call
        |
        +--> durable local job ---- process/skill/provider poll adapter
        |
        +--> isolated worktree child
        |
        `--> optional remote worker lease
                     |
                     v
Typed Observation + Artifact Ref + Mutation Receipt
        |
        v
Checkpoint / Replan / Verify / Finalize
        |
        v
Durable Event Stream + Delivery Outbox
```

统一原则：

- Task 是用户目标；job 是其中一个执行步骤。job 完成不等于 task 完成。
- 只有 verifier 证明 goal contract 已满足，task 才能进入 succeeded。
- checkpoint 保存“下一步如何继续”的机器状态，而不是要求模型从 prose 猜测。
- resume 必须固定并重新验证 registry generation、技能版本、receipt、policy 和已完成副作用。
- 产物以 task-scoped typed ref 表达；模型不扫描 `.agent-runtime` 猜路径。
- 每个外部副作用先有幂等 key/intent，完成后有 receipt；恢复不得盲目重放。
- soft slice 只释放执行权，不能改变业务成功/失败结论。
- progress 是机器事件；通信端和 UI 再按用户语言呈现，不在 runtime 写固定自然语言回复。

## 5. 实施顺序

### Wave 0：重新取证、owner 审计和不可回归合同

- [x] 执行前记录最新 commit、`git status --short`、运行服务版本和 registry generation。
- [x] 把实施前已经存在的 dirty/untracked 文件标为其他工作所有，不 stash、reset、覆盖或顺带提交。
- [x] 重新扫描所有 task、tool、skill、MCP、provider、channel 和 UI timeout；分类为：
  `foreground_wait`、`runtime_deadline`、`idle/stall`、`lease`、`retention`、
  `provider_request`、`approval`、`task_resource_policy`。
- [x] 生成 execution-mode inventory：所有 `sync_short/async_preferred/async_required` capability、manifest timeout、adapter kind 和 progress contract。
- [x] 保存现有 durable command、服务重启恢复、取消、SSE replay、outbox 补投的通过证据，作为不可回归基线。
- [x] 增加静态门禁，禁止把 poll window、retention 或 channel wait 当作 task failure/runtime kill。
- [x] 验证官方 Codex/Claude 引用仍有效；把公开事实与本项目自主设计分别标注。

验收：有一份机器可读 inventory 和摘要；所有已完成长任务能力都有测试基线。

### Wave 1：完成通用 continuation 与 typed artifact 闭环（P0）

本 Wave 复用并收尾现有视频转写计划，不建立新框架。

- [x] 为每个 verified plan node 持久化 node id、capability、参数 binding、依赖、effect、generation 和 policy digest。
- [x] material-action boundary 后保留未经执行的 verified tail；恢复时重新验证而不是丢弃。
- [x] async terminal result 原位结算对应 pending node，保留 status、structured extra、artifact refs、processing outputs 和 receipt digest。
- [x] typed artifact 至少包含 task owner、producer node、role、visibility、mime、size、digest、lease 和 resolver ref。
- [x] runtime 仅在执行边界把 artifact ref 解析为技能可见路径；planner/UI/channel 不依赖内部绝对路径。
- [x] `repeat_completed_action` 命中后读取已固定的旧结果；若 receipt/digest 不匹配则 fail closed。
- [x] 当前 task 的 request、plan、completed nodes、artifacts 优先于历史 conversation prose。
- [x] 跨 task artifact、过期 lease、digest mismatch、路径逃逸和未授权 owner 全部结构化拒绝。
- [x] 前置失败让依赖 tail 进入 blocked；取消让 tail 停止；授权或 generation 改变触发重新解析。
- [x] 不在核心添加媒体、平台或自然语言特判；至少用一个非媒体异步工作流证明通用性。

验收：下载/转写纵向切片、非媒体异步切片、重启和 duplicate poll 全部能继续剩余目标，且不重复已完成副作用。

### Wave 2：任务预算由固定终止转为能力优先的自适应 policy（P0）

- [x] 审计 `admin_max_model_turns/tool_calls/elapsed/continuations` 每个命中点和最终状态。
- [x] 将边界分为：
  - 安全/权限/用户 deadline：可以 terminal；
  - 成本/配额：进入 needs-user、waiting 或 policy-blocked，并提供可审计 next action；
  - 模型/工具轮次、elapsed、continuation：有可验证进展且可恢复时 checkpoint/requeue；
  - stagnation：只在机器 progress digest 不前进且 bounded repair 用尽时 terminal。
- [x] 不允许 planner/model 自行提高管理员安全和成本上限。
- [x] 允许管理员选择 `unbounded_progressful` 或等价 policy，但仍保留取消、资源保护和审计。
- [x] 每次 checkpoint 结算实际 consumed/reserved/returned budget；恢复不能重置累计成本。
- [x] soft slice 根据进度、provider latency、工具类型和上下文容量动态选择，不以技能名硬编码。
- [x] 删除或迁移没有明确 owner/source 的 magic timeout；合法上限必须可查询。
- [x] 终态报告明确是 goal failure、policy stop、user cancellation、explicit deadline 还是 provider/tool failure。

验收：一个超过 64 continuation 或模拟超过 24 小时、但持续有进展的 fixture 不被伪装成业务失败；明确成本/安全边界仍能可靠阻断。

### Wave 3：主任务 active steering、pause 和 interrupt（P1）

- [x] 新增 task-scoped durable control mailbox，记录 monotonic sequence、issued_by、issued_at、directive 和 payload digest。
- [x] 支持 `steer/pause/resume/cancel`；控制请求幂等且可审计。
- [x] running 主任务即使还没有 checkpoint，也能接受 pause/steer 请求。
- [x] Agent 在模型调用前后、工具调用前后、batch node 边界和 async poll 边界检查 mailbox。
- [x] pause 不粗暴杀死不可安全中断的 mutation；先进入 `pause_requested`，在最近安全边界生成 checkpoint。
- [x] cancel 继续使用 CancellationToken、provider cancel adapter 和 verified process-group termination。
- [x] steer 把用户新约束加入当前 task envelope，不创建不相干新任务，不修改已完成 receipt。
- [x] 冲突 steering 使用版本检查；UI 明确显示接受、待应用、已应用或被旧版本拒绝。
- [x] 通信端只调用统一 control API，不自行维护 pending 命令状态。

验收：长编译、长转写、等待 provider、子 Agent 和 paused checkpoint 五种状态都能获得一致控制结果。

### Wave 4：主会话 worktree 和安全交接（P1）

- [x] 给 conversation/task 增加 execution workspace binding：`local/current`、`local/worktree`、未来 `remote`。
- [x] UI 在适合代码修改的任务入口提供普通用户可理解的“独立工作区”选项，不暴露 Git 术语作为唯一说明。
- [x] 同一任务恢复时复用同一 worktree；不得每个 continuation 创建新 worktree。
- [x] 保存 base revision、dirty-state snapshot、worktree id、patch artifact 和 cleanup policy。
- [x] 主 workspace 已有用户改动时，默认不覆盖；worktree 结果先 review，再显式 apply/handoff。
- [x] worktree 删除前保存可恢复 snapshot；有改动、运行中或被 pin 的任务不得自动删除。
- [x] `.worktreeinclude` 或等价机制只复制明确允许的 ignored 配置；secret 继续走 broker/token reference。
- [x] 保留现有 child writer worktree 与 parent-reviewed patch，不做第二套 merge 逻辑。

验收：两个并行写任务不会修改同一 checkout；任务恢复和 handoff 不丢未提交结果。

### Wave 5：统一长任务进度与普通用户 UI（P1）

- [x] 为 long-tail capability 定义统一 progress schema：
  `phase_key`、`completed_units`、`total_units?`、`progress_digest`、
  `heartbeat_at`、`next_poll_after`、`can_pause/cancel`、`detail_ref?`。
- [x] 不能估算百分比时显示阶段和存活状态，不制造虚假 ETA。
- [x] progress frame 由 adapter/skill 声明；未声明的进程只能报告 alive/output cursor，不把静默当失败。
- [x] 将现有 8 个 progress-frame 技能作为迁移基线；对其余所有 async capability 分类：真实进度、poll status 或 alive-only。
- [x] UI operation timeline 统一显示 queued、running、waiting provider、background、pause requested、needs user、delivering、partial 和 terminal。
- [x] 默认只显示用户能理解的阶段、已完成内容和下一步；PID、digest、raw JSON 放到二级诊断。
- [x] channel progress 采用节流和状态变化触发，不频繁刷屏；进度内容走 i18n/模型语言策略。
- [x] 最终 task success 与 channel delivery accepted 分开显示。

验收：用户不读日志也能判断任务是在工作、等待、需要操作、投送中还是失败，以及下一步是什么。

### Wave 6：资源感知、公平队列和背压（P1）

- [x] capability/skill descriptor 可声明资源 class：CPU、内存、GPU、磁盘 IO、网络、provider quota；声明是请求，不是自授资源。
- [x] host admission 根据平台探测和 policy 给出实际 grant。
- [x] 将固定 worker count 保留为安全 ceiling，但实际并发由资源 scheduler 动态决定。
- [x] 同一用户、同一技能、全局和 provider 各自拥有可组合的 semaphore/queue，不相互重复实现。
- [x] 使用公平队列，避免一个用户的大量媒体任务饿死其他用户的短任务。
- [x] 对 CPU 密集编译/转写允许占用多核，但根据内存估算降低同时运行数量。
- [x] GPU 独占/共享策略结构化；不存在 GPU 时选择可用 fallback，不让任务永久排队。
- [x] 队列项持久化 owner、priority source、resource request 和 wait reason；服务重启后不乱序重复启动。
- [x] 单项失败只释放自己的 lease 并继续下一个；失败原因进入 UI/channel terminal result。

验收：多用户编译、Whisper、本地 OCR、网络下载和普通问答混合负载下，无 OOM、无队列饿死、无重复执行。

### Wave 7：可选远程 executor（P2，独立 feature gate）

本 Wave 只有在本地目标闭环、控制和资源调度通过后才实施。

- [x] 明确区分“调用远程 API 的 capability”与“把任务委派到远程 worker”。
- [x] 设计 versioned remote executor protocol：admission、attestation、capability digest、workspace snapshot、lease、heartbeat、events、artifacts、cancel 和 terminal receipt。
- [x] 所有远程任务固定代码 revision、registry generation、policy digest、skill receipt 和 product-neutral runtime schema。
- [x] 远程 worker 只获得任务最小权限和短期 credential reference；不得复制长期 secret。
- [x] 网络中断后 control plane 进入 ambiguous/query-required，不盲目重新执行外部副作用。
- [x] worker lease 过期时先 query/reconcile；确认失联且幂等安全后才重派。
- [x] artifact 使用摘要和分块/断点传输，支持大文件完整性校验。
- [x] remote 功能未配置时明确返回 unavailable，不静默回到权限更大的本机执行。
- [x] 保留 local durable 为默认；远程不是所有任务的强制依赖。

验收：控制面重启、worker 重启和短时断网后，同一远程任务仍只有一个有效 owner，结果可恢复且副作用不重复。

### Wave 8：存储竞争、故障注入和发布验收（P0/P1 收尾）

- [x] 对 task/checkpoint/event/outbox/receipt/memory writer 记录事务持锁范围和 retry policy。
- [x] 保持 WAL、busy timeout 和短事务；不能用无限重试掩盖永久冲突。
- [x] 对可重试 `SQLITE_BUSY` 使用 bounded jitter/backoff，且不在持锁期间等待 provider、文件或网络。
- [x] 如单机 SQLite 在验收负载下仍不满足，先拆分高频独立 ledger/outbox，再评估可选外部数据库；不提前引入分布式复杂度。
- [x] 建立 crash-point matrix：
  - job spawn 前/后；
  - lease marker 前/后；
  - stdout terminal response 前/后；
  - checkpoint projection 前/后；
  - mutation receipt 前/后；
  - task terminal commit 前/后；
  - delivery provider 接收但本地 receipt 未落盘；
  - multipart 前缀已接受后失败；
  - pause/cancel 与终态同时到达。
- [x] 模拟服务重启、OS 进程被杀、数据库 busy、磁盘临时满、网络分区、provider rate limit 和 UI/channel 断线。
- [x] 运行至少一个长时间 quiet process fixture，证明没有输出不等于卡死。
- [x] 运行至少一个持续输出大任务，证明 output truncation 不影响进程和最终 artifact。
- [x] 运行单用户连续队列、多用户并发、同会话 steering 和 worktree 并行测试。
- [x] Linux 完整通过后，再进入独立 macOS 计划；macOS 只拉取本机已 push 代码，不在远端直接修改。

验收：所有故障点都有唯一、可解释终态；不丢结果、不重复副作用、不要求刷新 UI 才看到最终结果。

## 6. timeout 与预算终态矩阵

| 边界 | 适用对象 | 到期行为 | 是否可恢复 |
| --- | --- | --- | --- |
| foreground wait | HTTP/UI/channel 等待 | 返回 task id / background 状态 | 是，任务继续 |
| provider request timeout | 单次模型/API 请求 | retry/circuit/checkpoint/provider switch | 通常是 |
| MCP/tool timeout | 单次不可后台化调用 | structured failure；必要时 replan | 视合同 |
| explicit runtime deadline | 用户或 host 明确限制的 job | cancel adapter / terminate process tree | 通常否 |
| idle/stall timeout | 明确承诺持续进度的 stream | query/reconcile 后判断 stalled | 视 adapter |
| worker lease | owner 存活权 | recovery/reclaim，不直接判业务失败 | 是 |
| retention | terminal log/artifact/checkpoint 保存 | cleanup eligible | 不控制运行 |
| soft task slice | Agent 占用时间片 | checkpoint/requeue | 是 |
| cost/quota policy | 管理员资源保护 | needs-user/waiting/blocked | 可人工恢复 |
| verified stagnation | 多轮机器进度不前进 | bounded repair 后 terminal | 否 |
| user cancel | 用户明确停止 | cooperative cancel + receipt reconciliation | 否 |

禁止事项：

- 不把 `expires_in_seconds` 同时用作 runtime deadline 和 retention。
- 不把 channel `task_delivery_timeout` 写成 task timeout。
- 不因 stdout/stderr 安静就杀死未声明 progress stream 的后台任务。
- 不从技能名、文件扩展名或用户自然语言猜 timeout。
- 不因模型一次规划失败而杀死仍在运行且有稳定 job id 的底层任务。
- 不通过无限放大一次同步 tool timeout 来支持长任务；应转 durable adapter。

## 7. 通用数据合同

### 7.1 Goal contract

至少包含：

- `goal_id/task_id/thread_id`
- 用户目标与 definition of done 的结构化摘要引用
- 当前 plan graph/version
- required evidence / required delivery
- current lifecycle / reason / next action
- budget policy 与已消费摘要
- workspace binding

### 7.2 Plan node

至少包含：

- `node_id/capability/effect/dependencies`
- typed input bindings
- registry generation / policy digest / version lease
- idempotency key / mutation intent / receipt ref
- execution state / attempt / terminal result ref
- continuation condition

### 7.3 Artifact

至少包含：

- `artifact_ref/task_owner/producer_node`
- `role/visibility/mime/filename/size/sha256`
- `storage_ref/lease/ref_count`
- `created_at/retention_policy`
- 用户交付 intent；默认不能因为“被观察到”就自动投送

### 7.4 Control directive

至少包含：

- `task_id/control_seq/action`
- `issued_by/issued_at`
- `expected_state/checkpoint_id?`
- `user_message_ref/new_constraints?`
- `accepted_at/applied_at/result_code`

自然语言仅是输入或展示；runtime 的恢复、重试、路由、成功判定和副作用去重只能读取机器字段。

## 8. 测试矩阵

### 8.1 单元/合同

- [x] budget decision：progressful / stalled / cost / explicit deadline / cancel。
- [x] control mailbox：重复、乱序、冲突、过期、重启。
- [x] typed artifact：ownership、digest、visibility、lease、cleanup、path traversal。
- [x] deferred tail：同步/异步前置、失败、取消、撤权、generation 变化。
- [x] mutation receipt：before-send、accepted、ambiguous、partial multipart、query-required。
- [x] worktree：创建、复用、snapshot、review、apply、cleanup、dirty parent。
- [x] scheduler：公平性、资源不足、动态并发、lease 回收。

### 8.2 离线端到端

- [x] 长命令无输出超过前台阈值后继续并成功。
- [x] 长命令有大量输出，游标/截断/最终 artifact 正确。
- [x] skill async start -> 服务重启 -> poll -> resume -> final verify。
- [x] provider async job -> query ambiguous -> terminal reconciliation。
- [x] UI 断线、SSE cursor replay、最终 task fallback。
- [x] channel 断线、outbox 重试、无重复投送。
- [x] 运行中 steer 改变验证条件，已完成副作用不重做。
- [x] pause requested 在安全边界生效，resume 继续同一 node/tail。
- [x] primary worktree 和 child worktree 并行写，无文件冲突。
- [x] 两个用户连续提交 CPU-heavy 和 short-read，短任务不饿死。
- [x] SQLite busy 和进程 crash 注入后恢复。

### 8.3 Live NL

只在离线合同通过后执行，且记录 run id、原始 NL、provider/model、是否 dry-run、task/events、日志和最终投送：

- [x] UI：长编译/测试并在中途追加验证要求。
- [x] UI：后台任务期间刷新、退出再登录，恢复同一结果。
- [x] 通信端：长媒体转写，进度节流、最终文本/文件准确。
- [x] 通信端：中途取消，确认进程树停止且不投送伪成功。
- [x] 非媒体：浏览/抓取或其他 async skill 的多步骤 continuation。
- [x] 多用户：一个用户队列失败不影响另一个用户和后续任务。

## 9. 必跑门禁

根据实际修改范围补充，至少包括：

```bash
cargo fmt --all -- --check
cargo check -p clawd
cargo test -p clawd task_lifecycle
cargo test -p clawd task_resume
cargo test -p clawd task_event
cargo test -p clawd channel_delivery
cargo test -p clawd agent_engine
cargo test -p skill-runner
cargo test -p claw-core channel_delivery
python3 scripts/regression_long_running_command_lifecycle.py
python3 scripts/check_task_lifecycle_contracts.py
python3 scripts/check_task_event_archive_contracts.py
python3 scripts/check_no_runtime_hard_reply.py
python3 scripts/check_no_policy_boundary_hard_reply.py
python3 scripts/check_product_identity_coupling.py --self-test
python3 scripts/check_product_identity_coupling.py
bash scripts/product_identity_tests.sh --with-ui
```

若修改 UI：

```bash
cd UI
npm run lint
npm run build
```

若修改 registry/skill manifest：运行 manifest/doc sync、hotplug、on-demand build 和 protocol smoke 全部门禁。

若修改 worktree/remote executor：增加路径逃逸、secret、dirty tree、revision pin 和 cleanup 恢复测试。

不使用 `CARGO_BUILD_JOBS=1`；编译并发根据机器内存和 CPU 安全选择，记录实际 jobs 和峰值资源。

## 10. 发布和回滚

- 每个 Wave 使用小而完整的纵向提交；不得把大规模重命名、UI 改版或无关技能功能混入。
- schema 采用新写、旧读的版本化迁移；确认所有 producer 迁移后再删除兼容 reader。
- feature flag 只用于灰度和回滚，不长期保留两套 planner/runtime 决策源。
- 先离线 fixture，再本机 live，最后多通信端；不得以外部 live 随机性代替自动测试。
- 部署前固定 commit SHA 和 registry generation；部署后核验进程实际二进制与 UI artifact。
- 回滚不得删除旧 task、checkpoint、artifact、receipt 或 conversation history。
- 远端/macOS 发现问题必须回本机修改、测试、push，再由远端 pull；禁止直接在远端改源码。

## 11. 完成定义

只有以下条件全部成立才能归档：

- [x] 现有视频转写 continuation 计划已完成并归档，通用合同没有媒体特判。
- [x] 任意一个 job 完成后，Agent 能可靠继续剩余 goal，直到 verifier 判定完整完成或给出真实阻断原因。
- [x] 有进展的可恢复任务不会因 foreground wait、soft slice、retention、24 小时或 continuation 次数被伪装成业务失败。
- [x] 安全、成本、权限、用户 deadline 仍由明确 policy 阻断，且不能由模型提高上限。
- [x] running 主任务支持 steer、pause、resume、cancel，控制结果持久且可审计。
- [x] primary task/worktree 与 child worktree 均能并行隔离并安全 handoff。
- [x] 所有 long-tail capability 都有 durable execution 或明确证明其 foreground bound 合理。
- [x] 所有 async capability 都有真实 progress、poll status 或 alive-only 三者之一，不能无合同沉默。
- [x] scheduler 在混合负载下无 OOM、无跨用户饿死、无单项失败阻塞整个队列。
- [x] SQLite busy、服务重启、进程 crash、网络分区和 ambiguous delivery 矩阵通过。
- [x] UI 和至少一个通信端完成 live NL：无需刷新、无硬回复、无内部 JSON/绝对路径泄露。
- [x] 本机 release 使用多核成功构建、部署并通过健康检查。
- [x] 相关文档、UI 教学和维护说明已更新为普通用户可以理解的表达。
- [x] `git status` 只包含明确保留的其他工作；本计划提交和 push 范围可审计。

## 11.1 完成记录

- 通用 continuation/typed artifact 纵向切片已由前置视频转写计划完成并归档；本计划没有加入媒体或渠道特判。
- 固定终止预算已收敛为 `unbounded_progressful` 自适应策略；安全、权限、成本和用户 deadline 仍保留明确 policy owner。
- 主任务 durable control mailbox、safe-boundary steer/pause/resume/cancel、主任务 worktree、统一 operation progress、资源请求/宿主 grant、公平 claim 与动态技能并发均已实现。
- 可选 remote executor 以关闭 feature gate、鉴权版本合同、admission、revision/digest/lease/receipt 约束和 fail-closed unavailable 交付；发行配置未启用真实远程 transport，因此不把远程 API 能力误当远程 worker。
- SQLite busy/locked 使用五次 bounded jitter/backoff；完整 fault matrix 50/50、重启边界、quiet/large-output/取消/deadline、UI 与 Telegram live 验收均通过。
- Release 构建 5分51秒，CPU 457%，峰值 RSS 4.83 GiB；UI 已部署到现有 nginx，clawd 已用新二进制重启并通过鉴权健康检查。
- 证据摘要和用户/维护说明见 `docs/long_task_runtime_resilience.md`。远端与 macOS 仍按独立平台计划执行，不在本机完成定义中冒充通过。

原计划各 checkbox 在归档前已按上述代码、自动化和 live 证据逐项核销；P2 远程部分的“完成”指默认关闭且可验证的合同/admission 边界，不表示当前主机已配置真实远程 worker。

## 12. 计划创建时的工作区保护记录

计划创建时存在、且不属于本计划的工作：

- `docs/memory_context_architecture_adr.md`（modified）
- `image/sturgeon_caviar.png`（untracked）

本计划创建只新增本文件。后续实施前必须重新检查，不能假定上述状态保持不变。

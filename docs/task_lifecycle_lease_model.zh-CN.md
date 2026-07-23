# 任务生命周期 Lease 模型

RustClaw 使用两层机器可读 lease 支持持久任务执行：

- SQLite 任务行 worker lease，用于认领 queued/running 工作并发送 heartbeat；
- `result_json` 中的 checkpoint resume-executor lease，用于恢复 paused/background 工作且不重放已完成副作用。

两层都是结构化协议。Runtime recovery、取消、resume 和 polling 不得解析用户可见 `text` 或 `error_text`。

## 当前模型

- `tasks.status` 是粗粒度数据库状态：`queued`、`running`、`succeeded`、`failed`、`canceled` 或 `timeout`。
- `tasks.lease_owner`、`tasks.lease_expires_at`、`tasks.claim_attempt` 和 `tasks.claimed_at` 是 task worker lease 字段。Queued claim 设置 owner 并增加 `claim_attempt`；heartbeat 只续租精确的 active owner/generation。
- `tasks.updated_at` 继续作为粗粒度 heartbeat 和排序时间。查询投影在 active state 中把它暴露为 `task_lifecycle.last_heartbeat_ts`，但它不再是唯一 lease 信号。
- `crates/clawd/src/task_lifecycle.rs` 投影的用户/operator 状态为 `queued`、`running`、`waiting`、`background`、`needs_user`、`succeeded`、`failed` 或 `cancelled`。
- Paused/background checkpoint recovery 使用 `result_json` 结构化字段，不使用自然语言：
  - `task_lifecycle.checkpoint_id`
  - `task_lifecycle.next_check_after`
  - `task_lifecycle.resume_executor.executor_state`
  - `task_lifecycle.resume_executor.lease_expires_at`
  - `task_lifecycle.resume_executor.resume_directive`
  - `task_checkpoint.resume_entrypoint`
  - `task_checkpoint.pending_async_job`
- Task query/list 在字段存在时投影 worker lease：
  - `task_lifecycle.lease_owner`
  - `task_lifecycle.lease_expires_at`
  - `task_lifecycle.claim_attempt`
  - `task_lifecycle.claimed_at`
  - `task_lifecycle.attempt_id`

## 状态词表

| 层 | Token | 含义 |
| --- | --- | --- |
| `tasks.status` | `queued` | 任务已接受，等待 worker claim。 |
| `tasks.status` | `running` | 任务处于活动状态，包括 checkpoint 后的 `waiting`、`background` 和 `needs_user`。 |
| `tasks.status` | `succeeded` | 任务成功终止。 |
| `tasks.status` | `failed` | 任务不可恢复地失败。 |
| `tasks.status` | `timeout` | 普通活动任务超过 stale-running recovery 上限；生命周期投影为 `failed`，并设置 `terminal_reason=worker_task_timeout`。 |
| `tasks.status` | `canceled` | 任务被结构化 task-control 路径取消；生命周期投影为 `cancelled`。 |
| `task_lifecycle.state` | `queued` | 面向 operator 的 queued 投影。 |
| `task_lifecycle.state` | `running` | 工作正在执行，当前没有公开 checkpoint wait。 |
| `task_lifecycle.state` | `waiting` | 工作在 checkpoint 暂停，等待 `next_check_after` 或手动 resume。 |
| `task_lifecycle.state` | `background` | 工作安全转入后台，通常等待 async poll 或 provider 恢复。 |
| `task_lifecycle.state` | `needs_user` | 工作暂停并保留 checkpoint，等待用户输入。 |
| `task_lifecycle.state` | `succeeded` | 面向 operator 的成功终态。 |
| `task_lifecycle.state` | `failed` | 面向 operator 的失败终态。 |
| `task_lifecycle.state` | `cancelled` | 面向 operator 的取消终态。 |

`paused` 不是 runtime 机器状态。UI 可以把它作为 `waiting` 或 `background` 的友好标签，但持久化代码必须使用上表明确 token。

## Lease 层

### Task Worker Lease

`claim_next_task()` 选择最早的 `queued` 行，原子地移动到 `running` 并写入：

- `lease_owner = state.worker.worker_id`
- `claimed_at = now`
- `lease_expires_at = now + max(worker_task_heartbeat_seconds * 4, 300)`
- `claim_attempt = claim_attempt + 1`

`touch_running_task()` 只有在 `lease_owner` 和 `claim_attempt` 都与 `ClaimedTask` 匹配时，才刷新 `updated_at` 与 `lease_expires_at`。它绝不分配或窃取所有权。进度、checkpoint、成功、失败、超时、mutation receipt 和 claim-owned journal event 写入都使用同一个精确 fence。被拒绝的 worker 写入返回结构化 `worker_lease_lost` 或任务状态冲突，不得静默修改行。

即使复用相同 worker id，也必须使用 generation。Recovery 或 takeover 增加 `claim_attempt` 后，旧 generation 不能继续 heartbeat、发布权威 worker event、提交 side-effect receipt 或完成任务。

### Checkpoint Resume-Executor Lease

Checkpoint 后的 `waiting`、`background` 和 `needs_user` 继续保存为 `tasks.status = 'running'`，可恢复状态位于 `result_json`。Checkpoint 到期时，recovery 路径通过 `task_lifecycle.resume_executor` 认领，并记录有界 claim：

- `resume_claim.claim_attempt`
- `executor_state`
- `previous_executor_state`
- `executor_state_at`
- `executor_claim_expires_at`
- `resume_executor_claim.owner`
- `resume_executor_claim.checkpoint_id`
- `resume_executor_claim.claimed_at`
- `resume_executor_claim.expires_at`

有效 resume-executor lease 在过期前阻止重复 resume。过期 claim 可以从 checkpoint 和已完成 side-effect ledger 恢复。认领到期 checkpoint recovery 会增加任务行 `claim_attempt`。此后的 resume work-item、executor plan、handoff、dispatch、执行结果、结果投影和 lease renewal 都携带该 generation，并要求相同 task-row owner/generation。

## 任务预算 Slice

交互式 agent 工作使用持久化 `TaskBudgetSlice`，不以 planner rounds 或 tool calls 作为普通完成上限。每个 model/tool 结果后，runtime 计算闭合 `BudgetDecision`：`continue`、`finish`、`checkpoint_requeue`、`waiting`、`needs_user` 或 `terminal`。

- Profile 从 verifier 批准的计划长度、capability effect、证据/交付要求、确认状态、execution recipe 和 continuation state 选择；用户措辞不是预算策略输入。
- Soft slice 到期且状态可恢复时生成 `checkpoint_requeue`，发布 claim-fenced `budget_decision` 事件，并在 checkpoint 中保存累计计数。
- Resume 恢复 model/tool/token/cost/elapsed 计数，且 continuation index 只增加一次。
- 重复、结构化停滞、取消、权限/沙箱策略和管理员 hard ceiling 仍是确定性边界。
- 显式 round/tool cap 只允许出现在非交互或 child-task 请求合同中，不是交互 loop 全局默认值。
- Worker 外层 timeout 只作为不可恢复卡死、heartbeat 丢失、管理员 hard ceiling 或 checkpoint 无法恢复时的最后终态边界。健康且可恢复的工作应先进入 soft checkpoint。

管理员 hard ceiling 覆盖累计模型轮次、工具调用、token、估算成本、耗时、continuation 次数和不可恢复工具运行时间，模型输出不能提高这些上限。

### Agent 阶段恢复状态

Agent-loop checkpoint 包含有界 `agent_loop_resume_state` 机器快照。`stage` 只能为 `planning`、`tool_execution`、`verification`、`patch_review` 或 `final_synthesis`。快照保存下一 worker 所需的近期结构化 observation、压缩历史、最后工具输出、最新验证结果和已合成交付候选。

大 observation 不得无界复制；持久 capability result、evidence ref、artifact ref 和已完成 side-effect fingerprint 仍由外部权威存储。Resume 在 planning 或 finalization 前恢复该状态。未知 stage token 回退到机器 `planning`，且不能选择用户可见回复或绕过 verifier、权限、预算和副作用 guard。Release fault matrix 覆盖五种 stage、有界快照、旧 loop threshold 跨越、管理员 hard ceiling 和 continuation 单次递增恢复。

## 副作用 Outbox

当 registry policy 标记 action 为修改型且非幂等时，planner 调用与显式 `kind=run_skill` 调用共享同一个 claim-fenced mutation ledger。权威阶段顺序为：

`intent_recorded -> attempt_started -> receipt_recorded -> verification_pending|verified -> committed`

不明确的 attempt 进入 `reconciliation_required`。绑定 fingerprint 的结构化 resume constraint 可以解析为 `not_applied`（使用相同确定性 key 重试）、`applied`（不重放，协调并提交）或 `still_unknown`（继续等待）。Runtime 不得解析用户可见文本来决定。

确定性 idempotency key 从任务身份和 canonical action fingerprint 派生。受支持 adapter 通过 runner `context.execution`、外部 HTTP `Idempotency-Key` 或隔离本地 adapter 环境接收。Receipt、verification、reconciliation 和 commit 写入都要求精确 active `(lease_owner, claim_attempt)`。即使 worker 在任务最终完成前重启，已带 receipt 的阶段也会阻止原 action 重放。

## 恢复规则

- 普通 stale `running` 任务可根据机器时间戳和 worker lease 状态标记为 `timeout`。
- 取消或终态 timeout 优先于迟到的 worker 结果。即使旧 owner 的进程后来完成，也不能覆盖终态行。
- `waiting`、`background` 和 `needs_user` checkpoint 在数据库中保留为 `running`，以便按 `checkpoint_id` 恢复。
- Resume executor claim 只在 `lease_expires_at` 有效期内有效；过期后可恢复。
- Direct `run_skill` async start 和 planner 触发的 async 工作需要后台 polling 时，都收敛到 `task_checkpoint.pending_async_job` 和 `resume_entrypoint = "poll_async_job"`。
- 取消只根据任务身份和机器状态执行，不得解析用户可见文本。

## 手动控制语义

- `cancel-by-task-id` 设置 `tasks.status=canceled`、`error_text=user_cancelled`，并保存 `task_lifecycle.state=cancelled` 和 `terminal_reason=user_cancelled`。
- `resume-by-task-id` 只适用于已有 checkpoint 的 `waiting` 或 `background` 任务。它把 `next_check_after` 设为当前时间，设置 `resume_due=true`，并保留原 `task_checkpoint`。
- `pause-by-task-id` 只适用于已有 checkpoint 的 `waiting` 或 `background` 任务。它延后 `next_check_after` 并保留原 checkpoint。
- 手动 pause/resume 不能停止工具调用中已经运行的任意代码。长尾工具必须先公开 checkpoint 或 async-job 字段，才能由 API/CLI/UI 安全暂停或恢复。
- `clawcli resume-task <task_id>`、`clawcli pause-task <task_id> --pause-seconds N` 和 `clawcli cancel-task <task_id>` 等 operator 入口只是结构化 task-control 的薄 wrapper，必须使用 task ID 和 lifecycle/checkpoint 机器字段。

## 决策

显式 task lease 列已经进入当前 SQLite schema。当前不需要独立分布式 worker 表，但 task claim 必须使用现有行级 lease 字段和 checkpoint resume lease。该模型支持：

- 前台提交并返回；
- task query 生命周期投影；
- stale 普通任务恢复；
- paused checkpoint 恢复；
- async job polling；
- 按 task ID 直接取消；
- 通过结构化 API 手动 pause/resume checkpoint。

未来多主机执行应建立在现有 task-row lease 列上。只有需要 host health、queue partitioning 或超出 `lease_owner` / `lease_expires_at` 的跨进程所有权时，才增加专属 worker registry。

## 必需检查

- `cargo test -p clawd task_lifecycle -- --quiet`
- `cargo test -p clawd task_resume_execution -- --quiet`
- `cargo test -p clawd async_poll_executor -- --quiet`
- `cargo test -p clawd task_by_id -- --quiet`

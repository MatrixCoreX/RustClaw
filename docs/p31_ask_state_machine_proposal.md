# §3.1 Ask 生命周期状态机 — Proposal

> 目标：把 ask 任务从"入队 → 出最终答覆"的完整生命周期显式建模为一个状态机，
> 每次状态转换写日志（带 `state_from / state_to / reason`），合法 transition
> 由 `debug_assert!` 强保证。受益：
> 1. 任何 ask 任务的轨迹可被一行日志或 task_journal 还原；
> 2. 后续 §3.3 finalize 三层合并时，能精准识别"哪个状态触发了什么 finalize 路径"；
> 3. 给未来超时/失败处理提供统一拦截点。

## 1. 现状盘点

ask 生命周期目前的"状态"散落在多个变量里：

| 维度 | 承载位置 | 表达力 |
|---|---|---|
| 路由模式 | `RouteResult.routed_mode` / `AskMode`（§3.2 已收敛） | ✅ 模式分类 |
| 最终结果 | `TaskJournalFinalStatus` (`success/failure/clarify/resume_failure`) | ⚠️ 仅终态 |
| Loop 内部 | `AgentLoopState.round_no` / `delivery_messages` | ⚠️ 隐式推进 |
| Finalize | `TaskJournalFinalizerStage` (`general/observed_*`) | ⚠️ 标签性，非状态 |
| 中间过程 | 散落 `info!` 日志（`prompt_invocation` / `intent_normalizer` / 等） | ❌ 无统一 phase 语义 |

主路径（worker/ask_pipeline.rs::execute_ask_dispatch 结构）：

```
AskReceived
  └→ AskRouting (intent_normalizer + post_route)
       ├→ AskClarifying      → AskCompleted (clarify_question)
       ├→ AskResumeDiscussing → AskCompleted (LLM 直答)
       ├→ AskResumeExecuting  → AskExecuting (agent loop) → AskFinalizing → AskCompleted
       ├→ AskScheduleDirect   → AskCompleted (deterministic 路径)
       ├→ AskChatting         → AskFinalizing → AskCompleted
       └→ AskActing           → AskExecuting (agent loop) → AskFinalizing → AskCompleted
   任何阶段失败 → AskFailed
```

## 2. 状态枚举设计

```rust
// crates/clawd/src/runtime/ask_state.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AskState {
    /// task 进入 worker，prepare 之前
    Received,
    /// intent_normalizer + post_route_policy 进行中
    Routing,
    /// 路由判定需要追问
    Clarifying,
    /// 路由结论是直接 LLM 直答（chat / classifier_direct）
    Chatting,
    /// 路由结论是恢复执行已暂停 task
    ResumeExecuting,
    /// 路由结论是讨论已暂停 task（非执行）
    ResumeDiscussing,
    /// 路由结论是 schedule deterministic 短路
    ScheduleDirect,
    /// agent loop 执行中（含 plan / execute / verify 子循环，本轮先不细分）
    Executing,
    /// finalize 阶段（loop_finalize / observed_output）
    Finalizing,
    /// 答覆已生成、即将返回 worker
    Completed,
    /// 任何阶段失败
    Failed,
}
```

## 3. 合法 transition 表

```text
Received    → Routing
Routing     → Clarifying | Chatting | ResumeExecuting | ResumeDiscussing
            | ScheduleDirect | Executing | Failed
Clarifying  → Completed | Failed
Chatting    → Finalizing | Completed | Failed
ResumeExecuting → Executing | Finalizing | Completed | Failed
ResumeDiscussing → Completed | Failed
ScheduleDirect → Completed | Failed
Executing   → Executing (loop 内部下一轮)
            | Finalizing | Failed
Finalizing  → Completed | Failed
Completed   → (终态，禁止再 transition)
Failed      → (终态，禁止再 transition)
```

> Chatting 是否经过 Finalizing 取决于是否走 `intercept_response_text_for_delivery`
> 等 finalize 路径。本轮按"chat 路径直接 → Completed"建模，简化；如果实测发现
> 有 Chatting → Finalizing 的真实路径，再放开此 transition。

## 4. 实施分 4 个阶段

### Stage A — 类型基础设施
- `runtime/ask_state.rs`：`AskState` enum + `as_str()` + `is_terminal()` + `can_transition_to(next)`（合法表查询）
- `runtime/ask_state.rs`：`AskTransition { from, to, reason, at_ms, round_no }` struct
- `task_journal::TaskJournal` 增 `transitions: Vec<AskTransition>` 字段
- 单测：合法 / 非法 transition 表全面覆盖

### Stage B — Logger + journal 接入
- `prompt_utils` 风格的 `log_ask_transition(state, task_id, from, to, reason, round)` 工具
  - 输出 `[ASK_STATE] task_id=... state_from=... state_to=... reason=... round=...`
  - 同步写入 `TaskJournal.transitions`
- 单测：transition 输出格式稳定

### Stage C — 主路径插桩（最重）
关键插点（参考 `worker/ask_pipeline.rs::execute_ask_dispatch`）：

| 文件 | 插点 | transition |
|---|---|---|
| `worker/mod.rs` | task claimed 后、prepare 之前 | `→ Received` |
| `worker/ask_pipeline.rs::prepare_ask_flow` | normalizer 调用前 | `Received → Routing` |
| `execute_ask_dispatch` 各分支首行 | 见下表 | |
| - clarify_only 分支 | | `Routing → Clarifying → Completed` |
| - resume_discussion 分支 | | `Routing → ResumeDiscussing → Completed` |
| - resume_execution 分支 | | `Routing → ResumeExecuting → Executing` |
| - schedule_direct 分支 | | `Routing → ScheduleDirect → Completed` |
| - classifier_direct / chat 分支 | | `Routing → Chatting → Completed` |
| - act 分支 | | `Routing → Executing` |
| `agent_engine::loop_control::run_agent_with_loop` 每轮开始 | 仅 round>1 | `Executing → Executing` (round bump) |
| `agent_engine::loop_finalize::finalize_loop_reply` 入口 | | `Executing → Finalizing` |
| `worker/ask_finalize` 完成 | | `Finalizing → Completed` |
| 任何 `Result::Err` 路径返回前 | | `* → Failed` |

### Stage D — 守卫 + 持久化
- 在每个 transition 调用点加 `debug_assert!(prev.can_transition_to(next))`
- `TaskJournal.transitions` 进 `task_journal_summary` JSON 输出
- `[ASK_STATE]` 日志 + journal transitions 落地验证

## 5. 风险评估

| 风险 | 缓解 |
|---|---|
| 主路径插桩会 ripple 改 5+ 文件 | 分 4 个 PR/commit；每个 stage 独立可编译可验证 |
| 漏插某个分支导致 transition 缺失 | Stage D 加 invariant：进入 Completed 前必经 Finalizing 或显式终态分支 |
| 异步 / spawned task 的 Failed transition 难捕捉 | Stage C 仅捕捉同步主路径的 Err；spawned/background 的失败由调用方决定是否标记 Failed |
| transition 写入与现有 task_journal 字段交互 | 新字段独立 Vec；不影响现有 journal 序列化 |

## 6. 实施工作量预估

- Stage A: 1-2 小时（纯类型 + 单测）
- Stage B: 1 小时（logger + journal hook）
- Stage C: 3-4 小时（主路径插桩，多文件）
- Stage D: 1-2 小时（守卫 + 验证）

合计约 6-9 小时（半天到一天，视测试发现的盲点）。

## 7. 与 Phase 3.3 finalize 合并的关系

§3.1 完成后，`Finalizing` 状态会成为 finalize 三层合并的"显式入口锚点"——届时
`loop_finalize::finalize_loop_reply` / `observed_output::observed_answer_fallback`
等多入口都先经 `transition_to(Finalizing)`，§3.3 重构时可以基于 transition 日志
判定哪些路径需要保留、哪些可以合并。

## 8. 不做的事

* 本轮**不**细分 Executing 内部的 plan / execute / verify 子状态（留给 §3.3 或后续 sub-PR）
* 本轮**不**做 transition 持久化到独立表（journal 内联即可，避免改 schema）
* 本轮**不**改造现有 `TaskJournalFinalStatus`（与 `AskState::{Completed,Failed}` 并存，
  完成后再考虑是否合并）

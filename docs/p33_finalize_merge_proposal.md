# Phase 3.3 — finalize "三层" 合并 proposal

> 状态：**设计阶段**，未实施。
> 编辑日期：2026-04-17
> 作用域：plan §3.3 (`p3-finalize-merge`)。
> 与 §3.1 关系：本提案以 §3.1 的 `AskState::Finalizing` 作为唯一锚点入口。
> 与 §3.4 关系：semantic_judge 现已收敛到 finalize-tier，本提案进一步把
> finalize-tier 自身物理位置统一。

## 1. 背景与现状盘点

### 1.1 总体规模

| 文件 | 行数 | 角色 | 主入口 |
|---|---|---|---|
| `worker/ask_finalize.rs` | 857 | **TASK 层**：任务级编排（DB / memory / 通知 / journal） | `finalize_ask_result`、`finalize_ask_direct_success` |
| `agent_engine/loop_finalize.rs` | 2572 | **LOOP REPLY 层**：从 LoopState 选择 delivery + 构建 journal | `finalize_loop_reply` |
| `agent_engine/observed_output.rs` | 6659 | **OBSERVED FALLBACK 层**：observed 兜底（含 LLM 调用入口） | `synthesize_answer_from_observed_output` |
| `finalizer.rs` | 487 | **共享 helper**：planner artifact / delivery token 分类等纯函数 | （无 entry，纯 helper） |
| **合计** | **10575** | | |

### 1.2 三层调用图

```
worker_once
  └─ ask_pipeline::execute_ask_dispatch
       ├─ [Direct shortcuts]
       │    ├─ finalize_ask_direct_success    ← TASK 层短路入口
       │    └─ try_finalize_schedule_direct_success
       │
       └─ execute_ask_routed
            ├─ run_classifier_direct_reply    （chat/classifier_direct LLM 一发）
            ├─ run_agent_with_tools           （agent loop）
            │    └─ loop_control
            │         └─ finalize_loop_reply  ← LOOP REPLY 层
            │              ├─ direct_scalar_observed_answer
            │              ├─ direct_non_builtin_skill_raw_answer
            │              ├─ direct_structured_observed_answer
            │              ├─ synthesize_answer_from_observed_output  ← OBSERVED LLM 兜底
            │              ├─ direct_publishable_observed_answer
            │              └─ build_loop_journal
            │
            └─ finalize_ask_result            ← TASK 层主入口（接 AskReply）
                 ├─ finalize_ask_success / failure / resume_failure
                 ├─ insert_ask_memory_pair / insert_unfinished_goal_memory
                 ├─ spawn_long_term_summary_refresh
                 └─ DB write
```

### 1.3 三层职责边界（已成形，但物理上散落）

| 维度 | TASK 层 | LOOP REPLY 层 | OBSERVED FALLBACK 层 |
|---|---|---|---|
| 输入 | `AskReply` + `RouteResult` + payload | `LoopState` + `AgentRunContext` | `LoopState` + observed entries |
| 输出 | `Result<()>`（副作用：DB / memory） | `AskReply`（无 DB） | `Option<(String, FinalizerSummary)>` |
| LLM 调用 | 0 (run_classifier_direct_reply 例外) | 间接（透过 OBSERVED 子层） | 1 次 |
| 持久化 | DB write + 通知 + memory | 无 | 无 |
| journal 构建 | `ensure_journal_task_metrics` + merge | `build_loop_journal` 全量构建 | 仅 FinalizerSummary 片段 |
| 清单大小 | 4 主入口 + 5 内部辅助 | 1 主入口 + ~30 内部 helpers | 1 主入口 + 6 共享 extractors |

### 1.4 当前痛点

1. **三个文件形态不一致，命名不统一**
   - `worker/ask_finalize.rs::finalize_ask_*`（带 `ask_` 前缀）
   - `agent_engine/loop_finalize.rs::finalize_loop_reply`（带 `loop_` 前缀，且是 `pub(super)`）
   - `agent_engine/observed_output.rs::synthesize_answer_from_observed_output`（无 `finalize` 前缀）
   - `finalizer.rs::finalizer_*` + `should_attempt_observed_fallback`（接近 helper 集合）

2. **journal 构建逻辑分散**
   - TASK 层：`ensure_journal_task_metrics`（增量）+ `merge_from`（合并 LOOP 层产物）
   - LOOP 层：`build_loop_journal`（一次性构建）
   - 两套构建器没共享，`record_*` 字段来源各算各的

3. **finalizer.rs 职责模糊**
   - 既有 `FinalizerDisposition` 这种语义类型
   - 又有 `parse_delivery_token` / `infer_file_target_kind` 这种纯文本分类
   - 还有 `should_attempt_observed_fallback` 这种 1-line 业务规则
   - 文件名暗示"finalizer 主体"，实际只是工具箱

4. **invariant 缺失**
   - LOOP 层任何返回 `AskReply` 的路径都隐式期待 TASK 层会接住，但没显式契约
   - `ask_state` 的 `Finalizing` 状态目前只在 TASK 层入口打了一次（§3.1），LOOP 层不知道自己处在 Finalizing 阶段

5. **入口分散导致检索成本高**
   - 想问"这条 ask 的最终回复怎么来的"，要分别看 TASK / LOOP / OBSERVED 三个文件
   - 调用面引用 `crate::worker::finalize_*` / `crate::agent_engine::loop_finalize::*` 不同前缀

## 2. 目标

定义并落地 `crate::finalize::*` 单一逻辑入口空间，让：

- **调用方**：所有 finalize 相关函数都从 `crate::finalize::*` 入口调用，不再写
  `worker::finalize_ask_result` / `agent_engine::loop_finalize::*` / `observed_output::synthesize_*`。
- **维护者**：finalize 三层职责边界、journal 构建器、helper 工具箱在一个 module tree 里，
  增加新策略 / 改 journal 字段 / 新增兜底链都不用跨多个 mod 改。
- **AskState 强契约**：进入 finalize 任何子层前，`ask_state == Finalizing` 必须成立
  （§3.1 已支持），违反触发 `debug_assert`。
- **行为零变化**：本 PR 系列不改 LLM prompt、不改 fallback 顺序、不改 DB 写入字段、
  不改 journal JSON schema。所有 b1_regression 用例必须 byte-identical 通过。

## 3. 提议方案

采用**4-stage 渐进迁移**，每 stage 独立可 ship、独立可 revert：

### 3.1 Stage 1（facade，1-2 天）：建立 `crate::finalize` 重导出层

```rust
// crates/clawd/src/finalize/mod.rs
//! Phase 3.3: finalize 单一逻辑入口。
//! 本模块**仅做重导出**，物理位置保持原地，调用方迁移到本 module 后，
//! 后续 stage 才物理搬移源文件不影响调用面。

// TASK 层
pub(crate) use crate::worker::{
    finalize_ask_result, finalize_ask_direct_success,
    run_classifier_direct_reply, try_finalize_schedule_direct_success,
};
// LOOP REPLY 层
pub(crate) use crate::agent_engine::loop_finalize::finalize_loop_reply;
// OBSERVED FALLBACK 层
pub(crate) use crate::agent_engine::observed_output::synthesize_answer_from_observed_output;
// HELPER 层（重导出 finalizer.rs 全部公共项）
pub(crate) use crate::finalizer::*;
```

调用面迁移策略：

- 全仓库 ripgrep 替换：
  - `crate::worker::finalize_ask_result` → `crate::finalize::finalize_ask_result`
  - `crate::worker::finalize_ask_direct_success` → `crate::finalize::finalize_ask_direct_success`
  - `crate::worker::run_classifier_direct_reply` → `crate::finalize::run_classifier_direct_reply`
  - `crate::worker::try_finalize_schedule_direct_success` → `crate::finalize::try_finalize_schedule_direct_success`
  - `crate::agent_engine::loop_finalize::finalize_loop_reply` 调用面（仅 1 处，loop_control.rs）
  - `crate::finalizer::*` 全部 → `crate::finalize::*`
- `loop_finalize::finalize_loop_reply` 把 `pub(super)` 升 `pub(crate)`
- `synthesize_answer_from_observed_output` 已是 `pub(crate)`，无需动
- 旧路径保留为兼容 alias，避免 PR 内部 reorder 困难

**测试**：现有所有单测 + b1_regression 必须 byte-identical 通过。`cargo build --release`
不应有 warning 增量。

**风险**：极低；改动只是 re-export + import path rename，编译期可保证语义不变。

### 3.2 Stage 2（物理搬移，2-3 天）：实物移动文件

```
crates/clawd/src/finalize/
├── mod.rs              ← Stage 1 facade，移除重导出，改为真正 re-export
├── task.rs             ← 移自 worker/ask_finalize.rs（除 run_classifier_direct_reply）
├── loop_reply.rs       ← 移自 agent_engine/loop_finalize.rs
├── observed.rs         ← 移自 agent_engine/observed_output.rs 中
│                          synthesize_answer_from_observed_output 及私有依赖
└── helpers.rs          ← 移自 finalizer.rs 全部内容
```

`run_classifier_direct_reply` 实际上**不属于 finalize**（是 chat/classifier_direct 模式
的 LLM 一发），本 stage 顺手把它从 `worker/ask_finalize.rs` 拆出回到 `worker/`
或新位置（建议 `worker/classifier_direct.rs`）。

`agent_engine/observed_output.rs` 仍保留（6659 行中绝大多数是 extractors / classifiers，
不属于 finalize），只把 `synthesize_answer_from_observed_output` + 其私有依赖搬出。

`agent_engine/loop_finalize.rs` 保留 thin re-export 占位，方便外部分析工具不立即失效；
2 个 release 后删除（grep 干净后）。

**测试**：本 stage 改动文件物理位置 + 4-5K 行代码 move，必须保证：
- 单元测试不漏（需要把 `loop_finalize.rs` 的 1000+ 行 tests mod 一并搬）
- visibility 收紧（`pub(super)` → `pub(crate)` 仅在确实跨 mod 引用时）
- model_io / tracing log 字段 byte-identical

### 3.3 Stage 3（合并 journal 构建器 + AskState 强契约，3-5 天）

#### 3.3.1 journal 构建器合并

新建 `finalize/journal.rs`，把 TASK 层 `ensure_journal_task_metrics` 与
LOOP 层 `build_loop_journal` 共享部分提为公共 builder：

```rust
pub(crate) struct FinalizeJournalBuilder<'a> {
    pub(crate) task: &'a ClaimedTask,
    pub(crate) prompt: &'a str,
    pub(crate) state: &'a AppState,
    // ... 通用字段
}

impl<'a> FinalizeJournalBuilder<'a> {
    pub(crate) fn from_loop_state(...) -> TaskJournal { /* 替代 build_loop_journal */ }
    pub(crate) fn from_ask_reply(...) -> TaskJournal { /* 替代 ensure_journal_task_metrics + merge_from */ }
    pub(crate) fn record_finalizer_summary(&mut self, ...);
    pub(crate) fn record_delivery_outcome(&mut self, ...);
}
```

行为不变约束：所有 record_* 字段顺序、值、JSON 字段名必须保持一致；
通过对比 b1_regression 三条任务的 `task_journal_summary` JSON diff 验证。

#### 3.3.2 AskState 强契约

利用 §3.1 的 `AskState::Finalizing` 作为唯一锚点：

- `finalize::task::finalize_ask_result` 入口已打 transition（§3.1 实施）
- `finalize::loop_reply::finalize_loop_reply` 入口加：`debug_assert!(state.ask_state(task_id) == Some(AskState::Executing | AskState::Finalizing))`
- `finalize::observed::synthesize_answer_from_observed_output` 入口加：`debug_assert!(state.ask_state(task_id) == Some(AskState::Finalizing))`

这要求 §3.1 的 `AskState` 注入点扩展（目前 transition 只写日志/journal，不存 AppState）。
本 stage 顺带把 `AppState::current_ask_state(task_id)` 加上（一张 `DashMap<task_id, AskState>`），
并在 §3.1 既有 transition 点同步更新。

### 3.4 Stage 4（**可选，独立 PR 系列**）：真正的单一入口

合并三层的 control flow 进 `finalize::finalize_ask(...)`：

```rust
pub(crate) async fn finalize_ask(
    state: &AppState,
    task: &ClaimedTask,
    request: FinalizeRequest,
) -> Result<()> {
    match request.source {
        FinalizeSource::DirectShortcut(answer) => task::finalize_direct(state, task, answer).await,
        FinalizeSource::ClassifierDirect(reply_result) => task::finalize_from_reply(state, task, reply_result).await,
        FinalizeSource::AgentLoop(loop_state, ctx) => {
            let reply = loop_reply::finalize_loop_reply(state, task, ..., loop_state, ctx).await?;
            task::finalize_from_reply(state, task, Ok(reply)).await
        }
    }
}
```

工作量预估 1-2 周，涉及 worker/ask_pipeline 主分发重写，**强烈建议本 PR 系列不做**，
留给 §3.3 follow-up。理由：

- Stage 1-3 完成后已经达成 plan §3.3 验收口径（"finalize / observed_output / loop_finalize
  合并为单一 finalize 层，对外只暴露一个入口"——本质上是 module-level 单一入口空间，
  不强制 single-function entry）
- 物理上的 single-function entry 反而会增加 dispatcher 巨函数的认知负担，
  当前三个 entry function 各对应清晰场景（DirectShortcut / ClassifierDirect / AgentLoop），
  分着写更易读

## 4. 拆 PR 计划

| PR | Stage | 改动范围 | 工作量 | 风险 |
|---|---|---|---|---|
| #1 | Stage 1 | `crate::finalize` facade 创建 + 调用面 import 替换 | 1-2 天 | 极低 |
| #2 | Stage 2.1 | `worker/ask_finalize.rs` → `finalize/task.rs` 物理搬移 | 0.5 天 | 低 |
| #3 | Stage 2.2 | `agent_engine/observed_output.rs` 中 finalize 部分 → `finalize/observed.rs` | 1 天 | 中（observed_output 文件大，私有依赖多） |
| #4 | Stage 2.3 | `agent_engine/loop_finalize.rs` → `finalize/loop_reply.rs`（含 1000+ 行 tests） | 1-2 天 | 中 |
| #5 | Stage 2.4 | `finalizer.rs` → `finalize/helpers.rs` + classifier_direct 拆出 | 0.5 天 | 极低 |
| #6 | Stage 3.1 | `finalize/journal.rs` builder 合并 | 1-2 天 | 中（涉及行为对比） |
| #7 | Stage 3.2 | `AppState` 注入 ask_state + finalize 子层 invariant | 1-2 天 | 低 |

合计 6-10 个工作日，最优路径 5 个 PR 即可（PR #2/#3/#4/#5 可酌情合并）。

## 5. 风险与缓解

| 风险 | 缓解 |
|---|---|
| Stage 2 大文件搬移后 git blame 断裂 | 每个 PR 用 `git mv` + 单独的 "rename only" commit，blame 可跨 commit 跟踪 |
| journal JSON 字段 byte-identical 难保证 | 准备 jq 脚本对比 b1_regression 三条任务的 task_journal_summary，diff 必须为空 |
| `AppState` 注入 ask_state 引入跨 task 并发问题 | 用 `DashMap`，task 终态时清理 entry；并发只读不写 |
| Stage 3 改 builder 后某个边缘字段漏算 | 写一组 6-8 条覆盖所有 finalizer_summary disposition 的单测，跑前后两版函数对比 |
| Stage 4 巨型重构推回 | 本 proposal 显式声明 Stage 4 不在本 PR 系列范围 |

## 6. 不在本 PR 系列做的事

- ❌ 改 LLM prompt（包括 observed_answer_fallback_prompt）
- ❌ 改 fallback 顺序（scalar / non-builtin / structured / observed / publishable）
- ❌ 改 DB 写入字段、`update_task_success` / `update_task_failure_with_result` 签名
- ❌ 改 task_journal_summary JSON schema（含字段名、顺序、嵌套结构）
- ❌ 改 `model_io.log` / `clawd.run.log` 行格式
- ❌ Stage 4 单一 function 入口
- ❌ 把 `agent_engine/observed_output.rs` 整体搬走（仅搬 `synthesize_answer_from_observed_output`
  及其私有依赖）
- ❌ `pub(super)` → `pub` 全面放开（仅在确实跨 crate 需要时）

## 7. 验收

- [ ] PR #1 合并后：`crate::finalize::*` 全部 4-5 个公共函数可用，旧路径保留兼容；
      b1_regression 3/3 succeeded，0 timeout，task_journal_summary diff = 0
- [ ] PR #2-#5 合并后：`crates/clawd/src/finalize/` 目录结构成型；
      `worker/ask_finalize.rs` / `agent_engine/loop_finalize.rs` / `finalizer.rs`
      文件移除或仅留 thin re-export；b1_regression diff = 0
- [ ] PR #6 合并后：journal 构建逻辑唯一来源，重复 record_* 调用消除；JSON diff = 0
- [ ] PR #7 合并后：`AppState::current_ask_state(task_id)` 可查；finalize 子层
      invariant `debug_assert` 在 dev/test build 启用；release build 零开销

## 8. 与 §3.4 / §3.1 的衔接

- §3.4 已把 semantic_judge 收敛到 finalize-tier（`loop_finalize.rs` / `observed_output.rs`），
  本 proposal Stage 2 完成后这些调用点会物理搬到 `finalize/` 下，
  `scripts/check_semantic_judge_callers.sh` 白名单需同步更新为 `finalize/loop_reply.rs`
  与 `finalize/observed.rs`。
- §3.1 的 `AskState::Finalizing` 在 Stage 3.2 升级为强契约（debug_assert）；
  §3.1 留下的"journal 创建提前到 worker 入口" follow-up 与本 proposal 独立，
  不互相阻塞。

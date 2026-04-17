# P3.2 路由模式二元收敛方案（mode collapse）

> 状态：**设计阶段**，未实施。
> 编辑日期：2026-04-17
> 作用域：仅 plan §3.2，不动 §3.3 finalize 三层（finalize 三合一另起 PR 系列）。

## 背景

`/home/guagua/.cursor/plans/llm_请求层优化完整计划_5b939b41.plan.md` §3.2 描述：

> 5 种模式（chat / act / chat_act / classifier_direct / resume_followup）压到 **2 种**：`ClarifyOrChat` 与 `Act`。
> classifier_direct / resume_followup 变成"进入 ClarifyOrChat 时的策略"（候选来源 / 上下文载入方式），不再是平级模式。

实际盘点后发现：plan 的"5 种"是个简化说法，真实状态空间是**枚举 × 三个独立 bool flag**的笛卡儿积，比 5 种更乱。本方案先把这个真实图景画清楚，再给二元收敛设计。

---

## 现状盘点

### 状态空间维度

| 维度 | 取值 | 定义点 | 主要分发点 |
|---|---|---|---|
| `RoutedMode` 枚举（必有） | `Chat / Act / ChatAct / AskClarify` | `runtime/types.rs:102` | `ask_flow.rs::execute_ask_routed` 4-way `match` |
| `classifier_direct_mode: bool` | true 时跳过 normalizer 直接用最终 LLM 给一句答 | `worker/ask_prepare.rs:599` 从 `CLASSIFIER_DIRECT_SOURCES` 静态名单判断 | `worker/ask_pipeline.rs::should_allow_classifier_direct` + 多处 `if classifier_direct_mode && ...` |
| `direct_resume_discussion: bool` | resume 上下文 → 走 followup 讨论 prompt | `worker/ask_prepare.rs` 从 resume_context 计算 | `worker/ask_pipeline.rs:438` resume 分发 |
| `direct_resume_execution: bool` | resume 上下文 → 复用上次 plan 直接执行 | 同上 | 同上 |

### `RoutedMode` 各 variant 实质语义

| variant | 行为 | 跟谁能合并 |
|---|---|---|
| `Chat` | LLM 直答，无技能调用，可被 `self_extension` 升级到 Act | → `ClarifyOrChat`（对用户输出文本） |
| `AskClarify` | LLM 反问澄清 | → `ClarifyOrChat` |
| `Act` | 跑 plan-and-execute，agent loop | → `Act`（调技能） |
| `ChatAct` | 跑 plan-and-execute，结尾用 chat finalizer 包装 | → `Act`（调技能 + finalize 风格不同） |

### 实测调用面（plan 上下游耦合）

`grep RoutedMode::|CLASSIFIER_DIRECT_SOURCES|direct_resume_*` 跨 18 个文件、约 230 处。其中较密集的：

| 文件 | 计数 | 分支用途 |
|---|---|---|
| `agent_engine/planning.rs` | 45 | "Act/ChatAct 才允许 plan_repair / 走 execution_recipe" 等 |
| `worker/ask_pipeline.rs` | 31 | classifier_direct + AskClarify + resume 分发 |
| `agent_engine/observed_output.rs` | 112 | observed answer 用 routed_mode 决定 prompt 语气 |
| `intent_router.rs` | 20 | normalizer 输出标 RoutedMode |
| `ask_flow.rs` | 10 | `execute_ask_routed` 主分发口 |
| `worker/ask_finalize.rs` | 7 | AskClarify 走特殊回复路径 |
| `agent_engine/loop_finalize.rs` | 10 | Act/ChatAct 测试桩 |
| `execution_recipe.rs` | 9 | needs_clarify || !Act|ChatAct → 跳 recipe |
| `self_extension.rs` | 11 | AskClarify 排除升级 |
| `verifier.rs` | 3 | enforce_content_evidence_execution_mode 输入 |

绝大部分调用是**只读判断**（`matches!(mode, Act | ChatAct)` / `matches!(mode, AskClarify)`），构造 `RouteResult` 的真正写入点只有：

- `intent_router.rs::run_intent_normalizer_with_resume`（main 路径）
- `ask_flow.rs::execute_ask_routed` 内部 `direct_route_decision_for_text` 等少数几个 fallback

---

## 设计

### 数据结构

新增（建议放 `runtime/ask_mode.rs`）：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AskMode {
    ClarifyOrChat {
        entry: ChatEntryStrategy,
    },
    Act {
        finalize: ActFinalizeStyle,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChatEntryStrategy {
    /// 原 `RoutedMode::Chat`：normalizer 标 mode=Chat
    NormalizerThenChat,
    /// 原 `RoutedMode::AskClarify`：normalizer 标 needs_clarify=true
    NormalizerThenClarify,
    /// 原 `classifier_direct_mode=true`：跳 normalizer，单 LLM 直答
    ClassifierDirect { source: String },
    /// 原 `direct_resume_discussion=true`：resume 上下文 + followup discussion prompt
    ResumeFollowupDiscussion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActFinalizeStyle {
    /// 原 `RoutedMode::Act`
    Plain,
    /// 原 `RoutedMode::ChatAct`：loop 结束后再 chat 收尾
    ChatWrapped,
    /// 原 `direct_resume_execution=true`：复用上次 plan
    ResumeContinue,
}

impl AskMode {
    pub(crate) fn is_act(&self) -> bool {
        matches!(self, AskMode::Act { .. })
    }
    pub(crate) fn is_clarify(&self) -> bool {
        matches!(self, AskMode::ClarifyOrChat { entry: ChatEntryStrategy::NormalizerThenClarify })
    }
    pub(crate) fn is_classifier_direct(&self) -> bool {
        matches!(
            self,
            AskMode::ClarifyOrChat { entry: ChatEntryStrategy::ClassifierDirect { .. } }
        )
    }
    pub(crate) fn finalize_chat_wrapped(&self) -> bool {
        matches!(self, AskMode::Act { finalize: ActFinalizeStyle::ChatWrapped })
    }
    pub(crate) fn resume_execution(&self) -> bool {
        matches!(self, AskMode::Act { finalize: ActFinalizeStyle::ResumeContinue })
    }
}
```

### 等价映射表（旧 → 新）

| (RoutedMode, classifier_direct, direct_resume_discussion, direct_resume_execution) | AskMode |
|---|---|
| (Chat, false, false, false) | `ClarifyOrChat { NormalizerThenChat }` |
| (AskClarify, *, *, *) | `ClarifyOrChat { NormalizerThenClarify }` |
| (\*, true, false, false) | `ClarifyOrChat { ClassifierDirect { source } }` |
| (Chat, false, true, false) | `ClarifyOrChat { ResumeFollowupDiscussion }` |
| (Act, false, false, false) | `Act { Plain }` |
| (ChatAct, false, false, false) | `Act { ChatWrapped }` |
| (\*, false, false, true) | `Act { ResumeContinue }` |

非法组合（构造期就被排除掉）：

- `classifier_direct=true && direct_resume_*=true` — 现状下 `classifier_direct_mode` 走 short-circuit 路径就不会再算 resume
- `direct_resume_discussion=true && direct_resume_execution=true` — 互斥
- `RoutedMode::AskClarify && classifier_direct=true` — `should_allow_classifier_direct` 已经禁止

把这些"非法但当前 bool 组合可表达"的态删掉，是收敛带来的真收益。

### 转换函数（过渡期）

```rust
impl AskMode {
    pub(crate) fn from_legacy(
        routed: RoutedMode,
        classifier_direct: bool,
        direct_resume_discussion: bool,
        direct_resume_execution: bool,
        classifier_direct_source: Option<&str>,
    ) -> Self { /* 按映射表 */ }

    pub(crate) fn to_routed_mode(&self) -> RoutedMode { /* 反向 */ }
}
```

反向函数用于在 Stage 1/2 让"还没改过来的下游代码"继续读 `RoutedMode`。等所有调用面切完，反向函数可删。

---

## 分阶段实施

每个 Stage 一个 commit，验证通过才进下一个。

### Stage A · 引入 `AskMode` 抽象（不改语义，0 行为变更）

- 新增 `runtime/ask_mode.rs`，定义上述 enum + 转换函数。
- 单测覆盖 7 种映射 + 反向 + 非法组合 panic/Err。
- `mod.rs` 导出。

**预期改动量**：1 个新文件 ~150 行 + 测试 ~80 行；其他文件 0 改动。

**验证**：单测通过 + `cargo check -p clawd` 干净。

### Stage B · `RouteResult` 加 `ask_mode` 字段（双轨期）

- `RouteResult` 增加 `pub(crate) ask_mode: AskMode`，**与 `routed_mode` 并存**。
- `intent_router.rs` 输出 `RouteResult` 时同时填两份。
- `worker/ask_prepare.rs::compute_ask_routing` 计算 `classifier_direct_mode` / `direct_resume_*` 的同时也算 `AskMode`，塞进 `PreparedAskRouting`。
- 所有现有读 `routed_mode` / `classifier_direct_mode` / `direct_resume_*` 的代码**不动**。
- 加个 debug-mode invariant：`assert_eq!(ask_mode.to_routed_mode(), routed_mode)` 在构造点验证两轨一致。

**预期改动量**：~3 个文件 + 双轨初始化代码 ~50 行。

**验证**：单测 + `_b1_regression` 5/5 + `nl_cases_singletons` 重跑全过。

### Stage C · 高频 `match`/`matches!` 切到 `AskMode`

逐文件改成 `match ask_mode` / `if ask_mode.is_act() { ... }`，按低风险顺序：

1. `execution_recipe.rs`（9 处，纯只读判断）
2. `self_extension.rs`（11 处）
3. `verifier.rs`（3 处）
4. `worker/ask_finalize.rs` AskClarify 判断（7 处）
5. `agent_engine/planning.rs`（45 处，最大；分两个 commit）
6. `agent_engine/observed_output.rs`（112 处，按函数切；可能要分 3-4 个 commit）
7. `worker/ask_pipeline.rs::should_allow_classifier_direct`（核心 short-circuit 谓词）
8. `worker/ask_pipeline.rs::dispatch_resume_or_continuation`
9. `ask_flow.rs::execute_ask_routed` 主分发口（最后改，因为它统领所有路径）

每改一个文件，跑一次单测 + b1，确认无回归再进下一个。

**预期改动量**：~9 个文件，~230 处 `matches!`/`match`，分约 6-8 个 commit。

**验证**：每 commit 跑单测；阶段尾跑 b1 + nl_singletons。

### Stage D · 删除 `routed_mode` / `classifier_direct_mode` / `direct_resume_*`

- 当所有读端都切到 `ask_mode` 后，从 `RouteResult` / `PreparedAskRouting` 删除老字段。
- `RoutedMode` 枚举本身**保留**（Stage E 再处理），只是从 `RouteResult` 字段层面隐藏。
- intent_router 输出端：`parse_mode_text` 仍解析 LLM 返的字符串到 `RoutedMode`，再立即 `AskMode::from_routed_mode_with_clarify(...)` 包装成新结构。

**预期改动量**：~5 个文件结构修改 + 测试 fixture 更新。

**验证**：单测 + b1 + nl_singletons。

### Stage E · `RoutedMode` 退场（可选，不在 A2 范围）

- 把 `intent_router.rs::parse_mode_text` 输出从 `RoutedMode` 改为 `AskMode`（含构造 entry strategy）。
- 删 `runtime/types.rs::RoutedMode` 枚举。

本轮先不做（属于 §3.2 的"洁癖收尾"，不影响功能；留到 §3.3 finalize 合并时一起做更合算）。

---

## 风险与回滚

### 主要风险

1. **`agent_engine/observed_output.rs` 112 处分支密集**：observed answer 是用户实际看到的回复，分支判断错就直接观测得到。Stage C 这一步必须分多个 commit 慢慢切，每次单测 + b1 + nl_singletons 三连。
2. **`should_allow_classifier_direct` 谓词语义微妙**：现状是"如果 routed_mode 是 Act/ChatAct/AskClarify 就拒绝 classifier_direct"。在新 enum 下，按映射表 classifier_direct 只能跟 Chat 共存，所以这个谓词在 AskMode 上变成永真——但要确认所有"先经 normalizer 再决定 classifier_direct"的代码路径不存在（grep `CLASSIFIER_DIRECT_SOURCES` 看似纯入口判断，已确认）。
3. **`PreparedAskRouting` 是 `worker` 内部数据结构**，但 `direct_resume_*` 经过多层传递（ask_prepare → ask_pipeline → ask_flow → ask_finalize），改字段时要全链路同步；可以靠 `cargo check` 兜底。
4. **测试 fixture 大量构造 `RouteResult { routed_mode: ..., ... }`** —— Stage B 引入 `ask_mode` 字段后，所有 fixture 要补这个字段。可写一个 `RouteResult::test_with_mode(routed_mode)` 助手，让 fixture 写起来不啰嗦。

### 回滚

每个 Stage 都是独立 commit，失败时 `git revert` 单独回退即可。Stage A/B 引入的双轨期允许任一时刻读旧字段，所以 Stage C 中途中止也不影响功能（只是部分文件用新 API 部分用旧 API，编译 OK 行为 OK）。

---

## 验证口径（A2/v2 对齐）

每个 Stage 必须通过：

1. `cargo test -p clawd --bin clawd`（约 566+ 测试）
2. `bash scripts/nl_tests/run.sh _b1_regression`（5/5 通过）
3. Stage B/C/D 还要跑 `bash scripts/nl_tests/run.sh nl_cases_singletons`（覆盖各种 entry strategy 的真实路由路径）

最终阶段（Stage D）通过后才提交 PR-merge-able 的串。

---

## 时间预估

| Stage | 预估工时 | 备注 |
|---|---|---|
| A | 30 min | 1 个新文件 + 单测 |
| B | 45 min | 双轨字段 + assert + fixture 补字段 |
| C | 3-4 h | 9 文件 / ~230 处分散修改，分 6-8 commit |
| D | 1 h | 删字段 + 收紧类型 |
| E | （本轮跳过） | 留到 §3.3 |
| **合计 A~D** | **5-6 h** | 跨多个 commit，可分多个 session 推进 |

`nl_singletons` 重跑约 5-10 min/次（视 LLM 响应速度），全程会跑约 4-5 次。

---

## 待确认问题

1. `AskMode` 放在 `runtime/ask_mode.rs` 还是 `runtime/types.rs`（与 `RoutedMode` 同位）？建议**新文件**，因为 `runtime/types.rs` 已经接近 250 行且杂。
2. Stage B 的双轨 `assert_eq!` 用 `debug_assert!` 还是 `if cfg!(debug_assertions)`？建议 `debug_assert!`，release build 自动消失。
3. `ChatEntryStrategy::ClassifierDirect { source: String }` 里 `source` 是 owned 还是 `Cow<'static, str>`？现状传入的全是 `&'static str` 的子字符串，可以省一次 alloc，但代码可读性下降。建议**先用 String**，性能确认有问题再改。

---

如果 proposal 没有明显错误，按 Stage A → B → C → D 顺序推进，每个 Stage 完成后回报验证结果。

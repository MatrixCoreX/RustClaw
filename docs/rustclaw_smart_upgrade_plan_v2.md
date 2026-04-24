# RustClaw 智能化升级计划 V2

日期：2026-04-02

## 1. 定位

`V2` 是结构升级计划，前提不是“先把功能做更多”，而是把 `V1` 已经验证有效的判据、门禁和指标，沉淀成长期稳定的主链架构能力。

## 2. V2 启动前置条件

只有当以下条件满足，才进入 `V2`：

1. 已有任务级指标：`used_evidence_ids_count`、`delivery_consistent`、`llm_calls_per_task`。
2. `summary`、`scalar`、`FILE token`、`clarify`、`locator binding` 聚焦回归集稳定可复现。
3. verifier 的低风险 enforce 已灰度验证通过。
4. 没有新增独立 validator LLM hop。

## 3. 目标

1. 让 finalizer 真正基于 Evidence Ledger 做“生成 + 自检”。
2. 把 skill registry 从“执行配置中心”升级为“规划契约中心”。
3. 把 sidecar judge 收敛到兜底角色，主判定回到主链。
4. 建立统一状态机和上下文冲突治理模型。

## 4. V2 只解决什么

1. 执行结果证据化。
2. registry 驱动的 planner / verifier / clarify。
3. 主链成功判据集中化。
4. 上下文冲突的分层治理。

## 5. V2 不解决什么

1. 不重新发明一套和 `V1` 指标脱钩的验证方式。
2. 不在没有灰度基线的情况下直接重写主链。
3. 不把所有 skill 一次性迁入 Evidence Ledger。
4. 不让 `semantic_judge` 在新架构里继续承担主成功判定职责。

## 6. 主要工作流

### A. Evidence Ledger 接入

把执行结果从松散文本拼接升级为可引用证据块：

- `evidence_id`
- `source`
- `kind`
- `target`
- `excerpt`
- `status`
- `step_no`

要求：

1. finalizer 输出必须给出 `used_evidence_ids`。
2. `completion_ok=true` 但 `used_evidence_ids` 为空时，不允许视为合格完成。
3. summary / 抽取 / 内容证据类任务必须稳定绑定证据。

### B. Registry 驱动的规划与澄清

优先消费 registry 中已有元数据：

- `input_schema`
- `output_schema`
- `risk_level`
- `requires_confirmation`
- `side_effect`
- `retryable`
- `group`
- `primary_fallback_role`

目标：

1. planner 对 skill 的理解更多来自 registry，而不是 prompt 文本猜测。
2. schedule、verifier、planner 对同一 skill 使用同一份契约来源。
3. 能基于 schema 缺参直接生成更准确的 clarify。

### C. 统一状态机

主链逐步统一到以下状态：

1. `NeedClarify`
2. `NeedTool`
3. `NeedFinalize`
4. `Done`
5. `FailSafe`

要求：

1. 不再允许“有输出即 Done”。
2. 不合格但可修复时进入重试或 finalize。
3. 不合格且不可修复时进入 fail-safe，而不是标记成功。

### D. 收敛 Sidecar Judge

1. `semantic_judge` 仅保留安全兜底、异常降级、极少量保护性判断。
2. 可发布性与完成度判断主要迁移到 finalizer 结构化自检。
3. 避免 planner、executor、judge、delivery 各自维护一套“成功”定义。

### E. 上下文冲突治理

把 route memory、recent execution、binding context 从“上下文堆料”推进到“证据分层”：

1. 当前轮工具结果优先级最高。
2. recent execution 仅在目标唯一且类型匹配时允许复用。
3. long-term memory 不参与具体目标覆盖，只参与偏好和长期事实补充。
4. 存在冲突时优先澄清，不做静默覆盖。

## 7. 实施阶段

### 阶段 0：架构准备

目标：

- 不直接大改主链，先把 V2 的数据模型和边界收清楚

工作：

1. 定义最小 Evidence Ledger 结构。
2. 定义 step 到 evidence 的映射边界。
3. 明确 finalizer、planner、verifier、journal 各自读取哪些证据字段。

完成标准：

1. 证据模型能覆盖首批目标场景。
2. 不需要为每个 skill 单独设计一套特殊格式。

### 阶段 1：Evidence Ledger MVP

目标：

- 让关键内容型任务先跑通证据化收尾

工作：

1. 在 agent loop 中产出最小 Evidence Ledger。
2. 让 finalizer 优先消费 evidence block，而不是原始松散文本。
3. 第一批只覆盖 `read_file`、抽取、file delivery 三类高价值场景。

完成标准：

1. `used_evidence_ids` 在关键任务中稳定非空。
2. summary / extract / file-grounded 任务的 `grounded_ok` 稳定。

### 阶段 2：Registry 深接入

目标：

- 让契约更多来自 registry，而不是 prompt 猜测

工作：

1. 暴露 planner / verifier 可直接消费的轻量契约访问接口。
2. 让 planner / clarify / confirm 优先读取 schema 和 risk 字段。
3. 清理 prompt 中与 registry 重复、且容易漂移的契约描述。

完成标准：

1. 缺参澄清更稳定。
2. risk / confirmation 行为与 registry 配置一致。

### 阶段 3：状态机与 sidecar 收敛

目标：

- 把成功判据集中回主链

工作：

1. 明确 `NeedClarify -> NeedTool -> NeedFinalize -> Done / FailSafe` 的状态跳转。
2. 收敛 `semantic_judge` 的主判定职责。
3. 减少 planner、executor、judge、delivery 间的重复成功定义。

完成标准：

1. 主链成功判定集中。
2. sidecar 仅保留兜底职责。

### 阶段 4：灰度、评估、回滚

目标：

- 让 V2 可上线、可证伪、可回滚

工作：

1. 先对默认 vendor 灰度。
2. 对比升级前后指标。
3. 保留关键回滚开关。
4. 对默认 vendor 与至少一个非默认 vendor 做抽样比对。

回滚条件：

1. 假成功率没有下降。
2. `llm_calls_per_task` 超预算。
3. P95 延迟显著恶化。
4. 关键场景回归失败。

## 8. 建议拆成 2 个工作包

### WP3：V2 证据化

1. 在 `crates/clawd/src/agent_engine` 引入最小 Evidence Ledger 结构体和 step-to-evidence 映射。
2. 让 [final_synthesis.rs](../crates/clawd/src/agent_engine/final_synthesis.rs) 优先消费 evidence block，而不是原始松散文本。
3. 第一批只覆盖 `read_file`、抽取、file delivery 三类高价值场景。

### WP4：Registry 驱动

1. 在 [skill_registry.rs](../crates/claw-core/src/skill_registry.rs) 暴露 planner / verifier 可直接消费的轻量契约访问接口。
2. 在 [intent_router.rs](../crates/clawd/src/intent_router.rs)、[post_route_policy.rs](../crates/clawd/src/post_route_policy.rs)、`crates/clawd/src/agent_engine/planning.rs` 优先消费 schema 与 risk 字段来决定 clarify / execute / confirm。
3. 清理 prompt 中与 registry 重复、且容易漂移的技能契约描述。

## 9. 建议按 2 个 PR 推进

### PR3：Evidence Ledger MVP

1. 接入最小证据块结构。
2. 让关键内容型任务消费证据块收尾。
3. 保证仅在首批场景启用，不一次性扩散到全部 skill。

### PR4：Registry 深接入与状态机收敛

1. 对 registry 契约做更深接入。
2. 收敛 `semantic_judge` 的主判定职责。
3. 明确主链状态机与 fail-safe 路径。

## 10. 每个 PR 的最小验证

1. `cargo check -p clawd -p skill-runner`
2. `cargo test -p clawd --no-run`
3. 至少补一组对应模块的定向测试或回归 case：
   - Evidence Ledger 改动：覆盖 read-file grounded 输出和 `used_evidence_ids`
   - registry / planner 改动：覆盖 clarify、missing args、risk / confirmation 行为
   - 状态机改动：覆盖 `Done`、`NeedClarify`、`FailSafe` 跳转
4. 若该轮顺手改到 UI，再按仓库门禁执行 `cd UI && npm run lint && npm run build`

## 11. V2 完成标准

1. `used_evidence_ids` 在关键内容型任务中稳定非空。
2. summary / extract / file-grounded 任务的 `grounded_ok` 稳定。
3. 主链成功判定集中，不再依赖分散 sidecar 逻辑。
4. 默认 vendor 与抽样 vendor 在关键场景上不存在明显行为分叉。

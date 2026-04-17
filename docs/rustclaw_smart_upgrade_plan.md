# RustClaw 智能化升级总览（V1 / V2 分拆）

日期：2026-04-02

## 1. 目的

原来的总计划同时承载了两类不同工作：

1. `V1`：低风险收口，把已经存在但未完全启用的能力真正落地。
2. `V2`：结构升级，把局部能力沉淀成长期稳定架构。

这两类工作的目标、风险和验证方式不同，继续写在一份文档里，容易造成“阶段”和“方案”混淆。因此从本次起拆成两份独立计划。

## 2. 当前共识

当前代码已经具备以下基础：

1. `finalizer` 已有结构化 schema 与基础 contract 判据。
2. `verifier` 已有 observe-only / enforce 两种模式与若干低风险门禁项。
3. `skill_registry` 已承载输入输出 schema、风险等级、确认要求等契约字段。
4. `task_journal` 已能记录 route / plan / verify / step trace。

当前仍存在以下共性 gap：

1. `used_evidence_ids` 还没有成为稳定的任务级指标与门禁项。
2. `delivery_consistent`、`llm_calls_per_task` 还缺统一汇总口径。
3. planner / clarify / schedule 对 registry 契约的消费还不够深。
4. “成功完成”的定义仍分散在 finalizer、verifier、delivery、sidecar 之间。

## 3. 两份计划的边界

### V1

定位：

- 生产收口计划

职责：

1. 统一 finalizer 完成判据。
2. 打通 verifier 的低风险灰度 enforce。
3. 补齐交付一致性、回归集、任务级指标。
4. 收紧 locator / binding / clarify 的误判空间。

对应文档：

- [V1 独立计划](./rustclaw_smart_upgrade_plan_v1.md)

### V2

定位：

- 结构升级计划

职责：

1. 引入 Evidence Ledger。
2. 让 registry 更深驱动 planner / verifier / clarify。
3. 统一主链状态机。
4. 收敛 `semantic_judge` 到兜底角色。

对应文档：

- [V2 独立计划](./rustclaw_smart_upgrade_plan_v2.md)

## 4. 执行顺序

执行顺序固定为：

1. 先做 `V1`
2. 再做 `V2`

原因：

1. `V1` 负责建立基线、门禁、指标和回滚条件。
2. `V2` 是结构重塑，如果没有 `V1` 的基线，很难证明收益，也难回滚。

## 5. V2 启动前置条件

只有当以下条件满足，才进入 `V2`：

1. 已有任务级指标：`used_evidence_ids_count`、`delivery_consistent`、`llm_calls_per_task`。
2. `summary`、`scalar`、`FILE token`、`clarify`、`locator binding` 聚焦回归集稳定可复现。
3. verifier 的低风险 enforce 已灰度验证通过。
4. 没有新增独立 validator LLM hop。

## 6. 相关文档

- [V1 独立计划](./rustclaw_smart_upgrade_plan_v1.md)
- [V2 独立计划](./rustclaw_smart_upgrade_plan_v2.md)
- [LLM 优先开发宗旨](./llm_first_development_principles.md)
- [动态回归样例](./dynamic_prompt_regression_cases_20260327.txt)
- [Agent 改造计划（历史专项）](../plans/agent_llm_refactor_plan_20260328.md)

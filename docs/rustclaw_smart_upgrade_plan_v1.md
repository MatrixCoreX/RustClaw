# RustClaw 智能化升级计划 V1

日期：2026-04-02

## 1. 定位

`V1` 是低风险收口计划，不重写主架构，目标是把已经存在但未完全收口的能力真正用起来。

## 2. 目标

1. 把 `finalizer` 变成主成功判据，而不是“有输出就算成功”。
2. 把 `verifier` 从“只记日志”推进到“可灰度 enforce”。
3. 把交付一致性、证据覆盖率、关键回归集纳入门禁。
4. 把路径/文件/上下文绑定做得更保守、更可解释，减少自信误判。

## 3. 当前基础

当前代码已具备以下前置基础：

1. [finalizer.rs](/tmp/rustclaw-workspace/crates/clawd/src/finalizer.rs) 已定义 `FinalizerSchemaOut`、`finalizer_contract_ok`、路径绑定与结构化回退判据。
2. [final_synthesis.rs](/tmp/rustclaw-workspace/crates/clawd/src/agent_engine/final_synthesis.rs) 已在 `observed_read` 与通用收尾路径消费 finalizer 结构化输出。
3. [verifier.rs](/tmp/rustclaw-workspace/crates/clawd/src/verifier.rs) 已覆盖 `SkillNotVisible`、`MissingRequiredArg`、`ConfirmationRequired`、`PrimaryFallbackConflict`、`RouteClarifyRequired` 等校验项。
4. [prepare_round.rs](/tmp/rustclaw-workspace/crates/clawd/src/agent_engine/prepare_round.rs) 已支持基于 `verify_enforce_enabled` 在 observe-only 与 enforce 两种模式之间切换。
5. [task_journal.rs](/tmp/rustclaw-workspace/crates/clawd/src/task_journal.rs) 已具备 route / plan / verify / step trace 的记录能力。

## 4. V1 只解决什么

1. finalizer 合格判据收口。
2. verifier 低风险灰度门禁。
3. `text/messages/FILE token` 的交付一致性。
4. locator / recent execution / clarify 的保守绑定。
5. 任务级指标与聚焦回归集。

## 5. V1 不解决什么

1. 不引入完整 Evidence Ledger。
2. 不重写 planner 主体。
3. 不把 registry 一次性接成全链路唯一契约源。
4. 不做长期双轨架构改造。

## 6. 主要工作流

### A. Finalizer 收口

沿用现有结构化输出：

- `answer`
- `completion_ok`
- `grounded_ok`
- `format_ok`
- `needs_clarify`
- `confidence`
- `used_evidence_ids`

规则：

1. `Done` 必须满足 `completion_ok && grounded_ok && format_ok`。
2. 解析失败、契约不合格、结构化 blob 污染允许回退，但不能把失败回退伪装成成功。
3. `observed_read` 和通用收尾路径使用同一套 contract 判据。

### B. Verifier 落地

1. 默认仍允许回退。
2. 增加 shadow 指标对比。
3. 先只在低风险场景灰度 enforce：
   - `needs_clarify` 却仍试图执行
   - skill 不可见
   - 缺少必填参数
   - primary/fallback 冲突
4. 高风险执行型 skill 继续维持严格确认策略。

### C. 交付一致性

1. 统一 `messages[-1]` 与 `text` 的优先级和镜像关系。
2. 强化 `one-sentence / scalar / FILE token` 契约裁剪。
3. 补 `delivery_consistent` 指标，避免“messages 正常但 text 弱化”。

### D. 绑定与澄清治理

1. 对“那个 README / 刚才那个日志 / 这个目录”这类请求，坚持唯一绑定才能直接执行。
2. 自动 locator 只作为辅助，不作为绝对主判据。
3. 最近执行上下文只能在目标唯一时复用，否则优先澄清。

### E. 指标与回归

聚焦回归集至少覆盖：

1. `summary`
2. `scalar`
3. `file delivery`
4. `locator binding`
5. `clarify`

最小指标集至少覆盖：

1. `used_evidence_ids_count`
2. `delivery_consistent`
3. `llm_calls_per_task`
4. 假成功率

## 7. 实施阶段

### 阶段 0：基线与门禁准备

目标：

- 先建立可比较的基线，而不是直接改行为

工作：

1. 整理现有回归 case。
2. 增加关键日志字段。
3. 建立假成功、澄清命中、交付一致率基线。

完成标准：

1. 能按任务维度统计关键指标。
2. 能复现重点回归 case。

### 阶段 1：V1 收口

目标：

- 把已有框架真正启用

工作：

1. 统一 finalizer 合格条件。
2. verifier shadow 跑通，并在低风险场景 enforce。
3. 交付一致性裁剪稳定。
4. locator / clarify 回归集补齐。

完成标准：

1. 假成功率显著下降。
2. `delivery_utils`、clarify、file delivery 关键 case 稳定通过。
3. 不增加新的独立 validator LLM hop。

### 阶段 2：灰度、评估、回滚

目标：

- 让 V1 可上线、可回滚

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

### WP1：指标与观察面

1. 在 [task_journal.rs](/tmp/rustclaw-workspace/crates/clawd/src/task_journal.rs) 增加 finalizer 结果摘要、证据使用情况、交付一致性、LLM 调用计数摘要。
2. 在 [final_synthesis.rs](/tmp/rustclaw-workspace/crates/clawd/src/agent_engine/final_synthesis.rs) 统一写入 contract 判定结果，避免日志和结果对象各记一套。
3. 在 [llm_gateway.rs](/tmp/rustclaw-workspace/crates/clawd/src/llm_gateway.rs)、[usage.rs](/tmp/rustclaw-workspace/crates/clawd/src/providers/usage.rs) 补任务级 LLM 调用汇总入口。

### WP2：V1 收口

1. 在 [finalizer.rs](/tmp/rustclaw-workspace/crates/clawd/src/finalizer.rs) 统一“合格完成 / 可回退 / 必须失败”的判据。
2. 在 [delivery_utils.rs](/tmp/rustclaw-workspace/crates/clawd/src/delivery_utils.rs) 及其子模块中固化 `text/messages/FILE token` 的单一出口裁剪。
3. 在 [verifier.rs](/tmp/rustclaw-workspace/crates/clawd/src/verifier.rs)、[prepare_round.rs](/tmp/rustclaw-workspace/crates/clawd/src/agent_engine/prepare_round.rs) 明确低风险 enforce 范围与用户提示文案。

## 9. 建议按 2 个 PR 推进

### PR1：基线与门禁

1. 补 `task_journal`、finalizer、provider usage 的任务级指标。
2. 固化聚焦回归集与最小对比脚本。
3. 保证新增观察面默认不改变线上行为。

### PR2：V1 收口

1. 打通 finalizer 合格判据的统一入口。
2. 打开 verifier 的低风险 enforce。
3. 收敛交付一致性与 clarify 兜底文案。

## 10. 每个 PR 的最小验证

1. `cargo check -p clawd -p skill-runner`
2. `cargo test -p clawd --no-run`
3. 至少补一组对应模块的定向测试或回归 case：
   - finalizer / delivery 相关改动：覆盖 `summary`、`scalar`、`FILE token`
   - verifier / planner 相关改动：覆盖 `clarify`、missing args、skill visibility、primary/fallback conflict
4. 若该轮顺手改到 UI，再按仓库门禁执行 `cd UI && npm run lint && npm run build`

## 11. V1 完成后才能进入 V2

`V1` 结束时至少要满足：

1. 已有任务级指标：`used_evidence_ids_count`、`delivery_consistent`、`llm_calls_per_task`。
2. verifier 的低风险 enforce 已灰度验证通过。
3. 聚焦回归集稳定可复现。
4. 没有新增独立 validator LLM hop。

## 12. 验收 V1 工作成果

### 12.1 必须通过的验收项

1. `finalizer` 验收：
   - `Done` 不再等同于“有输出”
   - 最终成功必须与 `completion_ok && grounded_ok && format_ok` 对齐
   - 解析失败或 contract 不合格时，系统不会把回退结果伪装成成功
2. `verifier` 验收：
   - `needs_clarify` 却仍试图执行、skill 不可见、缺少必填参数、primary/fallback 冲突，这 4 类低风险场景可被稳定拦截或记录 shadow 结果
   - 高风险执行型 skill 不会因为 V1 收口而放松确认要求
3. 交付一致性验收：
   - `messages[-1]`、`text`、`FILE token` 对外语义一致
   - `summary`、`scalar`、`FILE token` 三类输出不会被额外废话污染
4. 绑定与澄清验收：
   - “那个 README / 刚才那个日志 / 这个目录”这类请求在目标不唯一时会优先澄清
   - recent execution 不会在目标不唯一时静默覆盖当前请求
5. 指标与回归验收：
   - 任务级能稳定产出 `used_evidence_ids_count`、`delivery_consistent`、`llm_calls_per_task`
   - 聚焦回归集可重复跑通，并能对比变更前后结果

### 12.2 需要提交的验收证据

1. 一份回归结果摘要：
   - `summary`
   - `scalar`
   - `file delivery`
   - `locator binding`
   - `clarify`
2. 一份指标对比摘要：
   - 假成功率
   - `delivery_consistent`
   - `llm_calls_per_task`
   - `used_evidence_ids_count`
3. 一份灰度结果摘要：
   - 默认 vendor 结果
   - 至少一个非默认 vendor 抽样结果
   - 是否触发回滚条件
4. 一次可执行的最小验证记录：
   - `cargo check -p clawd -p skill-runner`
   - `cargo test -p clawd --no-run`
   - 如涉及 UI，`cd UI && npm run lint && npm run build`

### 12.3 直接判定 V1 不通过的情况

1. 关键场景里仍大量出现“有输出即成功”。
2. `delivery_consistent` 无法稳定产出，或统计口径前后不一致。
3. verifier 低风险 enforce 打开后出现明显误杀，且没有清晰回滚策略。
4. 聚焦回归集无法稳定复现，或通过结果依赖手工解释。
5. `llm_calls_per_task` 明显超预算，或为收口新增了独立 validator LLM hop。
6. 默认 vendor 看起来正常，但换一个 vendor 抽样后行为明显漂移。

### 12.4 验收结论模板

验收结论只允许有三种：

1. 通过：
   - 所有必须通过项满足
   - 没触发任一直接失败条件
2. 有条件通过：
   - 主链已达标
   - 仍存在明确、可控、已登记的残余问题
   - 不影响进入小流量灰度
3. 不通过：
   - 任一关键判据未满足
   - 或无法证明 V1 相比基线真的更稳

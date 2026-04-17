# RustClaw Agent 改造计划（LLM 潜力释放版）

日期：2026-03-28

> 历史说明：本计划写于 prompt 分层重构之前，文中若出现
> `prompts/vendors/*/...` 等路径，均属于当时的旧目录结构，不代表当前实现。
> 现行 prompt 主链已迁移到 `prompts/layers/`，skill 主体位于
> `prompts/layers/generated/skills/`，vendor 差异位于
> `prompts/layers/vendor_patches/<vendor>/skills/`。

## 1. 目标

1. 在不依赖硬关键词匹配的前提下，提升自然语言流程正确率，减少“假成功”。
2. 保持 LLM 请求层次简洁：非必要不新增请求层，不引入多余 hop。
3. 让 `status=succeeded` 更接近“任务真正完成”，而不是“有输出即成功”。

## 2. 约束

1. 不在 clawd 代码里增加语义硬匹配词表来判定成功/失败。
2. 优先在已有 LLM 层内完成判定，不额外拆分 validator 请求层。
3. 优先复用现有主链：`intent_normalizer -> routed execution(chat/act) -> finalizer`。
4. 保持请求预算可控：同等流量下单任务平均 LLM 请求数不显著上升。

## 3. 当前问题总结

1. 结构成功但语义未完成：读文件后返回标题、元话术、泛化句。
2. 输出契约偶发偏离：`text/messages` 一致性和“summary/one-liner”质量不稳定。
3. 判定分散：部分能力由 sidecar judge 处理，主链目标函数不够统一。

## 4. 目标架构（不新增层数）

1. 入口层（保留）：Intent Normalizer
- 输出继续包含：`resolved_user_intent`, `routed_mode`, `needs_clarify`, `output_contract`。
- 增强 `output_contract` 语义字段（见 4.3），但仍是同一请求。

2. 执行层（保留）：Chat / Act / ChatAct
- Chat 路由：直接回答。
- Act/ChatAct 路由：agent loop 规划与执行。

3. Finalizer 层（保留，重点改造）
- 在同一次 finalizer 请求里同时输出：
  - `answer`
  - `completion_ok`
  - `grounded_ok`
  - `format_ok`
  - `needs_clarify`
  - `confidence`
  - `used_evidence_ids`
- 由同一请求完成“生成 + 自检”，不再新增独立 validator hop。

### 4.1 Evidence Ledger（证据账本）

将执行结果从“长文本拼接”改为“可引用证据块”传给 finalizer：

字段建议：
- `evidence_id`（如 E1, E2）
- `source`（skill/tool）
- `kind`（read_file/run_cmd/list_dir/sql/query 等）
- `target`（path/table/query）
- `excerpt`（截断后的可读片段）
- `status`（success/failure）
- `step_no`

规则：
1. finalizer 必须给出 `used_evidence_ids`。
2. 如果 `completion_ok=true` 但 `used_evidence_ids` 为空，则视为不通过。

### 4.2 合并判定职责（减少 sidecar 分散）

1. 现有 `semantic_judge` 的短文本可发布判断，逐步下沉到 finalizer 的结构化自检字段。
2. `meta_respond` 这类判定保留兜底，但不再作为主成功判据。

### 4.3 output_contract 扩展

在 `output_contract` 中增加语义约束位（由 normalizer 同次输出）：

- `requires_grounding`（回答必须绑定证据）
- `requires_single_sentence`（一语总结）
- `requires_scalar_only`（仅数字/值）
- `requires_delivery_token`（FILE token）

执行侧只做确定性 gating，不做词表语义匹配。

### 4.4 请求层预算与上限

为避免“能力增强=层数膨胀”，定义运行时预算：

1. Chat 路径预算
- `normalizer(1) + chat(1)`，默认不触发 finalizer。

2. Act 路径预算
- `normalizer(1) + planner(N) + finalizer(<=1)`。
- 禁止新增独立 validator hop。

3. Clarify 路径预算
- `normalizer(1) + clarify(1)`。

4. 预算治理
- 在日志中记录 `llm_calls_per_task`，用于回归对比和门禁。

### 4.5 统一状态机（防假成功）

主链统一到 5 个状态，避免“有输出即成功”：

1. `NeedClarify`
2. `NeedTool`
3. `NeedFinalize`
4. `Done`
5. `FailSafe`

状态约束：
1. `Done` 必须满足 `completion_ok && grounded_ok && format_ok`。
2. 若未满足且无可重试空间，进入 `FailSafe`（确定性兜底），而不是标记成功。

### 4.6 text/messages 一致性策略

1. 统一交付优先级：`messages[-1]` 为主，`text` 为兼容镜像字段。
2. 若两者冲突，按 `output_contract` 重新裁剪并回填，禁止出现“messages 正常但 text 弱化/截断”。
3. 在结果落库前增加一致性检查日志字段：`delivery_consistent=true/false`。

## 5. 分阶段实施

### 阶段 A：统一契约（低风险）

1. 在 finalizer prompt 中引入结构化输出 schema（JSON）。
2. 保留现有文本输出兼容路径（解析失败时回退老逻辑）。
3. 打通 `completion_ok/grounded_ok/format_ok` 的日志落盘。

验收：
1. 不增加新的 LLM 调用层。
2. 关键 case 中 `used_evidence_ids` 非空且可回溯。

### 阶段 B：证据账本接入（中风险）

1. 由 `subtask_results` 生成 Evidence Ledger。
2. finalizer 上下文从“拼接原文”迁移为“证据块 + 用户请求 +契约”。
3. 若 schema 合格即直接交付；不合格仅允许一次同层重写。

验收：
1. “读文件后返回元话术”显著下降。
2. summary 类 case 的 grounded_ok 稳定通过。

### 阶段 C：收敛 sidecar judge（中风险）

1. 把可发布性判断主要迁移到 finalizer 自检。
2. semantic_judge 仅保留安全兜底与异常降级。

验收：
1. 主链判定逻辑集中。
2. 请求成本不高于现状或仅小幅增加。

### 阶段 D：灰度与门禁（低风险）

1. 先对 `default/openai` vendor 灰度，其他 vendor 跟进。
2. 门禁条件（任一不满足则回滚）：
- 假成功率未下降；
- `llm_calls_per_task` 超预算；
- P95 响应时延显著恶化；
- 关键 case（summary / scalar / file delivery）回归失败。

## 6. 代码落点（建议）

1. `crates/clawd/src/agent_engine.rs`
- 增加 finalizer 结构化输出解析。
- 接入 Evidence Ledger 组装。
- 统一成功判定 gating。

2. `crates/clawd/src/intent_router.rs`
- 扩展 output_contract 字段定义与解析。

3. `crates/clawd/src/ask_flow.rs`
- 统一 routed 执行后的交付契约收敛入口。

4. `crates/clawd/src/delivery_utils.rs`
- 实现 `text/messages` 一致性修复与契约裁剪。

5. `crates/clawd/src/semantic_judge.rs`
- 收敛为兜底角色，主判定迁移到 finalizer 自检字段。

6. `prompts/vendors/*/chat_skill_system_prompt.md`（历史写法；现行对应 layered prompt）
- 增加结构化输出与证据引用约束。

7. `prompts/vendors/*/single_plan_execution_prompt.md`（历史写法；现行对应 layered prompt）
- 约束 plan 的 respond 必须可被 evidence 支撑。

8. `prompts/vendors/*/loop_incremental_plan_prompt.md`（历史写法；现行对应 layered prompt）
- 多轮里补“未满足契约不得终止”规则。

## 7. 评估指标

1. 语义成功率（人工判定）
- 目标：相对当前提升 >= 20%。

2. 假成功率
- 定义：status 成功但 completion_ok 或 grounded_ok 为 false。
- 目标：下降 >= 50%。

3. 平均 LLM 请求数/任务
- 目标：不显著高于当前（允许 <= +10%）。

4. 证据覆盖率
- 定义：finalizer 输出中 `used_evidence_ids` 非空占比。
- 目标：summary/抽取类任务 >= 95%。

5. 交付一致率
- 定义：`text/messages` 一致且满足契约的比例。
- 目标：>= 99%。

6. 澄清命中率
- 该问时问，不该问时不问。

7. P95 延迟与 Token 成本
- 目标：P95 不显著恶化；token/case 增幅在预算内。

## 8. 回滚策略

1. 保留老 finalizer 文本路径开关：`finalizer_schema_enabled`。
2. 新逻辑异常时降级到老路径，不阻塞主流程。
3. 逐 vendor 灰度，先 default/openai，再扩展其他 vendor。

## 9. 立即执行清单（下一个迭代）

1. 落 finalizer JSON schema 与解析器。
2. 接入 Evidence Ledger 最小版本（read_file/run_cmd/list_dir 三类）。
3. 增加 `llm_calls_per_task`、`delivery_consistent`、`used_evidence_ids` 日志字段。
4. 跑聚焦回归集与全量手工集，生成对比报告。

## 10. 发布门禁清单

上线前必须全部满足：

1. 不新增独立 validator LLM hop。
2. 关键路径预算达标（Chat/Act/Clarify）。
3. 假成功率与交付一致率达到目标。
4. 回滚开关可用并经过演练。

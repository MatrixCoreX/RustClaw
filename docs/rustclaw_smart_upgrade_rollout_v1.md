# RustClaw Smart Upgrade V1 Rollout Guide

本文件补充 `docs/rustclaw_smart_upgrade_plan_v1.md` 的上线与回滚口径，不改动原计划，只给执行侧一个统一参照。

## 1. 默认 Vendor 灰度

默认 vendor 指当前生产主路径上已经启用、且历史行为最稳定的 provider。

建议按下面三段灰度推进：

1. `shadow-only`
   - 打开 V1 指标观察，不改默认执行行为。
   - 重点看 `task_journal.summary.task_metrics`：
     - `delivery_consistent`
     - `used_evidence_ids_count`
     - `llm_calls_per_task`
   - 同时关注 `task_journal.summary.finalizer_summary`：
     - `disposition`
     - `contract_ok`
     - `fallback`

2. `low-risk enforce`
   - 仅在默认 vendor 上开启 verifier 的低风险 enforce。
   - 本轮 enforce 范围限定为：
     - `RouteClarifyRequired`
     - `SkillNotVisible`
     - `MissingRequiredArg`
     - `PrimaryFallbackConflict`
     - `InvalidDependsOn`
   - 高风险执行型 skill 仍保留确认策略，不因为 V1 收口而放松。

3. `default-on`
   - 连续观察窗口内没有明显恶化后，再把 V1 作为默认行为。

## 2. 非默认 Vendor 抽样

非默认 vendor 不直接全量放开，先做抽样验证。

建议口径：

1. 先按固定样本任务回放
   - 覆盖 `summary / scalar / file delivery / clarify / missing args / skill visibility / primary-fallback conflict`。
   - 每类至少保留一条成功样本和一条拦截样本。

2. 再做小流量在线抽样
   - 建议先控制在单独任务批次或小比例流量内。
   - 观察项与默认 vendor 相同，但额外记录：
     - `finalizer_summary.disposition=must_fail` 的占比
     - `delivery_consistent=false` 的占比
     - `llm_calls_per_task` 是否明显抬升

3. 非默认 vendor 只要出现下面任一信号，就停止放量
   - `must_fail` 明显高于默认 vendor
   - `delivery_consistent=false` 连续出现
   - 同类请求的 `llm_calls_per_task` 明显高于默认 vendor
   - 明显出现“该拦截未拦截”或“该成功被误杀”

## 3. 回滚触发条件

满足任一条件即可回滚到 V1 之前的行为或仅保留 shadow：

1. 真实用户任务出现“结构化不合格但仍被当成成功返回”的回归。
2. `delivery_consistent=false` 在真实流量中连续出现，且影响最终通道交付。
3. 低风险 verifier enforce 出现误拦截，影响正常 `read_file` / 纯读取类任务。
4. 单任务 `llm_calls_per_task` 明显抬升，且无法用输入复杂度解释。

## 4. 观察面建议

上线后优先查看两处：

1. 任务结果里的 `task_journal`
   - 看单任务是否是 `qualified_completion / allow_fallback / must_fail`
   - 看最终 `delivery_consistent` 是否为 `true`

2. `logs/model_io.log`
   - 按 `task_id` 看调用次数与 vendor 分布
   - 对照 `llm_calls_per_task` 判断是否有额外 hop 或重试放大

## 5. 执行建议

默认 vendor 先灰度，再启用低风险 enforce；非默认 vendor 只做抽样，不直接全量。遇到 must-fail、delivery 不一致、或调用次数异常时，优先回滚到 shadow-only，而不是继续扩大范围。

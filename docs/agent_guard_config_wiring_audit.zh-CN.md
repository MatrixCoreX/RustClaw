# Agent Guard 配置接线审计

最后更新：2026-07-20

本文记录 `configs/agent_guard.toml` 字段的当前接线和所有权，作为 agent-loop 迁移后加固依据。

## 摘要

- 交互式任务 slice 和管理员 hard ceiling 通过 `task_budget_contract::load_task_budget_policy()` 接线；单计划 action 容量、重复、verifier 和 recipe 控制由 `agent_engine::support::load_agent_loop_guard_policy()` 负责。
- Route-authority runtime 开关已经从 `AgentLoopGuardPolicy` 删除。旧 route-authority、canary 和 `agent_decides_*` 名称只是被忽略的历史 key，不得回到生产配置。
- `registry_idempotency_guard_scope` 与 `answer_verifier_enforce_required_scope` 已收敛到最终 `all` 机器边界；历史非 `all` 值会规范化为 `all`，不能当作回滚/调试开关。
- 领域 action list、旧 dedup 字段、`dynamic_rules`、`messages` 和 `trace_messages` 已物理删除，`check_agent_loop_guard_final_scope.py` 阻止重新引入。
- 用户可见文案应使用 `message_key`，再由 finalizer/LLM/i18n 渲染。
- `agent.hooks` pre-tool policy 使用机器 token：deny、require-confirmation 和 background-wait 根据 action/tool ref 决定，不读取用户自然语言。

## 接线矩阵

| 配置路径 | 当前状态 | 所有者与处理 |
| --- | --- | --- |
| `agent.loop_guard.max_steps` | 已接线，为单计划 action 容量，不是整任务 round/tool completion limit | Plan execution guard；只有已验证计划无法表达完整阶段时才调整 |
| `answer_verifier_retry_limit` | 已物理删除 | Final-answer recovery 只允许由结构化 verifier 字段选择的一次有界 synthesis retry |
| `repeat_action_limit` | 已接线 | Loop repeat guard |
| `repeat_same_action_limit` | 已解析，兼容存在 | 去重清理确认无重复语义前保留 |
| `agent.task_budget.admin_max_*` | 已接线 | 累计 model turn、tool call、token、cost、elapsed、continuation 和 non-resumable runtime 管理员上限 |
| `profiles.*.soft_slice_seconds` | 已接线 | 可恢复 wall-time slice；用于 latency/checkpoint cadence，不用于限制任务复杂度 |
| `profiles.*.stagnation_tolerance` | 已接线 | 连续结构化无进展容忍度，应大于 1 |
| `provider_timeout_class` | 已接线 | 只能为 `short`、`standard` 或 `long_tail` |
| `tool_timeout_class` | 已接线 | 可恢复长尾工具必须公开 async/checkpoint 状态 |
| `max_rounds`、`max_tool_calls`、`no_progress_limit`、`recoverable_failure_extra_rounds`、`multi_round_enabled` | 已从交互配置、parser、loop state、stop signal 和 profile override 物理删除 | 不得恢复；显式 cap 只属于非交互/child request |
| 旧 `budget_profiles.*` loop threshold | 已由 `[agent.task_budget.profiles.*]` soft slice 和必要的 `max_steps` 替代 | 不得以 profile 名重新引入 round/tool threshold |
| `answer_verifier_enforce_required_scope` | 缺失或历史非 `all` 值规范化为 `all` | Answer Verifier 证据边界 |
| `answer_verifier_enforce_required` | 历史 bool，不解析 | 不得扩展 |
| `semantic_route_authority`、`agent_loop_canary_bucket`、`agent_decides_semantic_route`、`agent_decides_migration_class` | 已删除或忽略的历史 key | `check_route_authority_legacy_keys.py` 阻止重新进入生产 |
| `registry_idempotency_guard_scope` | 缺失或历史非 `all` 值规范化为 `all` | Registry 幂等边界 |
| `registry_idempotency_guard` | 历史 bool，不解析 | 不得扩展 |
| `agent.hooks.handlers` | 已接线为可信 command/HTTP/MCP 生命周期 handler | 使用 stage、trust/hash、bounds、retry、failure policy 和 blocking mode；只有 PreToolUse/PermissionRequest 可阻断 |
| 旧 hook action/tool list | 已物理删除 | 使用可信有界 handler 或已有 registry/permission owner |
| 旧领域 action list/dedup/prose section | 已物理删除 | 使用 registry policy metadata、prompt 和机器 reason/message key |

## 风险说明

- 把被忽略的历史 key 当作行为开关会造成虚假安全感。
- 无人读取的配置仍会误导 operator。
- 领域 action list 会复制 registry 元数据并产生漂移。
- `agent.messages` 如果直接接进 finalizer，会重新形成写死多语言回复路径。
- `dynamic_rules` 如果承载单个技能 prompt，会重新产生领域 prompt 债务。

## 必需后续

1. 启用 registry-driven guard 前保持本地与 Docker registry parity：
   `python3 scripts/check_skill_registry_parity.py --mode p3 --strict`。
2. 影响行为的原因使用稳定 `reason_code`。
3. 代码/计划完成后再跑广泛 NL canary；文档改动使用聚焦 Rust 测试和 hard-match scan。

## 验证

```bash
python3 scripts/check_no_nl_hardmatch.py
git diff --check
```

行为迁移前：

```bash
cargo test -p clawd support -- --nocapture
cargo test -p clawd loop_control -- --nocapture
bash scripts/nl_tests/run_suite.sh evidence_policy_offline
bash scripts/nl_tests/run_suite.sh runtime_capability_boundary
```

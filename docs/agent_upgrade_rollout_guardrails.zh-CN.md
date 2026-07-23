# Agent 升级 Rollout Guardrail

最后更新：2026-07-24

本文定义 agent loop、verifier、finalizer、registry、lifecycle 和自然语言执行改动的当前 rollout/rollback 边界。

## 不可妥协边界

- 普通语义 authority 保持在 planner/agent loop。
- 生产 runtime 不得匹配用户语言短语。
- Runtime、verifier、finalizer、policy 和 adapter 不得增加固定用户回复模板。
- 确定性代码只发出机器字段、status/reason code、路径、计数、`message_key`、evidence 和 artifact ref。
- 用户 prose 由模型或 i18n renderer 按请求语言合成。
- 模型输出不能绕过风险、权限、确认、dry-run、sandbox、副作用 reconciliation 和管理员 ceiling。
- 真实远端发布/修改必须满足已声明策略和确认。广泛 NL 对破坏性或付费媒体路径默认使用 dry-run/mock，除非测试明确拥有安全 live scope。

## 已接线 Runtime 控制

| 控制 | 位置 | 含义 | 回滚 |
| --- | --- | --- | --- |
| `max_steps` | `[agent.loop_guard]` | 单计划 action 容量，不是整任务 round/tool 上限 | 恢复旧值并重启 |
| `repeat_action_limit` | `[agent.loop_guard]` | 跨 round 重复 guard | 恢复旧值并重启 |
| `repeat_same_action_limit` | `[agent.loop_guard]` | 兼容重复 guard | 恢复旧值并重启 |
| `admin_max_*` | `[agent.task_budget]` | 累计 model/tool/token/cost/elapsed/continuation/non-resumable fail-closed 上限 | 仅管理员策略，模型不能提高 |
| `profiles.*.soft_slice_seconds` | task-budget profile | 可恢复 checkpoint 节奏 | 恢复旧 duration |
| `profiles.*.stagnation_tolerance` | task-budget profile | 连续结构化无进展容忍度 | 恢复旧 tolerance |
| `provider_timeout_class` | task-budget profile | Provider timeout class | `short`、`standard` 或 `long_tail` |
| `tool_timeout_class` | task-budget profile | Tool timeout class | 长尾工具需要 async/checkpoint |
| `answer_verifier_enforce_required_scope` | `[agent.loop_guard]` | 必需证据最终 scope，规范化为 `all` | 修 evidence contract，不关闭 |
| `registry_idempotency_guard_scope` | `[agent.loop_guard]` | Registry repeat/idempotency 最终 scope，规范化为 `all` | 修 registry policy，不关闭 |

不得恢复以下交互控制：

- `max_rounds`
- `max_tool_calls`
- `no_progress_limit`
- `recoverable_failure_extra_rounds`
- `multi_round_enabled`
- route-authority、canary、`agent_decides_*`、selected-contract 或 bool guard 兼容开关

完整 task-budget/runtime 迁移的回滚方式是代码回滚，不是保留双运行分支。显式 cap 只允许在非交互或 child-task 请求合同中使用。

## 归因要求

使用 TaskJournal 和带版本 task event，至少覆盖 planner action、capability resolution、verifier/evidence、permission/policy、budget/checkpoint、tool/capability result、mutation reconciliation、artifact/evidence ref、final attribution，以及 provider 可提供时的 LLM usage/cost。

历史 normalizer、first-layer 和 route-gate 字段只允许用于归档 artifact 对比，不得写入新的 runtime attribution。

不得存储原始 secret 或未脱敏私有 payload。原始 prompt/provider 数据只能保存在已有访问控制、脱敏与保留策略约束的 debug/教学记录中。

## 开发门禁

```bash
python3 scripts/check_no_nl_hardmatch.py
python3 scripts/check_no_runtime_hard_reply.py
python3 scripts/check_long_files.py
git diff --check
cargo check -p clawd -p claw-core
```

根据改动运行 resolver、verifier、budget、lifecycle、replay、policy、registry、finalizer、CLI 或 UI 聚焦合同测试。开发期使用最小受影响 NL。

Release-sensitive 删除或 runtime 行为发布前：

```bash
python3 scripts/nl_tests/build_release_gate_subset.py --check
bash scripts/nl_tests/run_suite.sh agent_parity_gate
```

生成 subset 是行数和类别数事实源，不得在 rollout policy 中保留旧固定数量。

## 回滚阈值

出现以下证据时停止并回滚责任改动：

- 确认、权限、sandbox 或 dry-run 被绕过；
- 非幂等副作用在没有 reconciliation 时重放；
- 新增未知生产 NL hard match 或固定 runtime 回复；
- route/plan/permission/verifier/final status replay 存在无法解释的 mismatch；
- 相对可比 baseline 出现显著 pass-rate、verifier false-block、LLM amplification 或 latency 回归；
- checkpoint 丢失、lease fencing 失败或无法安全 resume。

只在配置项确实是行为 owner 时回滚该值，否则回滚完整责任代码变更，不得 reset 无关用户工作。重启后重跑最小失败 case 和受影响确定性门禁，并记录 run ref、机器 reason/status、owner 和后续处理。

## 当前风险

- `repo/tasks.rs` 和 `UI/src/App.tsx` 接近 2,000 行。
- 精确输出 finalization 必须保持 zero-domain，不依赖 registry skill name。
- Provider usage/cost 可能 unknown；不能把 unknown 当作零成本。
- 长尾工具必须具备 heartbeat/checkpoint/async 状态才能安全 pause/resume。
- 历史 fixture 中的旧 route 字段必须与当前执行隔离。
- Main/Docker registry 必须保持 parity。
- 付费多媒体和远端修改测试需要显式安全 live scope，否则使用 dry-run/offline。

## 支持 Guard

```bash
python3 scripts/check_planner_runtime_boundary.py
python3 scripts/check_pre_planner_exit_inventory.py
python3 scripts/check_finalizer_architecture.py
python3 scripts/check_repair_boundary_inventory_coverage.py
python3 scripts/check_repair_no_user_text_fields.py
python3 scripts/check_policy_decision_tokens.py
python3 scripts/check_registry_policy_contracts.py
python3 scripts/check_skill_registry_aliases.py
python3 scripts/check_skill_registry_parity.py --mode all --strict
python3 scripts/check_cross_platform_contracts.py
```

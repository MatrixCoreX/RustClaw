# Agent Loop 前置决策清单

最后更新：2026-07-24

本文记录第一次普通 planner 调用前的当前机器工作。旧 intent normalizer、contract-repair judge、post-route 语义策略、`AskMode` 和 route-authority 开关已从实时 ask 路径物理删除。

## 当前 Authority 模型

- 每个普通 `kind=ask` 请求都进入 `agent_engine::run_agent_with_tools()`。
- 第一个 planner action 拥有普通 respond/clarify/execute/capability 语义。
- 边界代码只物化输入/上下文、验证显式协议 mode、执行安全策略和管理员 ceiling。
- 历史 route 字段只能从归档 fixture/log 读取，不能写成新的 route authority，也不能被执行路径消费。
- 任何 runtime 层都不得匹配用户语言短语或本地化 `text/error_text` 来选择行为。

## 主 `kind=ask` 路径

1. `worker::worker_once()`
2. `worker::process_ask_task()`
3. `worker::ask_input::prepare_ask_input()`
4. `worker::ask_planner_frontdoor::prepare_planner_owned_ask_routing()`
5. `worker::ask_execution_context::prepare_ask_execution_context()`
6. `worker::ask_runtime::execute_ask_dispatch()`
7. `agent_engine::run_agent_with_tools()`
8. `finalize::finalize_ask_result()`

## 决策面清单

| Surface | 当前 owner | 职责 | 是否可绕过普通 planner |
| --- | --- | --- | --- |
| Task claim/kind dispatch | `worker_once()` | Lease、heartbeat、timeout、取消和 `ask/run_skill` 派发 | 可以，因为这是任务协议 |
| 显式 capability payload | `run_capability` | 执行调用方给出的机器 capability 合同 | 只限显式 payload |
| 显式 schedule direct text | `maybe_finalize_schedule_direct_text_success()` | 交付 `schedule_task_mode=direct_text`，不推断语义 | 只限显式 scheduler 元数据 |
| 输入物化 | `prepare_ask_input()` 和 planner frontdoor | 文本、音频转写、附件 ref、显式命令/locator 事实 | 不可语义绕过 |
| 会话/上下文构建 | task context builder、`prepare_ask_execution_context()` | Memory、knowledge、image、alias、压缩历史和初始 observation | 不可语义绕过 |
| Planner loop | `run_agent_with_tools()` | Respond、clarify、plan、调用/观察/repair/synthesize | 普通语义 authority |
| Resolver/verifier/policy | resolver、verifier、hook、permission runtime | 验证机器合同并阻止不安全执行 | 可阻断/要求确认，不可重释 intent |
| Finalizer/delivery | `finalize/`、channel adapter | 精确序列化、grounded synthesis、持久化和交付 | 可保留精确机器输出，不可按语言/技能路由 |

## 保留在 Agent Loop 外的工作

- Task lease、heartbeat、timeout、取消、队列和 kind 派发。
- 鉴权、actor/session/channel 绑定和 workspace scope。
- 附件物化、转写、图片预处理和有界上下文压缩。
- Registry 可见性、schema、权限、风险、确认、dry-run、sandbox 和副作用策略。
- 路径 confinement、显式 locator 事实和 artifact 安全。
- Task-budget soft slice、管理员 hard ceiling、重复和结构化停滞 guard。
- Evidence admission、Answer Verifier、精确 selector、secret 脱敏和交付持久化。

## Agent Loop 内的语义决策

- 普通请求应 respond、clarify、execute、continue、wait 还是 stop。
- 根据任务含义选择 capability/action。
- 判断缺失信息是否为语义 blocker。
- 根据结构化 tool/provider/verifier observation 恢复。
- 从 grounded evidence 合成用户可见语言。

## 允许的 Planner 前 Provider 调用

音频转写、图片分析和模型辅助上下文压缩可以在结构化 trigger 存在时先运行，但只能产生输入/上下文证据，不能选择普通 route 或 final response。

## 已删除 Surface

- Intent normalizer route authority；
- pre-route contract-repair 语义 judge；
- post-route 语义 policy；
- active-clarify route shortcut；
- pre-planner direct-answer gate；
- direct existing-file 语义 shortcut；
- `agent_decides_semantic_route`、migration class、canary 或 rollback route 开关。

历史引用只允许存在于隔离 fixture、replay reader、迁移 inventory 和 guard self-test。

## 验证门禁

```bash
python3 scripts/check_planner_runtime_boundary.py
python3 scripts/check_pre_planner_exit_inventory.py
python3 scripts/check_route_authority_legacy_keys.py
python3 scripts/check_legacy_route_boundary.py
python3 scripts/check_no_nl_hardmatch.py
python3 scripts/check_no_runtime_hard_reply.py
python3 scripts/check_long_files.py
cargo test -p clawd ask_runtime -- --nocapture
cargo test -p clawd planner_frontdoor -- --nocapture
```

行为修改开发期运行最小受影响 NL；release-sensitive 删除前运行 release-gate 等价覆盖。

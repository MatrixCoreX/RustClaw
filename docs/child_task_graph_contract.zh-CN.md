# 持久化子任务图合同

RustClaw 把 planner 授权的 subagent 工作持久化为任务图。任务图是执行合同，不是语义路由器：模型提出角色、目标、依赖和结构化 scope；runtime 验证并执行可信角色策略、权限、所有权、就绪条件和生命周期。

## 持久化记录

- `child_task_graphs` 保存父任务、schema version、图状态和最大并行 child 数。
- `child_task_graph_nodes` 保存每个 child task ID、可信 role token、required 标记、就绪状态、权限和 merge policy、拥有的 workspace 路径、预算、模型/工具策略、结果合同和带版本 steering。
- `child_task_graph_edges` 保存声明的依赖和 runtime 新增依赖。

图和 child task 行在同一个 SQLite 事务中准入。节点缺失、自依赖、环、无效 workspace 路径、不可信角色或权限组合都会被拒绝。

## 就绪与所有权

节点使用稳定机器状态：

- `ready`
- `blocked_dependency`
- `blocked_capacity`
- `running`
- `succeeded`、`failed`、`timeout`、`canceled` 等终态

队列只认领 ready 节点。必需前置节点失败会取消依赖工作；可选节点失败不会自动使无关节点失败。每次终态迁移后都会重新协调容量与依赖。

可写 child 必须声明 workspace 相对 `owned_paths`。缺失时规范化为 workspace 根。相等或祖先重叠路径通过确定性 edge 串行化；路径不相交且使用隔离 worktree 的 writer 可并行。Patch 准入会根据持久化所有权重新检查每个变更文件，并拒绝过期、缺失或越界所有权。

## 角色与策略

可信角色定义位于 `configs/agent_guard.toml`。定义把 role token 绑定到 role family、默认和允许的权限 profile、结果合同要求及可选模型/工具策略。模型可以选择可信 token，但不能创建权限或扩大策略。

每个持久化节点记录生效后的角色、权限、预算、模型/工具策略、merge policy 和结果合同。Runtime 不得匹配用户自然语言来选择角色。

## 重启、重试与控制

- 启动协调根据权威任务行推导节点就绪状态。
- 过期 child claim 会重新入队，并在下一次认领时获得更大的 claim generation。
- 重试终态 child 会创建新 task/node，并在事务中重连入边和出边。
- Pause/resume 输入可以通过 compare-and-swap 保存 steering directive，包括 checkpoint、trigger、用户输入和结构化约束。
- 父任务失败、超时或取消时，取消未完成 child，并以机器状态关闭任务图。

## 事件与结果

Runtime 在父任务写入带版本的 `subagent_graph` 快照，并在 child 状态变化时写入 `subagent_node` 快照。图快照包含：

- 父/子任务 ID 和依赖；
- 就绪状态、角色、required、权限、所有权和 merge policy；
- 预算、模型/工具策略、结果合同和 steering version；
- child 状态、结构化结果、evidence/artifact/patch/findings ref；
- provider 报告的 token/成本用量。

CLI event/replay 命令保留这些 payload。UI task trace 显示精简的图或节点摘要，原始 JSON 通过渐进披露展示。消费者必须使用机器字段，不得解析 child 自然语言来推断就绪、成功、权限或 merge 资格。

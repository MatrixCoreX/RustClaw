# 任务状态与上下文

上一页：[安全与执行](02-security-execution.zh-CN.md) |
[架构索引](README.md) |
下一页：[编码与可观测性](04-coding-observability.zh-CN.md)

前台请求超时不会终止已经持久化的任务。Worker 使用 lease 与 heartbeat；需要续跑
的工作通过 checkpoint 和机器生命周期字段表达。

```mermaid
flowchart TD
    A[POST /v1/tasks] --> B[(queued task)]
    B --> C[返回 task_id]
    B --> D[Worker 认领<br/>lease_owner + claim_attempt]
    D --> E[Agent loop 或显式技能]
    E --> F{预算 / provider / async 状态}
    F -->|继续| E
    F -->|waiting 或 background| G[保存 TaskBudgetSlice + checkpoint]
    G --> H[释放精确 worker claim]
    H --> I{是否到恢复时间}
    I -->|否| J[调用方轮询同一 task_id]
    I -->|是| K[Recovery 认领新 generation]
    K --> L[恢复 observations、artifacts、<br/>side effects 与累计计数]
    L --> E
    F -->|终态| M[保存最终结果]
    M --> J
```

上下文从带 provenance 的显式来源组装，并受确定性预算约束。记忆和知识库检索只
提供候选内容，不参与语义路由。

```mermaid
flowchart TD
    A[当前任务与会话] --> B[Context builder]
    C[对话历史] --> B
    D[记忆与知识索引] --> E[Retrieval/use policy]
    F[目标、journal、artifacts、<br/>coding evidence] --> B
    E --> B
    B --> G[Provenance records<br/>source_ref + reason + scope]
    G --> H[ContextBudgetReport]
    H --> I{是否符合预算}
    I -->|是| J[Included refs]
    I -->|否| K[Excluded refs + 确定性压缩]
    J --> L[Planner context]
    K --> L
    L --> M[context_budget / context_compaction / memory_trace events]
```

记忆写入发生在用户可见回复之后，并使用结构化 intent schema。用户可以查看、
过期或删除保存的偏好和事实。

# 编码与可观测性

上一页：[任务状态与上下文](03-task-state-context.zh-CN.md) |
[架构索引](README.md) |
下一页：[技能、多媒体与模型](05-skills-media-models.zh-CN.md)

编码修改使用明确的路径所有权、patch 前置条件、补偿快照和真实观测到的验证结果。
检查失败会成为结构化 loop observation，而不是固定写死的用户回复。

```mermaid
flowchart TD
    A[编码请求或目标] --> B[检查 workspace 与证据]
    B --> C[Planner change contract]
    C --> D[Patch preview<br/>路径 + precondition hashes]
    D --> E[Verifier + 精确 mutation approval]
    E --> F[保存补偿快照后单次应用]
    F --> G[Patch checkpoint + 有界 diff artifact]
    G --> H[执行 verification contract]
    H --> I{观测结果}
    I -->|通过| J[Verified evidence]
    I -->|失败或缺失| K[repair_signal<br/>failure kind + attempt ledger]
    K --> L{恢复决策}
    L -->|重试| B
    L -->|等待| M[Checkpoint 与 resume]
    L -->|撤销| N[恢复精确补偿快照]
    N --> B
    L -->|终止| O[结构化残余风险]
    J --> P[Coding events + 有依据的最终报告]
    M --> P
    O --> P
```

可写的持久化 subagent 在任务专属 Git worktree 中工作；只读 child 返回 findings。
只有父任务检查路径所有权、过期、重叠和验证证据后，才能把 child patch 接纳进主
workspace。

```mermaid
flowchart TD
    A[Planner subagent capability] --> B[可信 role + 有界 scope]
    B --> C[持久化 child graph 与依赖]
    C --> D{Child role}
    D -->|explorer| E[只读 child<br/>findings + evidence refs]
    D -->|writer 或 tester| F[任务专属隔离 worktree]
    F --> G[编辑并验证]
    G --> H[保存 patch + precondition hashes + evidence]
    E --> I[父任务聚合]
    H --> I
    I --> J{父任务接纳审查}
    J -->|过期 / 重叠 / dirty / 失败| K[机器拒绝或修复]
    J -->|可接纳| L[父任务精确批准并应用]
    L --> M[父任务 diff + verification]
    K --> N[Subagent graph events]
    M --> N
```

教学模式是持久化任务事件和 provider 事件的投影。选择某条用户或助手消息后，
UI 根据对应 `task_id` 展示编号 LLM 调用、原始请求/响应字段、runtime stage、
代码入口、策略决策、checkpoint、工具和 child-task 事件。

```mermaid
flowchart LR
    A[一次对话轮次] --> B[保存 task_id 与消息 id]
    B --> C[Task event archive]
    B --> D[Provider call records<br/>LLM#1..N]
    C --> E[选中轮次的教学视图]
    D --> E
    E --> F[流程时间线]
    E --> G[模型原始请求与响应]
    E --> H[策略、预算、恢复、工具、<br/>编码与 subagent 证据]
```

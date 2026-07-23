# 安全与执行

上一页：[Agent Loop 与规划](01-agent-loop.zh-CN.md) |
[架构索引](README.md) |
下一页：[任务状态与上下文](03-task-state-context.zh-CN.md)

认证结果选择由服务端持有的执行策略。Registry 元数据、验证、授权、命令策略和
平台沙箱是相互独立的控制层；YOLO 只改变授权与沙箱策略，不会绕过其他边界。

```mermaid
flowchart TD
    A[已认证任务] --> B[服务端执行策略]
    C[Planner 机器动作] --> D[CapabilityResolver]
    D --> E[Registry 策略<br/>risk + effect + idempotency + schema]
    B --> F[PlanVerifier]
    E --> F
    F --> G{PermissionDecision}
    G -->|需要确认| H[后端授权请求<br/>精确 actor + session + resource + expiry]
    H -->|单次或作用域批准| I[后端签名授权]
    G -->|拒绝| J[结构化 blocker]
    G -->|允许| K[Pre-tool hook 与 adapter preflight]
    I --> K
    K --> L{执行后端}
    L -->|Linux 进程| M[Bubblewrap adapter]
    L -->|macOS 进程| N[Seatbelt adapter]
    L -->|MCP| O[Server allowlist + tool schema]
    M --> P[工具或技能执行]
    N --> P
    O --> P
    P --> Q[Observation + policy evidence]
    Q --> R[Post-tool hook + journal]
```

```mermaid
flowchart LR
    A[run_cmd 请求] --> B[不经过 shell 解析 argv]
    B --> C[命令策略<br/>allowlist + 禁止形式 + 工作目录]
    C -->|阻断| D[机器错误码]
    C -->|允许| E[平台适配器]
    E --> F[超时 + 取消 + 有界输出]
    F --> G[结构化退出状态与产物]
```

Linux 专用命令不得在 macOS 隐式执行。沙箱后端不可用时应 fail closed，并返回
结构化 unsupported 结果，不能静默退化为无沙箱执行。

# 安全与执行

<!-- ai-learning-navigation:start -->
上一页：[Agent Loop 与规划](01-agent-loop.zh-CN.md) |
[架构索引](README.md) |
下一页：[任务状态与上下文](03-task-state-context.zh-CN.md)

<!-- ai-learning-navigation:end -->

认证完成后，后端会签发由服务端持有的执行策略。Registry 元数据、验证、授权、命令策略和平台沙箱仍是相互独立的控制层。YOLO 请求 `approval_policy=never` 与 `sandbox_mode=danger_full`，但不会绕过 registry 策略、schema、取消、脱敏或审计证据。

```mermaid
flowchart TD
    A[已认证任务] --> B[服务端执行策略]
    C[Planner 机器动作] --> D[CapabilityResolver]
    D --> E[Registry 策略<br/>risk + effect + idempotency + schema]
    B --> F[PlanVerifier]
    E --> F
    F --> G{PermissionDecision}
    G -->|需要确认| H[后端授权请求<br/>精确 actor + session + resource]
    H --> I{封闭决策}
    I -->|approve_once| IA[任务绑定的单次批准]
    I -->|always_for_scope| IB[签名作用域授权<br/>capability + effect + 精确资源 + expiry]
    I -->|deny| J
    G -->|拒绝| J[结构化 blocker]
    G -->|允许| K[Pre-tool hook 与 adapter preflight]
    IA --> K
    IB --> K
    K --> L{执行边界}
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
    A[run_cmd<br/>command + working directory] --> B[合同 preflight + 命令策略]
    B --> C{策略决策}
    C -->|阻断| D[机器错误码]
    C -->|允许| E[平台沙箱封装<br/>bash -o pipefail -lc]
    E --> F[总超时/空闲超时 + 取消 + 有界输出]
    F --> G[结构化退出状态与产物]
```

`run_cmd` 有意支持 shell 语法，但必须先通过合同、权限和命令策略检查。Linux 专用命令不得在 macOS 隐式执行；沙箱后端不可用时应 fail closed，并返回结构化 unsupported 结果，不能静默退化为无沙箱执行。

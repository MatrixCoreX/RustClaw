# Agent Loop 与规划

[架构索引](README.md) | 下一页：[安全与执行](02-security-execution.zh-CN.md)

普通自然语言任务统一进入由 planner 掌握语义决策权的循环。前门只负责物化输入并
构造机器边界信封，不提前判断请求应该直答、澄清还是执行。

```mermaid
flowchart TD
    A[渠道 / UI / API] --> B[POST /v1/tasks]
    B --> C[持久化任务并返回 task_id]
    C --> D[Worker 认领与恢复检查]
    D --> E{任务类型}
    E -->|ask| F[物化文本、语音与附件]
    F --> G[TurnBoundaryEnvelope<br/>身份 + 显式事实 + 策略预算]
    G --> H[上下文包<br/>记忆 + 目标 + 日志 + 产物]
    H --> I[Planner LLM<br/>第一次语义决策]
    I --> J{机器动作}
    J -->|call_capability| K[CapabilityResolver]
    K --> L[PlanVerifier<br/>Schema + effect + permission]
    L --> M[工具 / 技能适配器]
    M --> N[CapabilityResultEnvelope<br/>证据 + 产物 + continuation]
    N --> O{BudgetDecision}
    O -->|继续或修复| I
    O -->|checkpoint 或等待| P[持久化 checkpoint<br/>释放 worker claim]
    O -->|完成| Q[模型生成有依据的回复]
    J -->|respond| Q
    E -->|run_skill| R[显式技能派发<br/>不做语义选择]
    R --> N
    Q --> S[输出合同守卫]
    S --> T[保存结果 + 交付 + journal]
```

优先使用 `call_capability`，让 planner 选择稳定能力，再由 resolver 映射到当前
tool 或 skill。`PlanVerifier` 只校验机器合同与策略，不承担第二层语义路由。
可恢复错误通过结构化 `RepairEnvelope` 作为 observation 返回同一循环。

`kind=run_skill` 是明确分开的 API 路径。调用方已经给出技能与参数，因此它绕过
planner 选择，但继续使用任务持久化、鉴权、生命周期和共享技能协议。

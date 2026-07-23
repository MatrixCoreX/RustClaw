# Agent Loop And Planning

[Architecture index](README.md) | Next: [Security and execution](02-security-execution.md)

Ordinary natural-language tasks enter one planner-owned loop. The front door
only materializes inputs and builds a machine-owned boundary envelope. It does
not decide whether the request should answer, clarify, or execute.

```mermaid
flowchart TD
    A[Channel / UI / API] --> B[POST /v1/tasks]
    B --> C[Persist task and return task_id]
    C --> D[Worker claim and recovery tick]
    D --> E{Task kind}
    E -->|ask| F[Materialize text, audio, and attachments]
    F --> G[TurnBoundaryEnvelope<br/>identity + explicit facts + policy budgets]
    G --> H[Context bundle<br/>memory + goal + journal + artifacts]
    H --> I[Planner LLM<br/>first semantic decision]
    I --> J{Machine action}
    J -->|call_capability| K[CapabilityResolver]
    K --> L[PlanVerifier<br/>schema + effect + permission]
    L --> M[Tool / skill adapter]
    M --> N[CapabilityResultEnvelope<br/>evidence + artifacts + continuation]
    N --> O{BudgetDecision}
    O -->|continue or repair| I
    O -->|checkpoint or wait| P[Persist checkpoint<br/>release worker claim]
    O -->|finish| Q[Model-authored grounded response]
    J -->|respond| Q
    E -->|run_skill| R[Explicit skill dispatch<br/>no semantic selection]
    R --> N
    Q --> S[Output contract guard]
    S --> T[Persist result + deliver + journal]
```

`call_capability` is preferred because the planner chooses a stable capability,
while the resolver maps it to the current tool or skill implementation.
`PlanVerifier` validates machine contracts and policy; it is not another
semantic router. Recoverable errors return to the same loop as structured
`RepairEnvelope` observations.

`kind=run_skill` is intentionally separate. The caller supplies the exact skill
and arguments, so this path bypasses planner selection while retaining task
persistence, authentication, lifecycle, and the shared skill protocol.

# Security And Execution

Previous: [Agent loop and planning](01-agent-loop.md) |
[Architecture index](README.md) |
Next: [Task state and context](03-task-state-context.md)

Authentication selects a server-owned execution policy. Registry metadata,
verification, approvals, command policy, and the platform sandbox remain
independent controls; YOLO changes approval and sandbox policy but does not
bypass the other boundaries.

```mermaid
flowchart TD
    A[Authenticated task] --> B[Server-owned execution policy]
    C[Planner machine action] --> D[CapabilityResolver]
    D --> E[Registry policy<br/>risk + effect + idempotency + schema]
    B --> F[PlanVerifier]
    E --> F
    F --> G{PermissionDecision}
    G -->|needs confirmation| H[Backend approval request<br/>exact actor + session + resource + expiry]
    H -->|approve once / scope| I[Signed backend grant]
    G -->|denied| J[Structured blocker]
    G -->|allowed| K[Pre-tool hook and adapter preflight]
    I --> K
    K --> L{Execution backend}
    L -->|Linux process| M[Bubblewrap adapter]
    L -->|macOS process| N[Seatbelt adapter]
    L -->|MCP| O[Server allowlist + tool schema]
    M --> P[Tool or skill execution]
    N --> P
    O --> P
    P --> Q[Observation + policy evidence]
    Q --> R[Post-tool hook + journal]
```

```mermaid
flowchart LR
    A[run_cmd request] --> B[Parse argv without a shell]
    B --> C[Command policy<br/>allowlist + denied forms + working directory]
    C -->|blocked| D[Machine error code]
    C -->|allowed| E[Platform adapter]
    E --> F[Timeout + cancellation + bounded output]
    F --> G[Structured exit status and artifacts]
```

Linux-only commands must not run implicitly on macOS. Missing sandbox support
fails closed with a structured unsupported result rather than silently falling
back to unrestricted execution.

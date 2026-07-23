# Task State And Context

Previous: [Security and execution](02-security-execution.md) |
[Architecture index](README.md) |
Next: [Coding and observability](04-coding-observability.md)

Foreground request timeout does not terminate a persisted task. The worker uses
leases and heartbeats, while resumable work is represented by checkpoints and
machine lifecycle fields.

```mermaid
flowchart TD
    A[POST /v1/tasks] --> B[(queued task)]
    B --> C[Return task_id]
    B --> D[Worker claim<br/>lease_owner + claim_attempt]
    D --> E[Agent loop or explicit skill]
    E --> F{Budget / provider / async state}
    F -->|continue| E
    F -->|waiting or background| G[Persist TaskBudgetSlice + checkpoint]
    G --> H[Release exact worker claim]
    H --> I{Resume due?}
    I -->|no| J[Caller polls same task_id]
    I -->|yes| K[Recovery claims new generation]
    K --> L[Restore observations, artifacts,<br/>side effects, and counters]
    L --> E
    F -->|terminal| M[Persist final result]
    M --> J
```

Context is assembled from explicit sources with provenance and a deterministic
budget. Memory and knowledge retrieval supply candidates; they do not select a
semantic route.

```mermaid
flowchart TD
    A[Current task and session] --> B[Context builder]
    C[Conversation] --> B
    D[Memory and knowledge index] --> E[Retrieval and use policy]
    F[Goal, journal, artifacts,<br/>coding evidence] --> B
    E --> B
    B --> G[Provenance records<br/>source_ref + reason + scope]
    G --> H[ContextBudgetReport]
    H --> I{Fits budget?}
    I -->|yes| J[Included refs]
    I -->|no| K[Excluded refs + deterministic compaction]
    J --> L[Planner context]
    K --> L
    L --> M[context_budget / context_compaction / memory_trace events]
```

Memory writes happen after the visible answer and use a structured intent
schema. Users can inspect, expire, or delete stored preferences and facts.

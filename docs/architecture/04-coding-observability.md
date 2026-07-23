# Coding and Observability

<!-- ai-learning-navigation:start -->
Previous: [Task state and context](03-task-state-context.md) |
[Architecture index](README.md) |
Next: [Skills, media, and models](05-skills-media-models.md)

<!-- ai-learning-navigation:end -->

Coding changes use explicit path ownership, patch preconditions, compensation
snapshots, and observed verification. A failed check becomes a structured loop
observation, not a hardcoded user reply.

```mermaid
flowchart TD
    A[Coding request or goal] --> B[Inspect workspace and evidence]
    B --> C[Planner change contract]
    C --> D[Patch preview<br/>paths + precondition hashes]
    D --> E[Verifier + exact mutation approval]
    E --> F[Apply patch once<br/>with compensation snapshot]
    F --> G[Patch checkpoint + bounded diff artifact]
    G --> H[Run verification contract]
    H --> I{Observed result}
    I -->|pass| J[Verified evidence]
    I -->|fail or missing| K[repair_signal<br/>failure kind + attempt ledger]
    K --> L{Recovery decision}
    L -->|retry| B
    L -->|wait| M[Checkpoint and resume]
    L -->|revert| N[Restore exact compensation snapshot]
    N --> B
    L -->|terminal| O[Structured residual risk]
    J --> P[Coding events + final grounded report]
    M --> P
    O --> P
```

Persistent writer/tester subagents operate in task-scoped Git worktrees.
Read-only children return findings. Only the parent task can admit a child
patch into the main workspace after checking ownership, staleness, overlap, and
verification evidence.

```mermaid
flowchart TD
    A[Planner subagent capability] --> B[Trusted role + bounded scope]
    B --> C[Persist child graph and dependencies]
    C --> D{Child role}
    D -->|explorer| E[Read-only child<br/>findings + evidence refs]
    D -->|writer or tester| F[Task-scoped isolated worktree]
    F --> G[Edit and verify]
    G --> H[Persist patch + precondition hashes + evidence]
    E --> I[Parent aggregation]
    H --> I
    I --> J{Parent admission review}
    J -->|stale / overlap / dirty / failed| K[Machine rejection or repair]
    J -->|admissible| L[Parent review + policy-approved apply]
    L --> M[Parent diff + verification]
    K --> N[Subagent graph events]
    M --> N
```

Teaching mode projects persisted task and provider events. Selecting either
side of a conversation turn resolves the corresponding `task_id`, then shows
numbered LLM calls, raw request/response fields, runtime stages, code entry
points, policy decisions, checkpoints, tools, and child-task events.

```mermaid
flowchart LR
    A[Conversation turn] --> B[Persist task_id and message ids]
    B --> C[Task event archive]
    B --> D[Provider call records<br/>LLM#1..N]
    C --> E[Selected-turn teaching view]
    D --> E
    E --> F[Process timeline]
    E --> G[Raw model request and response]
    E --> H[Policy, budget, resume, tool,<br/>coding, and subagent evidence]
```

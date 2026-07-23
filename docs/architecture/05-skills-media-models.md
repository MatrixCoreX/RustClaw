# Skills, Media, And Models

Previous: [Coding and observability](04-coding-observability.md) |
[Architecture index](README.md) |
Next: [Release validation](06-release-validation.md)

The registry is the machine source for skill availability, capabilities,
effects, risk, schema, install mode, and runner metadata. Natural-language
phrases do not belong in aliases or runtime dispatch branches.

```mermaid
flowchart TD
    A{Task source} -->|ask| B[Planner call_capability]
    A -->|run_skill| C[Explicit skill_name]
    B --> D[CapabilityResolver]
    C --> E[Canonical machine-token alias]
    D --> E
    E --> F[Skills registry<br/>enabled + kind + runner + policy]
    F --> G{Implementation}
    G -->|builtin| H[In-process adapter]
    G -->|runner| I[skill-runner subprocess]
    G -->|external| J[External adapter]
    I --> K[Skill binary<br/>one-line JSON stdin/stdout]
    H --> L[Structured skill response]
    J --> L
    K --> L
    L --> M[CapabilityResultEnvelope<br/>status codes + evidence + artifacts]
```

Long-tail media capabilities use start, poll, and cancel contracts. The
foreground task can return a checkpoint while provider work continues.

```mermaid
flowchart TD
    A[Image / audio / video / music capability] --> B[Registry async contract]
    B --> C[Verifier + provider preflight]
    C --> D[Start provider job]
    D --> E{Provider result}
    E -->|complete| F[Artifact refs + observation]
    E -->|pending| G[pending_async_job<br/>job_id + poll_ref]
    G --> H[Checkpoint<br/>next_check_after + can_poll + can_cancel]
    H --> I[Worker recovery or explicit poll]
    I --> J[Poll adapter]
    J -->|pending| G
    J -->|complete| F
    J -->|failed or unavailable| K[Structured wait / repair / terminal state]
    H --> L[Cancel capability]
    L --> M[Cancel adapter + terminal projection]
```

Model capabilities are projected through a catalog rather than inferred from
model-name phrases. Text planning, image/video understanding, generation,
streaming, tool calling, context size, credentials, async support, and dry-run
support are explicit fields used by UI, CLI, and runtime readiness checks.

```mermaid
flowchart LR
    A[Provider configuration] --> D[ModelCatalog builder]
    B[Media configuration] --> D
    C[Vendor capability patches] --> D
    D --> E[Catalog entries<br/>provider + model + modality flags]
    E --> F[Runtime readiness decision]
    E --> G[GET /v1/models/catalog]
    E --> H[clawcli models catalog/readiness]
    G --> I[UI model configuration]
    F --> J[Planner/provider call trace]
```

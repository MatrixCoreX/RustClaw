# Skills, Media, and Models

<!-- ai-learning-navigation:start -->
Previous: [Coding and observability](04-coding-observability.md) |
[Architecture index](README.md) |
Next: [Release validation](06-release-validation.md)

<!-- ai-learning-navigation:end -->

The registry is the machine source for skill availability, capabilities,
effects, risk, schema, install mode, and runner metadata. Natural-language
phrases do not belong in aliases or runtime dispatch branches.

```mermaid
flowchart TD
    A{Task source} -->|ask| B[Planner call_capability]
    A -->|run_skill| C[Explicit skill_name]
    B --> D[CapabilityResolver]
    C --> E[Canonical machine-token lookup]
    D --> F
    E --> F[Skills registry<br/>enabled + kind + runner + policy]
    F --> G{Implementation}
    G -->|builtin| H[In-process adapter]
    G -->|runner| I[skill-runner subprocess]
    G -->|external| J[External adapter]
    I --> K[Skill binary<br/>one-line JSON stdin/stdout]
    H --> L[Structured skill response]
    J --> L
    K --> L
    L --> M{Result consumer}
    M -->|agent loop| N[CapabilityResultEnvelope<br/>evidence + artifacts + continuation]
    M -->|direct run_skill| O[Persist direct task result]
```

Fixed/core skills are part of the normal build. Bundled optional skills live
under `optional_skills/` and are built or installed on demand; imported external
skills must pass validation and registration before they become available.

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
model-name phrases. The catalog exposes provider/model identity, API style,
configured model choices, input/output modalities, context window, timeout,
credential state, media understanding/generation flags, active text-provider
state, and async/dry-run metadata. UI, CLI, and runtime readiness checks consume
those fields directly.

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

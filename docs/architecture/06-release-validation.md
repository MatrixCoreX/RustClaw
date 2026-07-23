# Release Validation

Previous: [Skills, media, and models](05-skills-media-models.md) |
[Architecture index](README.md)

Release validation combines deterministic architecture contracts, focused
component tests, UI checks, and compact natural-language acceptance. Each gate
writes machine-readable evidence so a passing summary cannot hide a skipped or
malformed nested check.

```mermaid
flowchart TD
    A[Source change] --> B{Affected boundary}
    B --> C[Focused Rust / UI / script tests]
    B --> D[Architecture contract self-tests]
    B --> E[Registry, prompt, policy,<br/>multilingual, and long-file checks]
    C --> F[Agent parity gate]
    D --> F
    E --> F
    F --> G[Artifact contract validation<br/>content + path refs + nested summaries]
    G --> H[Compact NL acceptance<br/>capability and failure-class coverage]
    H --> I{Release evidence complete?}
    I -->|no| J[Structured finding<br/>fix and rerun affected scope]
    I -->|yes| K[Release candidate]
```

Important contract families include:

- planner/runtime boundaries, removed pre-route compatibility, and loop-only repair;
- policy decisions, approvals, registry effects, idempotency, and side-effect reconciliation;
- task lifecycle, checkpoint/resume, event archive/replay, context, coding, and subagents;
- generated skill prompts, registry parity, aliases, async media contracts, and model readiness;
- no natural-language hard matching, no fixed multilingual runtime replies, secret scanning,
  cross-platform boundaries, and long-file limits;
- CLI exec/replay/session/goal/TUI/LLM trace artifacts and UI lint/build/tests.

Live provider tests are acceptance evidence, not an excuse to encode a failed
sentence as a runtime branch. Failures must be repaired at the capability
contract, registry metadata, prompt, verifier, adapter, or provider boundary.

# Memory and context architecture ADR

Status: accepted and implemented through WP6; WP7 closed by evaluation gate

Date: 2026-08-05

Plan: `plan/queued/memory_context_codex_claude_gap_closure_plan_20260804.md`

## Decision summary

The runtime will keep conversation transcripts, working context, and durable memory as three
different data classes. Credentials will no longer be durable-memory ownership identifiers.
Memory settings will become revisioned runtime data, durable generation will use a recoverable job
queue, and retrieval indexes will remain rebuildable projections of canonical rows. Memory is
always data-only context and never an authority, permission, route, success, or retry signal.

This ADR froze the direction before schema or provider changes. The implementation now covers WP1
through WP6 with versioned migrations and contract tests. WP7 remains intentionally absent under
the conditional decision recorded below.

## Why this decision exists

The current runtime already has useful memory facts, retrieval indexes, conversation persistence,
compaction records, provenance, rewind, and user-facing inspection. The WP0 inventory also proves
several structural gaps:

- a credential-shaped `user_key` still acts as ownership and lineage;
- the UI writes a tracked release TOML file and requires restart for a runtime choice;
- project is a namespace in important paths, not an enforced scope;
- finalization starts detached memory extraction work;
- vectors are JSON local-hash values and only rerank recent/FTS candidates;
- canonical memory and long-term summaries can be truncated by prompt-oriented limits;
- row-count cleanup is global and can evict another principal's rows;
- compaction uses a fixed 75-percent trigger.

Treating any one of these as an isolated UI or prompt problem would preserve the unsafe coupling.
The implementation therefore proceeds in dependency order: baseline, identity/scope/settings,
durable lifecycle, user control, compaction, semantic retrieval, then active memory capability.

## State boundaries

### Conversation transcript

Transcript events remain the recoverable source for history, resume, rewind, branching, and audit.
Disabling durable-memory use or generation must not delete or hide an undeleted transcript.

### Working context

Working context is the bounded view used for the current turn: current authoritative instructions,
goal/plan state, recent events, tool evidence, and compaction projections. It can be rebuilt from
authoritative sources and event records. A compaction summary cannot become the only copy of a goal,
permission state, completed side effect, artifact, or evidence reference.

### Durable memory

Durable memory contains reusable facts, preferences, and validated experience. Each item needs
eligibility, scope, provenance, lifecycle status, evidence references, and user control. Recalled
content is marked `data_only=true` and `instruction_authority=none` at projection boundaries.

## Decisions

### D1: stable principal ownership

Use an opaque, stable `principal_id` for ownership. Authentication keys and channel credentials are
revocable bindings to that principal. Rotation creates and verifies a new binding, then revokes the
old binding without moving conversations, memory, or settings.

Migration must cover task/conversation ownership, channel bindings, web login ownership, memory
source references, settings, and audit snapshots together. A memory-only migration is rejected.
Legacy key-derived ownership remains a time-bounded compatibility reader, not a new writer or
lineage key.

Principal merge/split is explicit, audited, and guarded by writer freeze plus revision checks. The
runtime must never infer an account merge from similar content or channel metadata.

### D2: closed and enforced scope

Durable scope is a closed enum:

- `conversation`: visible only in the resolved conversation;
- `principal`: visible to that principal under policy;
- `project`: visible only in the resolved project and principal boundary.

Every canonical row stores `scope_kind + scope_ref`. Namespace remains classification and cannot
grant visibility. A single typed `MemoryScopeResolver` must supply SQL filters, vector filters, UI
counts, and capability access checks. Callers may not independently combine `user_key`, `user_id`,
or `chat_id`.

Project identity uses a data-root UUID. Git worktrees sharing a common directory can resolve to one
project; separate clones remain separate unless explicitly linked. Non-Git paths receive a mapped
UUID. Historical `project_facts` rows are not silently upgraded to project scope.

Group-channel content defaults to conversation scope. Messages authored by other members are never
promoted to one person's principal memory merely because they appeared in the same transcript.

### D3: revisioned settings precedence

Effective settings resolve from low to high precedence:

1. tracked release defaults;
2. data-root admin defaults;
3. principal defaults;
4. conversation overrides;
5. validated one-turn options.

Managed deny and privacy revocation are monotonic restrictions and cannot be overridden by a lower
layer. The effective result is pinned at the turn boundary with a revision and policy digest, then
rechecked before prompt outbound, remote embedding outbound, and durable commit.

Runtime changes are stored in the runtime data root or main runtime database, take effect without
restart, and use revision/ETag compare-and-swap. They never modify `configs/memory.toml`. Use-existing
memory and generate-future memory are independent switches. Remote extraction, consolidation, and
embedding consent are purpose- and provider-specific settings rather than consequences of those two
switches.

### D4: structured source eligibility

Every event receives a structured `MemoryGenerationEligibility` decision. At minimum it records:

- author class: user, assistant, tool, skill, subagent, system;
- source class: local, attachment/OCR/STT, web, MCP, private connector, scheduled, group channel;
- sensitivity and secret-scan result;
- available evidence references;
- allowed scope and policy digest;
- stable allow/skip reason codes.

External context uses `exclude`, `evidence_only`, or `allow`; the safe default is not to retain raw
external content. Assistant claims do not become high-trust facts without user confirmation or
verifiable evidence. An explicit user correction has higher conflict priority than model inference.

Eligibility affects durable-memory generation only. It cannot change request routing, capability
permission, retries, or task success.

### D5: durable background lifecycle

Transcript/source commit and generation-job enqueue share one transaction or transactional-outbox
boundary. Finalization does not rely on detached process-local tasks for completion.

Jobs pin source event range, principal and scope, settings revision, policy digest, model selection,
and idempotency key. Workers use leases, heartbeats, checkpoints, retry-after/circuit state,
cancellation, and compare-and-swap commit. Claim and commit recheck deletion and settings revocation.

The scheduler is fair across principals. Idle, minimum useful content, quota, provider capacity, and
source eligibility decide when a job may run. They do not impose a fixed wall-clock lifetime on the
whole operation. Individual network connect/idle/response deadlines and explicit cancellation remain.

### D6: canonical rows and retrieval projections

Canonical durable rows retain complete schema-valid facts, preferences, lineage, and evidence
references. Prompt budgets do not truncate canonical storage. Oversized model output is rejected,
split into valid items, or represented by a bounded fact plus source reference.

FTS, searchable text, compact indexes, vectors, ANN snapshots, query cache, and UI excerpts are
rebuildable projections. Removing or rebuilding a projection cannot delete its canonical row.

Embedding becomes a typed asynchronous batch adapter outside SQLite write transactions. Profiles
pin provider kind, credential reference, model, dimensions, normalization, projection version,
consent digest, and endpoint policy. Remote outbound is impossible without explicit consent and a
host-approved profile. Credentials and sensitive raw query text never enter logs, journals, or UI
responses.

Semantic retrieval must generate nearest-neighbor candidates across the whole eligible scope before
fusion with lexical, exact-identifier, trust, freshness, and recency signals. Merely reranking recent
or FTS candidates is not semantic retrieval. Vectors migrate away from long-lived JSON storage to a
versioned binary or separately evaluated backend with blue/green rebuild and rollback.

Knowledge-base skill data stays in its own `SkillStorageResolver` storage. The host may coordinate
results but must not move ordinary skill data into the runtime database.

### D7: deletion and retention

Delete first tombstones the canonical revision and increments its revision in the same transaction
that removes it from active recall. Lineage identifies derived facts, summaries, FTS rows, every
embedding profile, ANN tombstones, caches, evidence links, and queued/running jobs. Old jobs cannot
write after a revision mismatch.

Explicit durable items are not silently evicted by a global LRU. Retention and quota are separate for
transcripts, raw candidates, canonical durable items, summaries, vectors, and caches, and are fairly
accounted per principal/scope. Under storage pressure, rebuildable projections and backfill are
reclaimed or paused before canonical content. A rejected canonical write returns a structured status;
it never reports success and discards data.

Local deletion does not claim to erase data previously sent to a remote provider. UI disclosure must
distinguish local cleanup from the provider's retention terms.

### D8: adaptive context policy and focused compaction

Replace the fixed percentage with a typed `ContextWindowPolicy` based on provider/model window,
carried prefix, reserved output, tool/schema reserve, measured estimator error, and safety margin.
The trigger records its basis and version.

Explicit compaction may carry an optional one-shot focus supplied by the authenticated user. Focus is
bounded, treated as untrusted user data, not logged verbatim, and never persisted as future
instruction or durable memory.

Compaction operates on a fixed event snapshot under conversation lease and generation CAS. New tail
events stay unmodified. Rewind/branch invalidates records beyond the new head. Provider
context-length errors may cause one safe snapshot-and-compact retry before any model call whose side
effects could be replayed.

After compaction, current authoritative instructions, registry, policy, permission state, goal/plan,
and other machine state are reloaded from their current sources. Model summary failure retains the
existing deterministic fallback and original event stream.

### D9: bounded exact vector backend for the supported scale

The active vector backend is `exact_sqlite_f32le_v1`. It evaluates every eligible row in the
resolved principal/conversation/project scope; candidate limits apply only after similarity ranking.
This preserves semantic correctness and recovery simplicity across Linux/macOS and arm64/x86_64,
without a native extension or a second index truth source.

The configured release ceiling is 30,000 memory rows. A synthetic ceiling test inserts 30,000
24-dimensional rows and proves that the last row remains discoverable without lexical overlap or
pre-ranking truncation. The same test previously ran against the stale Rust fallback ceiling of
50,000 rows in 0.94 seconds and exposed the configuration drift; `MemoryConfig::default()` now
projects the tracked `configs/memory.toml` instead of maintaining a second release-default table.
An ANN backend is therefore not introduced in this release. If same-target p95, memory, or startup
benchmarks later exceed the declared SLO, the backend trait and blue/green snapshot contract are the
switch boundary; correctness must never be traded for a fixed first-N scan.

### D10: multilingual lexical and exact-identifier projection v2

`local-hash-v2` and `memory_searchable_projection_v2` are rebuild boundaries. The tokenizer uses
Unicode lowercase word tokens, complete adjacent bigrams for Han/Japanese/Hangul runs, semantic
symbol tokens, and case-preserving digests for paths, URLs, hashes, and code-like identifiers. It
does not retain the old first-10-CJK or first-8-FTS-keyword ceilings. Canonical text remains
unchanged; only rebuildable lexical/vector projections use the normalized tokens.

### D11: no incremental free-text session checkpoint

WP7's entry condition did not trigger. The repeated-compaction fixture retains 100 percent of the
goal, constraint, open-work, completed-side-effect, artifact/evidence, and exact-identifier machine
references across three compactions. Conversation leases and generation CAS preserve an uncovered
tail, while rewind invalidates only records past the new event head. Current instructions, goal/plan,
registry, policy, permissions, and capability state are reloaded from their authoritative stores.

Adding another free-text checkpoint would duplicate event stream, task-plan snapshot, and
compaction-record state while increasing drift, retention, and deletion surfaces. The runtime will
therefore continue to use those existing append-only/versioned sources. Removing all compaction
projections still leaves transcript/event recovery intact. WP7 may be reconsidered only if a future
fixed fixture falls below the reference-retention threshold and the failure cannot be repaired in
the existing typed sources.

## Baseline and acceptance policy

The synthetic fixture is `scripts/fixtures/memory_context/wp0_baseline_v1.json`. It separates:

- `baseline_non_regression`: what the current implementation must not regress while staged work is
  incomplete;
- `final_acceptance`: the target required before the related work packages can be declared complete.

Wall-clock latency, RSS, and database bytes are recorded but compared only on the same target/build
profile. Correctness and isolation metrics are deterministic gates. The initial measured report is
`scripts/baselines/memory_context_wp0_baseline.json`.

Current measurements intentionally expose rather than hide two deficits: false positives are high,
and project-scope leakage exists. Cross-principal leakage and expired/deleted retrieval residuals must
remain zero throughout the migration.

Run the behavior baseline with:

```bash
cargo test -p clawd wp0_fixture_measures_current_retrieval_and_lifecycle_baseline -- --nocapture
cargo test -p clawd wp0_compaction_fixture_preserves_machine_refs_across_repeated_compaction
```

Run the intentional legacy risk probes only when auditing or replacing those paths:

```bash
cargo test -p clawd wp0_diagnostic_reproduces_legacy_data_loss_risks -- --ignored --nocapture
```

The risk probe is diagnostic, not a forever contract. The test and inventory must be updated to assert
the safe target in the same change that removes each legacy risk.

## Migration and rollback rules

- Ship new schema readers before enabling new writers.
- Use one versioned migration truth source; module `ensure_*` paths become versioned compatibility
  readers/shims with a removal window.
- Backfill records a source watermark, count, digest, and schema/profile version; checkpoints contain
  no secret or raw credential.
- Writer feature gates are independent for principal migration, runtime settings, jobs, retention,
  vector backend, remote provider, compaction, and active memory capability.
- Rollback disables a new writer/profile and uses forward-compatible readers. It does not destructively
  down-migrate canonical rows or return to key-derived ownership as a writer.
- Revocation, cross-principal isolation, no tracked runtime writes, and no cross-principal eviction are
  safety ratchets and are not optional rollback items.

## Alternatives rejected

- A file such as `MEMORY.md` as the host source of truth: it does not fit multi-principal ownership,
  transactional deletion, or runtime audit requirements.
- Adding natural-language phrase checks for “remember” or “forget”: it bypasses the agent capability,
  resolver, verifier, and policy boundary.
- Treating memory as authoritative instructions: it permits prompt injection and stale policy to alter
  execution.
- Enabling a remote embedding model by changing a model-name string: it lacks consent, capability,
  endpoint, credential, and response-validation contracts.
- Keeping JSON vectors and only replacing the query embedder: indexed rows and query vectors would not
  share a reliable profile, and old semantic neighbors would still not become candidates.
- A detached task as durable lifecycle: process exit, deletion races, and retry cannot be proven safe.
- Global max-row cleanup for canonical memory: one principal can silently evict another principal's
  durable state.

## Official behavior references used for the gap audit

- Codex memories and configuration behavior: <https://learn.chatgpt.com/docs/customization/memories>
- Codex CLI commands: <https://learn.chatgpt.com/docs/developer-commands?surface=cli>
- Claude Code memory: <https://code.claude.com/docs/en/memory>
- Claude Code commands and compact focus: <https://code.claude.com/docs/en/commands>
- Claude Code context window: <https://code.claude.com/docs/en/context-window>
- Claude Code sessions: <https://code.claude.com/docs/en/sessions>

These references define public behavior only. This ADR does not claim knowledge of either product's
private ranking, embedding, or checkpoint implementation.

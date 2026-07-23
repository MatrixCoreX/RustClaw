# RustClaw

<img src="./RustClaw.png" width="420" />

Chinese version: `README.zh-CN.md`

RustClaw is a local Rust agent runtime centered on `clawd`. It combines multi-channel chat access, task execution, tool and skill routing, memory, scheduling, browser UI, and `user_key` based identity into one deployable stack.

## Overview

RustClaw is built for daily use and administration from messaging apps or a browser instead of a terminal-first workflow.

Current repository highlights:

- multi-channel entry points: Telegram, WeChat, Feishu, Lark, WhatsApp Cloud, WhatsApp Web, browser UI, and optional `webd`
- task runtime and HTTP API in `clawd`
- shared skill dispatch with in-process builtins, external adapters, and runner subprocesses through `skill-runner`
- built-in, external, and runner-based skills for system, files, web, image, audio, video, music, crypto, KB, and automation tasks
- local browser UI in `UI/`, including a standalone NNI device-signing page
- Raspberry Pi / small-screen desktop app in `pi_app/`
- shared Linux/macOS runtime contracts, with fail-closed Bubblewrap and
  Seatbelt process isolation selected through a machine-configured backend

## Agent Loop Architecture

RustClaw's main natural-language path now uses a Codex / Claude style agent loop by default. Before the first planner call, the front door only materializes text, audio transcripts, and attachments; binds task/session identity; and builds a machine-owned `TurnBoundaryEnvelope` containing explicit API fields, locators, permission/budget profiles, and safety context. It does not call a semantic router or decide whether an ordinary request should respond, clarify, or execute. Every ordinary `ask` enters the agent loop, which owns those semantic decisions. A native model turn selects either `call_capability` for another observation/effect or `respond` for the terminal model-authored answer; compatibility providers can still use the validated fallback plan protocol. Recoverable failures return through `RepairEnvelope` machine fields, attempt history, and checkpoint state instead of user-language phrase matching. The old intent normalizer, contract-repair judge, pre-agent semantic route switch, and route-selected rollback path have been physically removed.

### Request and Agent Loop Flow

```mermaid
flowchart TD
    A[Channel / UI / API request] --> B[POST /v1/tasks]
    B --> BA[Authenticated task execution policy<br/>server-owned machine envelope]
    BA --> C[Persist task + queue]
    C --> D[Return task_id<br/>caller can poll]
    D --> E0[worker_once recovery tick<br/>stale running + due checkpoint]
    E0 --> E1[Claim next queued task]
    E1 --> E{Task kind}
    E -->|run_skill| RS[Direct run_skill path<br/>explicit skill_name only]
    E -->|ask| F[Materialize text/audio/attachments]
    F --> G[TurnBoundaryEnvelope<br/>identity + explicit machine facts + safety/budget profile]
    G --> H[Ask context bundle<br/>memory provenance + recent execution + goal/journal refs]
    H --> J[Agent loop<br/>ordinary semantic authority]
    J --> L{Loop round}
    L --> N[Planner LLM<br/>native call_capability / respond<br/>validated fallback plan when required]
    N --> O
    O --> P[PlanVerifier<br/>permission_decision + risk + effect + contract]
    P --> Q{Verified step}
    Q -->|respond| R[Structured terminal response<br/>free_text or exact list contract]
    Q -->|synthesize_answer| S[Grounded synthesis]
    Q -->|call_tool / call_skill| QP[Pre-tool hooks + adapter preflight<br/>policy_decision + contract args]
    QP --> MG{Non-idempotent mutation?}
    RS --> RSG[No planner / resolver choice<br/>no verifier semantic selection]
    RSG --> MG
    MG -->|yes| MI[Persist intent + deterministic idempotency key<br/>persist attempt before invocation]
    MG -->|no| EX{Execution adapter<br/>sandbox backend diagnostics when process-backed}
    MI --> EX
    EX -->|long-tail async_start| AS[Async media/job adapter<br/>pending_async_job + poll/cancel contract + checkpoint]
    AS --> ASP[Progress machine reply<br/>checkpoint_id + poll_ref + next_check_after + can_poll/can_cancel]
    EX -->|call_tool| T[Tool execution]
    EX -->|call_skill / direct run_skill| U[Shared skill dispatch]
    T --> MR{Mutation receipt state}
    U --> MR
    MR -->|not a mutation| V[Observed result]
    MR -->|receipt returned| MC[Persist receipt + verification + commit<br/>under exact worker claim]
    MR -->|timeout / crash ambiguity| MU[Reconciliation checkpoint<br/>never infer from prose]
    MC --> V
    MU --> ASP
    S --> V
    V --> W[Evidence coverage + answer-shape check]
    W -->|repair / missing evidence| WR[RepairEnvelope<br/>issue codes + attempt ledger]
    WR --> J
    W -->|round observed| BD{BudgetDecision<br/>progress + deadline + policy + hard ceilings}
    BD -->|continue| J
    BD -->|finish| X[Observed-output finalizer]
    BD -->|checkpoint_requeue / waiting / needs_user| BC[Persist TaskBudgetSlice + task_checkpoint<br/>release exact claim]
    BC --> ASP
    BD -->|terminal| X
    R --> Y[User-visible message assembly]
    X --> Y
    Y --> Z[Output-contract guard + task result]
    Z --> AA[Channel delivery]
    Z --> AB[Journal + session update]
    AB --> AD[Task event stream<br/>goal + context + transition + checkpoint + tool/coding/team events]
    ASP --> AA
    ASP --> AB
    ASP --> AD
    AD --> AE[CLI / UI watch + report]
    AD --> AF[Teaching mode timeline<br/>per-turn trace selection + raw JSON details]
    Z -. optional .-> AC[Background memory refresh]
```

- `POST /v1/tasks`: channel daemons, the browser UI, and HTTP callers converge on the same persisted task queue.
- `Authenticated task execution policy`: `clawcli` stays on the configured approval/sandbox policy unless an enabled admin key explicitly requests global `--yolo`. Other communication adapters authenticated with an enabled admin key default to YOLO. The server removes caller-supplied policy envelopes, reissues the machine contract after authentication, and revalidates the current admin key before each use. YOLO means `approval_policy=never` plus `sandbox_mode=danger_full`; it does not bypass registry allow/deny, schemas, path validation, external-publish controls, cancellation, budgets, redaction, or audit evidence.
- `task_id polling`: API/channel request timeouts only affect how long the caller waits. The background task remains queryable through `GET /v1/tasks/{task_id}` unless worker lifecycle logic marks it terminal.
- `worker_once recovery tick`: before claiming new queued work, the worker checks stale running tasks, protected paused checkpoints, due resume work, async poll results, and result projections.
- `Task kind`: `kind=ask` enters the planner-owned natural-language path; `kind=run_skill` bypasses the planner loop, capability selection, and plan verifier, then calls the explicitly requested skill through the same shared skill dispatcher/protocol used by planner skill calls. Both task kinds persist results under the original `task_id`, so callers can still inspect final state through task query APIs.

### Ask and Run Skill Boundary

This boundary is intentionally explicit because `run_skill` is an API-level task kind, not a natural-language routing shortcut.

Quick facts for direct skill tasks:

- `kind=run_skill` does not run the planner / agent loop. The caller already supplied `payload.skill_name` and args.
- `kind=run_skill` still uses the shared skill dispatcher and skill protocol after the explicit skill name is accepted.
- `kind=run_skill` still creates and updates a normal task row, so the final state and result remain queryable by `task_id`.

| Question | `kind=ask` | `kind=run_skill` |
| --- | --- | --- |
| Is there a pre-planner semantic LLM/router? | No. The front door only builds `TurnBoundaryEnvelope` and context refs. | No. The caller already supplied the target skill. |
| Does it enter the planner / agent loop? | Yes for every ordinary natural-language task. | No. It does not ask the planner to choose a skill or action. |
| Does it use `CapabilityResolver` / `PlanVerifier` as semantic selectors? | No. The planner owns ordinary semantic choice; resolver/verifier resolve and validate planned steps before execution. | No. Direct skill tasks bypass semantic selection; the explicit skill call still uses dispatch/protocol validation. |
| Does it use the shared skill dispatcher? | Yes when the planner chooses `call_skill` or a capability resolved to a skill. | Yes. It dispatches `payload.skill_name` through the same builtin / external / runner skill protocol. |
| Is the result queryable by `task_id`? | Yes. | Yes. The direct skill result is saved under the original task row and can be read through `GET /v1/tasks/{task_id}` or `clawcli get`. |

Operationally: use `kind=ask` when the user gave a natural-language request and RustClaw should decide whether to answer, ask, plan, or execute. Use `kind=run_skill` when an API caller already knows the exact skill and args and only wants RustClaw to run that explicit skill under the task queue, auth, lifecycle, and result projection machinery.

- `Planner-owned front door`: materializes text/audio/attachments and builds `TurnBoundaryEnvelope` from task identity, explicit API fields, structured locator facts, and safety/budget profiles. It performs no semantic LLM call and contains no ordinary respond/clarify/execute branch.
- `Agent-loop semantic authority`: every ordinary natural-language task enters the loop. Native turns choose `call_capability` or structured `respond`; validated fallback providers may express the equivalent tool, skill, synthesis, clarification, repair, checkpoint, or stop steps through the fallback plan protocol.
- `CapabilityResolver / PlanVerifier`: resolves `call_capability` into the current tool or skill implementation, then checks visibility, required arguments, allowed action, risk/effect, confirmation, and output contract before execution.
- `permission_decision`: verifier and preflight blockers expose machine fields such as `allowed`, `needs_confirmation`, `denied_by_policy`, `dry_run_required`, `external_provider_blocked`, `risk_level`, `action_effect`, and registry dedup/idempotency metadata. UI, API clients, finalizers, and i18n should render these fields instead of parsing runtime prose.
- `Side-effect outbox`: non-idempotent mutations from both planner-owned execution and explicit `kind=run_skill` persist `intent_recorded` and `attempt_started` before invocation. A deterministic key derived from task plus canonical action fingerprint enters runner context, external HTTP `Idempotency-Key`, or supported local-adapter environment. Receipt, verification, reconciliation, and commit transitions are fenced by the exact worker claim. Receipt-bearing states suppress original-action replay; ambiguous timeout/crash state checkpoints as `mutation_reconciliation` and accepts only a fingerprint-bound structured `applied|not_applied|still_unknown` resume constraint.
- `Async job start`: long-tail tool work can publish a machine reply with `checkpoint_id`, `poll_ref`, `next_check_after`, `can_poll`, and `can_cancel` while the task remains recoverable through checkpoint polling. Media skills expose this shape through registry capabilities such as `image.generate` / `image.poll` / `image.cancel`, `audio.synthesize` / `audio.poll` / `audio.cancel`, `video.generate` / `video.poll` / `video.cancel`, and `music.generate` / `music.poll` / `music.cancel`.
- `Evidence coverage`: tool, skill, and synthesis outputs become loop observations. Missing evidence or recoverable failures go back into the loop with compact attempted-method history.
- `TaskBudgetSlice / BudgetDecision`: interactive work is governed by resumable soft wall-time slices and structured progress, not ordinary `max_rounds` or `max_tool_calls` completion thresholds. After each observed model/tool result, runtime chooses `continue`, `finish`, `checkpoint_requeue`, `waiting`, `needs_user`, or `terminal` from verifier-approved plan facts, evidence/artifact progress, continuation state, policy, cancellation, deadlines, and administrator hard ceilings. Profile timeout classes cap planner provider calls and agent-loop tool/MCP calls before the slice boundary; a timed-out mutation enters reconciliation instead of blind replay. The model can request continuation but cannot raise cost, permission, time, or resource ceilings.
- `RepairEnvelope`: repair is bounded loop recovery. Runtime supplies machine fields such as `repair_source`, `issue_codes`, `missing_evidence`, `permission_decision`, `provider_status`, `attempt_fingerprint`, `side_effect_fingerprint`, `checkpoint_id`, and `next_recovery_kind`; planner/finalizer can use those fields to replan, clarify, wait in background, or fail structurally without parsing localized prose.
- `Observed-output finalizer`: publishes grounded results only after the answer shape and evidence contract are satisfied.
- `Output-contract guard`: normalizes final text, message arrays, file tokens, scalar/strict shapes, and channel delivery consistency before the result is saved.
- `Journal + session update`: task state, observed facts, and active-session anchors are persisted after finalization; background memory work is optional and non-blocking.
- `Task event stream`: journal trace events expose machine-readable progress such as `task_goal`, `context_budget`, `context_compaction`, `budget_decision`, `task_transition`, `checkpoint_created`, `tool_started`, `tool_step`, `tool_finished`, `coding_checkpoint`, `coding_task_contract`, `coding_evidence`, `provider_call`, `agent_hook`, `subagent`, `subagent_graph`, `subagent_node`, `agent_team_started`, `subagent_started`, `subagent_finished`, `subagent_failed`, `agent_team_conflict_detected`, `agent_team_aggregated`, and `task_final`. Provider/context projections retain machine metrics such as `prompt_truncation_count` and `prompt_bytes_before_max`. CLI and UI render these fields directly, including the budget profile/decision, continuation index, cumulative model/tool/token/cost/elapsed counters, soft-slice state, goal and checkpoint fields, hook fields, coding verification fields, and persisted child-graph progress, instead of reading raw logs or localized text. Coding events are immutable snapshots: a resumed task appends a higher projection revision, and consumers select that latest projection while retaining earlier red-test evidence as history.

### Planner, LLM, and Capability Flow

Detailed flow: [Agent loop and planning](docs/architecture/01-agent-loop.md).

- `TurnBoundaryEnvelope`: is built deterministically from authenticated task/session state, attachments, explicit API fields, locators, and policy profiles. It is context for the planner, not a semantic route result.
- `Planner prompt`: is the first semantic LLM call for an ordinary `ask`. Resume and async-poll executors may restore a previously admitted machine checkpoint, but they cannot introduce a new pre-planner semantic decision path.
- `call_capability`: is the preferred planner action because it keeps skill/tool choice behind registry metadata and resolver policy.
- `respond`: is the native terminal action. Ordinary answers carry model-authored `free_text`; strict lists carry an item array and exact count; exact named-field/JSON replies carry unique field names plus validated JSON values in `object` shape. Runtime materializes list/object payloads without parsing multilingual user text or adding fixed prose. A single scalar, identifier, title, token, or path stays `free_text`.
- `Generated INTERFACE prompts`: come from `crates/skills/*/INTERFACE.md`, `optional_skills/*/INTERFACE.md`, `external_skills/*/INTERFACE.md`, and `prompts/layers/generated/skills/*`; new skills should improve these contracts instead of adding `clawd` main-flow branches.
- `Exact machine output`: the planner requests `response_shape=strict` plus a validated `structured_field_selector` such as `command_output`; the runtime projects only that field from `CapabilityResultEnvelope`. Free-form and one-sentence contracts remain model-synthesized.
- `PlanVerifier`: blocks unavailable capabilities, missing required fields, unsafe mutations, and disallowed output/evidence shapes before any executor runs. Denials should carry stable machine fields rather than user-facing fixed reply text.
- `Pre-tool hooks + adapter preflight`: loop execution and bounded recovery retries pass through the same hook, contract-argument, command-policy, and structured error checks before any effectful adapter runs.
- `Task journal event`: executor observations are projected into stable `task_goal`, `context_budget`, `context_compaction`, `tool_started`, `tool_step`, `tool_finished`, optional `coding_checkpoint`, optional `coding_task_contract`, optional `coding_evidence`, and optional team lifecycle events with refs, counts, status tokens, verification tokens, timing, and failure attribution for CLI/UI progress views.
- `agent.subagent` / `agent.subagent_batch` / `agent.subagent_persistent`: planner-authorized child work enters through the same native `call_capability` -> registry resolver -> verifier path as other runtime actions. The first capability runs one read-only inline child, the second runs a bounded read-only batch, and the third schedules explicitly high-risk resumable child work. Trusted role definitions and isolation/permission policy come from registry plus `agent_guard.toml`; planner-supplied policy fields are discarded. The queue claims only ready nodes, serializes overlapping writer ownership, and permits disjoint isolated worktree writers to run concurrently. The parent reviews persisted ownership, conflicts, dirty-parent state, and precondition hashes before admitting or rejecting a patch. Children never publish externally, and only an explicitly admitted local-current-workspace role may write outside an isolated task worktree.
- `Skill dispatcher`: uses the same dispatch layer for direct `run_skill` and planner skill calls. Direct `run_skill` does not ask the planner to choose a skill; it only dispatches the explicit `payload.skill_name`. Builtins run in-process, external skills use adapters, and runner skills launch `skill-runner` plus the concrete binary.
- `Skill process protocol`: runner skills exchange one-line JSON over stdin/stdout and should return stable machine fields in `extra` when runtime needs to make decisions.
- `synthesize_answer`: is scheduled inside the loop when evidence needs natural-language synthesis; it is not a fixed final LLM call after every task.
- `RepairEnvelope`: verifier, executor, permission, provider, and checkpoint recovery paths expose structured repair context to the next loop round; user-visible fallback prose should come from i18n, finalizer, UI, or the model, not runtime templates.
- `Output-contract finalization`: is a thin protocol boundary. It preserves exact validated machine fields and artifact transport, otherwise it publishes the model's evidence-grounded synthesis; it does not select skills or render domain-specific prose.

### Permission Plane and Command Policy

The permission plane is a structured execution boundary, not a second semantic router. Registry metadata from `configs/skills_registry.toml`, bundled evidence policy for non-capability output shapes, and verifier state are projected into `permission_decision` so UI/API/finalizer layers can explain what happened without hardcoded runtime prose. Ordinary registry capability families are selected by planner `call_capability` plus resolver metadata, not by historical route markers or compatibility hints.

- `risk_level`, `requires_confirmation`, `once_per_task`, `idempotent`, and `dedup_scope` come from registry and planner capability metadata where available.
- `action_effect` is derived from structured skill/action args and contract metadata, not from user-language phrase matching.
- `run_cmd` decisions include a nested `command_policy` for machine fields such as `policy_authority`, `literal_command_token`, `command_arg_present`, `unresolved_runtime_template_present`, and command effect flags.
- Explicit user command preservation is represented by `_clawd_literal_command`; otherwise `run_cmd` is treated as planner-structured command args and remains subject to contract and media-artifact blockers.
- Recovery paths such as non-interactive sudo retry are still adapter calls: they must reuse the same contract, hook, policy, and audit machinery as the original planner step.
- Risky local coding or file-mutation capabilities should declare an isolation profile in registry metadata. `local_temp_workspace` is for disposable previews, dry runs, and generated artifacts that can be cleaned through artifact refs; `local_worktree` is for deliberate workspace edits that must be visible through task evidence, changed-file refs, and verification commands. UI and CLI surfaces read `permission_decision.steps[].sandbox`, `workspace_scope`, and `registry_policy` instead of interpreting localized text.
- Confirmation decisions use the closed machine protocol `approve_once|always_for_scope|deny`. `always_for_scope` is exposed only for registry-declared local workspace mutations with an exact capability/effect/resource scope; it excludes `run_cmd`, network access, external publish, credential access, package installation, and privilege escalation. Grants are HMAC-signed, bound to the authenticated actor plus channel/chat session, expire after at most one hour, and are stored, matched, listed, and revoked by `clawd`. CLI/UI state never grants permission by itself.

### Sandbox and Cross-Platform Execution

`[tools].sandbox_backend` is independent from `sandbox_mode`. The default
backend is `auto`: it resolves to Bubblewrap on Linux and macOS Seatbelt
(`/usr/bin/sandbox-exec`) on macOS. Selecting a backend does not grant access;
the verifier still applies capability effect, risk, confirmation, workspace,
network, credential, and privilege policy before an adapter starts.


Detailed flow: [Security and execution](docs/architecture/02-security-execution.md).

| Backend | Host | Selection | Current contract |
| --- | --- | --- | --- |
| Bubblewrap | Linux | `auto` or `bubblewrap` | Filesystem write scope, PID/IPC/UTS namespaces, optional network namespace, parent-bound or durable lifetime. Missing `bwrap` fails closed. |
| Seatbelt | macOS | `auto` or `macos_seatbelt` | Read-all/write-scoped profile, optional network access, process policy, parent-bound or durable lifetime. Missing `sandbox-exec` fails closed. |
| Remote container | Any | explicit `remote_container` | Reserved executor contract only. It returns `sandbox_remote_backend_not_configured` until a remote executor is configured and is never an automatic fallback. |
| Direct | Any | explicit `sandbox_mode = "danger_full"` or authenticated per-task YOLO | No sandbox claim. This is an operator-selected bypass, not an availability fallback. |

`clawcli --yolo <task-producing command>` requests direct execution for that
task and requires a currently enabled admin key. It is intentionally not the
CLI default. Communication adapters other than `clawcli` default to the same
mode when their request is authenticated as admin, so channel admin keys must
be treated as full execution credentials. The backend remains the only policy
authority; request payloads, browser state, planner output, and user wording
cannot grant this mode.

Diagnostics report the requested/resolved backend, platform, availability,
fail-closed state, reason code, and filesystem/network/process/credential/
resource/environment control levels. Service discovery is also platform-owned:
Linux may use systemd/SysV, while macOS uses Homebrew services, launchd, or
process observation. An incompatible explicit manager returns a structured
`unsupported_platform` result without launching a Linux command. Development
and release scripts use `scripts/shell_compat.sh` instead of GNU-only file/date
commands or Bash 4-only collection syntax. See
[`docs/cross_platform_contract.md`](docs/cross_platform_contract.md).

## Natural Language Contract Boundary

RustClaw keeps natural-language understanding on the LLM side and deterministic execution on the runtime side. The planner may read user wording, examples, skill docs, and multilingual prompt guidance, but it must turn that understanding into structured actions before runtime code acts on it. The pre-planner front door may only expose authenticated machine facts through `TurnBoundaryEnvelope`; it cannot infer ordinary intent.

Runtime code should consume stable contracts such as:

- evidence-policy answer-shape fields, for example `final_answer_shape = "summary_with_evidence"` and `final_answer_shape_class = "grounded_summary"`
- planner-owned capability refs, for example `capability_ref = "package.detect_manager"` or `call_capability("package.detect_manager")`
- action names, for example `read_field`, `validate_config`, or `transform_data`
- registry metadata and `planner_capabilities`
- `EvidencePolicyContext` / `OutputContract` fields, target locators, and explicit `field_path` values
- JSON/TOML/YAML field paths, file extensions, structured tool output, exit codes, error kinds, and risk/effect metadata
- `permission_decision` and `command_policy` machine fields

Runtime code should not add per-language phrase tables or `prompt.contains(...)` branches to make a single natural-language case pass. If a new user wording needs better handling, update registry capability metadata, `INTERFACE.md`, generated skill prompts, planner schema, or a necessary vendor prompt patch so the LLM emits the same structured action in any language. Ordinary skills such as weather, web, image, photo, publishing, package manager, Docker, RSS, and market quote must flow through registry capability metadata. Historical route/semantic fields may be rendered only by isolated legacy-log readers; they cannot select a capability, modify a contract, or decide final answer shape. `python3 scripts/check_no_nl_hardmatch.py` is the local guard for this boundary.

## Memory System

RustClaw memory is split into short-term conversation records, structured user preferences, long-term fact cards, and retrieval indexes. The design goal is to make memory useful without letting old assistant output become a hidden instruction for a new task.

### Core Boundaries

Memory is scoped to the authenticated identity first, then to the current conversation. Channel IDs from Telegram, WeChat, Feishu, browser UI, and other adapters are normalized into the same task identity model, so a bound `user_key` can keep memory consistent across channels while still preserving `user_id` / `chat_id` level conversation state. Recent conversation state stores active-task anchors, alias bindings, and follow-up context separately from durable facts; it is allowed to help resolve “that file” or “the previous result”, but it is not treated as a new user instruction.

The memory layer has three hard boundaries:

- current user input always wins over recalled memory
- memory text is background context unless current task/session state explicitly binds it to the turn
- runtime code consumes memory through structured fields, source kinds, scores, safety flags, and use-policy decisions rather than per-language phrase branches

This keeps old assistant output, task logs, and knowledge snippets from silently steering execution. If recalled context conflicts with the current request, the planner prompt and memory-use policy require the current request to win.

### Storage Model

The main persisted memory stores are:

- `memories`: short-term conversation records and task-visible snippets. Rows keep role, memory type, salience, safety state, timestamps, success state, and source metadata.
- `conversation_states`: active per-chat state such as alias bindings, active task anchors, and follow-up state. This is session state, not durable knowledge.
- `user_preferences`: structured user preferences such as response language, response style, response format, and agent display name.
- `memory_facts`: durable fact cards with `fact_key`, `fact_value`, `fact_text`, source refs, confidence, status, expiry, and conflict-group metadata.
- `long_term_memories`: legacy / fallback summary rows used only where the current memory use policy allows summary recall.
- `memory_retrieval_index`: hybrid retrieval index over short-term records, preferences, fact cards, and knowledge snapshots.

`configs/memory.toml` controls budgets, retention, long-term refresh intervals, write filters, preference extraction, retrieval limits, and embedding/index behavior. Defaults are conservative: short acknowledgement messages can be filtered, assistant replies are marked, and LLM-written preferences must pass confidence and runtime validation before they are stored.

### Write Path

After an `ask` task finalizes, RustClaw can persist:

- short-term records in `memories`, scoped by `user_key`, `user_id`, `chat_id`, role, memory type, salience, and safety flag
- user preferences in `user_preferences`, such as `response_language`, `response_style`, `response_format`, and `agent_display_name`
- long-term fact cards in `memory_facts`, with source, confidence, scope, status, conflict group, expiry, and supersede metadata

Preference and fact writes go through a structured memory intent contract. The model is asked to emit `memory_actions` such as `upsert`, `delete`, `expire`, or `noop`; runtime code then validates action enum, kind, scope, confidence, source evidence, TTL, and safety fields before anything is stored. The runtime does not decide durable preference writes by matching a single natural-language phrase.

Long-term summary refresh still exists as a fallback summary path, but durable knowledge is stored as fact cards first. A fact card keeps `fact_key`, `fact_value`, human-readable `fact_text`, `source_ref`, `source_memory_ids_json`, `reason`, `confidence`, `expires_at_ts`, `conflict_group`, and `status`. New active facts in the same conflict group supersede older facts, and expired or deleted facts are removed from retrieval.

Memory writes are intentionally after-answer work. The user-visible response is saved first; then background memory refresh can run when configured. This prevents memory extraction latency from blocking normal replies and makes memory write failures non-fatal to the already completed task.

### Recall and Use Policy

Memory recall is built as structured context and then filtered by the current consumer's memory use policy:

- planner: can use unfinished goals, preferences, relevant facts, and knowledge docs, but excludes fallback long-term summaries, assistant results, similar triggers, and raw recent snippets
- chat: uses stable preferences and facts; bounded recent context is allowed only when current session state makes it relevant
- skill: `_memory` is cropped by the skill registry `memory_policy`; skills without a policy get a safe default scoped profile

The `photo_organize` skill, for example, declares a memory policy that allows preferences, relevant facts, and knowledge docs while excluding long-term summaries, recent events, assistant results, similar triggers, unfinished goals, and raw recent snippets.

Each use-policy decision records what it included and why. Prompt builders receive already-filtered structured context rather than raw database rows. The common policy is:

- new standalone tasks get stable facts and preferences, not old assistant results
- follow-up turns can use recent observations and active aliases only when session state says the user is continuing the same task
- planner prompts can see enough memory to avoid repeating work, but memory remains background and cannot override the current request
- skill `_memory` payloads are cropped per skill registry policy so specialized skills only receive the memory sources they are expected to use

### Retrieval Index

Hybrid recall uses `memory_retrieval_index` plus optional FTS. Each indexed row records `source_kind`, `source_ref`, memory kind, metadata, salience, success state, and embedding metadata:

- `embedding_model`
- `embedding_dims`
- `embedding_version`

The default provider is `local-hash-v1`, which runs offline. Unsupported or unavailable embedding providers fall back to local hash so the runtime keeps working. Retrieval only uses cosine scoring when the stored embedding metadata matches the current provider spec; mismatched rows fall back to lexical, salience, recency, and success-state scoring. Set `reindex_on_startup = true` in `configs/memory.toml`, or start with an empty index, to rebuild the retrieval index from short-term records, preferences, fact cards, and KB snapshots.

Retrieval combines several signals instead of trusting a single score: exact / lexical matches, vector similarity when compatible, salience, recency, source kind, success state, safety filter, and the current memory use policy. This makes the index useful for multilingual recall while keeping execution grounded in planner actions and output/evidence contracts.

### Knowledge Base Design Flow

The `kb` skill is the user-managed document knowledge-base path. It is selected like other ordinary capabilities: `ask` tasks let the agent loop plan `call_capability("kb.*")`, while direct API callers can use `kind=run_skill` with `skill_name=kb`. Runtime code does not special-case knowledge-base wording before the planner; it resolves and verifies registry capability metadata, then dispatches the same skill protocol as other runner skills.


Detailed flow: [Task state and context](docs/architecture/03-task-state-context.md).

Key boundaries:

- `kb.ingest` is a local mutation capability. Registry policy marks it medium risk, once per task, and async-preferred through the local process adapter.
- `kb.search`, `kb.list_namespaces`, and `kb.stats` are observe-mode capabilities. They return structured machine fields such as `namespace`, `hits`, `names`, `document_count`, and `chunk_count`.
- Namespace snapshots under `data/kb/by_user/...` keep the compatibility document index. Ingest also syncs chunks into the unified retrieval index as `kb_doc` / `knowledge_doc`, and startup reindex can rebuild those rows from snapshots.
- KB rows are scoped by `user_key` and workspace files, not by one chat thread. They can be recalled later only through the same memory use-policy filters as other knowledge docs, and current user input remains the authority.

### User Control

The browser console includes a Memory page. It shows counts, preferences, fact cards, and recent records for the current identity. Users can:

- delete a preference, fact, or recent memory item
- mark a fact card as expired
- clear recent records, preferences, facts, or all memory for the current identity
- enable or disable long-term memory through `configs/memory.toml`

The HTTP API behind the page is:

```text
GET    /v1/memory
GET    /v1/memory/recent
GET    /v1/memory/preferences
GET    /v1/memory/facts
DELETE /v1/memory/:id
POST   /v1/memory/:id/expire
POST   /v1/memory/clear
POST   /v1/memory/settings
```

Recent records with safety flags are hidden by default in the UI. Fact-card details such as reason, source, and conflict group are available in a secondary details view instead of being shown as raw JSON first.

### Trace and Troubleshooting

Task journal summaries and traces include `memory_trace`. This records the stage, use policy, recalled source refs, inclusion reason, and character budget without copying raw memory text. It is intended for debugging why a task used memory while reducing the chance of leaking sensitive stored content. The browser teaching-mode trace, `clawcli llm-trace`, and `/v1/debug/tasks/{task_id}` also show a compact `flow_summary` above numbered LLM calls, with stage/module/retry/verifier/finalizer/provider-error machine counts, structured memory/KB policy, `model_catalog_trace`, `model_catalog_trace.readiness`, and `resume_trace` next to raw request/response details. Each browser chat turn keeps a lightweight task/trace index. When teaching mode is selected, clicking either the user's question or the assistant's reply selects that turn and shows the corresponding task id, status, LLM call count, stage count, verifier/finalizer counts, goal/context/team/coding/checkpoint event timeline, model/provider capability decision, selected-model readiness decision, resume/checkpoint decision, and numbered raw LLM request/response details. When teaching mode is not selected, message clicks do not change the teaching trace.

Execution boundaries are exposed as machine fields instead of prose-only notes. Teaching mode, subagent review, `clawcli report`, and replay tooling should consume fields such as `workspace_root`, `current_process_cwd`, `current_workspace_scope`, `write_enabled`, `external_publish_enabled`, `allowed_roles`, `runtime_config.max_parallel_readonly`, `hook_stages`, `hook_decisions`, `handler_id`, `handler_kind`, `trust_status`, `failure_policy`, `permission_decision`, `policy_decision`, `risk_level`, `dry_run`, `checkpoint_id`, `poll_ref`, `next_check_after`, and `provider_blocker`. Subagent batch aggregation also carries `aggregation.execution_mode` so consumers do not need to infer the execution style from finalizer wording.

When debugging memory behavior, check these questions in order:

- Was the request a new task or a follow-up bound to an active session?
- Which stage built the memory context: route, planner, chat, schedule, image, or skill?
- Did `memory_trace` include the expected `source_kind` / `source_ref`?
- Did the use policy exclude recent assistant output or long-term summaries by design?
- Was the index stale because embedding metadata changed or `reindex_on_startup` was false?
- Did a fact conflict group supersede the older fact?
- Was the item hidden because it was expired, deleted, low confidence, or safety-risk flagged?

Useful code and config entry points:

- `configs/memory.toml`
- `crates/clawd/src/memory/intent.rs`
- `crates/clawd/src/memory/apply.rs`
- `crates/clawd/src/memory/facts.rs`
- `crates/clawd/src/memory/use_policy.rs`
- `crates/clawd/src/memory/retrieval.rs`
- `crates/clawd/src/memory/indexing.rs`
- `crates/clawd/src/memory/api.rs`

### Background, Resume, and Memory Flow

Detailed flow: [Task state and context](docs/architecture/03-task-state-context.md).

Important lifecycle details:

- Foreground HTTP/channel waits are short by design. A caller that stops waiting should keep polling the same `task_id`; it should not create a duplicate task or treat the background task as failed.
- `task_lifecycle` is machine-readable. Query APIs expose `state`, `db_status`, `can_poll`, `can_cancel`, `checkpoint_id`, `resume_due`, `resume_wait_seconds`, and heartbeat fields for UI rendering.
- Source of truth: `crates/clawd/src/task_lifecycle.rs` owns lifecycle projection, and `repo::get_task_query_record()` attaches that projection to `GET /v1/tasks/{task_id}`. UI, CLI, and channels should render these structured fields instead of deriving status from `text` or `error_text`.
- `clawcli get`, `clawcli watch`, and `clawcli wait <task_id> --until terminal|completed|background|needs-user` render or wait on lifecycle machine fields; `clawcli cancel-task <task_id>` uses the direct task-id cancellation API, while `clawcli cancel-index` is kept only for active-list index compatibility.
- `clawcli resume-task <task_id>` marks an existing checkpoint due for recovery; `clawcli continue <task_id> [message]` is a shorter structured resume entrypoint; `clawcli pause-task <task_id> --pause-seconds N` delays an existing waiting/background checkpoint. These commands do not restart tasks without checkpoint state.
- `clawcli submit --detach` returns a `task_id` quickly; `clawcli submit --wait` polls until terminal state; `--json` keeps submit/watch output script-friendly.
- `clawcli --yolo submit|exec|code|chat|run-skill ...` requests `approval_policy=never` and `sandbox_mode=danger_full` for newly submitted tasks. The backend accepts it only for a currently enabled admin key. This is a high-risk mode: it removes local approval prompts and process sandbox isolation, while registry, schema, external-publish, cancellation, budget, redaction, and audit controls remain active.
- `clawcli exec` is the CI/script-oriented runner: it submits or resumes an ask task, waits by default, returns stable exit classes/codes, supports `--profile quick|coding|release-gate|long-tail`, can stop on background checkpoints, prints budget/coding/resume evidence as `exec_compact_*` machine lines when present, and can write `summary.json`, `task.json`, `events.jsonl`, `verification.json`, `diff_summary.json`, `llm_summary.json`, `resume.json`, and `index.json` artifacts. `clawcli code` is the concise coding-agent shortcut for `exec --profile coding`. See `docs/clawcli_exec_replay.md`.
- `clawcli goal start/status/pause/resume/edit/clear` manages structured long-task goal contracts with `objective`, `done_conditions`, `verification_commands`, constraints, checkpoint resume fields, and redacted control responses.
- `clawcli active` prints a compact task table by default and supports `--json`; `clawcli events <task_id>` prints filtered task event streams with optional `--jsonl` and machine filters such as `--event-type`, `--checkpoint-id`, `--policy-decision`, `--subagent-id`, and `--async-job-id`.
- `clawcli tui --user-id <id> --chat-id <id>` is a terminal task console over the same task APIs; add `--once` for a single snapshot and `--task-id <task_id>` for selected task details. Selected-task snapshots include raw task data plus `selected_progress` and `selected_summary` machine fields for checkpoint/resume, goal/outcome state, LLM budget/calls, coding verification, side effects, and artifacts. Interactive key tokens are stable machine commands: `r` refresh, `w` watch, `p` pause, `c` cancel, `u` resume, `n` continue, `e` export, `1` report, `2` review, `3` subagents, `4` permission, and `q` quit.
- `clawcli session list/show/resume/archive/delete/fork` keeps a local session navigation store for `session_id`, `task_ids`, `active_goal_id`, `workspace_root`, checkpoint, event sequence, archive status, and fork source. This store is operator metadata under `RUSTCLAW_CLAWCLI_SESSION_STORE`, `$XDG_STATE_HOME/rustclaw/`, or `~/.local/state/rustclaw/`; it is not used as a natural-language route source.
- In interactive chat, `/continue` resumes the current background/checkpoint task from persisted thread state without copying a task id, `/approve` approves the exact pending action once, `/approve-scope` approves only the backend-provided exact capability/resource scope for the current session, and `/deny` closes the pending request. `clawcli permission grants` lists server-side scope grants and `clawcli permission revoke <grant_id>` revokes one immediately. The browser Tasks page exposes the same structured choices and revocation API.

```bash
clawcli session list --user-id 1 --chat-id 1 --json
clawcli session show task-123 --json
clawcli session resume task-123 "continue from the checkpoint" --json
clawcli session archive task-123 --json
clawcli session fork task-123 task-123.fork --json
clawcli session delete task-123 --json
```

```bash
clawcli goal start "make the focused change" --objective "ship the fix" --done tests_pass --verify "cargo test -p clawcli" --json
clawcli goal status task-123 --json
clawcli goal pause task-123 --pause-seconds 3600
clawcli goal resume task-123 --checkpoint-id ckpt-123 --message "continue from checkpoint"
clawcli goal edit task-123 --objective "updated goal" --done tests_pass --goal-status background
clawcli goal clear task-123
```

- `clawcli llm-trace <task_id> [--raw] [--limit N]` reads the task debug endpoint and prints numbered LLM calls with `llm_call_ref=LLM#1..N`, flow/code attribution, provider/model/status tokens, usage tokens, and optional raw request/response fields.
- Task event streams include goal, context budget/compaction, task-budget decisions, transitions, checkpoints, tool lifecycle, coding evidence, provider, hook, subagent/team, and final events. A bounded hot suffix serves SSE while an append-only redacted archive preserves the full task sequence, hash chain, and periodic source-range snapshots. `archive_replay` means the durable archive recovered an old hot cursor; `cursor_expired` now means the archive itself has a real gap. `clawcli events/watch`, reports, replay export, and browser task details consume these versioned events; raw event JSON stays in secondary details. See [`docs/task_event_archive_contract.md`](docs/task_event_archive_contract.md).
- `clawcli run-skill <skill_name> --args-json '{...}'` submits explicit `kind=run_skill` work without natural-language routing; add `--wait` to poll the same `task_id`.
- `clawcli skills` reads registry-backed skill metadata; `clawcli capabilities` and `clawcli permission capability` read flattened capability/policy metadata. Add `--json` when another script should consume the response.
- `clawcli replay export/run/diff` supports redacted recorded-only replay bundles for debugging and CI comparison without live model or tool calls; `replay run --coverage` exposes recorded coverage, `replay run --view llm|tools|checkpoints|summary` filters recorded evidence, and `replay diff` includes taxonomy tokens such as `route_changed`, `plan_changed`, `permission_changed`, and `final_status_changed`. See `docs/clawcli_exec_replay.md`.

CLI lifecycle and its persisted teaching evidence are documented in [Task state and context](docs/architecture/03-task-state-context.md) and [Coding and observability](docs/architecture/04-coding-observability.md).
- Stale ordinary `running` tasks become `timeout`; paused checkpoints in `waiting` or `background` stay `running` so recovery can claim them by checkpoint id.
- Async long-tail tools should start an external job, write `pending_async_job`, checkpoint, and publish an accepted machine reply with `checkpoint_id`, `poll_ref`, and `next_check_after`. Poll and cancel actions should be exposed as structured capabilities when the provider or dry-run adapter can support them. Worker recovery later polls through `poll_async_job`.
- Terminal async poll projection preserves an existing visible ask reply. If the ask task has only machine executor output, projection adds a machine JSON reply with `checkpoint_id`, `poll_ref`, `task_id`, and `final_result_json`.
- Seeded resume restores the persisted `TaskBudgetSlice`, cumulative model/tool/token/cost/elapsed counters, continuation index, observations, artifact refs, repair state, and completed side-effect fingerprints before re-entering the agent loop. A healthy task may cross the former 4-round/12-tool values when structured progress continues; only repeat/stagnation, cancellation, policy, or administrator hard ceilings stop it.
- Runtime recovery and projection code moves only machine fields such as `status_code`, `message_key`, `executor_state`, `resume_directive`, `job_id`, and artifact refs. User-facing prose is rendered later by finalizer, i18n, UI, or the model.
- Lease/heartbeat model: see `docs/task_lifecycle_lease_model.md`; every foreground and resume-executor write is fenced by the exact task-row `(lease_owner, claim_attempt)`. Heartbeat only renews that claim, checkpoint recovery advances the generation, and stale workers cannot publish claimed process events or terminal results.

<!-- ai-learning-exclude:start -->
## Detailed Architecture Guide

GitHub README pages do not support true pagination. Detailed diagrams are
maintained as an ordered guide so that each page stays focused. The AI Learning
UI renders these same Markdown sources instead of maintaining a second copy:

1. [Agent loop and planning](docs/architecture/01-agent-loop.md)
2. [Security and execution](docs/architecture/02-security-execution.md)
3. [Task state and context](docs/architecture/03-task-state-context.md)
4. [Coding and observability](docs/architecture/04-coding-observability.md)
5. [Skills, media, and models](docs/architecture/05-skills-media-models.md)
6. [Release validation](docs/architecture/06-release-validation.md)

Use the [architecture index](docs/architecture/README.md) for language selection and previous/next navigation.
<!-- ai-learning-exclude:end -->

## Main Components

- `crates/clawd`: core runtime, HTTP API, routing, memory, scheduling, auth, task queue
- `crates/skill-runner`: launches runner skill binaries; `clawd` resolves registry kind / `runner_name` before invoking it
- `crates/clawcli`: terminal CLI for talking to `clawd`
- `crates/webd`: optional reverse proxy and login session bridge for public/browser access
- `crates/telegramd`, `crates/wechatd`, `crates/feishud`, `crates/larkd`, `crates/whatsappd`, `crates/whatsapp_webd`: channel daemons
- `services/wa-web-bridge`: local Node bridge used by the WhatsApp Web channel
- `crates/skills/*`: fixed/core built-in skill implementations and `INTERFACE.md` specs
- `optional_skills/*`: bundled Skill Store skills compiled and installed on demand
- `external_skills/*`: externally submitted skills and their required `INTERFACE.md` specs
- `UI/`: Vite + React local console
- `pi_app/`: small-screen desktop monitor and launcher scripts

## Quick Start

### 1. Prerequisites

```bash
rustup default stable
python3 --version
```

`python3` is required. `npm` is needed when you want to build or deploy the UI.

### 2. Install the launcher

Recommended path:

```bash
# Install launcher only, skip nginx/UI deployment
bash install-rustclaw-cmd.sh --user --no-deploy-ui

# Build from source first, then install
bash install-rustclaw-cmd.sh --build --user --no-deploy-ui

# Build, install launcher, and deploy UI to nginx using script defaults
bash install-rustclaw-cmd.sh --build --user
```

Notes:

- `install-rustclaw-cmd.sh` installs the `rustclaw` launcher
- if `clawcli` was built, it is installed too
- by default the installer deploys `UI/dist` to nginx, writes nginx config, and reloads nginx when needed; pass `--no-deploy-ui` if you only want the launcher
- it also supports `--target <triple>`, `--dir <path>`, `--deploy-ui-nginx [path]`, and `--pi-app`; `--pi-app` only configures the small-screen desktop app on Raspberry Pi and is skipped on regular computers
- without `--build`, the script prefers existing binaries and only asks you to build/sync `release-bin` when they are missing

Verify:

```bash
command -v rustclaw
rustclaw -h
rustclaw -status
```

### 3. Configure runtime and channels

Main runtime config:

- `configs/config.toml`
- `configs/skills_registry.toml`

Split configs commonly edited:

- `configs/image.toml`
- `configs/audio.toml`
- `configs/crypto.toml`
- `configs/memory.toml`

Current channel config files:

- `configs/channels/telegram.toml`
- `configs/channels/wechat.toml`
- `configs/channels/feishu.toml`
- `configs/channels/lark.toml`
- `configs/channels/whatsapp.toml`
- `configs/channels/whatsapp-web.toml`
- `configs/channels/whatsapp-cloud.toml`
- `configs/channels/webd.toml`

### 4. Build from source

```bash
# Full release build: sync skill docs, build the workspace, and run the UI build/deploy script unless skipped
./build-all.sh

# Skip UI build
./build-all.sh no-ui

# Clean then rebuild
./build-all.sh clean

# Check Rust, Clang/libclang, protoc, Node.js, and npm updates without upgrading installed tools
./build-all.sh --check-toolchains

# Update installed build toolchains first, validate minimum versions, then build
./build-all.sh --update-toolchains

# Set the primary target
./build-all.sh --target aarch64-unknown-linux-gnu

# Raspberry Pi cross-build: defaults to 64-bit Raspberry Pi OS
./cross-build-pi.sh

# 32-bit Raspberry Pi OS
./cross-build-pi.sh --target pi32

# Build multiple targets in one run
./build-all.sh --target host --extra-target aarch64-unknown-linux-gnu
```

Current `build-all.sh` behavior:

- validates minimum compiler/tool versions on every build; normal builds only install missing prerequisites and do not update an existing host toolchain
- supports opt-in update discovery with `--check-toolchains` and opt-in pre-build upgrades with `--update-toolchains` through rustup plus the host package manager (`brew`, `apt`, `dnf`, `yum`, `zypper`, `pacman`, or `apk`)
- runs `scripts/sync_skill_docs.py` before the build starts
- always builds `release`, auto-discovers workspace binaries, and verifies that the expected outputs exist
- calls `build-ui-nginx.sh` when `UI/` exists and you did not pass `no-ui`, which means the default "build UI + deploy to nginx" path
- writes host outputs to `target/release` and cross-target outputs to `target/<triple>/release`
- `cross-build-pi.sh` prepares the Raspberry Pi linker / `cc` / bindgen environment before calling the existing build flow; it skips UI builds by default unless you pass `--with-ui`

You can still use plain `cargo build --workspace --release` for ad hoc local builds, but it does not include the repo-level sync, UI build, or output verification done by `build-all.sh`.

### 5. Start RustClaw

Examples with the launcher:

```bash
# Smallest startup path: release + channels=all + quick mode
rustclaw start -q

# Start with an explicit vendor/model
rustclaw -start --vendor openai --model gpt-5 --profile release --channels all --quick --skip-setup

# Start and require UI assets
rustclaw -start release all --with-ui
```

Current startup behavior:

- `rustclaw -start ...` ultimately calls `start-all.sh`
- `start-all.sh` starts services based on the `enabled` flags in `configs/channels/*.toml`
- when you pass `telegram | whatsapp_web | both | whatsapp_cloud | all`, the script writes the related Telegram / WhatsApp channel `enabled` values back into config files
- `all` here is a launcher preset, not "force-enable every daemon"; channels such as `webd`, `wechat`, `feishu`, and `lark` still follow their own config files
- `--with-ui` does not launch a frontend dev server; it requires a valid `UI/dist` build and stops with a hint if the assets are missing or stale
- `start-all.sh` no longer runs `sync_skill_docs.py` during startup

Equivalent script-based flow is still available:

```bash
./start-all.sh
./stop-rustclaw.sh
```

Single-service scripts are also available when you want finer control:

```bash
./component_start/start-clawd.sh
./component_start/start-telegramd.sh
./component_start/start-wechatd.sh
./component_start/start-feishud.sh
./component_start/start-larkd.sh
./component_start/start-whatsappd.sh
./component_start/start-whatsapp-webd.sh
./component_start/start-wa-web-bridge.sh
./component_start/start-clawd-ui.sh
```

When starting `clawd` alone:

- `./component_start/start-clawd.sh` checks for both `target/release/clawd` and `target/release/skill-runner`
- on first startup, if `selected_vendor` / `selected_model` are empty in `configs/config.toml`, it prompts for an interactive selection
- if the current vendor `api_key` is empty or still uses a `REPLACE_ME...` placeholder, it asks for the key before launch

### 6. Daily operations

```bash
rustclaw -status
rustclaw -logs clawd 200 --follow
rustclaw -health
rustclaw -stop
rustclaw -key list
```

## Identity and Access

RustClaw uses `user_key` as the main identity across the UI and messaging channels.

- permissions are resolved by `user_key`
- conversations are resolved by `channel + external_chat_id`
- the browser UI sends `X-RustClaw-Key`
- when the auth table is empty, `clawd` can bootstrap the first admin key

Key management:

```bash
rustclaw -key list
rustclaw -key generate user
rustclaw -key generate admin
rustclaw -key add rk-xxxx admin
rustclaw -key disable rk-xxxx
```

## UI, API, and `webd`

The main API still comes from `clawd`, but the current script flow prefers exposing the stack like this:

- `clawd` serves the internal API
- `webd` acts as the browser-facing bridge / reverse-proxy layer
- nginx serves `UI/dist` and proxies `/v1` and `/webd` to `webd`
- The `AI Learning` navigation page reads this bundled README, organizes top-level topics into pages, and renders Mermaid flows with zoom and full-screen controls; switching the UI language selects the matching README.

In the current defaults, `clawd` commonly listens on `0.0.0.0:8787` and `webd` commonly listens on `0.0.0.0:8788`; the deploy scripts derive the nginx upstream from `configs/channels/webd.toml`.

Useful endpoints (send `X-RustClaw-Key` for the current UI/user key):

- `GET /v1/health`
- `POST /v1/tasks`
- `GET /v1/tasks/{task_id}`
- `POST /v1/tasks/cancel`
- `POST /v1/tasks/cancel-by-task-id`
- `POST /v1/tasks/cancel-one`: compatibility endpoint that cancels by active-list index
- `POST /v1/services/{service}/{action}`: browser-console service start/stop/restart; failures return machine fields such as `error_code`, `status_code`, `message_key`, `service`, and `action`
- `GET /v1/auth/me`
- `POST /v1/auth/channel/bind`
- `GET/POST /v1/auth/crypto-credentials`: reads or overwrites exchange credentials scoped to the current `X-RustClaw-Key`
- `GET /v1/models/catalog`: returns the secret-free model/provider capability catalog used by the UI Models page and teaching-mode `model_catalog_trace`
- `GET /v1/skills/store`: returns the registry-driven catalog for optional bundled and imported skills, including separate `installed` and `enabled` states
- `POST /v1/skills/store/install`: installs and enables an optional catalog skill, then reloads runtime skill views
- `POST /v1/skills/store/remove`: removes an optional skill from runtime and planner visibility while retaining its bundled or imported package for reinstallation; always-on core and tool skills reject this action
- `GET /v1/nni/device/status`: reports NNI helper status, supported actions, and whether a device-signing chip is present
- `POST /v1/nni/device/action`: runs one of `pubkey`, `sign_timestamp`, `tng_device_pubkey`, `tng_device_cert`, `tng_signer_cert`, or `tng_root_cert`
- NNI request, heartbeat, and device-helper events are written as JSONL to `logs/nni.log`; `configs/config.toml` keeps only current NNI state and legacy record fallback.
- The standalone `nni_server` writes runtime events to `NNI_SERVER_LOG_PATH` (`logs/nni-server.log` by default) instead of `clawd.log`; enable `NNI_SERVER_LOG_STDOUT=1` only when an external supervisor intentionally captures those logs.

Quick example:

```bash
curl http://127.0.0.1:8787/v1/health \
  -H "X-RustClaw-Key: rk-xxxx"

curl -X POST http://127.0.0.1:8787/v1/tasks \
  -H "Content-Type: application/json" \
  -H "X-RustClaw-Key: rk-xxxx" \
  -d '{"user_id":1,"chat_id":1,"user_key":"rk-xxxx","channel":"ui","external_user_id":"local-ui","external_chat_id":"local-ui","kind":"ask","payload":{"text":"hello"}}'
```

## NL Regression Shortcuts

Use the smallest affected NL set while code is still moving, then widen coverage only at phase or release gates:

1. Static compact coverage: `python3 scripts/nl_tests/check_compact_coverage.py --report` verifies that the compact source-controlled case files cover basic skills, route/lifecycle classes, and media dry-run cases without calling a provider.
   The compact gate also requires Codex-style agent parity tags for coding, continuous development, shell/git/config/DB/web/KB, async, permission, subagent, memory, multilingual behavior, and failure recovery.
2. Focused affected suite: 10-30 hand-picked cases for the code path being changed.
3. Typical aggregate: compressed representative coverage after a phase batch.
4. Canary: 500 client-like cases before changing default authority or deleting old gates.
5. Safe aggregate: compact equivalent coverage first, then full 2100+ coverage only for high-risk deletion gates or release hardening.

Live NL runs should use `bash scripts/nl_tests/run_all_nl_with_server.sh`.
It creates a random loopback listener, isolated task/audit databases, and a
non-delivering `ui` channel by default, then removes that temporary state after
the run. Reusing a development server is opt-in with `--reuse-server`. Use
`--suite <name>` or `--category <name>` for a focused scope; numbered raw
`LLM#1..N` request/return fields stay enabled unless explicitly disabled.

Current `configs/agent_guard.toml` defaults keep verifier and registry guards enabled, including `answer_verifier_enforce_required_scope = "all"` and `registry_idempotency_guard_scope = "all"`. The old route-authority rollback/debug keys are no longer runtime configuration. If route boundary behavior changes, run the route-authority guard script and update replay/README flow descriptions instead of reintroducing semantic route switches.

Before physically deleting remaining compatibility code paths, use the current deletion gate rather than a fixed seven-day wait: compact live NL for the affected class, release-gate equivalent live coverage (`scripts/nl_tests/build_release_gate_subset.py --check` currently selects 285 rows covering all 211 declared categories), loop-boundary/replay review with no unexplained mismatch, and the planner/runtime/repair/static guards. The subset generator treats the shared compact contract, release behavior tags, machine capability/action-family tags, and suite breadth as mandatory; source filenames and undeclared incidental tags do not become release contracts. Contract repair cleanup must pass `python3 scripts/check_contract_repair_loop_observation_boundary.py`, which keeps worker contract repair as loop-observation output instead of a pre-planner route mutator. Planner/output-contract cleanup must also pass `python3 scripts/check_planner_runtime_boundary.py`, `python3 scripts/check_route_reason_marker_facade.py`, and `python3 scripts/check_finalizer_architecture.py`; repair cleanup must pass `python3 scripts/check_repair_boundary_inventory_coverage.py` plus `python3 scripts/check_repair_no_user_text_fields.py`. A longer observation window is still reasonable for high-risk production rollout, but it is not required for normal development cleanup.

Focused long-tail closed-loop entries:

- `bash scripts/nl_tests/run_suite.sh ops_closed_loop`
- `bash scripts/nl_tests/run_suite.sh long_tail_flows`
- `bash scripts/nl_tests/run_suite.sh ops_http_repair`
- `bash scripts/clawcli_smoke.sh`: compact CLI operator smoke for health, skills, submit, get, events, and watch. It uses `RUSTCLAW_CLI_SMOKE_KEY` / `RUSTCLAW_ADMIN_KEY` when provided, otherwise `clawcli` falls back to the local enabled admin key; optional env vars enable active/cancel/pause/resume/run-skill coverage. Set `RUSTCLAW_CLI_SMOKE_REQUIRE_CAPABILITIES=1` when the smoke must fail if `/v1/capabilities` is unavailable. New CLI-only logic is also covered by `cargo test -p clawcli`.

`ops_http_repair` is the focused bilingual retry suite for `ops_http_repair_then_validate_{zh,en}` and writes logs under `scripts/nl_suite_logs/ops_http_repair/<timestamp>/`.

UI notes:

- source lives in `UI/`
- built assets live in `UI/dist`
- `build-ui-nginx.sh` is the main "build UI + copy to nginx + refresh nginx config" path
- `deploy-ui-nginx.sh` is the "deploy existing `UI/dist`" path, with optional `--build`
- `install-rustclaw-cmd.sh` also deploys UI/nginx by default unless you pass `--no-deploy-ui`
- the browser UI has a standalone `NNI` navigation section backed by `/v1/nni/device/*`; devices without a signing chip surface `signature_chip_present=false` and show an explicit missing-chip state
- `工具/技能 / Tools/Skills` manages switches for installed skills; the adjacent `Skill Store` page owns optional-skill install, remove, reinstall, configuration retention, and third-party import flows
- service-control notices are rendered from backend machine codes (`error_code` / `message_key`) instead of parsing backend English strings
- `webd` can sit in front of `clawd` as a reverse proxy and login/session bridge

## Skills

RustClaw currently ships a broad skill set. Representative groups:

- system and ops: `system_basic`, `process_basic`, `service_control`, `health_check`, `log_analyze`, `task_control`
- files, config, and developer tools: `run_cmd`, `fs_basic`, `config_basic`, `config_edit`, `config_guard`, `archive_basic`, `fs_search`, `git_basic`, `package_manager`, `install_module`, `docker_basic`, `db_basic`
- network and content: `http_basic`, `rss_fetch`, `browser_web`, `doc_parse`, `transform`, `web_search_extract`
- multimodal and media generation: `image_generate` (`image.generate` / `image.poll` / `image.cancel`), `image_edit`, `image_vision`, `audio_transcribe`, `audio_synthesize` (`audio.synthesize` / `audio.poll` / `audio.cancel`), `video_generate` (`video.generate` / `video.poll` / `video.cancel`), `music_generate` (`music.generate` / `music.poll` / `music.cancel`)
- workflow and publishing: `schedule`, `extension_manager`, `photo_organize`, `invest_copy`, `x`
- domain and knowledge skills: `crypto`, `stock`, `weather`, `map_merchant`, `kb`

Skill installation and enablement are separate states. `skill_switches=false` keeps
an installed skill available in the normal inventory but disables it.
`uninstalled_skills` removes an optional skill from runtime, planner visibility,
and the normal Tools/Skills inventory while keeping it discoverable in Skill Store.
Bundled entries marked `install_mode="on_demand"` are excluded from the normal
`build-all.sh` release build. The default on-demand set is `crypto`, `invest_copy`,
`map_merchant`, `photo_organize`, `stock`, `weather`, and `x`; clicking Install
builds only that registry-declared Cargo package before enabling and reloading it.
Removing one of these skills deletes its compiled runner and asks whether its
registry-declared dedicated configuration files should be preserved. Reinstalling
never overwrites configuration files that still exist. Core skills and registry
entries whose `planner_kind=tool` are always available and cannot be removed through
Skill Store. Third-party import validates and registers the bundle, clears stale
uninstall state, enables it, and then exposes it in both Skill Store and Tools/Skills.

If you need to answer “how is this skill configured / bound / enabled, and what prerequisite is missing”, start with `prompts/references/skill_setup_guide.md`.

Skill discovery and runtime behavior are driven by:

- `configs/skills_registry.toml`
- `[skills]` in `configs/config.toml`
- `crates/skills/*/INTERFACE.md`
- `optional_skills/*/INTERFACE.md`
- `external_skills/*/INTERFACE.md`
- `prompts/layers/generated/skills/*.md`

Planner skill selection is registry-, capability-, and interface-driven. After a skill is registered, enabled, documented in `INTERFACE.md`, synced with `python3 scripts/sync_skill_docs.py`, and, when planner-facing, given `planner_capabilities` in `configs/skills_registry.toml`, the planner should learn when to use it from registry metadata plus the generated skill prompt. Do not add per-skill selection branches to `clawd` just to make new natural-language examples pass. If selection accuracy is weak, improve the registry capability metadata, skill interface, generated prompt, or model-specific vendor patch; keep Rust code for protocol validation, resolver/verifier boundaries, permission/safety checks, runner dispatch, output-contract enforcement, and deterministic execution compatibility.

Skill integration entry points:

- built-in and standard `runner` skills: `skill_develop/README.md`
- external skill example: `external_skills/example/README.md`
- skill setup and prerequisite reference: `prompts/references/skill_setup_guide.md`

### Local STT With whisper.cpp

`audio_transcribe` can use a local whisper.cpp server through the `custom` OpenAI-compatible provider. Use a dedicated local port such as `8178` so it does not collide with `clawd` or UI ports.

Download a multilingual model into the gitignored local model directory. The script picks `tiny` / `base` / `small` / `medium` from detected device memory, and `large-v3` is available only when explicitly requested with `--model large-v3`.

```bash
MODEL_PATH="$(bash scripts/download-whisper-model.sh --print-path-only)"
data/vendor/whisper.cpp/build/bin/whisper-server -m "$MODEL_PATH" \
  --host 127.0.0.1 --port 8178 \
  --request-path /v1 --inference-path /audio/transcriptions \
  --convert --language auto
```

Use a multilingual Whisper model for Chinese, for example `ggml-small.bin`, `ggml-medium.bin`, or `ggml-large-v3.bin`; avoid English-only `.en` models for Chinese audio.

```toml
[audio_transcribe]
default_vendor = "custom"
adapter_mode = "compat"
allow_compat_adapters = true
default_model = "local-whisper"
custom_models = ["local-whisper", "whisper-1"]

[audio_transcribe.providers.custom]
base_url = "http://127.0.0.1:8178/v1"
api_key = ""
model = "local-whisper"
timeout_seconds = 120
```

The empty `api_key` is accepted only for loopback `custom` providers (`localhost`, `127.0.0.1`, `::1`). Remote custom providers still require a real key.

## Directory Guide

- `configs/`: runtime, channel, model, memory, and skill configuration
- `crates/`: Rust services, daemons, CLI, and skills
- `external_skills/`: externally submitted skills and example scaffolds
- `prompts/`: prompt layers and generated skill prompt files
- `scripts/`: setup, regression, maintenance, and skill-call helpers
- `services/`: non-Rust helper services such as the WhatsApp Web bridge
- `UI/`: browser UI project
- `pi_app/`: desktop small-screen app
- `docker/`: docker-oriented configs and entrypoint files
- `systemd/`: service templates

## Pi App

The small-screen desktop app lives in `pi_app/`.

```bash
cd pi_app && ./run-small-screen.sh
cd pi_app && ./install-desktop.sh
cd pi_app && ./enable-autostart.sh
cd pi_app && ./open-small-screen.sh
```

It reads health status from `clawd`, so start the backend first.

The Pi App also carries the NNI device-signing helper used by the backend and browser UI. `pi_app/signature.py` supports Slot 0 public-key reads, timestamp signing, and TNG device/signer/root certificate reads when supported hardware and `cryptoauthlib` are present; see `pi_app/TNG_SERVER_GUIDE.md`. Devices without that chip are valid deployments and are reported as a missing-signature-chip state.

## Developer Notes

- `build-all.sh` is the most accurate repo-level build entry if you are building from source
- `install-rustclaw-cmd.sh` is the most convenient operator-facing entry because it can handle both launcher installation and optional UI/nginx deployment
- if you only want to refresh the static UI site, use `build-ui-nginx.sh` or `deploy-ui-nginx.sh`
- if you are integrating skills, run `python3 scripts/sync_skill_docs.py` explicitly; startup scripts no longer sync skill docs for you
- many helper and regression scripts live in `scripts/`
- for the local `ops_closed_loop` regression stack, run `bash scripts/regression_ops_closed_loop.sh`

## License

This project uses a non-commercial source-available license.

- English legal text: `LICENSE`
- Chinese reference translation: `LICENSE.zh-CN.md`

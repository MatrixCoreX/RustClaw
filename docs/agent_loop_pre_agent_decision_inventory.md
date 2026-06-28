# Agent Loop Pre-agent Decision Inventory

Last updated: 2026-06-21

This inventory supports `plan/agent_loop_ideal_state_convergence_plan_20260615.md`.
It records the current pre-agent decision surface after `semantic_route_authority`
became `agent_loop_default`, and separates boundary duties from ordinary semantic
decisions that should continue moving into the agent loop.

## Current Authority Model

- Live config uses `semantic_route_authority = "agent_loop_default"`.
- `agent_loop_default` selects any eligible low-risk bucket instead of requiring
  a single configured canary class. Canary debug selection uses
  `agent_loop_canary_bucket`; legacy `agent_decides_migration_class` is an
  ignored historical config key.
- The legacy selected-class fields remain for compatibility and are now derived
  from `agent_loop_eligibility()`:
  - `structured_field_read`
  - `exact_path_list`
  - `bound_path_summary`
  - `recent_artifacts_judgment`
  - `scalar_count`
  - `low_risk_tool_discovery`
- Boundary context also records generic eligibility buckets:
  - `low_risk_structured_read`
  - `low_risk_listing`
  - `low_risk_grounded_summary`
  - `low_risk_metadata_judgment`
  - `low_risk_scalar_observation`
  - `low_risk_status_observation`
  - `low_risk_config_read`
  - `low_risk_log_observation`
  - `low_risk_workspace_question`
  - `low_risk_tool_discovery`
- `low_risk_tool_discovery` is context-only: it does not require content
  evidence, and is selected only from structured `tool_discovery` machine
  semantics plus planner context availability.
- `agent_decides_semantic_route` and `agent_decides_migration_class` remain only
  in historical docs, logs, and regression tests. Runtime config loading ignores
  them; policy and rollout attribution use `semantic_route_authority`, and canary
  selection uses `agent_loop_canary_bucket`.
- Boundary context already records machine-readable roles:
  - `normalizer_role = "initial_hint"`
  - `post_route_role = "boundary_machine_gate"`
  - `direct_answer_gate_role = "fallback_safety_check"` for the top-level
    semantic routing role; detailed
    `pre_agent_gates.direct_answer_gate.ownership_class` may be
    `fallback_safety_check`, `contract_boundary`, `evidence_projection`,
    `agent_loop_activation`, or `semantic_policy_candidate`.
  - `agent_loop_authority_enabled`
  - `chosen_authority`
  - `selected_migration_class`
  - `agent_loop_eligibility_bucket`
  - `agent_loop_eligibility_blocked_reason`

## Main `kind=ask` Path

1. `worker::worker_once()`
2. `worker::ask_pipeline::prepare_ask_flow()`
3. `worker::ask_prepare::prepare_ask_routing()`
4. `worker::ask_prepare::prepare_ask_execution_context()`
5. `worker::ask_pipeline::apply_ask_post_route()`
6. `worker::ask_pipeline::execute_ask_dispatch()`
7. `ask_flow::execute_ask_routed()`
8. `agent_engine::run_agent_with_tools()` for selected agent-loop authority or
   planner-execute fallback

## Decision Surface Inventory

| Surface | Current owner | Current role | Classification | Target |
| --- | --- | --- | --- | --- |
| Task kind dispatch | `worker_once()` | Splits `ask` from `run_skill` and direct task paths. | boundary/safety | Keep outside agent loop. |
| Schedule direct text | `maybe_finalize_schedule_direct_text_success()` and schedule-direct branches | Finalizes explicit scheduled text without normal ask planning. | boundary/special task | Keep, constrained to explicit scheduler state. |
| Identity/session binding | `prepare_ask_routing()`, continuation resolver, conversation state | Binds user/chat/task state, active clarify state, aliases, and follow-up anchors. | boundary/context | Keep; ordinary semantic choice moves to loop. |
| Active clarify locator fast path | active clarify/follow-up locator rewrite | Turns a valid locator reply into a machine-bound execution context. | deterministic evidence shortcut | Keep only for machine locator state; no phrase rules. |
| Intent normalizer | `intent_router::run_intent_normalizer()` and `intent_router_*` modules | Produces `RouteResult`, output contract, turn analysis, and possible recipe. Some repair modules still mutate `FirstLayerDecision`. | mixed: initial hint + legacy semantic reroute | Keep schema/hint output; move ordinary reroute into agent loop. |
| Contract repair judge | `intent_router_normalizer_model` / contract repair prompt schema | Repairs schema-backed contract uncertainty before dispatch. | compatibility/schema repair | Keep only schema-backed repair; no final semantic authority. |
| Current-turn structural repair | `intent_router_current_turn_structural_repair.rs` and related tests | Repairs locators, field paths, archive/config/file-token contracts; may also adjust route decision. | mixed: machine repair + legacy semantic reroute | Split machine repair from decision mutation. |
| Ask context bundle | `prepare_ask_execution_context()` | Builds memory, attachment, workspace, recent execution context. | boundary/context | Keep outside loop as context builder. |
| Image attachment analysis | `analyze_attached_images_for_ask()` | Calls image skill/model before ask execution context when images are attached. | boundary/special modality | Keep as modality preprocessing; image/voice tested separately. |
| Post-route locator policy | `apply_ask_post_route()`, `post_route_policy::apply_post_route_policy()` | Applies locator resolution, missing-locator clarify, contract guards, and route refinements. | mixed: boundary gate + legacy semantic repair | Split into boundary locator/contract/delivery gates and legacy repair. |
| TaskContract matrix | `TaskContract::from_route_result()` and contract matrix snapshots | Normalizes semantic kind, allowed action, required evidence, and response shape. | boundary/contract | Keep as machine contract used by loop/verifier/finalizer. |
| Direct existing file delivery | `direct_existing_file_delivery_token()` path | Publishes already verified local file token. | deterministic delivery shortcut | Keep only for verified path/delivery machine state. |
| Runtime-grounded direct candidates | `ask_flow` direct candidates for scalar/status observations | Returns observed machine values without planner when evidence is already available. | deterministic evidence shortcut | Keep if all inputs are machine fields and final shape is contract-bound. |
| Direct-answer preflight | `run_direct_answer_gate()` and `DirectAnswerGate*` helpers | Decides direct answer, clarification, or promotion to execution. | ordinary semantic / fallback safety | Demote to fallback safety for non-selected/rollback paths; loop decides selected cases. |
| Clarify question generation | `generate_or_reuse_clarify_question()` | Generates or reuses user-visible clarification text from structured reason. | final rendering | Keep rendering, but decision must come from machine `terminal_intent` / missing slot. |
| Self-extension handler | `self_extension::maybe_handle_ask_self_extension()` | Handles extension protocol and handoff. | special protocol boundary | Keep protocol boundary; no natural-language skill branches in main flow. |
| Agent-loop authority selection | `agent_loop_authority_selected_migration_class()` | Selects eligible class and records boundary context. | migration control | Replace class whitelist with generic eligibility buckets, then delete long-term canary plumbing. |
| Planner/runtime loop | `agent_engine::run_agent_with_tools()` | Plans, calls capabilities/tools/skills, observes, verifies, synthesizes, responds. | target ordinary semantic authority | Expand to answer/clarify/tool/skill/continue/stop decisions. |
| PlanVerifier / contract action gate | `PlanVerifier`, verifier modules | Blocks disabled capabilities, missing args, risk/effect violations, confirmation needs, and unsafe mutations. | boundary/safety | Keep as main execution guard. |
| Evidence coverage / Answer Verifier | answer verifier, observed-output finalizer, finalizer modules | Checks required evidence, answer shape, grounding, delivery consistency. | boundary/final guard | Keep; make issue output machine-readable and language-neutral. |
| Channel delivery | channel adapters and final task persistence | Sends `text` / `messages` and persists result/journal. | delivery boundary | Keep; no semantic route decisions. |

## Keep Outside The Agent Loop

- Task claiming, task kind dispatch, cancellation, timeout, and retry lifecycle.
- Identity, user key, channel binding, session state, aliases, active-task anchors.
- Permission, risk ceiling, confirmation, dry-run, and side-effect policy.
- Workspace/path scope, locator existence, ambiguity count, and file delivery safety.
- Skill visibility, skill switches, capability availability, and registry policy.
- Budget profile, max rounds, max tool calls, repeat/no-progress guard.
- PlanVerifier, Answer Verifier, evidence coverage, output contract, delivery
  consistency, and secret redaction.

## Move Into The Agent Loop

- Ordinary “answer vs clarify vs execute” decisions for selected and future
  eligible low-risk requests.
- Direct-answer promotion/demotion for tool-backed requests.
- Clarification need when it is semantic uncertainty rather than missing machine
  locator or safety state.
- Capability choice when it is based on task meaning rather than permission or
  allowed-action safety.
- Contract selection for ordinary read/list/summarize/status/config-read/log-read
  tasks once registry/schema can express the capability.

## Intent-router Repair Classes

`intent_router` repair output is now classified with language-neutral machine
tokens so later deletion can distinguish boundary repairs from ordinary semantic
reroutes:

- `schema_normalization`: enum aliases, structured contract repair, execution
  recipe/schema normalization, and contract-driven planner promotion.
- `machine_locator_repair`: locator-kind normalization, current-turn anchor
  correction, missing locator machine state, and workspace scope locator hints.
- `boundary_safety_repair`: untrusted model fields ignored, unsafe fallback
  schema, and structural cleanup of answer candidates.
- `legacy_semantic_reroute`: any repair code not explicitly classified above;
  this conservative default prevents unknown normalizer repair from being
  treated as a permanent boundary gate.

Runtime code must continue to classify only internal repair source/detail tokens,
not user natural-language text.

Legacy semantic reroute is isolated during the agent-loop migration: a
contract-repair judge result may only change the first-layer decision when the
change is backed by machine contract fields, an execution recipe, or an
allow-listed machine marker. Ordinary semantic route changes that lack those
signals are rejected before they can mutate runtime state.

## Deterministic Shortcuts Allowed To Remain

These shortcuts may remain outside the planner only if they consume observed
machine state and keep final output contract-bound:

- verified `FILE:<path>` delivery;
- active clarify locator reply based on stored clarify state;
- scalar/count/status answers from already observed structured fields;
- schedule direct text from explicit scheduler state;
- alias binding acknowledgement based on structured locator/alias state and i18n
  `message_key`.

They must not parse user-language phrases or produce fixed user-language prose
from runtime code.

## Deletion Order

1. Add route-authority baseline metrics for selected class, chosen authority,
   agent-loop authority enabled, planner first action, verifier state, and
   delivery consistency.
2. Replace `agent_decides_eligible_migration_class()` whitelist with generic
   `AgentLoopEligibility` buckets. The compatibility wrapper remains until
   legacy selected-class attribution is deleted.
3. Move clarify/direct-answer terminal intent into `AgentLoopDecisionEnvelope`.
4. Split `post_route_policy` into boundary gates and legacy semantic repair.
5. Demote intent-router repair to schema/hint output for selected cases.
6. Keep `agent_decides_semantic_route` / `agent_decides_migration_class` out of
   runtime config parsing; remove remaining historical references only when
   log-reader and regression-test compatibility no longer need them.
7. Delete legacy semantic repair paths for selected/default agent-loop cases.

## Validation Gates

- Static:
  - `python3 scripts/check_no_nl_hardmatch.py`
  - `python3 scripts/check_long_files.py`
  - `git diff --check`
- Rust:
  - `cargo test -p clawd intent_router -- --nocapture`
  - `cargo test -p clawd post_route_policy -- --nocapture`
  - `cargo test -p clawd ask_flow -- --nocapture`
  - `cargo test -p clawd loop_control -- --nocapture`
  - `cargo test -p clawd verifier -- --nocapture`
  - `cargo check -p clawd`
- NL:
  - 10-30 affected cases for small changes.
  - focused 100 for each new eligibility bucket.
  - compressed typical aggregate for default-range expansion.
  - 500 canary plus 2100 safe aggregate before deleting old gates.
- 2026-06-15 `low_risk_tool_discovery` focused check:
  - `scripts/nl_suite_logs/client_like_continuous/run_20260615_101444`
  - `logs/agent_rollout_metrics/run_20260615_101444_rollout_metrics.json`
  - 6/6 pass across zh-CN/en/ja/ko, `planner_first_action_counts.respond=6`,
    `tool_call_count=0`, and
    `agent_loop_eligibility_bucket_counts.low_risk_tool_discovery=12`.

## Multilingual Reinforcement

<!--
zh-CN: 本 inventory 用于区分必须保留的边界层职责与应迁入 agent loop 的普通语义裁判；不得把自然语言短语匹配作为迁移手段。
en: This inventory separates boundary responsibilities that must remain outside the loop from ordinary semantic decisions that should move into the agent loop; natural-language phrase matching must not be used as a migration mechanism.
-->

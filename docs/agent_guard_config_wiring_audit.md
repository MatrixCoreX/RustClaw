# Agent Guard Config Wiring Audit

Last updated: 2026-07-20

This document originally supported the June 2026 agent-loop migration plan and
is now maintained as supporting documentation for the active post-migration
hardening plan. It classifies `configs/agent_guard.toml` fields by current
wiring and intended ownership.

## Summary

- Interactive task slices and administrator hard ceilings are wired through
  `task_budget_contract::load_task_budget_policy()`. Per-plan action capacity,
  repetition, verifier, and recipe controls remain in
  `agent_engine::support::load_agent_loop_guard_policy()`.
- Route-authority runtime switches have been removed from
  `AgentLoopGuardPolicy`; old route-authority, canary, and `agent_decides_*`
  names are ignored historical keys and must not return to production config.
- `registry_idempotency_guard_scope` and `answer_verifier_enforce_required_scope`
  have converged to final `all`
  machine boundaries; non-`all` historical values are normalized to `all` and
  must not be used as rollback/debug controls.
- Domain action lists, stale dedup compatibility fields, `dynamic_rules`,
  `messages`, and `trace_messages` were physically removed after reader audit.
  `check_agent_loop_guard_final_scope.py` rejects their re-entry.
- User-visible copy should move toward `message_key` plus finalizer/LLM/i18n
  rendering before it is used in runtime behavior.
- `agent.hooks` pre-tool policy is now wired as machine-token control:
  deny, require-confirmation, and background-wait decisions are driven by
  action refs/tool refs, not user natural-language text.

## Wiring Matrix

| Config path | Current wiring | Category | Owner | Next action |
| --- | --- | --- | --- | --- |
| `agent.loop_guard.max_steps` | Parsed by `load_agent_loop_guard_policy()` as per-plan action capacity. It is not a whole-task planner-round or tool-call completion limit. | Wired behavior. | Plan execution guard. | Keep; tune only when verified plans cannot represent a coherent phase. |
| `agent.loop_guard.answer_verifier_retry_limit` | Physically removed. Final-answer recovery permits exactly one bounded synthesis retry selected by structured verifier fields. | Removed legacy key. | Answer Verifier evidence boundary. | Do not restore a configurable retry count or generated-prose repair loop. |
| `agent.loop_guard.repeat_action_limit` | Parsed and used in repeat guard. | Wired behavior. | Loop repeat guard. | Keep. |
| `agent.loop_guard.repeat_same_action_limit` | Parsed into policy. | Wired behavior / compatibility. | Loop repeat guard. | Keep until dedup cleanup verifies no duplicate meaning. |
| `agent.task_budget.admin_max_*` | Parsed by `load_task_budget_policy()` into cumulative model-turn, tool-call, token, cost, elapsed, continuation, and non-resumable runtime ceilings. | Wired behavior. | Administrator safety boundary. | Keep high enough to remain an emergency boundary; model output cannot raise it. |
| `agent.task_budget.profiles.*.soft_slice_seconds` | Parsed into resumable wall-time slices and bounded by the worker timeout reserve. | Wired behavior. | Task budget coordinator. | Tune latency/checkpoint cadence, not task complexity. |
| `agent.task_budget.profiles.*.stagnation_tolerance` | Parsed as structured consecutive non-progress tolerance. | Wired behavior. | Progress evaluator. | Keep above one so a single inconclusive round is not terminal. |
| `agent.task_budget.profiles.*.provider_timeout_class` | Parsed as a machine timeout class for provider-call policy/observability. | Wired behavior. | Provider budget policy. | Keep `short`, `standard`, or `long_tail`. |
| `agent.task_budget.profiles.*.tool_timeout_class` | Parsed as a machine timeout class for tool policy/observability. | Wired behavior. | Tool budget policy. | Long-tail work must expose async/checkpoint state when resumable. |
| `agent.loop_guard.max_rounds`, `max_tool_calls`, `no_progress_limit`, `recoverable_failure_extra_rounds`, `multi_round_enabled` | Physically removed from interactive config, parser, loop state, stop signals, and profile overrides. | Removed legacy keys. | Task-budget migration guard. | Do not restore. Explicit caps belong only to non-interactive/child request contracts. |
| `agent.loop_guard.budget_profiles.*` loop thresholds | Replaced by `[agent.task_budget.profiles.*]` soft slices plus `max_steps` action capacity overrides where needed. | Removed legacy semantics. | Task-budget migration guard. | Do not reintroduce round/tool thresholds under profile names. |
| `agent.loop_guard.answer_verifier_enforce_required_scope` | Parsed as final machine token `all`; missing or non-`all` historical values normalize to `all`, so required-evidence force-failure behavior is not gated by route class. | Wired behavior. | Answer Verifier evidence boundary. | Keep verifier attribution review active; fix false blocks through evidence contracts, extractors, registry metadata, or planner prompts rather than disabling the guard. |
| `agent.loop_guard.answer_verifier_enforce_required` | Historical bool name; current runtime config loader does not parse it. | Ignored legacy key. | Config migration. | Do not document or extend as a config field; use `answer_verifier_enforce_required_scope`. |
| `agent.loop_guard.semantic_route_authority` | Retired historical key; current runtime config loader must not parse it. | Removed legacy key. | Config migration guard. | Do not add to new configs; `check_route_authority_legacy_keys.py` rejects production/config reentry. |
| `agent.loop_guard.agent_loop_canary_bucket` | Retired historical key; current runtime config loader must not parse it. | Removed legacy key. | Config migration guard. | Do not add to new configs; use focused tests/replay diffs for targeted debugging. |
| `agent.loop_guard.agent_decides_semantic_route` | Historical name ignored by current runtime config load. | Ignored legacy key. | Config migration guard. | Do not document or extend as a config field. |
| `agent.loop_guard.agent_decides_migration_class` | Historical name ignored by current runtime config load. | Ignored legacy key. | Config migration guard. | Do not document or extend as a config field. |
| `agent.loop_guard.registry_idempotency_guard_scope` | Parsed as final machine token `all`; missing or non-`all` historical values normalize to `all`, so execution-loop repeat/idempotency behavior is not gated by route class. | Wired behavior. | Registry idempotency boundary. | Keep repeat-block attribution review active; fix false repeat blocks through registry `effect`, `once_per_task`, `dedup_scope`, or verifier policy rather than disabling the guard. |
| `agent.loop_guard.registry_idempotency_guard` | Historical bool name; current runtime config loader does not parse it. | Ignored legacy key. | Config migration. | Do not document or extend as a config field; use `registry_idempotency_guard_scope`. |
| `agent.hooks.handlers` | Parsed as trusted command/HTTP/MCP lifecycle handlers with stage, trust/hash, bounds, retry, failure policy, and blocking mode. Handler output is a versioned machine contract merged through `PolicyDecision`; no configured handler leaves baseline execution unchanged. | Wired behavior. | Agent hook runtime. | Keep handler ids and decisions machine-token only. Repository command hooks require explicit trust plus a matching content hash; only PreToolUse and PermissionRequest may block. |
| Removed fixed hook action/tool lists | The former blocked-action, blocked-tool, confirmation-action, and background-action arrays and their evaluator were physically deleted after handler migration. | Removed legacy keys. | Hook deletion guard. | Do not restore parallel list policy. Use a trusted bounded handler or the existing registry/permission policy owner. `check_agent_hook_runtime_contracts.py` rejects re-entry. |
| Removed domain action lists / dedup / prose sections | Reader audit found no production owner, so the sections were physically deleted. | Removed legacy keys. | Registry metadata, prompt layers, machine reason codes, and language rendering. | Do not restore parallel config. Use `planner_capabilities` policy metadata, prompt files, or structured `message_key`/`reason_code` outputs. |

## Risk Notes

- Treating an ignored legacy config key as a long-term behavior switch is unsafe.
  The plan must distinguish "historical/log compatibility" from "runtime config
  input".
- Dead config is also risky because operators can believe they have changed a
  guard when runtime ignores the value.
- Domain action lists duplicate information that should live in registry
  metadata. Keeping both paths creates drift.
- `agent.messages` can become a hardcoded multilingual reply path if wired
  directly into finalizer branches. Use `message_key` and language rendering
  instead.
- `dynamic_rules` can reintroduce domain-specific prompt debt if new skills add
  one-off prompt strings instead of `INTERFACE.md`, generated prompts, schema, or
  registry metadata.

## Required Follow-Up

1. Keep registry parity checks between `configs/skills_registry.toml` and
   `docker/config/skills_registry.toml` before enabling registry-driven guards.
   Use `python3 scripts/check_skill_registry_parity.py --mode p3 --strict`.
2. Keep behavior-affecting reasons in stable `reason_code` fields.
3. Keep broad NL canary until after plan/code work is complete; use focused Rust
   tests and hard-match scan for document-only changes.

## Verification

For this audit class of change:

```bash
python3 scripts/check_no_nl_hardmatch.py
git diff --check
```

Before any behavior migration based on this audit:

```bash
cargo test -p clawd support -- --nocapture
cargo test -p clawd loop_control -- --nocapture
bash scripts/nl_tests/run_suite.sh evidence_policy_offline
bash scripts/nl_tests/run_suite.sh runtime_capability_boundary
```

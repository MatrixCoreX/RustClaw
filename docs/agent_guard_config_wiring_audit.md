# Agent Guard Config Wiring Audit

Last updated: 2026-07-18

This document originally supported the June 2026 agent-loop migration plan and
is now maintained as supporting documentation for the active post-migration
hardening plan. It classifies `configs/agent_guard.toml` fields by current
wiring and intended ownership.

## Summary

- Core loop budgets and budget profiles are wired through
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
| `agent.loop_guard.max_steps` | Parsed by `load_agent_loop_guard_policy()` and consumed by agent planning/loop budgets. | Wired behavior. | Boundary Layer budget guard. | Keep. Test with `cargo test -p clawd support -- --nocapture` and loop tests. |
| `agent.loop_guard.max_rounds` | Parsed and applied to `LoopState.max_rounds`. | Wired behavior. | Boundary Layer budget guard. | Keep task-class profile overrides; avoid removing config. |
| `agent.loop_guard.recoverable_failure_extra_rounds` | Parsed and used by recoverable failure loop extension. | Wired behavior. | Loop budget/repair guard. | Keep; observe LLM cost in canaries. |
| `agent.loop_guard.multi_round_enabled` | Parsed and logged/used by loop planning path. | Wired behavior. | Loop coordinator. | Keep as emergency rollback toward single-round behavior. |
| `agent.loop_guard.answer_verifier_retry_limit` | Parsed and used for verifier retry loops. | Wired behavior. | Answer Verifier repair budget. | Keep; canary cost and verifier block rate. |
| `agent.loop_guard.repeat_action_limit` | Parsed and used in repeat guard. | Wired behavior. | Loop repeat guard. | Keep. |
| `agent.loop_guard.no_progress_limit` | Parsed and used in no-progress stop logic. | Wired behavior. | Loop progress guard. | Keep task-class overrides. |
| `agent.loop_guard.max_tool_calls` | Parsed and used in execution loop tool-call guard. | Wired behavior. | Boundary Layer budget guard. | Keep. |
| `agent.loop_guard.repeat_same_action_limit` | Parsed into policy. | Wired behavior / compatibility. | Loop repeat guard. | Keep until dedup cleanup verifies no duplicate meaning. |
| `agent.loop_guard.budget_profiles.fast_read` | Parsed and selected for fast read/status tasks. | Wired behavior. | Budget profile selector. | Keep. |
| `agent.loop_guard.budget_profiles.grounded_summary` | Parsed and selected for summary/evidence tasks. | Wired behavior. | Budget profile selector. | Keep. |
| `agent.loop_guard.budget_profiles.multi_step_workspace` | Parsed and selected for workspace/write/delivery tasks. | Wired behavior. | Budget profile selector. | Keep. |
| `agent.loop_guard.ops_closed_loop` | Parsed and selected for `ops_closed_loop` execution recipes. | Wired behavior. | Ops closed-loop budget/repair guard. | Keep. |
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

# Agent Upgrade Rollout Guardrails

Last updated: 2026-07-24

This document defines current rollout and rollback boundaries for changes to
the agent loop, verifier, finalizer, registry, lifecycle, and natural-language
execution.

## Non-Negotiable Boundaries

- Ordinary semantic authority stays in the planner/agent loop.
- Production runtime must not match user-language phrases.
- Runtime, verifier, finalizer, policy and adapters must not add fixed
  user-facing reply templates.
- Deterministic code emits machine fields, status/reason codes, paths, counts,
  `message_key`, evidence and artifact refs.
- User prose is synthesized by the model or i18n renderer in the request
  language.
- Risk, permission, confirmation, dry-run, sandbox, side-effect reconciliation
  and administrator ceilings cannot be bypassed by model output.
- Real remote publication or mutation requires the declared policy and
  confirmation. Broad NL uses dry-run/mock for destructive or paid media paths
  unless the test explicitly owns a safe live scope.

## Wired Runtime Controls

| Control | Location | Meaning | Rollback |
| --- | --- | --- | --- |
| `max_steps` | `[agent.loop_guard]` | Per-plan action capacity, not a whole-task round/tool completion limit | Restore previous value and restart |
| `repeat_action_limit` | `[agent.loop_guard]` | Cross-round repeat guard | Restore previous value and restart |
| `repeat_same_action_limit` | `[agent.loop_guard]` | Compatibility repeat guard | Restore previous value and restart |
| `admin_max_*` | `[agent.task_budget]` | Fail-closed cumulative model/tool/token/cost/elapsed/continuation/non-resumable ceilings | Administrator policy only; model cannot raise |
| `profiles.*.soft_slice_seconds` | `[agent.task_budget.profiles.*]` | Resumable checkpoint cadence | Restore previous duration |
| `profiles.*.stagnation_tolerance` | `[agent.task_budget.profiles.*]` | Consecutive structured non-progress tolerance | Restore previous tolerance |
| `provider_timeout_class` | task-budget profile | Provider timeout class | Use `short`, `standard`, or `long_tail` |
| `tool_timeout_class` | task-budget profile | Tool timeout class | Long-tail tools need async/checkpoint support |
| `answer_verifier_enforce_required_scope` | `[agent.loop_guard]` | Final required-evidence scope, normalized to `all` | Fix evidence contract; do not disable |
| `registry_idempotency_guard_scope` | `[agent.loop_guard]` | Final registry repeat/idempotency scope, normalized to `all` | Fix registry policy; do not disable |

Removed interactive controls must not return:

- `max_rounds`
- `max_tool_calls`
- `no_progress_limit`
- `recoverable_failure_extra_rounds`
- `multi_round_enabled`
- route-authority, canary, `agent_decides_*`, selected-contract, or bool guard
  compatibility switches

Rollback for a coherent task-budget/runtime migration is a code revert, not a
dual runtime branch. Explicit caps remain valid only in non-interactive or
child-task request contracts.

## Attribution Requirements

Use TaskJournal and versioned task events. Current rollout evidence should
include:

- planner round/action and decision envelope;
- capability request, resolution and concrete tool/skill;
- verifier/evidence coverage and missing fields;
- permission/policy/risk/confirmation decisions;
- budget profile, decision, cumulative usage and checkpoint;
- tool/capability result status, artifacts and evidence refs;
- mutation receipt, reconciliation and idempotency state;
- final status, failure attribution and delivery consistency;
- LLM call/retry/truncation/token/cost fields when available.

Historical normalizer, first-layer and route-gate fields may be present only
when comparing archived artifacts. Do not copy them into new runtime
attribution.

Never store raw secrets or unredacted private payloads. Raw prompt/provider data
is allowed only in the dedicated access-controlled debug/teaching record with
its existing redaction and retention policy.

## Development Gates

Minimum:

```bash
python3 scripts/check_no_nl_hardmatch.py
python3 scripts/check_no_runtime_hard_reply.py
python3 scripts/check_long_files.py
git diff --check
cargo check -p clawd -p claw-core
```

Run area-specific contract tests for the modified resolver, verifier, budget,
lifecycle, replay, policy, registry, finalizer, CLI or UI boundary. During
active development, use the smallest affected NL set.

Before release-sensitive deletion or runtime behavior release:

```bash
python3 scripts/nl_tests/build_release_gate_subset.py --check
bash scripts/nl_tests/run_suite.sh agent_parity_gate
```

The generated subset is the source of truth for row/category counts. Do not
hardcode an old count in rollout policy.

## Rollback Thresholds

Stop and revert the responsible coherent change when evidence shows:

- any confirmation, permission, sandbox or dry-run bypass;
- any non-idempotent side effect replay without reconciliation;
- new unknown production NL hard match or fixed runtime reply;
- unexplained route/plan/permission/verifier/final-status replay mismatch;
- statistically meaningful pass-rate, verifier false-block, LLM amplification
  or latency regression against a comparable baseline;
- lost checkpoint, lease fencing failure, or inability to resume safely.

Rollback procedure:

1. Restore a wired policy value only when that value is the true behavior
   owner.
2. Otherwise revert the responsible coherent code change without resetting
   unrelated user work.
3. Restart with known-good config/environment.
4. Re-run the smallest failing cases and affected deterministic gates.
5. Record run refs, machine reason/status, changed owner and follow-up.

## Current Risk Areas

- `repo/tasks.rs` and `UI/src/App.tsx` are near the 2,000-line ceiling.
- Exact-output finalization must remain zero-domain and independent of registry
  skill names.
- Provider usage/cost can be unknown; unknown cost records must not be treated
  as zero cost.
- Long-tail tools need heartbeat/checkpoint/async state before pause/resume is
  considered safe.
- Legacy route fields still present in historical fixtures must remain isolated
  from current execution.
- Registry main/Docker copies must remain in parity.
- Paid multimedia and remote mutation tests need explicit safe live scope;
  otherwise use dry-run/offline contracts.

## Required Supporting Guards

Use the guards relevant to the changed surface:

```bash
python3 scripts/check_planner_runtime_boundary.py
python3 scripts/check_pre_planner_exit_inventory.py
python3 scripts/check_finalizer_architecture.py
python3 scripts/check_repair_boundary_inventory_coverage.py
python3 scripts/check_repair_no_user_text_fields.py
python3 scripts/check_policy_decision_tokens.py
python3 scripts/check_registry_policy_contracts.py
python3 scripts/check_skill_registry_aliases.py
python3 scripts/check_skill_registry_parity.py --mode all --strict
python3 scripts/check_cross_platform_contracts.py
```

# Agent Loop Pre-Agent Decision Inventory

Last updated: 2026-07-24

This inventory records the current machine work before the first ordinary
planner call. The old intent normalizer, contract-repair judge, post-route
semantic policy, `AskMode`, and route-authority switches are physically absent
from the live ask path.

## Current Authority Model

- Every ordinary `kind=ask` request enters `agent_engine::run_agent_with_tools()`.
- The first planner action owns ordinary respond/clarify/execute/capability
  semantics.
- Boundary code may materialize inputs and context, validate explicit protocol
  modes, enforce safety, and apply administrator ceilings.
- Historical route fields may be read from archived fixtures and logs, but
  cannot be written as new route authority or consumed by execution.
- No runtime layer may select behavior by matching user-language phrases or
  localized `text` / `error_text`.

## Main `kind=ask` Path

1. `worker::worker_once()`
2. `worker::process_ask_task()`
3. `worker::ask_input::prepare_ask_input()`
4. `worker::ask_planner_frontdoor::prepare_planner_owned_ask_routing()`
5. `worker::ask_execution_context::prepare_ask_execution_context()`
6. `worker::ask_runtime::execute_ask_dispatch()`
7. `agent_engine::run_agent_with_tools()`
8. `finalize::finalize_ask_result()`

## Decision Surface Inventory

| Surface | Current owner | Role | May bypass the ordinary planner |
| --- | --- | --- | --- |
| Task claim and kind dispatch | `worker_once()` | Lease, heartbeat, timeout, cancellation, `ask` / `run_skill` dispatch | Yes, because this is task protocol |
| Explicit capability payload | `run_capability` | Execute a caller-supplied machine capability contract | Yes, only for the explicit payload |
| Explicit schedule direct text | `maybe_finalize_schedule_direct_text_success()` | Deliver `schedule_task_mode=direct_text` without semantic inference | Yes, only for explicit scheduler metadata |
| Input materialization | `prepare_ask_input()` and planner frontdoor | Text, audio transcription, attachment refs, explicit command/locator facts | No semantic bypass |
| Session/context construction | task context builder and `prepare_ask_execution_context()` | Memory, knowledge, image context, aliases, compacted history, initial observations | No semantic bypass |
| Planner loop | `run_agent_with_tools()` | Respond, clarify, plan, call capabilities/tools/skills, observe, repair, synthesize | This is the ordinary authority |
| Resolver/verifier/policy | capability resolver, verifier, hooks, permission runtime | Validate machine contracts and block unsafe execution | May block or require confirmation, but may not reinterpret intent |
| Finalizer/delivery | `finalize/`, channel adapters | Exact serialization, grounded model synthesis, persistence, channel delivery | May preserve exact machine output; may not route by language or skill name |

## Work Kept Outside The Agent Loop

- Task lease, heartbeat, timeout, cancellation, queue and kind dispatch.
- Authentication, actor/session/channel binding and workspace scope.
- Attachment materialization, transcription, image preprocessing and bounded
  context compaction.
- Registry visibility, schema validation, permissions, risk, confirmation,
  dry-run, sandbox and side-effect policy.
- Path confinement, explicit locator facts and artifact safety.
- Task-budget soft slices, administrator hard ceilings, repetition and
  structured stagnation guards.
- Evidence admission, Answer Verifier, exact selector serialization, secret
  redaction and delivery persistence.

## Semantic Decisions In The Agent Loop

- Respond, clarify, execute, continue, wait or stop for an ordinary request.
- Select a capability/action from task meaning.
- Decide whether missing information is a semantic blocker.
- Recover from structured tool/provider/verifier observations.
- Synthesize user-visible language from grounded evidence.

## Allowed Pre-Planner Provider Calls

Audio transcription, image analysis, and model-assisted context compaction may
run before the planner when their structured trigger is present. They only
produce input/context evidence. Their outputs cannot select an ordinary route
or final response.

## Deleted Surfaces

The following live surfaces must not return:

- intent normalizer route authority;
- pre-route contract-repair semantic judge;
- post-route semantic policy;
- active-clarify route shortcut;
- pre-planner direct-answer gate;
- direct existing-file semantic shortcut;
- `agent_decides_semantic_route`, migration-class, canary, or rollback route
  switches.

Archived references remain acceptable only in isolated fixtures, replay
readers, migration inventories, and guard self-tests.

## Validation Gates

```bash
python3 scripts/check_planner_runtime_boundary.py
python3 scripts/check_pre_planner_exit_inventory.py
python3 scripts/check_route_authority_legacy_keys.py
python3 scripts/check_legacy_route_boundary.py
python3 scripts/check_no_nl_hardmatch.py
python3 scripts/check_no_runtime_hard_reply.py
python3 scripts/check_long_files.py
cargo test -p clawd ask_runtime -- --nocapture
cargo test -p clawd planner_frontdoor -- --nocapture
```

Behavior changes also require the smallest affected NL set during development
and release-gate-equivalent coverage before release-sensitive deletion.

## Multilingual Reinforcement

<!--
zh-CN: 本 inventory 区分 planner 前必须保留的机器边界与 agent loop 拥有的普通语义决策；不得以自然语言短语匹配实现迁移。
en: This inventory separates machine boundaries that remain before planning from ordinary semantic decisions owned by the agent loop; natural-language phrase matching must not implement the migration.
-->

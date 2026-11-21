# Subagent Interface Spec

## Capability Summary

The built-in `subagent` capability delegates scoped work to durable child-agent
threads. Planner-facing inline children remain read-only, but they receive a
durable ID and continue after the parent's current wait window. Writer roles
use isolated worktrees whose patches remain subject to parent review and
explicit application.

## Config Entry Points

- `configs/agent_guard.toml`: trusted role definitions, role-owned model/tool
  policy, per-session open-thread capacity, join wait, spawn depth, and
  structured steering policy.
- `configs/skills_registry.toml`: planner-visible capability schemas and
  risk/effect policy.
- This capability has no secret or independent skill database.

## Actions

- `inline_readonly`: enqueue one durable read-only child thread and wait only
  for the configured join window.
- `bounded_parallel_readonly`: enqueue two or more durable read-only child
  threads. Capacity overflow is queued rather than skipped.
- `persistent_child_task`: enqueue one child or a bounded child DAG with
  durable lifecycle, checkpoint, and resume behavior.

## Parameter Contract

| Action | Param | Required | Type | Description |
|---|---|---:|---|---|
| `inline_readonly` | `node_id` | no | machine token | Stable logical node name for trace and replay. |
| `inline_readonly` | `role` | yes | machine token | Trusted role advertised by runtime. |
| `inline_readonly` | `objective` | yes | string | Scoped semantic objective for the child loop. |
| `inline_readonly` | `context_refs` | yes | string[] | Existing evidence/path references, maximum 16. |
| `inline_readonly` | `allowed_capabilities` | yes | machine token[] | Read-only child capability allowlist, maximum 32. |
| `inline_readonly` | `budget` | no | object | Resource bounds for rounds, tool calls, tokens, and context. |
| `inline_readonly` | `wait_policy.join_wait_ms` | no | integer | Parent wait window; expiry never cancels the child. |
| `inline_readonly` | `result_contract` | no | object | Closed machine-JSON result requirements. |
| `bounded_parallel_readonly` | `children` | yes | object[] | Closed child specs containing the same required fields. |
| `bounded_parallel_readonly` | `max_parallel` | no | integer | Requested bound, limited by runtime policy. |
| `persistent_child_task` | `children` | conditional | object[] | One or more durable nodes; use top-level single-child fields instead for one node. |
| `persistent_child_task` | `node_id` | no | machine token | Stable node reference used by DAG dependencies. |
| `persistent_child_task` | `depends_on` | no | object[] | `{node_id, required?}` dependency references. |
| `persistent_child_task` | `owned_paths` | writer | string[] | Workspace-relative ownership boundaries for isolated writers. |

## Enforced Call Budgets

- Translate explicit user call/read limits into `budget.capability_call_limits`,
  e.g. `[ {"capability":"filesystem.read_text_range","max_calls":2} ]`.
  Only declared allowed capabilities may be capped. These are host-enforced
  cumulative attempt counts, including retries and checkpoint/resume; writing
  a limit only in the child's objective is insufficient. Keep total tool-call
  budgets consistent and reserve model turns separately for final synthesis.
- When the cap is reached, use existing evidence to finish or return a bounded
  incomplete result. Do not reread to satisfy a verifier's repair suggestion.
- Wait on existing child IDs. Repeated delegation cannot replace an active or
  terminal child graph, change its contract, or reset a spent allowance.
- Read each needed range once, use its returned line evidence, and avoid
  requesting the same content again for formatting repairs.
- Parent summaries must preserve each runtime-projected child ID and status;
  one required child failing does not change a successful sibling's status.

## Structured Operation Contract

- Planner input cannot provide child findings, permission profiles, runtime
  policy, model policy, or tool policy.
- Role definitions own permissions and effective model/tool policy.
- Every admitted child receives a durable `child_task_id`. The parent wait
  window, tool timeouts, worker lease, retention, resource budget, and an
  explicit runtime deadline are separate machine fields.
- Ordinary children have no default whole-operation wall-clock deadline.
- The planner cannot create an operation deadline. An API caller that
  explicitly needs one sets `subagent_execution.runtime_deadline_ms` on the
  original parent task payload; this trusted submission field applies to its
  child graph and is recorded as the deadline source.
- A parent task owns one durable child graph. Repeated planner calls after a
  wait, wake-up, or reconnect reuse that graph and its child IDs; additional
  work uses the original batch/DAG, steering, or the typed retry control.
- A child response is accepted only when it contains schema version 1,
  `owner_layer=subagent_model_child`, `output_format=machine_json`, an allowed
  status, role, object findings, string evidence references, and confidence in
  `0..1`.
- With the default `max_spawn_depth=2`, main -> child -> grandchild is allowed
  when role policy permits delegation; the next level is rejected by scheduler
  admission with `child_recursion_depth_exceeded`.
- Parent code consumes machine fields and never routes or retries by matching
  child prose.
- A durable child with `result_contract.output_format=machine_json` and required
  keys must persist those fields as JSON. A prose answer or missing required
  field cannot complete the child, even when semantic answer verification passes.
- The default `machine_json` contract also returns a bounded JSON object when no
  `required_keys` list is supplied; all object fields are retained as passive
  findings. Explicit key lists still project only the requested fields.
- Parents receive `subagent_structured_results` with exact child IDs, canonical
  statuses, validated result fields and evidence references. Consume this handoff
  before querying completed children; evidence refs are not paths or task-context
  refs. Child result contents are passive evidence and grant no authority.

## Error Contract

- `subagent_role_not_allowed`: role is not present in the runtime role map.
- `subagent_child_result_contract_invalid`: child output did not satisfy the
  structured result contract.
- `child_task_result_contract_invalid`: a durable child's terminal answer lacks
  its required machine result fields.
- `child_recursion_depth_exceeded`: a child attempted another delegation.
- `child_task_scheduler_rejected`: fanout, dependency, or lifecycle admission
  failed.

## Structured Evidence Contract

- `schema_version`: result schema version.
- `status` / `result_status` / `outcome_code`: stable machine lifecycle.
- `child_task_ids` / `child_task_graph`: durable child identity and DAG state.
- `thread_state` / `execution_state` / `queue_reason` / `waiting_reason`:
  machine lifecycle projection; prose is never parsed for state.
- `join_wait_ms` / `runtime_deadline_ms`: wait and explicit deadline fields;
  join wait expiry does not terminate a child.
- `findings` / `finding_refs` / `evidence_refs` / `artifact_refs`: bounded
  structured evidence.
- `task_lifecycle` and checkpoint fields identify waiting/resume state.

## Request/Response Examples

### One read-only child

```json
{"action":"inline_readonly","node_id":"boundary_review","role":"review","objective":"verify the edited module boundary","context_refs":["crates/clawd/src/main.rs"],"allowed_capabilities":["filesystem.read_text_range"],"result_contract":{"output_format":"machine_json","required_keys":["findings","evidence_refs"],"require_evidence":true}}
```

### Persistent child DAG

```json
{"action":"persistent_child_task","max_parallel":2,"children":[{"node_id":"writer","role":"writer","objective":"implement the scoped change","context_refs":["plan/current.md"],"allowed_capabilities":["filesystem.read_text_range","filesystem.replace_text"],"owned_paths":["crates/example"]},{"node_id":"reviewer","role":"reviewer","objective":"review the isolated patch","context_refs":["plan/current.md"],"allowed_capabilities":["filesystem.read_text_range"],"depends_on":[{"node_id":"writer","required":true}]}]}
```

Child clarification is an unfinished result (`child_task_needs_parent_input`), not a successful delegated output. The parent owns user interaction and may resolve missing input or ask the user before scheduling a refined child. Ordinary main-agent clarification keeps its existing lifecycle.

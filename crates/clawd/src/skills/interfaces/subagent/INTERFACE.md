# Subagent Interface Spec

## Capability Summary

The built-in `subagent` capability delegates bounded work to a child agent
loop. Inline children are read-only and evidence-scoped. Persistent children
run as durable tasks; writer roles use isolated worktrees whose patches remain
subject to parent review and explicit application.

## Config Entry Points

- `configs/agent_guard.toml`: trusted role definitions, role-owned model/tool
  policy, default timeout, and inline parallel limit.
- `configs/skills_registry.toml`: planner-visible capability schemas and
  risk/effect policy.
- This capability has no secret or independent skill database.

## Actions

- `inline_readonly`: execute one bounded read-only child agent loop.
- `bounded_parallel_readonly`: execute two or more bounded read-only child
  loops and aggregate their structured results.
- `persistent_child_task`: enqueue one child or a bounded child DAG with
  durable lifecycle, checkpoint, and resume behavior.

## Parameter Contract

| Action | Param | Required | Type | Description |
|---|---|---:|---|---|
| `inline_readonly` | `role` | yes | machine token | Trusted role advertised by runtime. |
| `inline_readonly` | `objective` | yes | string | Scoped semantic objective for the child loop. |
| `inline_readonly` | `context_refs` | yes | string[] | Existing evidence/path references, maximum 16. |
| `inline_readonly` | `allowed_capabilities` | yes | machine token[] | Read-only child capability allowlist, maximum 32. |
| `inline_readonly` | `budget` | no | object | Bounded rounds, tool calls, context chars, and timeout. |
| `inline_readonly` | `result_contract` | no | object | Closed machine-JSON result requirements. |
| `bounded_parallel_readonly` | `children` | yes | object[] | Closed child specs containing the same required fields. |
| `bounded_parallel_readonly` | `max_parallel` | no | integer | Requested bound, limited by runtime policy. |
| `persistent_child_task` | `children` | conditional | object[] | One or more durable nodes; use top-level single-child fields instead for one node. |
| `persistent_child_task` | `node_id` | no | machine token | Stable node reference used by DAG dependencies. |
| `persistent_child_task` | `depends_on` | no | object[] | `{node_id, required?}` dependency references. |
| `persistent_child_task` | `owned_paths` | writer | string[] | Workspace-relative ownership boundaries for isolated writers. |

## Structured Operation Contract

- Planner input cannot provide child findings, permission profiles, runtime
  policy, model policy, or tool policy.
- Role definitions own permissions and effective model/tool policy.
- A child response is accepted only when it contains schema version 1,
  `owner_layer=subagent_model_child`, `output_format=machine_json`, an allowed
  status, role, object findings, string evidence references, and confidence in
  `0..1`.
- Child agents cannot recursively delegate beyond the runtime child-depth
  bound.
- Parent code consumes machine fields and never routes or retries by matching
  child prose.

## Error Contract

- `subagent_role_not_allowed`: role is not present in the runtime role map.
- `subagent_child_result_contract_invalid`: child output did not satisfy the
  structured result contract.
- `child_recursion_depth_exceeded`: a child attempted another delegation.
- `child_task_scheduler_rejected`: fanout, dependency, or lifecycle admission
  failed.

## Structured Evidence Contract

- `schema_version`: result schema version.
- `status` / `result_status` / `outcome_code`: stable machine lifecycle.
- `child_task_ids` / `child_task_graph`: durable child identity and DAG state.
- `findings` / `finding_refs` / `evidence_refs` / `artifact_refs`: bounded
  structured evidence.
- `task_lifecycle` and checkpoint fields identify waiting/resume state.

## Request/Response Examples

### One read-only child

```json
{"action":"inline_readonly","role":"review","objective":"verify the edited module boundary","context_refs":["crates/clawd/src/main.rs"],"allowed_capabilities":["filesystem.read_text_range"],"result_contract":{"output_format":"machine_json","required_keys":["findings","evidence_refs"],"require_evidence":true}}
```

### Persistent child DAG

```json
{"action":"persistent_child_task","max_parallel":2,"children":[{"node_id":"writer","role":"writer","objective":"implement the scoped change","context_refs":["plan/current.md"],"allowed_capabilities":["filesystem.read_text_range","filesystem.replace_text"],"owned_paths":["crates/example"]},{"node_id":"reviewer","role":"reviewer","objective":"review the isolated patch","context_refs":["plan/current.md"],"allowed_capabilities":["filesystem.read_text_range"],"depends_on":[{"node_id":"writer","required":true}]}]}
```

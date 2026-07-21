# Durable Child Task Graph Contract

RustClaw persists planner-authorized subagent work as a task graph. The graph is
an execution contract, not a semantic router: the model proposes roles,
objectives, dependencies, and structured scope, while the runtime validates and
enforces trusted role policy, permissions, ownership, readiness, and lifecycle.

## Persisted Records

- `child_task_graphs` stores the parent task, schema version, graph status, and
  maximum parallel children.
- `child_task_graph_nodes` stores each child task ID, trusted role token,
  required flag, readiness, permission and merge policies, owned workspace
  paths, budget, model/tool policy, result contract, and versioned steering.
- `child_task_graph_edges` stores declared and runtime-added dependencies.

Graph and child task rows are admitted in one SQLite transaction. Admission
rejects missing nodes, self-dependencies, cycles, invalid workspace paths, and
untrusted role or permission combinations.

## Readiness And Ownership

Node readiness uses stable machine states:

- `ready`
- `blocked_dependency`
- `blocked_capacity`
- `running`
- terminal task states such as `succeeded`, `failed`, `timeout`, or `canceled`

The queue claims only ready nodes. A required predecessor failure cancels
dependent work; an optional failure does not automatically fail unrelated
nodes. Capacity and dependency changes are reconciled after every terminal
transition.

Write-enabled children must declare workspace-relative `owned_paths`. Missing
ownership is normalized to the workspace root. Equal or ancestor-overlapping
paths are serialized with a deterministic edge; disjoint isolated worktree
writers may run concurrently. Patch admission rechecks every changed file
against the persisted ownership record and rejects stale, missing, or
out-of-scope ownership.

## Roles And Policy

Trusted role definitions live in `configs/agent_guard.toml`. A definition binds
the role token to a role family, default and allowed permission profiles,
result-contract requirements, and optional model/tool policy. The model may
select a trusted token but cannot create permissions or widen policy.

Each persisted node records its effective role, permission, budget, model/tool
policy, merge policy, and result contract. Runtime code must not match user
language to choose a role.

## Restart, Retry, And Control

- Startup reconciliation derives node readiness from authoritative task rows.
- An expired child claim is requeued and receives a newer claim generation on
  its next claim.
- Retrying a terminal child creates a new task/node and transactionally rewires
  incoming and outgoing graph edges.
- Pause/resume input can record a compare-and-swap steering directive with
  checkpoint, trigger, user input, and structured constraints.
- Parent failure, timeout, or cancellation cancels unfinished children and
  closes the graph with a machine status.

## Events And Results

The runtime emits versioned `subagent_graph` snapshots on the parent task and
`subagent_node` snapshots on child transitions. A graph snapshot contains:

- parent/child task IDs and dependencies;
- readiness, role, required flag, permissions, ownership, and merge policy;
- budget, model/tool policy, result contract, and steering version;
- child status, structured result, evidence/artifact/patch/findings refs;
- token/cost usage when reported.

CLI event/replay commands preserve these payloads. The UI task trace presents a
compact graph or node summary and keeps raw JSON behind progressive disclosure.
Consumers must use these machine fields and must not parse child prose to infer
readiness, success, permissions, or merge eligibility.

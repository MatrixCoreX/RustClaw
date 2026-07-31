# Task Plan Interface Spec

## Capability Summary

The built-in `task_plan` host tool stores one revisioned, machine-readable
execution plan for the current task. The snapshot is durable in the runtime
database and is projected as data-only task evidence.

## Actions

- `set_plan`: create or replace the ordered plan at an expected revision.
- `update_steps`: update existing steps by stable `step_id`.
- `read_plan`: read the current snapshot.

## Parameter Contract

| Action | Param | Required | Type | Description |
|---|---|---:|---|---|
| `set_plan` | `plan_revision` | no | non-negative integer | Expected current revision; defaults to `0` for initial creation. |
| `set_plan` | `steps` | yes | object[] | Ordered steps with `step_id`, `title`, and `status`. |
| `update_steps` | `plan_revision` | yes | non-negative integer | Expected current revision. |
| `update_steps` | `updates` | yes | object[] | Existing `step_id` plus a new `title` and/or `status`. |
| `read_plan` | — | — | — | Takes no additional arguments. |

Step status is one of `pending`, `in_progress`, `completed`, or `cancelled`.
Step IDs are unique and stable. At most one step may be `in_progress`.

## Response Contract

Successful responses contain:

- `schema_version=1`, `source=task_plan`, `status=ok`, and the action.
- Current `task_id`, `plan_revision`, `updated_at_ms`, and ordered `steps`.
- A `checkpoint` reference in the form `task_plan:<task_id>:<revision>` when a
  plan exists.

Writes also publish a `task_plan_updated` task event with `data_only=true` and
`render_owner=ui_cli_channel_projection`.

## Error Contract

- `task_plan_invalid`: malformed, duplicate, unknown, or conflicting step
  content.
- `task_plan_revision_required`: an update omitted its expected revision.
- `task_plan_revision_conflict`: expected and current revisions differ;
  `retryable=true` and both revisions are returned.
- `task_plan_event_publish_failed`: persistence succeeded but task-event
  projection failed; read the current snapshot before retrying.

## Request/Response Examples

```json
{"action":"set_plan","plan_revision":0,"steps":[{"step_id":"inspect","title":"Inspect current state","status":"in_progress"},{"step_id":"implement","title":"Implement the change","status":"pending"}]}
```

```json
{"action":"update_steps","plan_revision":1,"updates":[{"step_id":"inspect","status":"completed"},{"step_id":"implement","status":"in_progress"}]}
```

```json
{"schema_version":1,"source":"task_plan","status":"ok","action":"read_plan","task_id":"task-1","plan_revision":2,"updated_at_ms":123,"steps":[{"step_id":"inspect","title":"Inspect current state","status":"completed"},{"step_id":"implement","title":"Implement the change","status":"in_progress"}],"checkpoint":{"kind":"task_plan","ref":"task_plan:task-1:2","plan_revision":2}}
```

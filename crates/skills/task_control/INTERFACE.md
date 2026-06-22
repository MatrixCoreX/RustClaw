# task_control Interface Spec

## Capability Summary

- `task_control` lets the current user inspect unfinished tasks in the current chat, query a task detail by `task_id`, and cancel unfinished tasks safely.
- Scope is limited to the caller's own `queued` and `running` tasks in the current chat.
- Supports natural-language task operations such as "看看我现在有哪些任务", "结束当前任务", and "结束第 2 个任务".
- When a `user_key` is present in the runner request/context, it is forwarded to `clawd` for authenticated task queries and cancellations.

## Actions

- `list` - List current unfinished tasks (`running` + `queued`) for this user/chat.
- `list_with_first_detail` - List current unfinished tasks and, when at least one exists, fetch detail for the first listed task so lifecycle field visibility can be answered from structured data.
- `get` - Query one task detail by stable `task_id`, including `data.lifecycle` machine fields when available.
- `cancel_all` - Cancel all unfinished tasks for this user/chat, excluding the current control task itself.
- `cancel_one` - Cancel one unfinished task by 1-based index from the current active-task ordering.

## Parameter Contract

| Param | Required | Type | Default | Description |
|---|---|---|---|---|
| `action` | yes | string | - | One of: `list`, `list_with_first_detail`, `get`, `cancel_all`, `cancel_one`. |
| `task_id` | required for `get` | string | - | Stable RustClaw task id, usually a UUID. |
| `index` | required for `cancel_one` | number | - | 1-based active-task index. |
| `dry_run` | optional for cancel actions | boolean | `false` | Return a no-mutation cancellation preview with required fields and projected lifecycle fields. |

Notes:

- Active-task ordering is: `running` first, then `queued`, then oldest first.
- The control task itself is excluded automatically, so users do not accidentally cancel the task that is serving the request.

## Output Contract

- Human-readable text.
- `list` returns numbered tasks.
- `list_with_first_detail` returns compact JSON text and `extra.field_value` with list fields, selected `task_id`, detail availability, `db_status`, `lifecycle`, and `lifecycle_field_presence` booleans for `state`, `can_poll`, `can_cancel`, `last_heartbeat_ts`, and `checkpoint_id`.
- `get` returns a compact JSON text and `extra` object with `action=get`, `task_id`, `db_status`, and `lifecycle`.
- `cancel_all` returns canceled count and a short summary list.
- `cancel_one` returns the canceled task number and summary.

## Error Contract

- Unknown action -> readable error text.
- `get` without `task_id` -> structured `status=missing_task_id` with lifecycle field slots.
- `get` with an invalid `task_id` shape -> structured `status=invalid_task_id` with lifecycle field slots.
- `cancel_one` without valid `index` -> readable error text.
- Invalid index -> readable error text telling the user to query tasks first.
- Missing/invalid auth for task APIs -> readable error text from `clawd` (for example unauthorized user or invalid user key).

## Request/Response Examples

### list

Request:
```json
{"request_id":"r1","args":{"action":"list"},"user_id":1,"chat_id":2}
```

Response text example:
```text
当前未完成任务（2 个）：
1. [running][ask] 查看最近币圈新闻（已运行 18s）
2. [queued][run_skill] run_skill:stock（已运行 3s）
```

### cancel_all

Request:
```json
{"request_id":"r2","args":{"action":"cancel_all"},"user_id":1,"chat_id":2}
```

### get

Request:
```json
{"request_id":"r4","args":{"action":"get","task_id":"00000000-0000-4000-8000-000000000000"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"get","task_id":"00000000-0000-4000-8000-000000000000","db_status":"succeeded","lifecycle":{"state":"succeeded","can_poll":true,"can_cancel":false}}
```

### list_with_first_detail

Request:
```json
{"request_id":"r6","args":{"action":"list_with_first_detail"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"list_with_first_detail","status":"ok","count":1,"selected_task_id":"00000000-0000-4000-8000-000000000000","field_value":{"detail_available":true,"db_status":"running","lifecycle_field_presence":{"state":true,"can_poll":true,"can_cancel":true,"last_heartbeat_ts":true,"checkpoint_id":false}}}
```

### cancel_one

Request:
```json
{"request_id":"r3","args":{"action":"cancel_one","index":2},"user_id":1,"chat_id":2}
```

### cancel dry-run

Request:
```json
{"request_id":"r5","args":{"action":"cancel_all","dry_run":true},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"cancel_all","status":"dry_run","would_mutate":false,"required_fields":["task_id","state","can_cancel"],"result_projection_fields":{"state":"cancel_requested_or_canceled","can_cancel":false,"can_poll":true,"db_status":"canceled_or_terminal"}}
```

<!-- AUTO-GENERATED: sync_skill_docs.py -->
## Role & Boundaries
- You are the `task_control` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `crates/skills/task_control/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
- `task_control` lets the current user inspect unfinished tasks in the current chat and cancel them safely.
- Scope is limited to the caller's own `queued` and `running` tasks in the current chat.
- Supports natural-language task operations such as "看看我现在有哪些任务", "结束当前任务", and "结束第 2 个任务".

## Actions (from interface)
- `list` - List current unfinished tasks (`running` + `queued`) for this user/chat.
- `cancel_all` - Cancel all unfinished tasks for this user/chat, excluding the current control task itself.
- `cancel_one` - Cancel one unfinished task by 1-based index from the current active-task ordering.

## Parameter Contract (from interface)
| Param | Required | Type | Default | Description |
|---|---|---|---|---|
| `action` | yes | string | - | One of: `list`, `cancel_all`, `cancel_one`. |
| `index` | required for `cancel_one` | number | - | 1-based active-task index. |

Notes:

- Active-task ordering is: `running` first, then `queued`, then oldest first.
- The control task itself is excluded automatically, so users do not accidentally cancel the task that is serving the request.

## Error Contract (from interface)
- Unknown action -> readable error text.
- `cancel_one` without valid `index` -> readable error text.
- Invalid index -> readable error text telling the user to query tasks first.

## Request/Response Examples (from interface)
### list

Request:
```json
{"request_id":"r1","args":{"action":"list"},"user_id":1,"chat_id":2}
```

Response text example:
```text
当前未完成任务（2 个）：
1. [running][ask] 查看最近币圈新闻（已运行 18s）
2. [queued][run_skill] run_skill:chat（已运行 3s）
```

### cancel_all

Request:
```json
{"request_id":"r2","args":{"action":"cancel_all"},"user_id":1,"chat_id":2}
```

### cancel_one

Request:
```json
{"request_id":"r3","args":{"action":"cancel_one","index":2},"user_id":1,"chat_id":2}
```

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.

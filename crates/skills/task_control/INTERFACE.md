# task_control Interface Spec

## Capability Summary

- Task lifecycle, structured session-alias binding, provider-failure, retryable-failure observation, and side-effect-free coding-repair previews.
- `task_control` lets the current user inspect unfinished tasks in the current chat, query a task detail by `task_id`, cancel unfinished tasks safely, resume or pause checkpointed long-running tasks, bind a planner-selected alias to a concrete target for later turns, inspect provider-failure recovery policy, preview the structured observation supplied after a retryable tool failure, and preview a structured coding repair loop without side effects.
- Scope is limited to the caller's own `queued` and `running` tasks in the current chat.
- Planner-facing selection should use structured capability/action fields from the registry. Do not add phrase-specific routing rules for any user language.
- Task cancel/resume/pause dry-run or lifecycle-field preview requests must call the skill with `dry_run=true` and return observed machine fields. Do not answer those flows from static prose alone, even when the user has not supplied a concrete `task_id` or index.
- When a `user_key` is present in the runner request/context, it is forwarded to `clawd` for authenticated task queries, cancellations, resume, and pause actions.

## Actions

- `list` - List current unfinished tasks (`running` + `queued`) for this user/chat.
- `list_with_first_detail` - List current unfinished tasks and, when at least one exists, fetch detail for the first listed task so lifecycle field visibility can be answered from structured data.
- `get` - Query one task detail by stable `task_id`, including `data.lifecycle` machine fields when available.
- `cancel_all` - Cancel all unfinished tasks for this user/chat, excluding the current control task itself.
- `cancel_one` - Cancel one unfinished task by 1-based index from the current active-task ordering.
- `preview_resume` - Return the no-mutation resume entrypoint and renewable execution-lease contract for a stable `task_id`.
- `preview_provider_failure` - Return the shared no-mutation provider failure, retry, waiting-state, and checkpoint policy for a canonical `failure_class`.
- `preview_retryable_failure_observation` - Return a synthetic no-mutation machine contract for a retryable tool failure, including the stable error, recovery, repeat-prevention, and bounded-attempt fields consumed by the planner.
- `preview_coding_repair` - Return a synthetic no-mutation coding-loop contract containing checkpoint, diff, failed verification, repair attempt, passing verification, and rewind references.
- `bind_session_alias` - Return an exact structured `session_alias_bindings` update for a planner-selected `alias` and `target`; the runtime persists only this machine result and does not infer bindings from user-language phrases.
- `resume` - Mark an existing checkpointed task due for recovery by stable `task_id`.
- `pause` - Delay an existing waiting/background checkpoint by stable `task_id`.
- Cancellation dry-runs are executable observations, not static prose: use `cancel_all` with `dry_run=true` when no specific index is supplied, or `cancel_one` with both `index` and `dry_run=true` when the user supplied a numbered task.

## Parameter Contract

| Param | Required | Type | Default | Description |
|---|---|---|---|---|
| `action` | yes | string | - | One of: `list`, `list_with_first_detail`, `get`, `cancel_all`, `cancel_one`, `preview_resume`, `preview_provider_failure`, `preview_retryable_failure_observation`, `preview_coding_repair`, `bind_session_alias`, `resume`, `pause`. |
| `alias` | required for `bind_session_alias` | string | - | Exact user-defined alias surface selected by the planner; maximum 256 characters. |
| `target` | required for `bind_session_alias` | string | - | Concrete locator or stable target to retain for later turns; maximum 4096 characters. |
| `failure_class` | required for `preview_provider_failure` | string | - | One canonical provider failure token: `timeout`, `transport_retryable`, `provider_retryable_response`, `rate_limited`, `quota_exhausted`, `provider_non_retryable_business`, or `local_non_retryable`. |
| `task_id` | required for `get`, `resume`, `pause` | string | - | Stable RustClaw task id, usually a UUID. |
| `index` | required for `cancel_one` | number | - | 1-based active-task index. |
| `checkpoint_id` | optional for `preview_resume` and `resume` | string | - | Restrict resume to a specific checkpoint id. |
| `resume_reason` | optional for `resume` | string | - | Machine reason token to store with the resume request. |
| `user_message` | optional for `resume` | string | - | User follow-up text stored as resume metadata; runtime does not parse it for routing. |
| `new_constraints` | optional for `resume` | object | - | Structured constraints for the resumed task. |
| `pause_seconds` | optional for `pause` | number | `3600` | Delay duration for a checkpointed waiting/background task. |
| `dry_run` | optional for cancel/resume/pause actions | boolean | `false` | Return a no-mutation preview with required fields and projected lifecycle fields. |

Notes:

- Active-task ordering is: `running` first, then `queued`, then oldest first.
- The control task itself is excluded automatically, so users do not accidentally cancel the task that is serving the request.
- For a no-mutation cancel preview, `dry_run=true` returns the required cancellation input fields and projected lifecycle fields without calling the cancel API mutation path.

## Output Contract

- `text` is compact JSON mirroring `extra`; final user-facing prose should be generated by the finalizer/i18n layer from `message_key` and `field_value`.
- All successful actions return `extra.schema_version=1`, `extra.action`, `extra.status`, `extra.message_key`, and a `field_value` object.
- `list` returns `items` with `index`, `task_id`, `kind`, `status`, `summary`, and `age_seconds`.
- `list_with_first_detail` returns compact JSON text and `extra.field_value` with list fields, selected `task_id`, detail availability, `db_status`, `lifecycle`, and `lifecycle_field_presence` booleans for `state`, `can_poll`, `can_cancel`, `last_heartbeat_ts`, and `checkpoint_id`.
- `get` returns a compact JSON text and `extra` object with `action=get`, `task_id`, `db_status`, and `lifecycle`.
- `cancel_all` returns `canceled_count`, `requested_count`, `items`, and `field_value.task_ids`.
- `cancel_one` returns `canceled_task` and `field_value.task_id`.
- `cancel_all` / `cancel_one` with `dry_run=true` returns `status=dry_run`, `dry_run=true`, `would_mutate=false`, `required_fields`, and `result_projection_fields` containing `state`, `can_cancel`, `can_poll`, and `db_status`.
- `preview_resume` always returns `status=dry_run`, `dry_run=true`, and `would_mutate=false`, plus `resume_entrypoint` and the renewable `lease` contract. It never calls the resume API.
- `preview_provider_failure` always returns `status=dry_run` and `would_mutate=false`, plus `failure_class`, `provider_retryable`, `provider_blocker`, `retry_policy`, `retry_after_seconds`, `waiting_state`, and checkpoint recovery fields. It reads the same machine policy used by runtime provider handling and never calls a provider.
- `preview_retryable_failure_observation` always returns `status=dry_run`, `synthetic=true`, and `would_mutate=false`. Its `observation` and `field_value` expose `retryable`, `error_code`, `recovery_action`, a redacted `forbidden_repeat_signature` presence marker, and bounded repair counters. Real repeat signatures remain internal attempt-ledger evidence and are never copied into a user-visible preview. `max_attempts` and `remaining_attempts` remain `null` because production limits come from the active runtime soft budget rather than a fixed skill constant.
- `preview_coding_repair` always returns `status=dry_run`, `synthetic=true`, `would_mutate=false`, and `would_execute_command=false`. Its structured fields model the fail -> repair -> pass lifecycle without claiming that a real repository, patch, command, or test was executed.
- `bind_session_alias` returns `session_alias_bindings=[{"alias":...,"target":...}]` in `extra`. Conversation state consumes only a successful exact `session.bind_alias` / `bind_session_alias` capability result; `text` and `error_text` are never parsed for state updates.
- `resume` returns the local task-control API response under `response`, plus projected `task_id`, `db_status`, `lifecycle`, and `field_value`.
- `pause` returns the local task-control API response under `response`, plus projected `task_id`, `db_status`, `lifecycle`, and `field_value`.

## Error Contract

- Unknown action -> `error_text=unsupported_action`.
- `get` without `task_id` -> structured `status=missing_task_id` with lifecycle field slots.
- `get` with an invalid `task_id` shape -> structured `status=invalid_task_id` with lifecycle field slots.
- `cancel_one` without valid `index` -> `error_text=cancel_one_missing_index`.
- `preview_resume` / `resume` / `pause` without `task_id` -> structured `status=missing_task_id`.
- `preview_resume` / `resume` / `pause` with an invalid `task_id` shape -> structured `status=invalid_task_id`.
- `preview_provider_failure` without `failure_class` -> structured `status=missing_failure_class`.
- `preview_provider_failure` with a non-canonical token -> structured `status=unsupported_failure_class`.
- `bind_session_alias` without `alias` or `target` -> `error_text=bind_session_alias_missing_alias|bind_session_alias_missing_target`.
- `bind_session_alias` with an oversized value -> `error_text=bind_session_alias_value_too_long`.
- Invalid index -> structured `clawd` API error propagated as `error_text`.
- Missing/invalid auth for task APIs -> readable error text from `clawd` (for example unauthorized user or invalid user key).

## Request/Response Examples

### bind_session_alias

Request:
```json
{"request_id":"alias-1","args":{"action":"bind_session_alias","alias":"release note","target":"document/release.md"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"bind_session_alias","status":"ok","message_key":"task_control.bind_session_alias.ok","session_alias_bindings":[{"alias":"release note","target":"document/release.md"}],"field_value":{"action":"bind_session_alias","status":"ok","alias":"release note","target":"document/release.md"}}
```

### list

Request:
```json
{"request_id":"r1","args":{"action":"list"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"list","status":"ok","message_key":"task_control.list.ok","count":1,"task_count":1,"has_unfinished":true,"items":[{"index":1,"task_id":"00000000-0000-4000-8000-000000000000","kind":"ask","status":"running","summary":"task-summary","age_seconds":18}],"field_value":{"action":"list","status":"ok","message_key":"task_control.list.ok","count":1,"task_count":1,"has_unfinished":true}}
```

### cancel_all

Request:
```json
{"request_id":"r2","args":{"action":"cancel_all"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"cancel_all","status":"ok","message_key":"task_control.cancel_all.ok","canceled_count":1,"requested_count":1,"items":[{"index":1,"task_id":"00000000-0000-4000-8000-000000000000","kind":"ask","status":"running","summary":"task-summary","age_seconds":18}],"field_value":{"action":"cancel_all","status":"ok","message_key":"task_control.cancel_all.ok","canceled_count":1,"requested_count":1,"task_ids":["00000000-0000-4000-8000-000000000000"]}}
```

### get

Request:
```json
{"request_id":"r4","args":{"action":"get","task_id":"00000000-0000-4000-8000-000000000000"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"get","status":"succeeded","message_key":"task_control.get.ok","task_id":"00000000-0000-4000-8000-000000000000","db_status":"succeeded","lifecycle":{"state":"succeeded","can_poll":true,"can_cancel":false}}
```

### list_with_first_detail

Request:
```json
{"request_id":"r6","args":{"action":"list_with_first_detail"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"list_with_first_detail","status":"ok","message_key":"task_control.list_with_first_detail.ok","count":1,"selected_task_id":"00000000-0000-4000-8000-000000000000","field_value":{"detail_available":true,"db_status":"running","lifecycle_field_presence":{"state":true,"can_poll":true,"can_cancel":true,"last_heartbeat_ts":true,"checkpoint_id":false}}}
```

### cancel_one

Request:
```json
{"request_id":"r3","args":{"action":"cancel_one","index":2},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"cancel_one","status":"ok","message_key":"task_control.cancel_one.ok","canceled_task":{"index":2,"task_id":"00000000-0000-4000-8000-000000000000","kind":"ask","status":"running","summary":"task-summary","age_seconds":18},"field_value":{"action":"cancel_one","status":"ok","message_key":"task_control.cancel_one.ok","index":2,"task_id":"00000000-0000-4000-8000-000000000000","db_status":"running"}}
```

### cancel dry-run

Request:
```json
{"request_id":"r5","args":{"action":"cancel_all","dry_run":true},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"cancel_all","status":"dry_run","message_key":"task_control.cancel_all.dry_run","dry_run":true,"would_mutate":false,"required_fields":["task_id","state","can_cancel"],"result_projection_fields":{"state":"cancel_requested_or_canceled","can_cancel":false,"can_poll":true,"db_status":"canceled_or_terminal"}}
```

### resume dry-run

Request:
```json
{"request_id":"r7","args":{"action":"resume","task_id":"00000000-0000-4000-8000-000000000000","checkpoint_id":"ckpt-1","dry_run":true},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"resume","status":"dry_run","message_key":"task_control.resume.dry_run","dry_run":true,"would_mutate":false,"task_id":"00000000-0000-4000-8000-000000000000","checkpoint_id":"ckpt-1","required_fields":["task_id"],"optional_fields":["checkpoint_id","resume_reason","user_message","new_constraints"],"result_projection_fields":{"state":"running_or_background_or_terminal","db_status":"running_or_terminal","resume_due":true,"can_poll":true,"can_cancel":true,"checkpoint_id":"optional"}}
```

### resume preview

Request:
```json
{"request_id":"r9","args":{"action":"preview_resume","task_id":"00000000-0000-4000-8000-000000000000","checkpoint_id":"ckpt-1"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"preview_resume","status":"dry_run","message_key":"task_control.preview_resume.dry_run","dry_run":true,"would_mutate":false,"task_id":"00000000-0000-4000-8000-000000000000","checkpoint_id":"ckpt-1","resume_entrypoint":"checkpoint_declared","lease":{"required":true,"scope":"resume_execution","mode":"renewable","seconds_source":"runtime_config","heartbeat_renewal":true}}
```

### pause dry-run

Request:
```json
{"request_id":"r8","args":{"action":"pause","task_id":"00000000-0000-4000-8000-000000000000","pause_seconds":120,"dry_run":true},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"pause","status":"dry_run","message_key":"task_control.pause.dry_run","dry_run":true,"would_mutate":false,"task_id":"00000000-0000-4000-8000-000000000000","pause_seconds":120,"required_fields":["task_id"],"optional_fields":["pause_seconds"],"result_projection_fields":{"state":"waiting_or_background","db_status":"running","resume_due":false,"resume_wait_seconds":120,"can_poll":true,"can_cancel":true}}
```

### provider failure preview

Request:
```json
{"request_id":"r10","args":{"action":"preview_provider_failure","failure_class":"quota_exhausted"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"preview_provider_failure","status":"dry_run","message_key":"task_control.preview_provider_failure.dry_run","dry_run":true,"would_mutate":false,"failure_class":"quota_exhausted","provider_retryable":false,"provider_blocker":true,"retry_policy":"background_wait","retry_after_seconds":10800,"waiting_state":"waiting","checkpoint":{"required":true,"recovery_action":"wait_background","resume_reason":"provider_blocker_wait_background","resume_entrypoint":"next_planner_round"}}
```

### retryable failure observation preview

Request:
```json
{"request_id":"r11","args":{"action":"preview_retryable_failure_observation"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"preview_retryable_failure_observation","status":"dry_run","message_key":"task_control.preview_retryable_failure_observation.dry_run","dry_run":true,"synthetic":true,"would_mutate":false,"observation":{"retryable":true,"error_code":"tool_retryable_failure","recovery_action":"replan","forbidden_repeat_signature":"[REDACTED]","bounded_repair_attempts":{"observed_attempt_count":1,"repair_attempt_count":0,"max_attempts":null,"remaining_attempts":null,"limit_source":"runtime_soft_budget"}}}
```

### coding repair preview

Request:
```json
{"request_id":"r12","args":{"action":"preview_coding_repair"},"user_id":1,"chat_id":2}
```

Response text example:
```json
{"schema_version":1,"action":"preview_coding_repair","status":"dry_run","synthetic":true,"would_mutate":false,"would_execute_command":false,"checkpoint":{"status":"planned","checkpoint_ref":"dry_run:checkpoint:pre_patch"},"diff":{"status":"planned","diff_ref":"dry_run:diff:repair_patch"},"failed_verification":{"status":"failed","verification_ref":"dry_run:verification:first"},"repair_attempt":{"status":"planned","attempt":1,"repair_ref":"dry_run:repair:attempt_1"},"passing_verification":{"status":"passed","verification_ref":"dry_run:verification:second"},"rewind_references":["dry_run:checkpoint:pre_patch","dry_run:diff:repair_patch"]}
```

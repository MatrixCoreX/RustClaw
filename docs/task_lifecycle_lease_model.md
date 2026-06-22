# Task Lifecycle Lease Model

RustClaw currently uses existing task and checkpoint machine fields for lease and heartbeat behavior. This keeps the runtime compatible with the current SQLite schema while preserving a path to explicit worker leases later.

## Current Model

- `tasks.status` remains the coarse database state: `queued`, `running`, `succeeded`, `failed`, `canceled`, or `timeout`.
- `tasks.updated_at` is the coarse heartbeat timestamp for ordinary active tasks. Query projection exposes it as `task_lifecycle.last_heartbeat_ts` for active states.
- `task_lifecycle.state` is the user/operator state projected by `crates/clawd/src/task_lifecycle.rs`: `queued`, `running`, `waiting`, `background`, `needs_user`, `succeeded`, `failed`, or `cancelled`.
- Paused/background checkpoint recovery uses structured JSON fields in `result_json`, not natural-language text:
  - `task_lifecycle.checkpoint_id`
  - `task_lifecycle.next_check_after`
  - `task_lifecycle.resume_executor.executor_state`
  - `task_lifecycle.resume_executor.lease_expires_at`
  - `task_lifecycle.resume_executor.resume_directive`
  - `task_checkpoint.resume_entrypoint`
  - `task_checkpoint.pending_async_job`

## State Vocabulary

| Layer | Token | Meaning |
| --- | --- | --- |
| `tasks.status` | `queued` | Task is accepted and waiting for a worker claim. |
| `tasks.status` | `running` | Task is active, including checkpointed `waiting`, `background`, and `needs_user` lifecycle states. |
| `tasks.status` | `succeeded` | Task reached a successful terminal state. |
| `tasks.status` | `failed` | Task reached an unrecoverable terminal failure. |
| `tasks.status` | `timeout` | Ordinary active task exceeded stale-running recovery limits. Lifecycle projection maps this to `failed` with `terminal_reason=worker_task_timeout`. |
| `tasks.status` | `canceled` | Task was cancelled by a structured task-control path. Lifecycle projection maps this to `cancelled`. |
| `task_lifecycle.state` | `queued` | Operator-facing projection of queued DB state. |
| `task_lifecycle.state` | `running` | Work is actively executing and no checkpoint wait is currently exposed. |
| `task_lifecycle.state` | `waiting` | Work is paused on a checkpoint until `next_check_after` or manual resume. |
| `task_lifecycle.state` | `background` | Work is safely backgrounded, usually waiting for async poll or provider availability. |
| `task_lifecycle.state` | `needs_user` | Work is paused until user input; the checkpoint is preserved. |
| `task_lifecycle.state` | `succeeded` | Operator-facing successful terminal state. |
| `task_lifecycle.state` | `failed` | Operator-facing failed terminal state. |
| `task_lifecycle.state` | `cancelled` | Operator-facing cancelled terminal state. |

`paused` is not a runtime machine state. UI may use it as a friendly label for `waiting` or `background`, but persisted code should keep the explicit lifecycle tokens above.

## Recovery Rules

- Ordinary stale `running` tasks can be marked `timeout` from machine timestamps.
- `waiting`, `background`, and `needs_user` checkpoint states are preserved as `running` in the database so worker recovery can claim the checkpoint by `checkpoint_id`.
- A resume executor claim is valid only while its `lease_expires_at` is active. Expired claims become eligible for recovery.
- Direct `run_skill` async starts and planner-triggered async work both converge through `task_checkpoint.pending_async_job` and `resume_entrypoint = "poll_async_job"` when they need background polling.
- Cancellation should operate on task identity and machine state. It must not parse `text`, `error_text`, or user-facing prose.

## Manual Control Semantics

- `cancel-by-task-id` sets `tasks.status=canceled`, writes `error_text=user_cancelled`, and stores `task_lifecycle.state=cancelled` with `terminal_reason=user_cancelled`.
- `resume-by-task-id` only applies to an existing checkpointed `waiting` or `background` task. It sets `next_check_after` to now, `resume_due=true`, and keeps the original `task_checkpoint`.
- `pause-by-task-id` only applies to an existing checkpointed `waiting` or `background` task. It pushes `next_check_after` into the future and keeps the original `task_checkpoint`.
- Manual pause/resume does not stop arbitrary code already executing inside a tool call. Long-tail tools must expose checkpoint or async-job fields before they can be safely paused or resumed by API/CLI/UI.

## Decision

No new task lease columns are required for the current single-runtime SQLite deployment. The existing `updated_at` heartbeat plus checkpoint `resume_executor` lease fields are sufficient for:

- foreground submit-and-return flows,
- task query lifecycle projection,
- stale ordinary task recovery,
- paused checkpoint recovery,
- async job polling,
- direct task-id cancellation.
- manual checkpoint pause/resume through structured task-control APIs.

Add explicit database columns such as `worker_id`, `lease_owner`, or `lease_expires_at` only when RustClaw supports concurrent durable workers that can claim the same task queue across processes or hosts.

## Required Checks

- `cargo test -p clawd task_resume_execution -- --nocapture`
- `cargo test -p clawd async_poll_executor -- --nocapture`
- `cargo test -p clawd run_skill_finalize -- --nocapture`
- focused task-control tests matching `task_by_id`

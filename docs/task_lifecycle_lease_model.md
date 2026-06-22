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

## Recovery Rules

- Ordinary stale `running` tasks can be marked `timeout` from machine timestamps.
- `waiting`, `background`, and `needs_user` checkpoint states are preserved as `running` in the database so worker recovery can claim the checkpoint by `checkpoint_id`.
- A resume executor claim is valid only while its `lease_expires_at` is active. Expired claims become eligible for recovery.
- Direct `run_skill` async starts and planner-triggered async work both converge through `task_checkpoint.pending_async_job` and `resume_entrypoint = "poll_async_job"` when they need background polling.
- Cancellation should operate on task identity and machine state. It must not parse `text`, `error_text`, or user-facing prose.

## Decision

No new task lease columns are required for the current single-runtime SQLite deployment. The existing `updated_at` heartbeat plus checkpoint `resume_executor` lease fields are sufficient for:

- foreground submit-and-return flows,
- task query lifecycle projection,
- stale ordinary task recovery,
- paused checkpoint recovery,
- async job polling,
- direct task-id cancellation.

Add explicit database columns such as `worker_id`, `lease_owner`, or `lease_expires_at` only when RustClaw supports concurrent durable workers that can claim the same task queue across processes or hosts.

## Required Checks

- `cargo test -p clawd task_resume_execution -- --nocapture`
- `cargo test -p clawd async_poll_executor -- --nocapture`
- `cargo test -p clawd run_skill_finalize -- --nocapture`
- focused direct cancellation tests for `cancel_task_by_id`

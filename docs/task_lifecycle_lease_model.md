# Task Lifecycle Lease Model

RustClaw uses two machine-readable lease layers for durable task execution:

- task-row worker leases in SQLite, used to claim and heartbeat queued/running work;
- checkpoint resume-executor leases in `result_json`, used to resume paused/background work without replaying completed side effects.

Both layers are structural protocol. Runtime recovery, cancellation, resume, and polling must not parse user-visible `text` or `error_text`.

## Current Model

- `tasks.status` remains the coarse database state: `queued`, `running`, `succeeded`, `failed`, `canceled`, or `timeout`.
- `tasks.lease_owner`, `tasks.lease_expires_at`, `tasks.claim_attempt`, and `tasks.claimed_at` are the task worker lease fields. A queued claim sets the owner and increments `claim_attempt`; heartbeat only renews the exact active owner/generation.
- `tasks.updated_at` is still the coarse heartbeat and ordering timestamp. Query projection exposes it as `task_lifecycle.last_heartbeat_ts` for active states, but it is no longer the only lease signal.
- `task_lifecycle.state` is the user/operator state projected by `crates/clawd/src/task_lifecycle.rs`: `queued`, `running`, `waiting`, `background`, `needs_user`, `succeeded`, `failed`, or `cancelled`.
- Paused/background checkpoint recovery uses structured JSON fields in `result_json`, not natural-language text:
  - `task_lifecycle.checkpoint_id`
  - `task_lifecycle.next_check_after`
  - `task_lifecycle.resume_executor.executor_state`
  - `task_lifecycle.resume_executor.lease_expires_at`
  - `task_lifecycle.resume_executor.resume_directive`
  - `task_checkpoint.resume_entrypoint`
  - `task_checkpoint.pending_async_job`
- Task query/list projections include worker lease fields when present:
  - `task_lifecycle.lease_owner`
  - `task_lifecycle.lease_expires_at`
  - `task_lifecycle.claim_attempt`
  - `task_lifecycle.claimed_at`
  - `task_lifecycle.attempt_id`

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

## Lease Layers

### Task Worker Lease

`claim_next_task()` selects the oldest `queued` row, atomically moves it to `running`, and writes:

- `lease_owner = state.worker.worker_id`
- `claimed_at = now`
- `lease_expires_at = now + max(worker_task_heartbeat_seconds * 4, 300)`
- `claim_attempt = claim_attempt + 1`

`touch_running_task()` refreshes only `updated_at` and `lease_expires_at`, and only when both `lease_owner` and `claim_attempt` match the `ClaimedTask`. It never assigns or steals ownership. Progress, checkpoint, success, failure, timeout, mutation receipt, and claimed journal-event writes use the same exact claim fence. A rejected worker write returns a structured `worker_lease_lost` or task-state conflict instead of silently mutating the row.

The generation is required even when the same worker id is reused. If recovery or takeover advances `claim_attempt`, work produced by an older generation cannot heartbeat, publish authoritative worker events, commit a side-effect receipt, or finalize the task.

### Checkpoint Resume-Executor Lease

Checkpointed `waiting`, `background`, and `needs_user` work remains in `tasks.status = 'running'` and stores the recoverable state in `result_json`. When a checkpoint is due, the recovery path claims it through `task_lifecycle.resume_executor` and records a bounded claim:

- `resume_claim.claim_attempt`
- `executor_state`
- `previous_executor_state`
- `executor_state_at`
- `executor_claim_expires_at`
- `resume_executor_claim.owner`
- `resume_executor_claim.checkpoint_id`
- `resume_executor_claim.claimed_at`
- `resume_executor_claim.expires_at`

An active resume-executor lease blocks duplicate resume work until it expires. Expired claims become eligible for recovery from the checkpoint and completed side-effect ledger. Claiming due checkpoint recovery increments the task-row `claim_attempt`. Every subsequent resume work-item, executor-plan, handoff, dispatch, execution result, result projection, and lease renewal carries that generation and requires the same task-row owner/generation.

## Recovery Rules

- Ordinary stale `running` tasks can be marked `timeout` from machine timestamps and worker lease state.
- Cancellation or terminal timeout wins over a late worker result. A terminal row cannot be overwritten by the former owner even if its process completes later.
- `waiting`, `background`, and `needs_user` checkpoint states are preserved as `running` in the database so worker recovery can claim the checkpoint by `checkpoint_id`.
- A resume executor claim is valid only while its `lease_expires_at` is active. Expired claims become eligible for recovery.
- Direct `run_skill` async starts and planner-triggered async work both converge through `task_checkpoint.pending_async_job` and `resume_entrypoint = "poll_async_job"` when they need background polling.
- Cancellation should operate on task identity and machine state. It must not parse `text`, `error_text`, or user-facing prose.

## Manual Control Semantics

- `cancel-by-task-id` sets `tasks.status=canceled`, writes `error_text=user_cancelled`, and stores `task_lifecycle.state=cancelled` with `terminal_reason=user_cancelled`.
- `resume-by-task-id` only applies to an existing checkpointed `waiting` or `background` task. It sets `next_check_after` to now, `resume_due=true`, and keeps the original `task_checkpoint`.
- `pause-by-task-id` only applies to an existing checkpointed `waiting` or `background` task. It pushes `next_check_after` into the future and keeps the original `task_checkpoint`.
- Manual pause/resume does not stop arbitrary code already executing inside a tool call. Long-tail tools must expose checkpoint or async-job fields before they can be safely paused or resumed by API/CLI/UI.
- Operator entrypoints such as `clawcli resume-task <task_id>`, `clawcli pause-task <task_id> --pause-seconds N`, and `clawcli cancel-task <task_id>` are thin wrappers over these structured task-control paths. They must use task ids and lifecycle/checkpoint machine fields, not localized result text.

## Decision

Explicit task lease columns are now part of the current SQLite schema. RustClaw does not need a separate distributed worker table yet, but task claiming must use the existing row-level lease fields plus checkpoint resume leases. The current model supports:

- foreground submit-and-return flows,
- task query lifecycle projection,
- stale ordinary task recovery,
- paused checkpoint recovery,
- async job polling,
- direct task-id cancellation.
- manual checkpoint pause/resume through structured task-control APIs.

Future multi-host execution should build on the existing task-row lease columns. Add a dedicated worker registry only when RustClaw needs host health, queue partitioning, or cross-process lease ownership beyond `lease_owner` and `lease_expires_at`.

## Required Checks

- `cargo test -p clawd task_lifecycle -- --quiet`
- `cargo test -p clawd task_resume_execution -- --quiet`
- `cargo test -p clawd async_poll_executor -- --quiet`
- `cargo test -p clawd task_by_id -- --quiet`

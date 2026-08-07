# Long-task runtime resilience

Task is the durable goal. This runtime treats a user task as the durable goal and a process, provider job,
tool call, or skill call as one step. A step becoming terminal does not by
itself make the user task successful. The verifier must close the remaining
verified plan and output contract.

## User controls

- **Pause** records a durable `pause` directive. A running mutation is allowed
  to finish its unsafe section, then the agent checkpoints at the next model,
  tool, batch, or poll boundary.
- **Adjust** records a monotonic `steer` directive on the same task. The next
  safe planner boundary receives it as structured context; completed receipts
  are not rewritten.
- **Resume** consumes the pinned checkpoint and preserves accumulated budget,
  artifact ownership, registry generation, and completed side effects.
- **Cancel** uses the existing cancellation token, provider adapter, or verified
  process-group termination. Cancellation is not represented as a timeout.

The UI shows a measured count only when an adapter supplies a real total. For
unknown-duration work it shows the current phase, heartbeat, and next action
without inventing a percentage or ETA.

## Budget policy

`configs/agent_guard.toml` owns the task budget policy. Under
`unbounded_progressful`, model turns, tool calls, elapsed time, and continuation
counts become checkpoint/requeue boundaries while machine progress continues.
Explicit safety rules and user deadlines may still terminate. Known cost or
quota exhaustion enters a user/policy decision state instead of being reported
as goal failure. Equivalent progress digests still allow bounded stagnation
repair and a final, auditable stop.

## Workspace isolation

An ordinary ask task can request an independent workspace. The task reuses one
task-scoped Git worktree across continuations and records its base revision,
patch artifact, precondition hashes, and cleanup reference. Changed, running,
or explicitly pinned worktrees are excluded from age-based cleanup. Applying a
patch remains a separate, reviewed mutation; the main checkout is never
silently overwritten.

Remote API calls execute through their local capability adapter and are not
called remote executors. `remote_executor` is reserved for an authenticated
worker assignment. Its versioned contract pins revision, registry generation,
policy/capability/receipt digests, lease owner, short-lived credential refs,
chunked artifact digests, event sequence, and terminal receipt. The feature is
off by default. Missing transport or attestation returns structured unavailable
and never falls back to a more privileged local execution. A transport loss
after a possible external effect becomes `ambiguous` and requires query and
reconciliation before reassignment.

The design borrows only documented product behavior: Codex Goals and App
Server expose pause/resume/steer, durable threads/turns/items and streamed
events; Codex worktrees keep one isolated checkout for a chat. Claude Code
documents interruptible/background tasks, task identifiers, session resume,
and worktree agents. This runtime keeps its stronger existing durable process
supervisor rather than assuming undocumented infinite execution.

## SQLite writer ownership and retry

All pools enable WAL, a configured busy timeout, `synchronous=NORMAL`, and
foreign keys. Provider, filesystem, model, and channel network I/O must remain
outside SQLite transactions.

| Writer | Transaction scope | Busy behavior | External I/O in lock |
| --- | --- | --- | --- |
| task claim/checkpoint | compare-and-swap task/lease fields | pool busy timeout; lease retry by scheduler | none |
| event stream/archive | backfill suffix, deduplicate, append hot+archive, trim | bounded jitter/backoff around whole short transaction | none |
| mutation receipt ledger | one phase/receipt/reconcile transition | pool busy timeout; idempotency key makes caller retry safe | none |
| control mailbox | enqueue monotonic directive or mark applied | pool busy timeout; control ID deduplicates retries | none |
| channel terminal outbox | claim/finish one delivery lease | pool busy timeout; expired lease is reclaimable | provider send occurs after commit |
| memory jobs | claim or settle one job/checkpoint | pool busy timeout; job lease recovery | provider embedding occurs after claim commit |
| audit log | append in a separate SQLite database and pool | independent busy timeout | none |

The bounded retry helper retries only SQLite `BUSY`/`LOCKED`, drops the failed
transaction before sleeping, and exposes permanent contention after five
attempts. Other SQL errors are never hidden by retry.

## Fault and recovery matrix

| Fault point | Required recovery invariant |
| --- | --- |
| before job spawn | no process identity exists; safe to retry the intent |
| after spawn, before lease marker | reconcile PID/PGID and command fingerprint before respawn |
| terminal stdout before projection | poll recovers the pinned structured result and artifact refs |
| checkpoint projection before/after restart | one checkpoint ID and one resume lease own continuation |
| mutation attempt before receipt | state is reconciliation-required; never replay blindly |
| receipt before task terminal commit | completed receipt is reused and remaining verified tail continues |
| provider accepted delivery before local receipt | stable idempotency key queries/retries without duplicating accepted prefix |
| multipart prefix accepted then failure | receipt records accepted parts and retries only the missing suffix |
| pause/cancel races with terminal | committed terminal receipt wins; stale control is rejected or audited |
| SQLite temporarily busy | bounded retry commits once or returns a visible structured error |
| disk temporarily full | no partial success; immutable artifacts keep digest verification |
| quiet long process | heartbeat/process identity proves liveness; silence is not a kill condition |
| large-output process | cursor/artifact preserves output beyond inline display limits |

## Release checks

The generated machine inventory is
`configs/long_task_execution_inventory.json`. Regenerate it with
`python3 scripts/check_long_task_execution_inventory.py --write`; CI/check flows
run the checker without `--write` and reject stale execution-mode, timeout, or
remote-executor classifications. Long-task changes must also run task lifecycle,
adaptive-limit, long-file, cross-platform, UI lint/build, Rust tests, release
build, service restart, and live UI plus one real channel acceptance.

## 2026-08-07 acceptance record

- Full Rust verification passed: `clawd` 3837 passed / 0 failed / 3 ignored,
  `claw-core` 265 passed, `skill-runner` 24 passed, and `agent-skill-sdk`
  67 passed. UI lint/build and both product-identity builds also passed.
- The release build finished in 5m51s at 457% aggregate CPU with a 4.83 GiB
  peak RSS. No single-job Cargo override was used.
- The release fault matrix passed 50/50 cases. The long-command regression
  covered quiet execution, large UTF-8 output, explicit deadline,
  cancellation, and concurrent health checks. The restart-boundary regression
  evidence is `target/clawd_restart_boundaries_20260807_233006/summary.json`.
- Startup recovery now adopts resumable checkpoints from the previous worker
  generation. A checkpoint with an executor keeps one owner; an unclaimed
  checkpoint releases the dead process lease for normal recovery.
- Live UI task `d26f9ba6-23d4-40ec-b6a4-4b86f61c8a5c` accepted and applied a
  monotonic steering directive, survived a 180-second provider timeout through
  a durable wait checkpoint, resumed automatically, completed one 20-second
  local process exactly once, and reached `succeeded` without page refresh.
- Live Telegram task `b435eb21-d999-4a85-9af6-20469922004c` completed through
  the real `system_basic` runner. Its terminal outbox completed on the first
  attempt and the Telegram provider receipt was `accepted`.
- UI artifacts deployed through the configured nginx site matched `UI/dist` by
  SHA-256. The deployed `clawd` SHA-256 was
  `9c0e599b508427f4cdd9cc6435ad804211d431ed3c742c2f452e7a306d55d017`.
- `scripts/restart_clawd_latest.sh` now waits for the internal listener instead
  of treating a startup longer than two seconds as failure. The wait is a
  bounded deployment readiness check, not a task runtime deadline.

The optional remote-executor feature remains disabled in release configuration.
Its authenticated versioned contract, admission rules, revision/digest pins,
lease/reconciliation states, artifact chunk digests, and fail-closed
`unavailable` behavior are covered by contract tests. No capability is labeled
as a remote executor merely because it calls a remote API, and no missing remote
transport silently falls back to a more privileged local execution.

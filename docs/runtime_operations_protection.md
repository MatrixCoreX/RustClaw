# Runtime operations protection

The host samples system, workspace, temporary, runtime-database and audit-database filesystems every 15 seconds. SQLite memory databases do not require a disk probe. Disk status is additive telemetry at authenticated `GET /v1/health` (`storage_pressure`), and the console displays a localized warning.

Warning reserve is the larger of 2 GiB or 3% of filesystem capacity. Critical reserve is the larger of 512 MiB or 1%. At critical or unavailable status, dispatch rechecks storage before starting capabilities whose approved action metadata permits filesystem writes (or does not establish read-only behavior). This check precedes execution isolation and artifact creation. Read-only capabilities, status, cancellation and recovery remain available; existing executions are not replayed. Background memory and embedding claims pause until storage admits writes. This reserve protects control-plane writes; it is not a per-file quota or a promise to abort an already running download.

Foreground execution, stale recovery, cleanup, scheduler, both message outboxes, memory generation and embedding workers use the same owned-future supervisor. Panics and lost progress trigger cancellation followed by a bounded restart delay. Successfully renewed memory/embedding leases preserve long executions. Started blocking ticks retain a guard until actual completion, so cancelling a waiter cannot overlap a replacement with a still-running database operation. Per-worker telemetry is under `worker_supervision.background_workers`.

Memory startup reconciliation preserves the source task's model selection when reconstructing an unbound task. Pinned model selection uses the catalog/vendor identity, while wire protocol remains a separate snapshot field. Recovery validates the reconstructed binding before queueing. It never stamps an empty override or selects the latest unrelated chat rows; historical recovery requires exact input and the canonical saved reply, with an unambiguous, owner-matched source pair. Unavailable model snapshots are not enqueued. Preflight failures are terminal immediately; transient provider/database failures alone use the bounded retry budget. `memory_jobs.error_code`, `retryable`, and `checkpoint_json.failure` retain machine classification, message key, phase and optional provider failure class. Provider responses, credentials and prompts are excluded. Extraction failures are no longer reported as successful jobs.

Authenticated `GET /v1/memory/jobs` returns the caller's latest 100 job snapshots, including errors. Explicit `POST /v1/memory/jobs/retry-unavailable` repairs at most 100 failed jobs belonging to that principal and matching the legacy `unavailable` model binding with zero progress, including audited legacy retries that failed at model binding before execution. It validates the authentic source task, current consent/revision/digest, principal ownership, source evidence and digests, quota and configured model before requeueing. It retains a `legacy_repair` audit checkpoint. Cancelled, progressed, differently pinned, cross-principal and revoked jobs are not replayed. No model is called during repair preflight. Settings failures remain diagnosed and blocked.

Detached launchers preserve the previous log before starting a daemon and then append to the new log. Each prior run is gzip-compressed, byte-verified and retained as `<component>.log.previous.<timestamp>.<pid>.log.gz`, with the newest 20 prior runs retained per component. Compression failure retains the uncompressed prior run and fails startup rather than truncating evidence. Never invoke the rotation helper against a live writer. Model I/O continues to use its existing daily rotation policy.

Validation runs through exact-SHA candidate workflows including `runtime-operations-ci.yml` on Linux and macOS, plus ARM64 cross compilation. Tests cover typed permanent/transient errors, model reconstruction, stale lease fencing, principal boundaries, stalled blocking work, disk thresholds, restart recovery and lossless log rotation including disk-write failure.

## Verified control-plane checkpoints

`python3 scripts/runtime_checkpoint.py create --root ROOT --destination BACKUP_ROOT`
creates an owner-private checkpoint on a different filesystem. It checks that foreground
and distributed work are idle and briefly stops the selected runtime service. After a
successful truncating WAL checkpoint it establishes an actual SQLite read transaction at
end-mark zero and requires the WAL to remain empty during pinning. This read fence prevents
writes to the main database file while allowing later runtime commits to append to WAL.
Non-WAL stores are compressed under their read fences before service recovery; WAL stores
are validated and compressed after the runtime is back online. Main-file identity, size
and timestamps are checked around compression; original SHA-256 and archive/member digests
are verified before publication. Reader fences are released as soon as each database is
packed. A source-space guard and bounded read-fence duration prevent unlimited WAL retention.

Only control files need temporary copies, not the full database. Temporary space is
actually reserved before stopping a service, catching per-user quotas that statvfs cannot
see; tmpfs also requires RAM headroom. A private resume marker, SIGTERM cleanup and the
systemd timer's ExecStopPost recovery restart the service after an interrupted freeze.
An active task, unavailable service controller or insufficient destination space blocks the
operation. Older checkpoints are retired only after a complete new checkpoint is verified.
No build is performed. The default retention is one checkpoint; choose a larger value only
when the destination can accommodate the extra verified copy plus the next staging peak.

The scope includes runtime/audit and skill SQLite databases, configuration, local broker
credentials, runtime binaries, UI, admission generations and installed-package pointers.
This is a **control-plane checkpoint**, not a full machine backup. Keep task attachment
files, models, immutable installed package payloads and external connector storage
separately. The manifest lists these exclusions. A restored runtime requires the exact
matching installed packages and their receipts; never mix old programs with a newer database.

`verify --checkpoint PATH` rechecks every archive/member and original database digest.
`restore --checkpoint PATH --output NEW_DIRECTORY` refuses existing output, rejects
links/path escapes, verifies each restored digest and runs SQLite quick checks. It does
not replace or activate the live runtime. Retain the currently working installation until
the restored configuration, package receipts, authentication and a real task are verified.

`install_runtime_checkpoint_timer.py --root ROOT --destination BACKUP_ROOT --install`
installs a daily systemd timer on Linux; it uses the current operator account and the
existing non-interactive service-control permissions. Preview without `--install`.
Unsupported service managers return an explicit unsupported result; macOS supports
verified offline checkpoints/restores, with service quiescence managed by the operator.
The administrator console shows the latest attempted checkpoint status without exposing
backup paths or credentials. A failed attempt does not mean an older backup was removed.

## Historical memory job review

`GET /v1/memory/jobs/review` projects only the authenticated principal's latest 100 failed
jobs. `POST /v1/memory/jobs/:job_id/disposition` accepts `archive`, `unarchive`, or `retry`
with the current `review_token`. Stale, cross-principal and running-job mutations fail.
Archive is a reversible checkpoint annotation: it retains source evidence, errors, attempts
and job status and excludes that job from legacy bulk repair. Retry uses the existing
conservative legacy model-binding repair, including ownership/source/consent/quota/model
checks; an unsuccessful preflight never becomes a model call or an implicit success.

## Usage and shutdown diagnostics

Both OpenAI-compatible request paths request `stream_options.include_usage=true` for
streamed completions. Non-stream requests are unchanged. Terminal usage frames remain
separate from model text and survive bounded diagnostic logging. An interrupted stream or
a provider that omits usage still reports unknown cost, never zero or an invented estimate;
historical missing usage cannot be reconstructed from text length.

The last browser-service owner now cancels the background reaper and asks each bridge to
close its browser children before exiting. A bounded five-second grace period retains the
process-group kill fallback. Dropping one clone does not close another clone's sessions.
Regression tests require the original parent and all observed descendants to disappear;
failure diagnostics retain process-group members and `/proc` states on Linux.

Old deployment staging can be deduplicated with
`deduplicate_deployment_staging.py --root ROOT --apply`: only identical inactive regular
files with matching modes are hard-linked. Paths, database backups and evidence remain.
Never edit those historical staging files in place after deduplication; replace a path when
creating a different version. The command previews by default and refuses incomplete
process-reference audits.

The checkpoint followup CI can reuse a successful full native/Pi runtime ancestor only
when a strict path allowlist proves every later change is limited to checkpoint Python,
its tests/docs, this focused workflow and the checkpoint console card. It waits for that
ancestor to succeed, verifies ancestry/diffs, then runs the final exact-SHA Python and UI
checks on Linux/macOS. The signed deployment artifact is still built from the final SHA;
any Rust, Cargo, registry, prompt or other runtime change requires full native/Pi validation.


`runtime_checkpoint_remote.py --root ROOT --destination BACKUP_ROOT --host SSH_HOST
--remote-directory PRIVATE_DIRECTORY` transfers the latest verified checkpoint through
SSH with strict existing-host-key verification and batch authentication. It requires a
private owner-only destination, verifies every compressed/raw digest on the receiver,
pins the exact committed verification module by SHA-256 and records the matching tool
source revision. Only index-tracked previous checkpoints are retired after the new copy
and index are durable. It does not change the receiving host's application or services.

Add `--restore-test` for an isolated database recovery exercise on the receiving host.
It checks every restored database digest and SQLite quick-check result, preserves a
2 GiB free-space floor before and during restoration, and always removes its newly
created recovery directory. Existing output is refused and retained. Control-file archives
are still fully verified; the exercise restores database files only to limit temporary
disk use. Local/remote copies remain private compressed files at rest; SSH encrypts the
transport. This feature does not claim separate encryption of storage at rest.

The timer installer accepts `--remote-host SSH_HOST --remote-directory PRIVATE_DIRECTORY`.
A failed remote copy retains the verified local checkpoint and the previous remote copy;
the console distinguishes that state from a failed local checkpoint. An independent host
protects against loss of the local disk; two local partitions alone do not.

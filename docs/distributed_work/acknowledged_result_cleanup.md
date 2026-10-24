# Acknowledged delivery retention

A host can opt in to retiring the local result of **new** distributed capability
assignments by adding `artifact_cleanup_grants` to its private NNI work policy.
`APP_NNI_WORK_POLICY_PATH` selects that policy file; its existing default is
`<workspace>/data/nni/work/policy.json`. The host operator controls this file.
Only `clawd` needs updating; no skill reinstall, SDK or skill-runner update is
required. Installing the code alone leaves cleanup disabled.


```json
{
  "schema_version": 1,
  "principal_id": "<existing local principal>",
  "capabilities": ["<granted capability>"],
  "artifact_origins": ["https://media.example.org"],
  "artifact_cleanup_grants": [{"origin": "https://media.example.org", "capability": "<granted capability>"}],
  "max_file_bytes": 536870912
}
```

The field defaults to an empty list. Each exact origin/capability pair must be
covered by the normal `artifact_origins` and `capabilities` grants. Another
capability using the same origin does not inherit cleanup. The existing
argument and upload-size grants still apply.
No skill package change or new execution permission is implied.

Only a freshly verified assignment with a matching delivery origin receives a
private host enrollment. The enrollment is created for the proposed task UUID
**before** insertion into the queue. If idempotency returns another task UUID,
that old task must already have the same private enrollment; missing enrollment
never becomes an instruction to clean old work. A request's `_task_source` or
`_delivery_cleanup` JSON fields alone cannot opt in an ordinary task.

The node waits for Core's signed `accepted` / `already_reported` reply bound to
the actual poll request's work ID, attempt, lease and result. This ends that
attempt, whether execution succeeded, failed or was canceled. A signed, bound
terminal lease rejection (missing task, inactive/expired lease or invalid lease
token) also closes the attempt. An upload failure before final confirmation,
an invalid ACK, a temporary report error or a malformed-result rejection keeps
the local files available for retry. Other applications are unchanged.

The node first persists `cleanup_pending` and discards the report's text. It
waits for in-flight host artifact writers, then durably records the complete
file plan under an exclusive task gate before deleting anything. It can resume
this local state after restart without running the skill, uploading again or requiring
another ACK. Cleanup failures keep the state pending and prevent new work until
retry succeeds.

The result becomes a tombstone: task identity, ownership, timestamps, terminal
status and idempotency remain, plus the result digest, retired event count and
final event hash. Original arguments, result/continuation/checkpoint JSON,
errors and task event bodies, archives, snapshots and large event payloads are
removed for that exact task. Mutation ledger fingerprints/status/hash stay;
its receipt, verification and reconciliation bodies are cleared. The existing
minimal NNI work audit is preserved. Retired event chains are not presented as
replayable, and late event producers cannot recreate retired payloads.

Deletion covers only the enrolled task UUID in host delivery, invocation,
async-process, skill-output, task-storage and evidence directories. Runner jobs are recorded
in a private task-to-job index before launch. Their exit record (or a stable second observation of a lost process), invocation
ownership, stopped process and stopped resource monitor are checked before
removing their stdout/stderr and directory. A persisted file plan remains valid
after partial deletion. No path is inferred from natural-language output;
traversal, noncanonical UUIDs and symlink ancestors/roots are rejected.

For an enrolled task, a declared directory storage descriptor is resolved into
a fresh task-owned namespace through the existing SkillStorageResolver. The
normal sandbox/storage grant and schema still apply; browser profiles,
checkpoints and their caches live there. Ordinary Agent and other applications
keep the original descriptor. A declared SQLite storage for this opt-in mode
fails explicitly instead of silently sharing persistent data.

From enrollment onward this execution does not copy its arguments/results into
shared process logs or successful-outcome memory. Existing shared memories and
logs are never scanned or rewritten. Skill-private state, browser profiles,
models, caches, unrelated jobs and ordinary Agent results are unchanged.

This is forward-only. Installing the feature or changing policy never scans or
purges existing task history or old download directories.

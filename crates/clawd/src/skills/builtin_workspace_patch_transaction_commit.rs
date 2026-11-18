use super::*;

pub(super) fn publish(
    root: &Path,
    path: &Path,
    manifest: &PatchTransactionManifest,
    record: &PatchShardRecord,
    attempt: &attempt::ClaimedAttempt,
    filesystem: &HostFilesystemPolicy,
) -> Result<String, String> {
    let outcome = if !attempt.matches(manifest, record) {
        Err(operation_failure(
            "transaction_attempt_binding_missing",
            &manifest.transaction_id,
            [],
        ))
    } else {
        attempt::verify(root, manifest, record, filesystem)
            .and_then(|_| write_manifest(root, path, manifest, filesystem))
    };
    if let Err(error) = outcome {
        // A failed publication may already be visible. Never compensate after
        // that uncertainty; the pre-execution receipt and checkpoint stay bound.
        return Err(operation_failure(
            "transaction_commit_unconfirmed",
            &manifest.transaction_id,
            [
                ("checkpoint_id", json!(record.checkpoint_id)),
                ("shard_index", json!(record.shard_index)),
                ("cause", json!(error)),
                ("compensation_attempted", json!(false)),
                ("checkpoint_cleanup_attempted", json!(false)),
                ("receipt_cleanup_attempted", json!(false)),
                ("failure_phase", json!("commit")),
                ("commit_state", json!("unknown")),
            ],
        ));
    }
    encode_manifest_result(manifest, Some(record), "shard_applied", "apply_patch_shard")
}

#[cfg(test)]
#[path = "builtin_workspace_patch_transaction_commit_tests.rs"]
mod tests;

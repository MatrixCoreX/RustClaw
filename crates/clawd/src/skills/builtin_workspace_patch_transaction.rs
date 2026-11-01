use crate::runtime::host_filesystem::HostFilesystemPolicy;
use claw_core::config::ToolSandboxBackend;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Read;

const TRANSACTION_SCHEMA_VERSION: u32 = 1;
const MAX_TRANSACTION_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_DEPENDENCY_PREVIEW: usize = 64;

#[path = "builtin_workspace_patch_transaction_attempt.rs"]
mod attempt;
#[path = "builtin_workspace_patch_transaction_commit.rs"]
mod commit;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PatchTransactionManifest {
    schema_version: u32,
    transaction_id: String,
    task_id: String,
    shard_count: usize,
    state: String,
    shards: Vec<PatchShardRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PatchShardRecord {
    shard_index: usize,
    checkpoint_id: String,
    patch_id: String,
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_digest: Option<String>,
}

pub(super) fn apply_patch_shard(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
) -> Result<String, String> {
    super::ensure_only_keys(
        args,
        &[
            "action",
            "transaction_id",
            "shard_index",
            "shard_count",
            "patch",
            "precondition_hashes",
        ],
    )?;
    let transaction_id = super::required_token(args, "transaction_id")?;
    super::validate_checkpoint_id(transaction_id)?;
    let shard_index = required_usize(args, "shard_index")?;
    let shard_count = required_usize(args, "shard_count")?;
    if shard_count == 0 || shard_index >= shard_count {
        return Err(transaction_error(
            "invalid_shard_position",
            transaction_id,
            json!({"shard_index": shard_index, "shard_count": shard_count}),
        ));
    }
    let root = super::canonical_workspace_root(workspace_root)?;
    let path = transaction_manifest_path(&root, transaction_id, filesystem)?;
    let _operation = lock_transaction(&root, &path, transaction_id, filesystem)?;
    let mut manifest = load_or_create_manifest(
        &root,
        &path,
        transaction_id,
        task_id,
        shard_count,
        filesystem,
    )?;
    if manifest.task_id != task_id || manifest.shard_count != shard_count {
        return Err(transaction_error(
            "transaction_contract_mismatch",
            transaction_id,
            json!({"expected_shard_count": manifest.shard_count}),
        ));
    }
    if !matches!(
        manifest.state.as_str(),
        "planned" | "partial" | "partial_rejected" | "applied"
    ) {
        return Err(replay_error(
            "transaction_not_active",
            &manifest,
            Value::Null,
        ));
    }
    let request_digest = request_digest(args)?;
    if let Some(existing) = manifest
        .shards
        .iter()
        .find(|shard| shard.shard_index == shard_index)
    {
        let Some(expected_digest) = existing.request_digest.as_deref() else {
            return Err(replay_error(
                "shard_request_binding_missing",
                &manifest,
                json!({"shard_index":shard_index}),
            ));
        };
        if expected_digest != request_digest {
            return Err(replay_error(
                "shard_request_conflict",
                &manifest,
                json!({"shard_index":shard_index}),
            ));
        }
        verify_applied_state(&root, &manifest, filesystem)?;
        return encode_manifest_result(
            &manifest,
            Some(existing),
            "already_applied",
            "apply_patch_shard",
        );
    }
    // Stored shard indexes are validated as a contiguous prefix. An arbitrary
    // requested index must not create an unbounded scan or diagnostic array.
    if shard_index > manifest.shards.len() {
        let missing_dependencies = (manifest.shards.len()..shard_index)
            .take(MAX_DEPENDENCY_PREVIEW)
            .collect::<Vec<_>>();
        return Err(transaction_error(
            "shard_dependency_not_applied",
            transaction_id,
            json!({"missing_shard_indices": missing_dependencies,
                "missing_shard_count":shard_index - manifest.shards.len(),
                "next_shard_index":manifest.shards.len(),
                "dependency_preview_truncated":shard_index - manifest.shards.len() > MAX_DEPENDENCY_PREVIEW}),
        ));
    }
    verify_applied_state(&root, &manifest, filesystem)?;

    let patch_id = super::sha256_label(super::required_string(args, "patch")?.as_bytes());
    let attempt = attempt::ClaimedAttempt::reserve(
        &root,
        &manifest,
        shard_index,
        &request_digest,
        &patch_id,
        filesystem,
    )?;
    let record = attempt.shard_record();

    let mut patch_args = Map::new();
    patch_args.insert("action".to_string(), json!("apply_patch"));
    patch_args.insert(
        "patch".to_string(),
        args.get("patch").cloned().unwrap_or(Value::Null),
    );
    if let Some(preconditions) = args.get("precondition_hashes") {
        patch_args.insert("precondition_hashes".to_string(), preconditions.clone());
    }
    let applied = match super::apply_patch_at_checkpoint(
        workspace_root,
        task_id,
        &patch_args,
        filesystem,
        backend,
        attempt.checkpoint_id(),
    ) {
        Ok(value) => value,
        Err(error) => {
            // Only typed pre-dispatch refusal proves a corrected shard may
            // be attempted. Failed compensation or unknown effects do not.
            let original_extra =
                crate::skills::parse_structured_skill_error(&error).and_then(|error| error.extra);
            let rejected_before_dispatch = original_extra.as_ref().is_some_and(|extra| {
                extra["side_effect_applied"] == false && extra["failure_phase"] == "pre_dispatch"
            });
            manifest.state = if rejected_before_dispatch {
                "partial_rejected"
            } else {
                "partial_failed"
            }
            .to_string();
            let publication_error = write_manifest(&root, &path, &manifest, filesystem).err();
            let retirement = if rejected_before_dispatch && publication_error.is_none() {
                Some(attempt.retire_rejected(&root, filesystem, &error))
            } else {
                None
            };
            let retired = retirement.as_ref().is_some_and(Result::is_ok);
            let receipt_retained = if retired {
                Some(false)
            } else if retirement.is_some() {
                None
            } else {
                Some(true)
            };
            let failure = operation_failure(
                "shard_apply_failed",
                transaction_id,
                [
                    ("shard_index", json!(shard_index)),
                    ("checkpoint_id", json!(record.checkpoint_id)),
                    ("cause", json!(error)),
                    ("publication_error", json!(publication_error)),
                    ("retirement_error", json!(retirement.and_then(Result::err))),
                    ("receipt_retained", json!(receipt_retained)),
                    ("rollback_action", json!("transaction_rewind")),
                    (
                        "rollback_checkpoint_ids",
                        json!(rollback_checkpoint_ids(&manifest)),
                    ),
                ],
            );
            if retired {
                let extra = original_extra.as_ref().expect("typed pre-dispatch refusal");
                return Err(crate::skills::annotate_structured_skill_error_not_applied(
                    &failure,
                    "pre_dispatch",
                    extra["retryable"].as_bool(),
                    extra["recovery_action"].as_str(),
                ));
            }
            return Err(failure);
        }
    };
    let applied: Value = serde_json::from_str(&applied).unwrap_or(Value::Null);
    if applied["status"] != "ok"
        || applied["action"] != "apply_patch"
        || applied["checkpoint_id"].as_str() != Some(record.checkpoint_id.as_str())
        || applied["patch_id"].as_str() != Some(record.patch_id.as_str())
    {
        return Err(operation_failure(
            "shard_result_invalid",
            transaction_id,
            [
                ("shard_index", json!(shard_index)),
                ("checkpoint_id", json!(record.checkpoint_id)),
                ("receipt_retained", json!(true)),
            ],
        ));
    }
    let manifest = manifest_with_applied_shard(manifest, record.clone());
    commit::publish(&root, &path, &manifest, &record, &attempt, filesystem)
}

fn manifest_with_applied_shard(
    mut manifest: PatchTransactionManifest,
    record: PatchShardRecord,
) -> PatchTransactionManifest {
    manifest.shards.push(record);
    manifest.shards.sort_by_key(|shard| shard.shard_index);
    manifest.state = if manifest.shards.len() == manifest.shard_count {
        "applied".to_string()
    } else {
        "partial".to_string()
    };
    manifest
}

fn operation_failure<const N: usize>(
    code: &str,
    transaction_id: &str,
    fields: [(&str, Value); N],
) -> String {
    let message_key = format!("workspace.patch.{code}");
    let mut extra: Map<String, Value> = fields
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    extra.extend([
        ("error_code".into(), json!(code)),
        ("message_key".into(), json!(&message_key)),
        ("transaction_id".into(), json!(transaction_id)),
        ("retryable".into(), json!(false)),
        ("complete".into(), json!(false)),
        (
            "recovery".into(),
            json!({"action":"transaction_status","transaction_id":transaction_id}),
        ),
    ]);
    super::super::builtin_error(
        "workspace_patch",
        code,
        message_key,
        None,
        None,
        Some(Value::Object(extra)),
    )
}

fn lock_transaction(
    root: &Path,
    path: &Path,
    transaction_id: &str,
    filesystem: &HostFilesystemPolicy,
) -> Result<std::fs::File, String> {
    filesystem
        .try_lock_operation_file(Some(root), &path.with_extension("lock"))
        .map_err(|error| {
            let code = if HostFilesystemPolicy::is_private_access_denied(&error) {
                "host_private_directory_access_denied"
            } else if error.kind() == std::io::ErrorKind::WouldBlock {
                "transaction_busy"
            } else {
                "transaction_lock_unavailable"
            };
            operation_failure(
                code,
                transaction_id,
                [("io_kind", json!(format!("{:?}", error.kind())))],
            )
        })
}

fn request_digest(args: &Map<String, Value>) -> Result<String, String> {
    let patch = super::required_string(args, "patch")?;
    if patch.is_empty() || patch.len() > super::MAX_PATCH_BYTES {
        return Err(super::patch_error(
            "invalid_patch_size",
            "workspace.patch.invalid_size",
            Value::Null,
        ));
    }
    let mut preconditions = BTreeMap::new();
    if let Some(value) = args.get("precondition_hashes") {
        let object = value.as_object().ok_or_else(|| {
            super::patch_error(
                "invalid_precondition_hashes",
                "workspace.patch.invalid_preconditions",
                Value::Null,
            )
        })?;
        for (path, value) in object {
            let expected = value.as_str().ok_or_else(|| {
                super::patch_error(
                    "invalid_precondition_hash",
                    "workspace.patch.invalid_precondition_hash",
                    json!({"path":path}),
                )
            })?;
            preconditions.insert(path, expected);
        }
    }
    let bytes = serde_json::to_vec(&("workspace-patch-shard-request-v1", patch, preconditions))
        .map_err(|_| {
            transaction_error("transaction_request_encode_failed", "unknown", Value::Null)
        })?;
    Ok(super::sha256_label(&bytes))
}

fn verify_applied_state(
    root: &Path,
    manifest: &PatchTransactionManifest,
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    // Later shards can update the same path. Check their composed final state,
    // not an earlier shard's after hash against a legitimately later revision.
    let mut expected_files = BTreeMap::new();
    for shard in &manifest.shards {
        attempt::verify(root, manifest, shard, filesystem)?;
        let checkpoint_dir = super::resolve_checkpoint_dir(root, &shard.checkpoint_id, filesystem)?;
        let checkpoint = super::read_manifest(&checkpoint_dir, filesystem)?;
        if shard.state != "applied"
            || checkpoint.state != "applied"
            || checkpoint.task_id != manifest.task_id
            || checkpoint.patch_id != shard.patch_id
        {
            return Err(replay_error(
                "shard_state_conflict",
                manifest,
                json!({"shard_index":shard.shard_index,"checkpoint_id":shard.checkpoint_id}),
            ));
        }
        for file in checkpoint.files {
            super::validate_relative_patch_path(root, &file.path)?;
            expected_files.insert(file.path, file.after_sha256);
        }
    }
    for (path, expected) in expected_files {
        if super::hash_existing_file(root, &root.join(&path), filesystem)? != expected {
            return Err(replay_error(
                "shard_state_conflict",
                manifest,
                json!({"path":path}),
            ));
        }
    }
    Ok(())
}

fn replay_error(code: &str, manifest: &PatchTransactionManifest, details: Value) -> String {
    let message_key = format!("workspace.patch.{code}");
    super::super::builtin_error(
        "workspace_patch",
        code,
        message_key.clone(),
        None,
        None,
        Some(json!({
            "error_code":code,"message_key":message_key,
            "transaction_id":manifest.transaction_id,"state":manifest.state,
            "complete":false,"retryable":false,"details":details,
            "recovery":{"action":"transaction_status","auto_retry":false}
        })),
    )
}

pub(super) fn transaction_status(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
    filesystem: &HostFilesystemPolicy,
) -> Result<String, String> {
    super::ensure_only_keys(args, &["action", "transaction_id", "shard_index"])?;
    let transaction_id = super::required_token(args, "transaction_id")?;
    let root = super::canonical_workspace_root(workspace_root)?;
    super::validate_checkpoint_id(transaction_id)?;
    if args.contains_key("shard_index") {
        return attempt::status(
            &root,
            task_id,
            transaction_id,
            required_usize(args, "shard_index")?,
            filesystem,
        )
        .and_then(|value| super::encode_result(value));
    }
    let path = manifest_path(&root, transaction_id);
    if filesystem
        .metadata(Some(&root), &path)
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        && attempt::lookup(&root, task_id, transaction_id, 0, filesystem)?.is_some()
    {
        let mut value = attempt::status(&root, task_id, transaction_id, 0, filesystem)?;
        value["manifest_available"] = json!(false);
        value["requires_reconciliation"] = json!(true);
        return super::encode_result(value);
    }
    let manifest = read_manifest(&root, &path, filesystem)?;
    if manifest.task_id != task_id {
        return Err(transaction_error(
            "transaction_owner_mismatch",
            transaction_id,
            Value::Null,
        ));
    }
    let mut value: Value = serde_json::from_str(&encode_manifest_result(
        &manifest,
        None,
        "status",
        "transaction_status",
    )?)
    .map_err(|_| transaction_error("transaction_manifest_invalid", transaction_id, Value::Null))?;
    if manifest.shards.len() < manifest.shard_count {
        if let Some(receipt) = attempt::lookup(
            &root,
            task_id,
            transaction_id,
            manifest.shards.len(),
            filesystem,
        )? {
            value["pending_attempt"] = serde_json::to_value(receipt).map_err(|_| {
                transaction_error("transaction_manifest_invalid", transaction_id, Value::Null)
            })?;
            value["continuation"] = Value::Null;
            value["complete"] = json!(false);
            value["requires_reconciliation"] = json!(true);
            value["execution_outcome"] = json!("unknown");
        }
    }
    super::encode_result(value)
}

pub(super) fn transaction_rewind(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
    filesystem: &HostFilesystemPolicy,
) -> Result<String, String> {
    super::ensure_only_keys(args, &["action", "transaction_id"])?;
    let transaction_id = super::required_token(args, "transaction_id")?;
    let root = super::canonical_workspace_root(workspace_root)?;
    let path = transaction_manifest_path(&root, transaction_id, filesystem)?;
    let _operation = lock_transaction(&root, &path, transaction_id, filesystem)?;
    let mut manifest = read_manifest(&root, &path, filesystem)?;
    if manifest.task_id != task_id {
        return Err(transaction_error(
            "transaction_owner_mismatch",
            transaction_id,
            Value::Null,
        ));
    }
    if manifest.shards.len() < manifest.shard_count {
        if let Some(receipt) = attempt::lookup(
            &root,
            task_id,
            transaction_id,
            manifest.shards.len(),
            filesystem,
        )? {
            return Err(operation_failure(
                "transaction_attempt_unresolved",
                transaction_id,
                [
                    ("shard_index", json!(receipt.shard_index)),
                    ("checkpoint_id", json!(receipt.checkpoint_id)),
                ],
            ));
        }
    }
    for index in (0..manifest.shards.len()).rev() {
        if manifest.shards[index].state != "applied" {
            continue;
        }
        let mut rewind_args = Map::new();
        rewind_args.insert("action".to_string(), json!("rewind"));
        rewind_args.insert(
            "checkpoint_id".to_string(),
            json!(manifest.shards[index].checkpoint_id),
        );
        if let Err(error) = super::rewind(workspace_root, &rewind_args, filesystem) {
            manifest.state = "rollback_failed".to_string();
            let _ = write_manifest(&root, &path, &manifest, filesystem);
            return Err(transaction_error(
                "transaction_rollback_failed",
                transaction_id,
                json!({
                    "failed_shard_index": manifest.shards[index].shard_index,
                    "remaining_checkpoint_ids": rollback_checkpoint_ids(&manifest),
                    "cause": error,
                }),
            ));
        }
        manifest.shards[index].state = "rewound".to_string();
        write_manifest(&root, &path, &manifest, filesystem)?;
    }
    manifest.state = "rewound".to_string();
    write_manifest(&root, &path, &manifest, filesystem)?;
    encode_manifest_result(&manifest, None, "rewound", "transaction_rewind")
}

fn load_or_create_manifest(
    root: &Path,
    path: &Path,
    transaction_id: &str,
    task_id: &str,
    shard_count: usize,
    filesystem: &HostFilesystemPolicy,
) -> Result<PatchTransactionManifest, String> {
    match filesystem.metadata(Some(root), path) {
        Ok(_) => return read_manifest(root, path, filesystem),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(super::patch_io_error(
                "transaction_manifest_read_failed",
                "workspace.patch.transaction_manifest_read_failed",
                error,
            ))
        }
    }
    Ok(PatchTransactionManifest {
        schema_version: TRANSACTION_SCHEMA_VERSION,
        transaction_id: transaction_id.to_string(),
        task_id: task_id.to_string(),
        shard_count,
        state: "planned".to_string(),
        shards: Vec::new(),
    })
}

fn transaction_manifest_path(
    root: &Path,
    transaction_id: &str,
    filesystem: &HostFilesystemPolicy,
) -> Result<PathBuf, String> {
    super::validate_checkpoint_id(transaction_id)?;
    let directory = super::checkpoint_root(root).join("patch-transactions");
    super::reject_symlink_if_present(&directory)?;
    filesystem
        .create_directory(Some(root), &directory, true)
        .map_err(|error| {
            super::patch_io_error(
                "transaction_manifest_create_failed",
                "workspace.patch.transaction_manifest_create_failed",
                error,
            )
        })?;
    Ok(manifest_path(root, transaction_id))
}

fn manifest_path(root: &Path, transaction_id: &str) -> PathBuf {
    super::checkpoint_root(root)
        .join("patch-transactions")
        .join(format!("{transaction_id}.json"))
}

fn read_manifest(
    root: &Path,
    path: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<PatchTransactionManifest, String> {
    let (file, _) = filesystem.open_file(Some(root), path).map_err(|error| {
        super::patch_io_error(
            "transaction_manifest_read_failed",
            "workspace.patch.transaction_manifest_read_failed",
            error,
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_TRANSACTION_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            super::patch_io_error(
                "transaction_manifest_read_failed",
                "workspace.patch.transaction_manifest_read_failed",
                error,
            )
        })?;
    let expected_id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    if bytes.len() > MAX_TRANSACTION_MANIFEST_BYTES {
        return Err(transaction_error(
            "transaction_manifest_too_large",
            expected_id,
            json!({"max_bytes":MAX_TRANSACTION_MANIFEST_BYTES}),
        ));
    }
    let manifest: PatchTransactionManifest = serde_json::from_slice(&bytes)
        .map_err(|_| transaction_error("transaction_manifest_invalid", expected_id, Value::Null))?;
    if manifest.schema_version != TRANSACTION_SCHEMA_VERSION
        || manifest.transaction_id != expected_id
        || manifest.shard_count == 0
        || manifest.shards.len() > manifest.shard_count
        || manifest.shards.iter().enumerate().any(|(index, shard)| {
            shard.shard_index != index || !matches!(shard.state.as_str(), "applied" | "rewound")
        })
    {
        return Err(transaction_error(
            "transaction_manifest_invalid",
            expected_id,
            Value::Null,
        ));
    }
    Ok(manifest)
}

fn write_manifest(
    root: &Path,
    path: &Path,
    manifest: &PatchTransactionManifest,
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    let bytes = encode_manifest(manifest)?;
    filesystem
        .atomic_write_file_with_options(Some(root), path, &bytes, false)
        .map_err(|error| {
            super::patch_io_error(
                "transaction_manifest_write_failed",
                "workspace.patch.transaction_manifest_write_failed",
                error,
            )
        })
}

fn encode_manifest(manifest: &PatchTransactionManifest) -> Result<Vec<u8>, String> {
    struct ManifestBuffer {
        bytes: Vec<u8>,
        exceeded_limit: bool,
    }

    impl std::io::Write for ManifestBuffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_TRANSACTION_MANIFEST_BYTES.saturating_sub(self.bytes.len()) {
                self.exceeded_limit = true;
                return Err(std::io::Error::other("transaction_manifest_size_limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut output = ManifestBuffer {
        bytes: Vec::new(),
        exceeded_limit: false,
    };
    let result = serde_json::to_writer_pretty(&mut output, manifest);
    if output.exceeded_limit {
        return Err(operation_failure(
            "transaction_manifest_too_large",
            &manifest.transaction_id,
            [("max_bytes", json!(MAX_TRANSACTION_MANIFEST_BYTES))],
        ));
    }
    result.map_err(|_| {
        transaction_error(
            "transaction_manifest_encode_failed",
            &manifest.transaction_id,
            Value::Null,
        )
    })?;
    Ok(output.bytes)
}

fn encode_manifest_result(
    manifest: &PatchTransactionManifest,
    current: Option<&PatchShardRecord>,
    outcome: &str,
    action: &str,
) -> Result<String, String> {
    let complete = manifest.state == "applied"
        && manifest.shards.len() == manifest.shard_count
        && manifest.shards.iter().all(|shard| shard.state == "applied");
    super::encode_result(json!({
        "schema_version": 1,
        "source": "workspace_patch_transaction",
        "status": "ok",
        "action": action,
        "outcome": outcome,
        "transaction_id": manifest.transaction_id,
        "state": manifest.state,
        "complete": complete,
        "shard_count": manifest.shard_count,
        "applied_shard_count": manifest.shards.iter().filter(|shard| shard.state == "applied").count(),
        "current_shard": current,
        "shards": manifest.shards,
        "continuation": (!complete
            && matches!(manifest.state.as_str(), "planned" | "partial" | "partial_rejected")
            && manifest.shards.iter().all(|shard| shard.state == "applied")).then(|| json!({
            "kind": "verified_shard",
            "action": "apply_patch_shard",
            "next_shard_index": manifest.shards.len(),
        })),
        "rollback_plan": {
            "action": "transaction_rewind",
            "transaction_id": manifest.transaction_id,
            "checkpoint_ids_reverse_order": rollback_checkpoint_ids(manifest),
            "scope": "owned_shards_only",
        },
    }))
}

fn rollback_checkpoint_ids(manifest: &PatchTransactionManifest) -> Vec<String> {
    manifest
        .shards
        .iter()
        .rev()
        .filter(|shard| shard.state == "applied")
        .map(|shard| shard.checkpoint_id.clone())
        .collect()
}

fn required_usize(args: &Map<String, Value>, key: &str) -> Result<usize, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| {
            transaction_error("invalid_shard_position", "unknown", json!({"field": key}))
        })
}

fn transaction_error(code: &str, transaction_id: &str, details: Value) -> String {
    super::patch_error(
        code,
        &format!("workspace.patch.{code}"),
        json!({
            "transaction_id": transaction_id,
            "details": details,
            "complete": false,
            "recovery": {
                "kind": "verified_shard",
                "rollback_action": "transaction_rewind",
            }
        }),
    )
}

#[cfg(test)]
#[path = "builtin_workspace_patch_transaction_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "builtin_workspace_patch_transaction_manifest_tests.rs"]
mod manifest_tests;

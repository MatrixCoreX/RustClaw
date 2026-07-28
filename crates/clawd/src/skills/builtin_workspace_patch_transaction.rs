use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

const TRANSACTION_SCHEMA_VERSION: u32 = 1;

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
}

pub(super) fn apply_patch_shard(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
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
    let path = transaction_manifest_path(&root, transaction_id)?;
    let mut manifest = load_or_create_manifest(&path, transaction_id, task_id, shard_count)?;
    if manifest.task_id != task_id || manifest.shard_count != shard_count {
        return Err(transaction_error(
            "transaction_contract_mismatch",
            transaction_id,
            json!({"expected_shard_count": manifest.shard_count}),
        ));
    }
    if let Some(existing) = manifest
        .shards
        .iter()
        .find(|shard| shard.shard_index == shard_index)
    {
        return encode_manifest_result(&manifest, Some(existing), "already_applied");
    }
    let missing_dependencies = (0..shard_index)
        .filter(|required| {
            !manifest
                .shards
                .iter()
                .any(|shard| shard.shard_index == *required && shard.state.as_str() == "applied")
        })
        .collect::<Vec<_>>();
    if !missing_dependencies.is_empty() {
        return Err(transaction_error(
            "shard_dependency_not_applied",
            transaction_id,
            json!({"missing_shard_indices": missing_dependencies}),
        ));
    }

    let mut patch_args = Map::new();
    patch_args.insert("action".to_string(), json!("apply_patch"));
    patch_args.insert(
        "patch".to_string(),
        args.get("patch").cloned().unwrap_or(Value::Null),
    );
    if let Some(preconditions) = args.get("precondition_hashes") {
        patch_args.insert("precondition_hashes".to_string(), preconditions.clone());
    }
    let applied = match super::apply_patch(workspace_root, task_id, &patch_args) {
        Ok(value) => value,
        Err(error) => {
            manifest.state = "partial_failed".to_string();
            let _ = write_manifest(&path, &manifest);
            return Err(transaction_error(
                "shard_apply_failed",
                transaction_id,
                json!({
                    "shard_index": shard_index,
                    "rollback_action": "transaction_rewind",
                    "rollback_checkpoint_ids": rollback_checkpoint_ids(&manifest),
                    "cause": error,
                }),
            ));
        }
    };
    let applied: Value = serde_json::from_str(&applied).map_err(|_| {
        transaction_error(
            "shard_result_invalid",
            transaction_id,
            json!({"shard_index": shard_index}),
        )
    })?;
    let record = PatchShardRecord {
        shard_index,
        checkpoint_id: applied
            .get("checkpoint_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        patch_id: applied
            .get("patch_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        state: "applied".to_string(),
    };
    if record.checkpoint_id.is_empty() || record.patch_id.is_empty() {
        return Err(transaction_error(
            "shard_result_invalid",
            transaction_id,
            json!({"shard_index": shard_index}),
        ));
    }
    manifest.shards.push(record.clone());
    manifest.shards.sort_by_key(|shard| shard.shard_index);
    manifest.state = if manifest.shards.len() == shard_count {
        "applied".to_string()
    } else {
        "partial".to_string()
    };
    if let Err(error) = write_manifest(&path, &manifest) {
        let mut rewind_args = Map::new();
        rewind_args.insert("action".to_string(), json!("rewind"));
        rewind_args.insert("checkpoint_id".to_string(), json!(record.checkpoint_id));
        let _ = super::rewind(workspace_root, &rewind_args);
        return Err(error);
    }
    encode_manifest_result(&manifest, Some(&record), "shard_applied")
}

pub(super) fn transaction_status(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
) -> Result<String, String> {
    super::ensure_only_keys(args, &["action", "transaction_id"])?;
    let transaction_id = super::required_token(args, "transaction_id")?;
    let root = super::canonical_workspace_root(workspace_root)?;
    let manifest = read_manifest(&transaction_manifest_path(&root, transaction_id)?)?;
    if manifest.task_id != task_id {
        return Err(transaction_error(
            "transaction_owner_mismatch",
            transaction_id,
            Value::Null,
        ));
    }
    encode_manifest_result(&manifest, None, "status")
}

pub(super) fn transaction_rewind(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
) -> Result<String, String> {
    super::ensure_only_keys(args, &["action", "transaction_id"])?;
    let transaction_id = super::required_token(args, "transaction_id")?;
    let root = super::canonical_workspace_root(workspace_root)?;
    let path = transaction_manifest_path(&root, transaction_id)?;
    let mut manifest = read_manifest(&path)?;
    if manifest.task_id != task_id {
        return Err(transaction_error(
            "transaction_owner_mismatch",
            transaction_id,
            Value::Null,
        ));
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
        if let Err(error) = super::rewind(workspace_root, &rewind_args) {
            manifest.state = "rollback_failed".to_string();
            let _ = write_manifest(&path, &manifest);
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
        write_manifest(&path, &manifest)?;
    }
    manifest.state = "rewound".to_string();
    write_manifest(&path, &manifest)?;
    encode_manifest_result(&manifest, None, "rewound")
}

fn load_or_create_manifest(
    path: &Path,
    transaction_id: &str,
    task_id: &str,
    shard_count: usize,
) -> Result<PatchTransactionManifest, String> {
    if path.is_file() {
        return read_manifest(path);
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

fn transaction_manifest_path(root: &Path, transaction_id: &str) -> Result<PathBuf, String> {
    super::validate_checkpoint_id(transaction_id)?;
    let directory = super::checkpoint_root(root).join("patch-transactions");
    super::reject_symlink_if_present(&directory)?;
    fs::create_dir_all(&directory).map_err(|error| {
        super::patch_io_error(
            "transaction_manifest_create_failed",
            "workspace.patch.transaction_manifest_create_failed",
            error,
        )
    })?;
    Ok(directory.join(format!("{transaction_id}.json")))
}

fn read_manifest(path: &Path) -> Result<PatchTransactionManifest, String> {
    let bytes = fs::read(path).map_err(|error| {
        super::patch_io_error(
            "transaction_manifest_read_failed",
            "workspace.patch.transaction_manifest_read_failed",
            error,
        )
    })?;
    let manifest: PatchTransactionManifest = serde_json::from_slice(&bytes)
        .map_err(|_| transaction_error("transaction_manifest_invalid", "unknown", Value::Null))?;
    if manifest.schema_version != TRANSACTION_SCHEMA_VERSION {
        return Err(transaction_error(
            "transaction_manifest_invalid",
            &manifest.transaction_id,
            Value::Null,
        ));
    }
    Ok(manifest)
}

fn write_manifest(path: &Path, manifest: &PatchTransactionManifest) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|_| {
        transaction_error(
            "transaction_manifest_encode_failed",
            &manifest.transaction_id,
            Value::Null,
        )
    })?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes).map_err(|error| {
        super::patch_io_error(
            "transaction_manifest_write_failed",
            "workspace.patch.transaction_manifest_write_failed",
            error,
        )
    })?;
    fs::rename(temporary, path).map_err(|error| {
        super::patch_io_error(
            "transaction_manifest_write_failed",
            "workspace.patch.transaction_manifest_write_failed",
            error,
        )
    })
}

fn encode_manifest_result(
    manifest: &PatchTransactionManifest,
    current: Option<&PatchShardRecord>,
    outcome: &str,
) -> Result<String, String> {
    let complete = manifest.shards.len() == manifest.shard_count
        && manifest.shards.iter().all(|shard| shard.state == "applied");
    super::encode_result(json!({
        "schema_version": 1,
        "source": "workspace_patch_transaction",
        "status": "ok",
        "action": "apply_patch_shard",
        "outcome": outcome,
        "transaction_id": manifest.transaction_id,
        "state": manifest.state,
        "complete": complete,
        "shard_count": manifest.shard_count,
        "applied_shard_count": manifest.shards.iter().filter(|shard| shard.state == "applied").count(),
        "current_shard": current,
        "shards": manifest.shards,
        "continuation": (!complete && manifest.state != "rewound").then(|| json!({
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

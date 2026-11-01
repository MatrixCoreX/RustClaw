use crate::runtime::host_filesystem::{BoundedFileReadError, HostFilesystemPolicy};
use claw_core::config::{ToolSandboxBackend, ToolSandboxMode};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use super::builtin_workspace_mutation::pre_dispatch_replan_error;
use crate::{AppState, ClaimedTask};

#[path = "builtin_workspace_patch_transaction.rs"]
mod transaction;
#[path = "builtin_workspace_unified_diff.rs"]
mod unified_diff;

const MAX_PATCH_BYTES: usize = 2 * 1024 * 1024;
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const MAX_CHECKPOINT_METADATA_BYTES: usize = 2 * 1024 * 1024;
const CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const CHECKPOINT_RESTORE_FAILED_STATE: &str = "restore_failed";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PatchCheckpoint {
    schema_version: u32,
    checkpoint_id: String,
    task_id: String,
    patch_id: String,
    state: String,
    created_at: i64,
    files: Vec<CheckpointFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointFile {
    path: String,
    existed: bool,
    before_sha256: Option<String>,
    after_sha256: Option<String>,
    backup_file: Option<String>,
    additions: Option<u64>,
    deletions: Option<u64>,
}

#[derive(Debug, Clone)]
struct PatchStat {
    path: String,
    additions: Option<u64>,
    deletions: Option<u64>,
}

pub(super) fn execute_workspace_patch(
    state: &AppState,
    task: Option<&ClaimedTask>,
    args: &Map<String, Value>,
) -> Result<String, String> {
    let task_id = task
        .map(|task| task.task_id.as_str())
        .unwrap_or("test-task");
    execute_workspace_patch_with_policy(
        &state.skill_rt.workspace_root,
        task_id,
        args,
        &state.skill_rt.host_filesystem,
        state.skill_rt.tools_policy.sandbox_backend,
    )
}

pub(super) fn execute_workspace_patch_with_policy(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
) -> Result<String, String> {
    let action = required_token(args, "action")?;
    match action {
        "apply_patch" => apply_patch(workspace_root, task_id, args, filesystem, backend),
        "apply_patch_shard" => {
            transaction::apply_patch_shard(workspace_root, task_id, args, filesystem, backend)
        }
        "transaction_status" => {
            transaction::transaction_status(workspace_root, task_id, args, filesystem)
        }
        "transaction_rewind" => {
            transaction::transaction_rewind(workspace_root, task_id, args, filesystem)
        }
        "preview_replace_text" | "replace_text" => {
            super::builtin_workspace_replace::execute_workspace_replace_with_policy(
                workspace_root,
                task_id,
                args,
                filesystem,
            )
        }
        "diff" => diff(workspace_root, args, filesystem, backend),
        "rewind" => rewind(workspace_root, args, filesystem),
        _ => Err(patch_error(
            "unsupported_action",
            "workspace.patch.unsupported_action",
            serde_json::json!({"action": action}),
        )),
    }
}

fn apply_patch(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
) -> Result<String, String> {
    let checkpoint_id = format!("patch_{}", uuid::Uuid::new_v4().simple());
    apply_patch_at_checkpoint(
        workspace_root,
        task_id,
        args,
        filesystem,
        backend,
        &checkpoint_id,
    )
}

fn apply_patch_at_checkpoint(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
    checkpoint_id: &str,
) -> Result<String, String> {
    ensure_only_keys(args, &["action", "patch", "precondition_hashes"])
        .map_err(pre_dispatch_replan_error)?;
    let patch = required_string(args, "patch").map_err(pre_dispatch_replan_error)?;
    if patch.is_empty() || patch.len() > MAX_PATCH_BYTES {
        return Err(pre_dispatch_replan_error(patch_error(
            "invalid_patch_size",
            "workspace.patch.invalid_size",
            serde_json::json!({
                "patch_bytes": patch.len(),
                "max_patch_bytes": MAX_PATCH_BYTES,
                "complete": false,
                "partial_reason": "single_transaction_safety_limit",
                "recovery": {
                    "kind": "verified_shard",
                    "action": "apply_patch_shard",
                    "max_shard_bytes": MAX_PATCH_BYTES,
                },
            }),
        )));
    }

    validate_checkpoint_id(checkpoint_id).map_err(pre_dispatch_replan_error)?;
    let checkpoint_id = checkpoint_id.to_string();
    let root = canonical_workspace_root(workspace_root).map_err(pre_dispatch_replan_error)?;
    let checkpoint_dir = create_checkpoint_dir(&root, &checkpoint_id, filesystem)
        .map_err(pre_dispatch_replan_error)?;
    let patch_file = checkpoint_dir.join("change.patch");
    if let Err(err) = filesystem.atomic_write_file_with_options(
        Some(&checkpoint_dir),
        &patch_file,
        patch.as_bytes(),
        false,
    ) {
        remove_checkpoint_dir(&root, &checkpoint_dir, filesystem);
        return Err(pre_dispatch_replan_error(patch_io_error(
            "patch_stage_failed",
            "workspace.patch.stage_failed",
            err,
        )));
    }

    let git_workspace = is_git_workspace(&root, filesystem, backend)?;
    let mut git_path_strip = 1;
    let mut pure_patch = None;
    let stats = if git_workspace {
        match inspect_patch(&root, &patch_file, filesystem, backend) {
            Ok((stats, path_strip)) => {
                git_path_strip = path_strip;
                stats
            }
            Err(err) => {
                remove_checkpoint_dir(&root, &checkpoint_dir, filesystem);
                return Err(pre_dispatch_replan_error(err));
            }
        }
    } else {
        match unified_diff::inspect(&root, patch, filesystem) {
            Ok((parsed, pure_stats)) => {
                pure_patch = Some(parsed);
                pure_stats
                    .into_iter()
                    .map(|stat| PatchStat {
                        path: stat.path,
                        additions: Some(stat.additions),
                        deletions: Some(stat.deletions),
                    })
                    .collect()
            }
            Err(err) => {
                remove_checkpoint_dir(&root, &checkpoint_dir, filesystem);
                return Err(pre_dispatch_replan_error(err));
            }
        }
    };
    if stats.is_empty() {
        remove_checkpoint_dir(&root, &checkpoint_dir, filesystem);
        return Err(pre_dispatch_replan_error(patch_error(
            "empty_patch",
            "workspace.patch.empty",
            Value::Null,
        )));
    }
    for stat in &stats {
        if let Err(err) = validate_relative_patch_path(&root, &stat.path) {
            remove_checkpoint_dir(&root, &checkpoint_dir, filesystem);
            return Err(pre_dispatch_replan_error(err));
        }
    }
    if let Err(err) = verify_preconditions(&root, args, &stats, filesystem) {
        remove_checkpoint_dir(&root, &checkpoint_dir, filesystem);
        return Err(pre_dispatch_replan_error(err));
    }

    let mut files = match snapshot_files(&root, &checkpoint_dir, &stats, filesystem) {
        Ok(files) => files,
        Err(err) => {
            remove_checkpoint_dir(&root, &checkpoint_dir, filesystem);
            return Err(pre_dispatch_replan_error(err));
        }
    };
    let prepared_manifest = PatchCheckpoint {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        checkpoint_id: checkpoint_id.clone(),
        task_id: task_id.to_string(),
        patch_id: format!("sha256:{}", sha256_hex(patch.as_bytes())),
        state: "prepared".to_string(),
        created_at: crate::now_ts_u64() as i64,
        files: files.clone(),
    };
    if let Err(error) = validate_checkpoint_capacity(&prepared_manifest)
        .and_then(|()| write_manifest(&checkpoint_dir, &prepared_manifest, filesystem))
    {
        remove_checkpoint_dir(&root, &checkpoint_dir, filesystem);
        return Err(pre_dispatch_replan_error(error));
    }
    if git_workspace {
        let check = git_apply(
            &root,
            &patch_file,
            true,
            git_path_strip,
            filesystem,
            backend,
        )
        .map_err(|err| {
            pre_dispatch_replan_error(patch_io_error(
                "patch_check_failed",
                "workspace.patch.check_failed",
                err,
            ))
        })?;
        if !check.status.success() {
            remove_checkpoint_dir(&root, &checkpoint_dir, filesystem);
            return Err(pre_dispatch_replan_error(command_error(
                "patch_context_mismatch",
                "workspace.patch.context_mismatch",
                &check,
            )));
        }

        let applied = match git_apply(
            &root,
            &patch_file,
            false,
            git_path_strip,
            filesystem,
            backend,
        ) {
            Ok(output) => output,
            Err(error) => {
                return Err(compensate_failed_patch(
                    &root,
                    &checkpoint_dir,
                    &files,
                    patch_io_error("patch_apply_failed", "workspace.patch.apply_failed", error),
                    filesystem,
                ))
            }
        };
        if !applied.status.success() {
            return Err(compensate_failed_patch(
                &root,
                &checkpoint_dir,
                &files,
                command_error(
                    "patch_apply_failed",
                    "workspace.patch.apply_failed",
                    &applied,
                ),
                filesystem,
            ));
        }
    } else if let Err(err) = unified_diff::apply(
        &root,
        pure_patch
            .as_ref()
            .expect("pure_patch_present_for_non_git_workspace"),
        filesystem,
    ) {
        return Err(compensate_failed_patch(
            &root,
            &checkpoint_dir,
            &files,
            err,
            filesystem,
        ));
    }

    if let Err(err) = capture_after_hashes(&root, &mut files, filesystem) {
        return Err(compensate_failed_patch(
            &root,
            &checkpoint_dir,
            &files,
            err,
            filesystem,
        ));
    }

    let patch_id = format!("sha256:{}", sha256_hex(patch.as_bytes()));
    let manifest = PatchCheckpoint {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        checkpoint_id: checkpoint_id.clone(),
        task_id: task_id.to_string(),
        patch_id: patch_id.clone(),
        state: "applied".to_string(),
        created_at: prepared_manifest.created_at,
        files,
    };
    publish_applied_checkpoint(&checkpoint_dir, &manifest, filesystem)?;

    let additions = manifest
        .files
        .iter()
        .filter_map(|file| file.additions)
        .sum::<u64>();
    let deletions = manifest
        .files
        .iter()
        .filter_map(|file| file.deletions)
        .sum::<u64>();
    encode_result(serde_json::json!({
        "schema_version": 1,
        "source": "workspace_patch",
        "status": "ok",
        "action": "apply_patch",
        "message_key": "workspace.patch.applied",
        "patch_id": patch_id,
        "checkpoint_id": checkpoint_id,
        "isolation_root": "workspace://current",
        "reversible": true,
        "changed_files": manifest.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(),
        "additions": additions,
        "deletions": deletions,
        "hunk_count": patch.lines().filter(|line| line.starts_with("@@ ")).count(),
        "changed_hunks": patch.lines().filter(|line| line.starts_with("@@ ")).count(),
        "patch_engine": if git_workspace { "git" } else { "pure_rust" },
        "files": manifest.files,
        "artifact_refs": [
            {"kind": "workspace_patch", "ref": format!("workspace_patch:{patch_id}")},
            {"kind": "workspace_checkpoint", "ref": format!("workspace_checkpoint:{}", manifest.checkpoint_id)},
        ],
    }))
}

fn publish_applied_checkpoint(
    checkpoint_dir: &Path,
    manifest: &PatchCheckpoint,
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    write_manifest(checkpoint_dir, manifest, filesystem).map_err(|error| {
        super::checkpoint_commit_unconfirmed_error(
            "workspace_patch",
            "workspace.patch.commit_unconfirmed",
            &manifest.task_id,
            &manifest.checkpoint_id,
            "commit",
            error,
        )
    })
}

fn diff(
    workspace_root: &Path,
    args: &Map<String, Value>,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
) -> Result<String, String> {
    ensure_only_keys(args, &["action", "checkpoint_id", "paths"])?;
    let root = canonical_workspace_root(workspace_root)?;
    if let Some(checkpoint_id) = optional_string(args, "checkpoint_id") {
        validate_checkpoint_id(checkpoint_id)?;
        let checkpoint_dir = resolve_checkpoint_dir(&root, checkpoint_id, filesystem)?;
        if super::builtin_workspace_mutation::checkpoint_is_structured_mutation(
            &checkpoint_dir,
            filesystem,
        )? {
            return super::builtin_workspace_mutation::structured_mutation_diff_with_policy(
                workspace_root,
                checkpoint_id,
                filesystem,
            );
        }
        let manifest = read_manifest(&checkpoint_dir, filesystem)?;
        // Saved diffs re-admit current paths without materializing their contents.
        for file in &manifest.files {
            drop(open_optional_regular_file(
                &root,
                &root.join(&file.path),
                filesystem,
            )?);
        }
        let bytes = filesystem
            .read_file_bounded(
                Some(&checkpoint_dir),
                &checkpoint_dir.join("change.patch"),
                MAX_PATCH_BYTES,
            )
            .map_err(|error| match error {
                BoundedFileReadError::Io(error) => patch_io_error(
                    "checkpoint_patch_read_failed",
                    "workspace.patch.read_failed",
                    error,
                ),
                BoundedFileReadError::LimitExceeded {
                    limit_bytes,
                    observed_bytes_at_least,
                } => patch_error(
                    "checkpoint_too_large",
                    "workspace.patch.checkpoint_too_large",
                    serde_json::json!({"max_bytes":limit_bytes,
                        "patch_bytes_at_least":observed_bytes_at_least}),
                ),
            })?;
        let patch = String::from_utf8(bytes).map_err(|error| {
            patch_io_error(
                "checkpoint_patch_read_failed",
                "workspace.patch.read_failed",
                std::io::Error::new(std::io::ErrorKind::InvalidData, error),
            )
        })?;
        let observed_patch_id = sha256_label(patch.as_bytes());
        let expected_patch_id = manifest
            .patch_id
            .strip_prefix("sha256:")
            .filter(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .map(|_| manifest.patch_id.as_str());
        if expected_patch_id != Some(observed_patch_id.as_str()) {
            return Err(patch_error(
                "checkpoint_patch_hash_mismatch",
                "workspace.patch.patch_hash_mismatch",
                serde_json::json!({"expected_patch_id":expected_patch_id,
                    "expected_patch_id_valid":expected_patch_id.is_some(),
                    "observed_patch_id":observed_patch_id}),
            ));
        }
        return encode_result(serde_json::json!({
            "schema_version": 1,
            "source": "workspace_patch",
            "status": "ok",
            "action": "diff",
            "message_key": "workspace.patch.diff_ready",
            "checkpoint_id": manifest.checkpoint_id,
            "patch_id": manifest.patch_id,
            "state": manifest.state,
            "isolation_root": "workspace://current",
            "reversible": manifest.state == "applied",
            "changed_files": manifest.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(),
            "patch": patch,
        }));
    }

    let paths = optional_paths(args, "paths")?;
    for path in &paths {
        validate_relative_patch_path(&root, path)?;
    }
    let mut command = git_command(&root, filesystem, backend, false)?;
    command.arg("-C").arg(&root).args([
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--binary",
        "--",
    ]);
    command.args(&paths);
    let output = command.output().map_err(|err| {
        patch_io_error("workspace_diff_failed", "workspace.patch.diff_failed", err)
    })?;
    if !output.status.success() {
        return Err(command_error(
            "workspace_diff_failed",
            "workspace.patch.diff_failed",
            &output,
        ));
    }
    if output.stdout.len() > MAX_DIFF_BYTES {
        return Err(patch_error(
            "diff_too_large",
            "workspace.patch.diff_too_large",
            serde_json::json!({
                "diff_bytes": output.stdout.len(),
                "max_diff_bytes": MAX_DIFF_BYTES,
            }),
        ));
    }
    let patch = String::from_utf8(output.stdout).map_err(|_| {
        patch_error(
            "diff_not_utf8",
            "workspace.patch.diff_not_utf8",
            Value::Null,
        )
    })?;
    encode_result(serde_json::json!({
        "schema_version": 1,
        "source": "workspace_patch",
        "status": "ok",
        "action": "diff",
        "message_key": "workspace.patch.diff_ready",
        "changed": !patch.is_empty(),
        "paths": paths,
        "patch": patch,
    }))
}

#[cfg(test)]
#[path = "builtin_workspace_checkpoint_finalization_tests.rs"]
mod finalization_tests;

fn rewind(
    workspace_root: &Path,
    args: &Map<String, Value>,
    filesystem: &HostFilesystemPolicy,
) -> Result<String, String> {
    ensure_only_keys(args, &["action", "checkpoint_id"])?;
    let root = canonical_workspace_root(workspace_root)?;
    let checkpoint_id = required_string(args, "checkpoint_id")?;
    validate_checkpoint_id(checkpoint_id)?;
    let checkpoint_dir = resolve_checkpoint_dir(&root, checkpoint_id, filesystem)?;
    if super::builtin_workspace_mutation::checkpoint_is_structured_mutation(
        &checkpoint_dir,
        filesystem,
    )? {
        return super::builtin_workspace_mutation::rewind_structured_mutation_with_policy(
            workspace_root,
            checkpoint_id,
            filesystem,
        );
    }
    let mut manifest = read_manifest(&checkpoint_dir, filesystem)?;
    if manifest.state != "applied" {
        return Err(patch_error(
            "checkpoint_not_applied",
            "workspace.patch.checkpoint_not_applied",
            serde_json::json!({
                "checkpoint_id": checkpoint_id,
                "state": manifest.state,
            }),
        ));
    }

    validate_restore_metadata(&root, &manifest.files)?;
    for file in &manifest.files {
        validate_relative_patch_path(&root, &file.path)?;
        let current_hash = hash_existing_file(&root, &root.join(&file.path), filesystem)?;
        if current_hash != file.after_sha256 {
            return Err(patch_error(
                "rewind_precondition_failed",
                "workspace.patch.rewind_precondition_failed",
                serde_json::json!({
                    "checkpoint_id": checkpoint_id,
                    "path": file.path,
                    "expected_sha256": file.after_sha256,
                    "actual_sha256": current_hash,
                }),
            ));
        }
    }
    restore_snapshot_strict(&root, &checkpoint_dir, &manifest.files, filesystem)?;
    manifest.state = "rewound".to_string();
    write_manifest(&checkpoint_dir, &manifest, filesystem)?;
    encode_result(serde_json::json!({
        "schema_version": 1,
        "source": "workspace_patch",
        "status": "ok",
        "action": "rewind",
        "message_key": "workspace.patch.rewound",
        "checkpoint_id": checkpoint_id,
        "patch_id": manifest.patch_id,
        "compensates_checkpoint_id": checkpoint_id,
        "compensates_patch_id": manifest.patch_id,
        "state": "rewound",
        "isolation_root": "workspace://current",
        "reversible": false,
        "restored_files": manifest.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(),
        "artifact_refs": [
            {"kind": "workspace_checkpoint", "ref": format!("workspace_checkpoint:{checkpoint_id}")},
        ],
    }))
}

fn inspect_patch(
    root: &Path,
    patch_file: &Path,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
) -> Result<(Vec<PatchStat>, u8), String> {
    let output = git_command(root, filesystem, backend, false)?
        .arg("-C")
        .arg(root)
        .args(["apply", "--numstat", "-z", "-p0", "--"])
        .arg(patch_file)
        .output()
        .map_err(|err| {
            patch_io_error(
                "patch_inspect_failed",
                "workspace.patch.inspect_failed",
                err,
            )
        })?;
    if !output.status.success() {
        return Err(command_error(
            "invalid_patch",
            "workspace.patch.invalid",
            &output,
        ));
    }
    let mut stats = parse_numstat(&output.stdout)?;
    let prefixed = stats
        .iter()
        .filter(|stat| stat.path.starts_with("a/") || stat.path.starts_with("b/"))
        .count();
    if prefixed != 0 && prefixed != stats.len() {
        return Err(patch_error(
            "mixed_patch_path_conventions",
            "workspace.patch.mixed_path_conventions",
            Value::Null,
        ));
    }
    let strip = u8::from(prefixed != 0);
    if strip == 1 {
        for stat in &mut stats {
            stat.path = stat.path[2..].to_string();
        }
    }
    Ok((stats, strip))
}

fn parse_numstat(bytes: &[u8]) -> Result<Vec<PatchStat>, String> {
    let records = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut stats = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if record.is_empty() {
            continue;
        }
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let additions = parse_numstat_count(fields.next())?;
        let deletions = parse_numstat_count(fields.next())?;
        let path_bytes = fields.next().ok_or_else(|| {
            patch_error(
                "invalid_patch_stat",
                "workspace.patch.invalid_stat",
                Value::Null,
            )
        })?;
        if path_bytes.is_empty() {
            return Err(patch_error(
                "rename_not_supported",
                "workspace.patch.rename_not_supported",
                Value::Null,
            ));
        }
        let path = std::str::from_utf8(path_bytes).map_err(|_| {
            patch_error(
                "path_not_utf8",
                "workspace.patch.path_not_utf8",
                Value::Null,
            )
        })?;
        if stats.iter().any(|stat: &PatchStat| stat.path == path) {
            return Err(patch_error(
                "duplicate_patch_path",
                "workspace.patch.duplicate_path",
                serde_json::json!({"path": path}),
            ));
        }
        stats.push(PatchStat {
            path: path.to_string(),
            additions,
            deletions,
        });
    }
    Ok(stats)
}

fn parse_numstat_count(field: Option<&[u8]>) -> Result<Option<u64>, String> {
    let field = field.ok_or_else(|| {
        patch_error(
            "invalid_patch_stat",
            "workspace.patch.invalid_stat",
            Value::Null,
        )
    })?;
    if field == b"-" {
        return Ok(None);
    }
    let value = std::str::from_utf8(field)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            patch_error(
                "invalid_patch_stat",
                "workspace.patch.invalid_stat",
                Value::Null,
            )
        })?;
    Ok(Some(value))
}

fn snapshot_files(
    root: &Path,
    checkpoint_dir: &Path,
    stats: &[PatchStat],
    filesystem: &HostFilesystemPolicy,
) -> Result<Vec<CheckpointFile>, String> {
    let mut targets = std::collections::BTreeSet::new();
    for stat in stats {
        if !targets.insert(lexical_workspace_path(root, &stat.path)?) {
            return Err(patch_error(
                "duplicate_patch_path",
                "workspace.patch.duplicate_path",
                serde_json::json!({"path":stat.path}),
            ));
        }
    }
    let backup_dir = checkpoint_dir.join("before");
    filesystem
        .create_directory(Some(checkpoint_dir), &backup_dir, true)
        .map_err(|err| {
            patch_io_error(
                "checkpoint_create_failed",
                "workspace.patch.checkpoint_create_failed",
                err,
            )
        })?;
    let mut files = Vec::with_capacity(stats.len());
    for (index, stat) in stats.iter().enumerate() {
        let path = root.join(&stat.path);
        let bytes = read_optional_regular_file(root, &path, filesystem)?;
        let backup_file = bytes.as_ref().map(|_| format!("before/{index}.bin"));
        if let (Some(bytes), Some(backup_file)) = (&bytes, &backup_file) {
            filesystem
                .atomic_write_file_with_options(
                    Some(checkpoint_dir),
                    &checkpoint_dir.join(backup_file),
                    bytes,
                    false,
                )
                .map_err(|err| {
                    patch_io_error(
                        "checkpoint_write_failed",
                        "workspace.patch.checkpoint_write_failed",
                        err,
                    )
                })?;
        }
        files.push(CheckpointFile {
            path: stat.path.clone(),
            existed: bytes.is_some(),
            before_sha256: bytes.as_deref().map(sha256_label),
            after_sha256: None,
            backup_file,
            additions: stat.additions,
            deletions: stat.deletions,
        });
    }
    Ok(files)
}

fn capture_after_hashes(
    root: &Path,
    files: &mut [CheckpointFile],
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    for file in files {
        validate_relative_patch_path(root, &file.path)?;
        file.after_sha256 = hash_existing_file(root, &root.join(&file.path), filesystem)?;
    }
    Ok(())
}

fn verify_preconditions(
    root: &Path,
    args: &Map<String, Value>,
    stats: &[PatchStat],
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    let Some(value) = args.get("precondition_hashes") else {
        return Ok(());
    };
    let object = value.as_object().ok_or_else(|| {
        patch_error(
            "invalid_precondition_hashes",
            "workspace.patch.invalid_preconditions",
            Value::Null,
        )
    })?;
    for (path, expected) in object {
        if !stats.iter().any(|stat| stat.path == *path) {
            return Err(patch_error(
                "precondition_path_not_in_patch",
                "workspace.patch.precondition_path_not_in_patch",
                serde_json::json!({"path": path}),
            ));
        }
        validate_relative_patch_path(root, path)?;
        let expected = expected.as_str().ok_or_else(|| {
            patch_error(
                "invalid_precondition_hash",
                "workspace.patch.invalid_precondition_hash",
                serde_json::json!({"path": path}),
            )
        })?;
        let actual = hash_existing_file(root, &root.join(path), filesystem)?;
        let matches = match expected {
            "missing" => actual.is_none(),
            value => actual.as_deref() == Some(value),
        };
        if !matches {
            return Err(patch_error(
                "patch_precondition_failed",
                "workspace.patch.precondition_failed",
                serde_json::json!({
                    "path": path,
                    "expected_sha256": expected,
                    "actual_sha256": actual,
                }),
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_relative_patch_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let target = lexical_workspace_path(root, path)?;
    let relative = target
        .strip_prefix(root)
        .map_err(|_| invalid_path_error(path))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(patch_error(
                    "symlink_path_denied",
                    "workspace.patch.symlink_denied",
                    serde_json::json!({"path": path}),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => {
                return Err(patch_io_error(
                    "path_inspection_failed",
                    "workspace.patch.path_inspection_failed",
                    err,
                ));
            }
        }
    }
    Ok(target)
}

fn read_optional_regular_file(
    root: &Path,
    path: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<Option<Vec<u8>>, String> {
    let Some(mut file) = open_optional_regular_file(root, path, filesystem)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| regular_file_error(path, error))?;
    Ok(Some(bytes))
}

fn open_optional_regular_file(
    root: &Path,
    path: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<Option<fs::File>, String> {
    match filesystem.open_file(Some(root), path) {
        Ok((file, _)) => Ok(Some(file)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(regular_file_error(path, err)),
    }
}

fn regular_file_error(path: &Path, err: std::io::Error) -> String {
    if HostFilesystemPolicy::is_private_access_denied(&err) {
        patch_io_error(
            "host_private_directory_access_denied",
            "workspace.patch.private_path_denied",
            err,
        )
    } else if err.kind() == std::io::ErrorKind::PermissionDenied {
        patch_error(
            "unsupported_file_type",
            "workspace.patch.unsupported_file_type",
            serde_json::json!({"path": path.display().to_string()}),
        )
    } else {
        patch_io_error("file_read_failed", "workspace.patch.file_read_failed", err)
    }
}

fn hash_existing_file(
    root: &Path,
    path: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<Option<String>, String> {
    let Some(mut file) = open_optional_regular_file(root, path, filesystem)? else {
        return Ok(None);
    };
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let size = match file.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|error| regular_file_error(path, error))?,
        };
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    Ok(Some(format!("sha256:{:x}", digest.finalize())))
}

fn compensate_failed_patch(
    root: &Path,
    checkpoint_dir: &Path,
    files: &[CheckpointFile],
    cause: String,
    filesystem: &HostFilesystemPolicy,
) -> String {
    if let Err(restore_error) = restore_snapshot_strict(root, checkpoint_dir, files, filesystem) {
        if let Ok(mut manifest) = read_manifest(checkpoint_dir, filesystem) {
            manifest.state = CHECKPOINT_RESTORE_FAILED_STATE.to_string();
            if let Err(error) = write_manifest(checkpoint_dir, &manifest, filesystem) {
                tracing::error!(error = %error, "workspace_patch_recovery_state_write_failed");
            }
        }
        let retained = match filesystem.open_directory(Some(root), checkpoint_dir) {
            Ok(_) => Some(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
            Err(_) => None,
        };
        let checkpoint_id = checkpoint_dir.file_name().and_then(|name| name.to_str());
        return super::builtin_error(
            "workspace_patch",
            "patch_apply_and_restore_failed",
            "workspace.patch.restore_failed",
            None,
            None,
            Some(serde_json::json!({
                "error_code":"patch_apply_and_restore_failed",
                "message_key":"workspace.patch.restore_failed",
                "retryable":false, "failure_phase":"execution", "side_effect_applied":null,
                "checkpoint_retained":retained, "checkpoint_id":checkpoint_id,
                "artifact_refs":checkpoint_id.map(|id| serde_json::json!([{
                    "kind":"workspace_checkpoint", "ref":format!("workspace_checkpoint:{id}")
                }])),
                "details":{"operation_error":cause, "restore_error":restore_error},
            })),
        );
    }
    remove_checkpoint_dir(root, checkpoint_dir, filesystem);
    cause
}

fn validate_restore_metadata(root: &Path, files: &[CheckpointFile]) -> Result<(), String> {
    if files.is_empty() {
        return Err(patch_error(
            "checkpoint_invalid",
            "workspace.patch.checkpoint_invalid",
            serde_json::json!({"reason":"empty_file_set"}),
        ));
    }
    let mut targets = std::collections::BTreeSet::new();
    // Complete metadata admission precedes backup IO and every target effect.
    for (index, file) in files.iter().enumerate() {
        let target = lexical_workspace_path(root, &file.path)?;
        let invalid = |code, reason| {
            patch_error(
                code,
                if code == "checkpoint_backup_invalid" {
                    "workspace.patch.backup_invalid"
                } else {
                    "workspace.patch.checkpoint_invalid"
                },
                serde_json::json!({"path":file.path,"reason":reason}),
            )
        };
        if !targets.insert(target) {
            return Err(invalid("checkpoint_invalid", "duplicate_target"));
        }
        if file.existed {
            let backup = file.backup_file.as_deref().ok_or_else(|| {
                patch_error(
                    "checkpoint_backup_missing",
                    "workspace.patch.checkpoint_backup_missing",
                    serde_json::json!({"path":file.path}),
                )
            })?;
            if backup != format!("before/{index}.bin") {
                return Err(invalid(
                    "checkpoint_backup_invalid",
                    "backup_reference_invalid",
                ));
            }
            if !file
                .before_sha256
                .as_deref()
                .is_some_and(valid_sha256_label)
            {
                return Err(invalid("checkpoint_backup_invalid", "before_hash_invalid"));
            }
        } else if file.backup_file.is_some() || file.before_sha256.is_some() {
            return Err(invalid(
                "checkpoint_invalid",
                "missing_preimage_has_payload",
            ));
        }
        if file
            .after_sha256
            .as_deref()
            .is_some_and(|hash| !valid_sha256_label(hash))
        {
            return Err(invalid("checkpoint_invalid", "after_hash_invalid"));
        }
    }
    Ok(())
}

fn valid_sha256_label(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn restore_snapshot_strict(
    root: &Path,
    checkpoint_dir: &Path,
    files: &[CheckpointFile],
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    validate_restore_metadata(root, files)?;
    let mut restored = Vec::new();
    for file in files {
        let path = validate_restore_path(root, &file.path)?;
        match filesystem.metadata(Some(root), &path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(patch_io_error(
                    "restore_inspection_failed",
                    "workspace.patch.restore_failed",
                    error,
                ))
            }
        }
        let bytes = if file.existed {
            let backup = file.backup_file.as_deref().ok_or_else(|| {
                patch_error(
                    "checkpoint_backup_missing",
                    "workspace.patch.checkpoint_backup_missing",
                    serde_json::json!({"path": file.path}),
                )
            })?;
            let bytes = filesystem
                .read_file(Some(checkpoint_dir), &checkpoint_dir.join(backup))
                .map_err(|err| {
                    patch_io_error(
                        "checkpoint_read_failed",
                        "workspace.patch.checkpoint_read_failed",
                        err,
                    )
                })?;
            if file.before_sha256.as_deref() != Some(sha256_label(&bytes).as_str()) {
                return Err(patch_error(
                    "checkpoint_backup_invalid",
                    "workspace.patch.backup_invalid",
                    serde_json::json!({"path": file.path}),
                ));
            }
            Some(bytes)
        } else {
            None
        };
        restored.push((path, bytes));
    }
    for (path, bytes) in restored {
        if let Some(bytes) = bytes {
            if filesystem
                .metadata(Some(root), &path)
                .is_ok_and(|metadata| metadata.file_type == rustix::fs::FileType::Symlink)
            {
                filesystem
                    .remove_path(Some(root), &path, false)
                    .map_err(|error| {
                        patch_io_error(
                            "restore_remove_failed",
                            "workspace.patch.restore_remove_failed",
                            error,
                        )
                    })?;
            }
            filesystem
                .atomic_write_file_with_options(Some(root), &path, &bytes, true)
                .map_err(|err| {
                    patch_io_error(
                        "restore_write_failed",
                        "workspace.patch.restore_write_failed",
                        err,
                    )
                })?;
        } else {
            match filesystem.remove_path(Some(root), &path, false) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(patch_io_error(
                        "restore_remove_failed",
                        "workspace.patch.restore_remove_failed",
                        err,
                    ));
                }
            }
        }
    }
    Ok(())
}

fn git_apply(
    root: &Path,
    patch_file: &Path,
    check: bool,
    path_strip: u8,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
) -> std::io::Result<Output> {
    let mut command =
        git_command(root, filesystem, backend, !check).map_err(std::io::Error::other)?;
    command.arg("-C").arg(root).arg("apply");
    command.arg(format!("-p{path_strip}"));
    if check {
        command.arg("--check");
    }
    command.args(["--whitespace=nowarn", "--"]);
    command.arg(patch_file).output()
}

fn is_git_workspace(
    root: &Path,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
) -> Result<bool, String> {
    match filesystem.metadata(Some(root), &root.join(".git")) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(patch_io_error(
                "git_metadata_inspection_failed",
                "workspace.patch.git_metadata_failed",
                error,
            ))
        }
    }
    Ok(git_command(root, filesystem, backend, false)?
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .is_ok_and(|output| {
            if !output.status.success() {
                return false;
            }
            let Ok(path) = std::str::from_utf8(&output.stdout) else {
                return false;
            };
            Path::new(path.trim_end_matches(['\r', '\n']))
                .canonicalize()
                .is_ok_and(|git_root| git_root == root)
        }))
}

fn git_command(
    root: &Path,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
    writable: bool,
) -> Result<Command, String> {
    let private = filesystem.process_directories().map_err(|error| {
        patch_io_error(
            "host_policy_changed",
            "workspace.patch.host_policy_changed",
            error,
        )
    })?;
    let prepared = crate::process_sandbox::prepare_process_command(
        "git",
        crate::process_sandbox::ProcessSandboxRequest {
            mode: if writable {
                ToolSandboxMode::WorkspaceWrite
            } else {
                ToolSandboxMode::ReadOnly
            },
            backend,
            workspace_root: root,
            execution_root: root,
            network: crate::process_sandbox::ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
            host_private_directories: &private,
            browser_subprocess: false,
        },
    )
    .map_err(|code| patch_error("sandbox_unavailable", code, Value::Null))?;
    let mut command = prepared.command.into_std();
    command
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .current_dir(root)
        .env_clear()
        .envs(crate::skills::SkillRunnerEnvironment::capture().into_pairs())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(std::process::Stdio::null());
    Ok(command)
}

pub(super) fn canonical_workspace_root(root: &Path) -> Result<PathBuf, String> {
    root.canonicalize().map_err(|err| {
        patch_io_error(
            "workspace_unavailable",
            "workspace.patch.workspace_unavailable",
            err,
        )
    })
}

fn checkpoint_root(root: &Path) -> PathBuf {
    claw_core::workspace_state::workspace_state_root(root).join("checkpoints")
}

pub(super) fn create_checkpoint_dir(
    root: &Path,
    checkpoint_id: &str,
    filesystem: &HostFilesystemPolicy,
) -> Result<PathBuf, String> {
    let state_dir = claw_core::workspace_state::workspace_state_root(root);
    reject_symlink_if_present(&state_dir)?;
    filesystem
        .create_directory(Some(root), &state_dir, true)
        .map_err(|err| {
            patch_io_error(
                "checkpoint_create_failed",
                "workspace.patch.checkpoint_create_failed",
                err,
            )
        })?;
    let checkpoints = checkpoint_root(root);
    reject_symlink_if_present(&checkpoints)?;
    filesystem
        .create_directory(Some(root), &checkpoints, true)
        .map_err(|err| {
            patch_io_error(
                "checkpoint_create_failed",
                "workspace.patch.checkpoint_create_failed",
                err,
            )
        })?;
    let canonical = checkpoints.canonicalize().map_err(|err| {
        patch_io_error(
            "checkpoint_create_failed",
            "workspace.patch.checkpoint_create_failed",
            err,
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(patch_error(
            "checkpoint_path_outside_workspace",
            "workspace.patch.checkpoint_path_outside_workspace",
            Value::Null,
        ));
    }
    let checkpoint_dir = canonical.join(checkpoint_id);
    filesystem
        .create_directory(Some(root), &checkpoint_dir, false)
        .map_err(|err| {
            patch_io_error(
                "checkpoint_create_failed",
                "workspace.patch.checkpoint_create_failed",
                err,
            )
        })?;
    restrict_directory_permissions(root, &checkpoint_dir, filesystem)?;
    Ok(checkpoint_dir)
}

pub(super) fn resolve_checkpoint_dir(
    root: &Path,
    checkpoint_id: &str,
    filesystem: &HostFilesystemPolicy,
) -> Result<PathBuf, String> {
    filesystem
        .open_directory(Some(root), &checkpoint_root(root))
        .map_err(|error| {
            patch_io_error(
                "checkpoint_read_failed",
                "workspace.patch.checkpoint_read_failed",
                error,
            )
        })?;
    let checkpoints = checkpoint_root(root).canonicalize().map_err(|err| {
        patch_io_error(
            "checkpoint_read_failed",
            "workspace.patch.checkpoint_read_failed",
            err,
        )
    })?;
    if !checkpoints.starts_with(root) {
        return Err(patch_error(
            "checkpoint_path_outside_workspace",
            "workspace.patch.checkpoint_path_outside_workspace",
            Value::Null,
        ));
    }
    let checkpoint_dir = checkpoints.join(checkpoint_id);
    filesystem
        .open_directory(Some(root), &checkpoint_dir)
        .map_err(|error| {
            patch_io_error(
                "checkpoint_read_failed",
                "workspace.patch.checkpoint_read_failed",
                error,
            )
        })?;
    reject_symlink_if_present(&checkpoint_dir)?;
    let canonical = checkpoint_dir.canonicalize().map_err(|err| {
        patch_io_error(
            "checkpoint_read_failed",
            "workspace.patch.checkpoint_read_failed",
            err,
        )
    })?;
    if !canonical.starts_with(&checkpoints) {
        return Err(patch_error(
            "checkpoint_path_outside_workspace",
            "workspace.patch.checkpoint_path_outside_workspace",
            Value::Null,
        ));
    }
    Ok(canonical)
}

fn reject_symlink_if_present(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(patch_error(
            "checkpoint_symlink_denied",
            "workspace.patch.checkpoint_symlink_denied",
            Value::Null,
        )),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(patch_io_error(
            "checkpoint_inspection_failed",
            "workspace.patch.checkpoint_inspection_failed",
            err,
        )),
    }
}

fn validate_restore_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let target = lexical_workspace_path(root, path)?;
    let parent = target.parent().unwrap_or(root);
    let relative_parent = parent
        .strip_prefix(root)
        .map_err(|_| invalid_path_error(path))?;
    let mut current = root.to_path_buf();
    for component in relative_parent.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(patch_error(
                    "symlink_path_denied",
                    "workspace.patch.symlink_denied",
                    serde_json::json!({"path": path}),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => {
                return Err(patch_io_error(
                    "path_inspection_failed",
                    "workspace.patch.path_inspection_failed",
                    err,
                ));
            }
        }
    }
    Ok(target)
}

fn lexical_workspace_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || Path::new(path).is_absolute() {
        return Err(invalid_path_error(path));
    }
    let mut relative = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(value) => {
                if value == ".git"
                    || value
                        .to_str()
                        .is_some_and(claw_core::workspace_state::is_known_workspace_state_dir_name)
                {
                    return Err(invalid_path_error(path));
                }
                relative.push(value);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(invalid_path_error(path));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(invalid_path_error(path));
    }
    Ok(root.join(relative))
}

fn write_manifest(
    checkpoint_dir: &Path,
    manifest: &PatchCheckpoint,
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    let bytes = encode_checkpoint_manifest(manifest, "current_metadata")?;
    filesystem
        .atomic_write_file_with_options(
            Some(checkpoint_dir),
            &checkpoint_dir.join("manifest.json"),
            &bytes,
            false,
        )
        .map_err(|err| {
            patch_io_error(
                "checkpoint_write_failed",
                "workspace.patch.checkpoint_write_failed",
                err,
            )
        })
}

fn validate_checkpoint_capacity(manifest: &PatchCheckpoint) -> Result<(), String> {
    let mut projected = manifest.clone();
    // This is the longest produced state; each optional after hash may become
    // a fixed-width digest. No path, statistic or backup name grows at dispatch.
    projected.state = CHECKPOINT_RESTORE_FAILED_STATE.to_string();
    let digest = sha256_label(&[]);
    for file in &mut projected.files {
        file.after_sha256 = Some(digest.clone());
    }
    encode_checkpoint_manifest(&projected, "projected_metadata").map(|_| ())
}

fn encode_checkpoint_manifest(
    manifest: &PatchCheckpoint,
    metadata_scope: &str,
) -> Result<Vec<u8>, String> {
    use super::builtin_workspace_checkpoint_json::{encode_pretty, EncodeError};
    encode_pretty(manifest, MAX_CHECKPOINT_METADATA_BYTES).map_err(|error| match error {
        EncodeError::LimitExceeded { limit_bytes } => patch_error(
            "checkpoint_too_large",
            "workspace.patch.checkpoint_too_large",
            serde_json::json!({
                "max_bytes": limit_bytes,
                "manifest_bytes_at_least": limit_bytes + 1,
                "metadata_scope": metadata_scope,
            }),
        ),
        EncodeError::Serialization(error) => patch_error(
            "checkpoint_encode_failed",
            "workspace.patch.checkpoint_encode_failed",
            serde_json::json!({"error":error.to_string(),"metadata_scope":metadata_scope}),
        ),
    })
}

fn read_manifest(
    checkpoint_dir: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<PatchCheckpoint, String> {
    let bytes = filesystem
        .read_file_bounded(
            Some(checkpoint_dir),
            &checkpoint_dir.join("manifest.json"),
            MAX_CHECKPOINT_METADATA_BYTES,
        )
        .map_err(|error| match error {
            BoundedFileReadError::Io(error) => patch_io_error(
                "checkpoint_read_failed",
                "workspace.patch.checkpoint_read_failed",
                error,
            ),
            BoundedFileReadError::LimitExceeded {
                limit_bytes,
                observed_bytes_at_least,
            } => patch_error(
                "checkpoint_too_large",
                "workspace.patch.checkpoint_too_large",
                serde_json::json!({"max_bytes":limit_bytes,
                    "manifest_bytes_at_least":observed_bytes_at_least}),
            ),
        })?;
    let manifest: PatchCheckpoint = serde_json::from_slice(&bytes).map_err(|err| {
        patch_error(
            "checkpoint_invalid",
            "workspace.patch.checkpoint_invalid",
            serde_json::json!({"error": err.to_string()}),
        )
    })?;
    if manifest.schema_version != CHECKPOINT_SCHEMA_VERSION
        || checkpoint_dir
            .file_name()
            .and_then(|value| value.to_str())
            .is_none_or(|value| manifest.checkpoint_id != value)
    {
        return Err(patch_error(
            "checkpoint_invalid",
            "workspace.patch.checkpoint_invalid",
            Value::Null,
        ));
    }
    Ok(manifest)
}

pub(super) fn validate_checkpoint_id(value: &str) -> Result<(), String> {
    if value.len() < 8
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(patch_error(
            "invalid_checkpoint_id",
            "workspace.patch.invalid_checkpoint_id",
            Value::Null,
        ));
    }
    Ok(())
}

fn optional_paths(args: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    if let Some(path) = value.as_str() {
        return Ok(vec![path.to_string()]);
    }
    value
        .as_array()
        .ok_or_else(|| {
            patch_error(
                "invalid_paths",
                "workspace.patch.invalid_paths",
                Value::Null,
            )
        })?
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                patch_error(
                    "invalid_paths",
                    "workspace.patch.invalid_paths",
                    Value::Null,
                )
            })
        })
        .collect()
}

fn ensure_only_keys(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(key) = args.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(patch_error(
            "unexpected_arg",
            "workspace.patch.unexpected_arg",
            serde_json::json!({"arg": key}),
        ));
    }
    Ok(())
}

fn required_token<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    required_string(args, key).map(str::trim)
}

fn required_string<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| {
        patch_error(
            "missing_arg",
            "workspace.patch.missing_arg",
            serde_json::json!({"arg": key}),
        )
    })
}

fn optional_string<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn sha256_label(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

fn encode_result(value: Value) -> Result<String, String> {
    serde_json::to_string(&value).map_err(|err| {
        patch_error(
            "result_encode_failed",
            "workspace.patch.result_encode_failed",
            serde_json::json!({"error": err.to_string()}),
        )
    })
}

fn command_error(error_code: &str, message_key: &str, output: &Output) -> String {
    patch_error(
        error_code,
        message_key,
        serde_json::json!({
            "status_code": output.status.code(),
            "stderr": bounded_text(&output.stderr),
        }),
    )
}

fn patch_io_error(error_code: &str, message_key: &str, err: std::io::Error) -> String {
    let error_code = if HostFilesystemPolicy::is_private_access_denied(&err) {
        "host_private_directory_access_denied"
    } else {
        error_code
    };
    patch_error(
        error_code,
        message_key,
        serde_json::json!({"io_kind": format!("{:?}", err.kind())}),
    )
}

fn patch_error(error_code: &str, message_key: &str, details: Value) -> String {
    super::builtin_error(
        "workspace_patch",
        error_code,
        message_key,
        None,
        None,
        Some(serde_json::json!({
            "error_code": error_code,
            "message_key": message_key,
            "details": details,
        })),
    )
}

pub(super) fn invalid_path_error(path: &str) -> String {
    patch_error(
        "invalid_patch_path",
        "workspace.patch.invalid_path",
        serde_json::json!({"path": path}),
    )
}

fn bounded_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_string()
}

pub(super) fn remove_checkpoint_dir(root: &Path, path: &Path, filesystem: &HostFilesystemPolicy) {
    if let Err(err) = filesystem.remove_path(Some(root), path, true) {
        tracing::warn!(error = %err, path = %path.display(), "workspace_patch_checkpoint_cleanup_failed");
    }
}

pub(super) fn restrict_directory_permissions(
    root: &Path,
    path: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    filesystem
        .set_directory_permissions(Some(root), path, fs::Permissions::from_mode(0o700))
        .map_err(|error| {
            patch_io_error(
                "checkpoint_permissions_failed",
                "workspace.patch.checkpoint_create_failed",
                error,
            )
        })
}

#[cfg(test)]
#[path = "builtin_workspace_patch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "builtin_workspace_unified_diff_tests.rs"]
mod unified_diff_tests;

#[cfg(test)]
#[path = "builtin_workspace_patch_recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
#[path = "builtin_workspace_patch_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "builtin_workspace_patch_diff_read_tests.rs"]
mod diff_read_tests;

#[cfg(test)]
#[path = "builtin_workspace_patch_restore_metadata_tests.rs"]
mod restore_metadata_tests;

#[cfg(test)]
#[path = "builtin_workspace_patch_capacity_tests.rs"]
mod capacity_tests;

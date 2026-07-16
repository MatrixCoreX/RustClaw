use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use crate::{AppState, ClaimedTask};

const MAX_PATCH_BYTES: usize = 2 * 1024 * 1024;
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const CHECKPOINT_SCHEMA_VERSION: u32 = 1;

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
    execute_workspace_patch_for_root(&state.skill_rt.workspace_root, task_id, args)
}

fn execute_workspace_patch_for_root(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
) -> Result<String, String> {
    let action = required_token(args, "action")?;
    match action {
        "apply_patch" => apply_patch(workspace_root, task_id, args),
        "diff" => diff(workspace_root, args),
        "rewind" => rewind(workspace_root, args),
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
) -> Result<String, String> {
    ensure_only_keys(args, &["action", "patch", "precondition_hashes"])?;
    let patch = required_string(args, "patch")?;
    if patch.is_empty() || patch.len() > MAX_PATCH_BYTES {
        return Err(patch_error(
            "invalid_patch_size",
            "workspace.patch.invalid_size",
            serde_json::json!({
                "patch_bytes": patch.len(),
                "max_patch_bytes": MAX_PATCH_BYTES,
            }),
        ));
    }

    let root = canonical_workspace_root(workspace_root)?;
    let checkpoint_id = format!("patch_{}", uuid::Uuid::new_v4().simple());
    let checkpoint_dir = create_checkpoint_dir(&root, &checkpoint_id)?;
    restrict_directory_permissions(&checkpoint_dir);
    let patch_file = checkpoint_dir.join("change.patch");
    if let Err(err) = fs::write(&patch_file, patch.as_bytes()) {
        remove_checkpoint_dir(&checkpoint_dir);
        return Err(patch_io_error(
            "patch_stage_failed",
            "workspace.patch.stage_failed",
            err,
        ));
    }

    let stats = match inspect_patch(&root, &patch_file) {
        Ok(stats) => stats,
        Err(err) => {
            remove_checkpoint_dir(&checkpoint_dir);
            return Err(err);
        }
    };
    if stats.is_empty() {
        remove_checkpoint_dir(&checkpoint_dir);
        return Err(patch_error(
            "empty_patch",
            "workspace.patch.empty",
            Value::Null,
        ));
    }
    for stat in &stats {
        if let Err(err) = validate_relative_patch_path(&root, &stat.path) {
            remove_checkpoint_dir(&checkpoint_dir);
            return Err(err);
        }
    }
    if let Err(err) = verify_preconditions(&root, args, &stats) {
        remove_checkpoint_dir(&checkpoint_dir);
        return Err(err);
    }

    let mut files = match snapshot_files(&root, &checkpoint_dir, &stats) {
        Ok(files) => files,
        Err(err) => {
            remove_checkpoint_dir(&checkpoint_dir);
            return Err(err);
        }
    };
    let check = git_apply(&root, &patch_file, true)
        .map_err(|err| patch_io_error("patch_check_failed", "workspace.patch.check_failed", err))?;
    if !check.status.success() {
        remove_checkpoint_dir(&checkpoint_dir);
        return Err(command_error(
            "patch_context_mismatch",
            "workspace.patch.context_mismatch",
            &check,
        ));
    }

    let applied = git_apply(&root, &patch_file, false)
        .map_err(|err| patch_io_error("patch_apply_failed", "workspace.patch.apply_failed", err))?;
    if !applied.status.success() {
        restore_snapshot(&root, &checkpoint_dir, &files);
        remove_checkpoint_dir(&checkpoint_dir);
        return Err(command_error(
            "patch_apply_failed",
            "workspace.patch.apply_failed",
            &applied,
        ));
    }

    if let Err(err) = capture_after_hashes(&root, &mut files) {
        restore_snapshot(&root, &checkpoint_dir, &files);
        remove_checkpoint_dir(&checkpoint_dir);
        return Err(err);
    }

    let patch_id = format!("sha256:{}", sha256_hex(patch.as_bytes()));
    let manifest = PatchCheckpoint {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        checkpoint_id: checkpoint_id.clone(),
        task_id: task_id.to_string(),
        patch_id: patch_id.clone(),
        state: "applied".to_string(),
        created_at: crate::now_ts_u64() as i64,
        files,
    };
    if let Err(err) = write_manifest(&checkpoint_dir, &manifest) {
        restore_snapshot(&root, &checkpoint_dir, &manifest.files);
        remove_checkpoint_dir(&checkpoint_dir);
        return Err(err);
    }

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
        "changed_files": manifest.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(),
        "additions": additions,
        "deletions": deletions,
        "hunk_count": patch.lines().filter(|line| line.starts_with("@@ ")).count(),
        "files": manifest.files,
        "artifact_refs": [
            {"kind": "workspace_patch", "ref": format!("workspace_patch:{patch_id}")},
            {"kind": "workspace_checkpoint", "ref": format!("workspace_checkpoint:{}", manifest.checkpoint_id)},
        ],
    }))
}

fn diff(workspace_root: &Path, args: &Map<String, Value>) -> Result<String, String> {
    ensure_only_keys(args, &["action", "checkpoint_id", "paths"])?;
    let root = canonical_workspace_root(workspace_root)?;
    if let Some(checkpoint_id) = optional_string(args, "checkpoint_id") {
        validate_checkpoint_id(checkpoint_id)?;
        let checkpoint_dir = resolve_checkpoint_dir(&root, checkpoint_id)?;
        let manifest = read_manifest(&checkpoint_dir)?;
        let patch = fs::read_to_string(checkpoint_dir.join("change.patch")).map_err(|err| {
            patch_io_error(
                "checkpoint_patch_read_failed",
                "workspace.patch.read_failed",
                err,
            )
        })?;
        return encode_result(serde_json::json!({
            "schema_version": 1,
            "source": "workspace_patch",
            "status": "ok",
            "action": "diff",
            "message_key": "workspace.patch.diff_ready",
            "checkpoint_id": manifest.checkpoint_id,
            "patch_id": manifest.patch_id,
            "state": manifest.state,
            "changed_files": manifest.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(),
            "patch": patch,
        }));
    }

    let paths = optional_paths(args, "paths")?;
    for path in &paths {
        validate_relative_patch_path(&root, path)?;
    }
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(&root)
        .args(["diff", "--no-ext-diff", "--no-color", "--binary", "--"]);
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

fn rewind(workspace_root: &Path, args: &Map<String, Value>) -> Result<String, String> {
    ensure_only_keys(args, &["action", "checkpoint_id"])?;
    let root = canonical_workspace_root(workspace_root)?;
    let checkpoint_id = required_string(args, "checkpoint_id")?;
    validate_checkpoint_id(checkpoint_id)?;
    let checkpoint_dir = resolve_checkpoint_dir(&root, checkpoint_id)?;
    let mut manifest = read_manifest(&checkpoint_dir)?;
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

    for file in &manifest.files {
        validate_relative_patch_path(&root, &file.path)?;
        let current_hash = hash_existing_file(&root.join(&file.path))?;
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
    restore_snapshot_strict(&root, &checkpoint_dir, &manifest.files)?;
    manifest.state = "rewound".to_string();
    write_manifest(&checkpoint_dir, &manifest)?;
    encode_result(serde_json::json!({
        "schema_version": 1,
        "source": "workspace_patch",
        "status": "ok",
        "action": "rewind",
        "message_key": "workspace.patch.rewound",
        "checkpoint_id": checkpoint_id,
        "patch_id": manifest.patch_id,
        "restored_files": manifest.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(),
        "artifact_refs": [
            {"kind": "workspace_checkpoint", "ref": format!("workspace_checkpoint:{checkpoint_id}")},
        ],
    }))
}

fn inspect_patch(root: &Path, patch_file: &Path) -> Result<Vec<PatchStat>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["apply", "--numstat", "-z", "--"])
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
    parse_numstat(&output.stdout)
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
) -> Result<Vec<CheckpointFile>, String> {
    let backup_dir = checkpoint_dir.join("before");
    fs::create_dir_all(&backup_dir).map_err(|err| {
        patch_io_error(
            "checkpoint_create_failed",
            "workspace.patch.checkpoint_create_failed",
            err,
        )
    })?;
    let mut files = Vec::with_capacity(stats.len());
    for (index, stat) in stats.iter().enumerate() {
        let path = root.join(&stat.path);
        let bytes = read_optional_regular_file(&path)?;
        let backup_file = bytes.as_ref().map(|_| format!("before/{index}.bin"));
        if let (Some(bytes), Some(backup_file)) = (&bytes, &backup_file) {
            fs::write(checkpoint_dir.join(backup_file), bytes).map_err(|err| {
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

fn capture_after_hashes(root: &Path, files: &mut [CheckpointFile]) -> Result<(), String> {
    for file in files {
        validate_relative_patch_path(root, &file.path)?;
        file.after_sha256 = hash_existing_file(&root.join(&file.path))?;
    }
    Ok(())
}

fn verify_preconditions(
    root: &Path,
    args: &Map<String, Value>,
    stats: &[PatchStat],
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
        let actual = hash_existing_file(&root.join(path))?;
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

fn validate_relative_patch_path(root: &Path, path: &str) -> Result<PathBuf, String> {
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

fn read_optional_regular_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(patch_error(
                "unsupported_file_type",
                "workspace.patch.unsupported_file_type",
                serde_json::json!({"path": path.display().to_string()}),
            ))
        }
        Ok(_) => fs::read(path).map(Some).map_err(|err| {
            patch_io_error("file_read_failed", "workspace.patch.file_read_failed", err)
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(patch_io_error(
            "path_inspection_failed",
            "workspace.patch.path_inspection_failed",
            err,
        )),
    }
}

fn hash_existing_file(path: &Path) -> Result<Option<String>, String> {
    read_optional_regular_file(path).map(|bytes| bytes.as_deref().map(sha256_label))
}

fn restore_snapshot(root: &Path, checkpoint_dir: &Path, files: &[CheckpointFile]) {
    if let Err(err) = restore_snapshot_strict(root, checkpoint_dir, files) {
        tracing::error!(error = %err, "workspace_patch_restore_failed");
    }
}

fn restore_snapshot_strict(
    root: &Path,
    checkpoint_dir: &Path,
    files: &[CheckpointFile],
) -> Result<(), String> {
    for file in files {
        let path = validate_restore_path(root, &file.path)?;
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            fs::remove_file(&path).map_err(|err| {
                patch_io_error(
                    "restore_remove_failed",
                    "workspace.patch.restore_remove_failed",
                    err,
                )
            })?;
        }
        if file.existed {
            let backup = file.backup_file.as_deref().ok_or_else(|| {
                patch_error(
                    "checkpoint_backup_missing",
                    "workspace.patch.checkpoint_backup_missing",
                    serde_json::json!({"path": file.path}),
                )
            })?;
            let bytes = fs::read(checkpoint_dir.join(backup)).map_err(|err| {
                patch_io_error(
                    "checkpoint_read_failed",
                    "workspace.patch.checkpoint_read_failed",
                    err,
                )
            })?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    patch_io_error(
                        "restore_create_failed",
                        "workspace.patch.restore_create_failed",
                        err,
                    )
                })?;
            }
            fs::write(&path, bytes).map_err(|err| {
                patch_io_error(
                    "restore_write_failed",
                    "workspace.patch.restore_write_failed",
                    err,
                )
            })?;
        } else {
            match fs::remove_file(&path) {
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

fn git_apply(root: &Path, patch_file: &Path, check: bool) -> std::io::Result<Output> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).arg("apply");
    if check {
        command.arg("--check");
    }
    command.args(["--whitespace=nowarn", "--"]);
    command.arg(patch_file).output()
}

fn canonical_workspace_root(root: &Path) -> Result<PathBuf, String> {
    root.canonicalize().map_err(|err| {
        patch_io_error(
            "workspace_unavailable",
            "workspace.patch.workspace_unavailable",
            err,
        )
    })
}

fn checkpoint_root(root: &Path) -> PathBuf {
    root.join(".rustclaw").join("checkpoints")
}

fn create_checkpoint_dir(root: &Path, checkpoint_id: &str) -> Result<PathBuf, String> {
    let state_dir = root.join(".rustclaw");
    reject_symlink_if_present(&state_dir)?;
    fs::create_dir_all(&state_dir).map_err(|err| {
        patch_io_error(
            "checkpoint_create_failed",
            "workspace.patch.checkpoint_create_failed",
            err,
        )
    })?;
    let checkpoints = checkpoint_root(root);
    reject_symlink_if_present(&checkpoints)?;
    fs::create_dir_all(&checkpoints).map_err(|err| {
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
    fs::create_dir(&checkpoint_dir).map_err(|err| {
        patch_io_error(
            "checkpoint_create_failed",
            "workspace.patch.checkpoint_create_failed",
            err,
        )
    })?;
    Ok(checkpoint_dir)
}

fn resolve_checkpoint_dir(root: &Path, checkpoint_id: &str) -> Result<PathBuf, String> {
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
                if value == ".git" || value == ".rustclaw" {
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

fn write_manifest(checkpoint_dir: &Path, manifest: &PatchCheckpoint) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|err| {
        patch_error(
            "checkpoint_encode_failed",
            "workspace.patch.checkpoint_encode_failed",
            serde_json::json!({"error": err.to_string()}),
        )
    })?;
    let temporary = checkpoint_dir.join("manifest.json.tmp");
    fs::write(&temporary, bytes).map_err(|err| {
        patch_io_error(
            "checkpoint_write_failed",
            "workspace.patch.checkpoint_write_failed",
            err,
        )
    })?;
    fs::rename(temporary, checkpoint_dir.join("manifest.json")).map_err(|err| {
        patch_io_error(
            "checkpoint_write_failed",
            "workspace.patch.checkpoint_write_failed",
            err,
        )
    })
}

fn read_manifest(checkpoint_dir: &Path) -> Result<PatchCheckpoint, String> {
    let bytes = fs::read(checkpoint_dir.join("manifest.json")).map_err(|err| {
        patch_io_error(
            "checkpoint_read_failed",
            "workspace.patch.checkpoint_read_failed",
            err,
        )
    })?;
    if bytes.len() > MAX_PATCH_BYTES {
        return Err(patch_error(
            "checkpoint_too_large",
            "workspace.patch.checkpoint_too_large",
            serde_json::json!({"manifest_bytes": bytes.len()}),
        ));
    }
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

fn validate_checkpoint_id(value: &str) -> Result<(), String> {
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

fn invalid_path_error(path: &str) -> String {
    patch_error(
        "invalid_patch_path",
        "workspace.patch.invalid_path",
        serde_json::json!({"path": path}),
    )
}

fn bounded_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_string()
}

fn remove_checkpoint_dir(path: &Path) {
    if let Err(err) = fs::remove_dir_all(path) {
        tracing::warn!(error = %err, path = %path.display(), "workspace_patch_checkpoint_cleanup_failed");
    }
}

#[cfg(unix)]
fn restrict_directory_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_directory_permissions(_path: &Path) {}

#[cfg(test)]
#[path = "builtin_workspace_patch_tests.rs"]
mod tests;

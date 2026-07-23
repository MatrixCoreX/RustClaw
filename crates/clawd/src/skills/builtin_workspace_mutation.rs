use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::builtin_workspace_patch::{
    canonical_workspace_root, create_checkpoint_dir, remove_checkpoint_dir, resolve_checkpoint_dir,
    restrict_directory_permissions, validate_checkpoint_id,
};

const CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const CHECKPOINT_KIND: &str = "structured_mutation";
const MAX_SNAPSHOT_ENTRIES: usize = 4096;
const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SnapshotEntry {
    path: String,
    kind: String,
    sha256: Option<String>,
    size_bytes: Option<u64>,
    backup_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MutationCheckpointManifest {
    schema_version: u32,
    checkpoint_kind: String,
    checkpoint_id: String,
    task_id: String,
    action: String,
    state: String,
    created_at: i64,
    target_path: String,
    missing_parent_paths: Vec<String>,
    before: Vec<SnapshotEntry>,
    after: Vec<SnapshotEntry>,
    mutation_id: Option<String>,
}

struct SnapshotBudget {
    entries: usize,
    bytes: u64,
}

pub(super) struct StructuredMutationCheckpoint {
    root: PathBuf,
    checkpoint_dir: PathBuf,
    manifest: MutationCheckpointManifest,
}

pub(super) fn atomic_write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "atomic_write_parent_missing")
    })?;
    let existing_permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let temporary = parent.join(format!(
        ".rustclaw-write-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        if let Some(permissions) = existing_permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        sync_parent_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    match fs::File::open(parent)?.sync_all() {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(super) fn run_checkpointed_workspace_mutation<F>(
    workspace_root: &Path,
    task_id: &str,
    action: &str,
    target: &Path,
    operation: F,
) -> Result<String, String>
where
    F: FnOnce() -> Result<(), String>,
{
    let mut checkpoint =
        StructuredMutationCheckpoint::prepare(workspace_root, task_id, action, target)?;
    if let Err(operation_error) = operation() {
        if let Err(restore_error) = checkpoint.restore_before_state() {
            return Err(mutation_error(
                "mutation_and_restore_failed",
                "workspace.mutation.restore_failed",
                json!({
                    "checkpoint_id": checkpoint.manifest.checkpoint_id,
                    "operation_error": operation_error,
                    "restore_error": restore_error,
                }),
            ));
        }
        remove_checkpoint_dir(&checkpoint.checkpoint_dir);
        return Err(operation_error);
    }
    checkpoint.finish()
}

pub(super) fn checkpoint_is_structured_mutation(checkpoint_dir: &Path) -> Result<bool, String> {
    let value = read_manifest_value(checkpoint_dir)?;
    Ok(value.get("checkpoint_kind").and_then(Value::as_str) == Some(CHECKPOINT_KIND))
}

pub(super) fn structured_mutation_diff(
    workspace_root: &Path,
    checkpoint_id: &str,
) -> Result<String, String> {
    validate_checkpoint_id(checkpoint_id)?;
    let root = canonical_workspace_root(workspace_root)?;
    let checkpoint_dir = resolve_checkpoint_dir(&root, checkpoint_id)?;
    let manifest = read_manifest(&checkpoint_dir)?;
    encode_result(json!({
        "schema_version": 1,
        "source": "workspace_mutation",
        "status": "ok",
        "action": "diff",
        "message_key": "workspace.mutation.diff_ready",
        "checkpoint_id": manifest.checkpoint_id,
        "mutation_id": manifest.mutation_id,
        "state": manifest.state,
        "target_path": manifest.target_path,
        "isolation_root": "workspace://current",
        "reversible": manifest.state == "applied",
        "diff_available": false,
        "before": public_snapshot_entries(&manifest.before),
        "after": public_snapshot_entries(&manifest.after),
        "artifact_refs": [
            {"kind": "workspace_checkpoint", "ref": format!("workspace_checkpoint:{}", manifest.checkpoint_id)},
        ],
    }))
}

pub(super) fn rewind_structured_mutation(
    workspace_root: &Path,
    checkpoint_id: &str,
) -> Result<String, String> {
    validate_checkpoint_id(checkpoint_id)?;
    let root = canonical_workspace_root(workspace_root)?;
    let checkpoint_dir = resolve_checkpoint_dir(&root, checkpoint_id)?;
    let mut manifest = read_manifest(&checkpoint_dir)?;
    if manifest.state != "applied" {
        return Err(mutation_error(
            "checkpoint_not_applied",
            "workspace.mutation.checkpoint_not_applied",
            json!({
                "checkpoint_id": checkpoint_id,
                "state": manifest.state,
            }),
        ));
    }
    let current = capture_snapshot(&root, &manifest.target_path, None)?;
    if current != manifest.after {
        return Err(mutation_error(
            "rewind_precondition_failed",
            "workspace.mutation.rewind_precondition_failed",
            json!({
                "checkpoint_id": checkpoint_id,
                "target_path": manifest.target_path,
                "expected": public_snapshot_entries(&manifest.after),
                "actual": public_snapshot_entries(&current),
            }),
        ));
    }
    restore_snapshot(&root, &checkpoint_dir, &manifest)?;
    manifest.state = "rewound".to_string();
    write_manifest(&checkpoint_dir, &manifest)?;
    encode_result(json!({
        "schema_version": 1,
        "source": "workspace_mutation",
        "status": "ok",
        "action": "rewind",
        "message_key": "workspace.mutation.rewound",
        "checkpoint_id": checkpoint_id,
        "mutation_id": manifest.mutation_id,
        "compensates_checkpoint_id": checkpoint_id,
        "compensates_mutation_id": manifest.mutation_id,
        "state": "rewound",
        "target_path": manifest.target_path,
        "isolation_root": "workspace://current",
        "reversible": false,
        "restored_files": snapshot_file_paths(&manifest.before),
        "artifact_refs": [
            {"kind": "workspace_checkpoint", "ref": format!("workspace_checkpoint:{checkpoint_id}")},
        ],
    }))
}

impl StructuredMutationCheckpoint {
    fn prepare(
        workspace_root: &Path,
        task_id: &str,
        action: &str,
        target: &Path,
    ) -> Result<Self, String> {
        let root = canonical_workspace_root(workspace_root)?;
        let target_path = safe_relative_target(&root, target)?;
        let checkpoint_id = format!("mutation_{}", uuid::Uuid::new_v4().simple());
        let checkpoint_dir = create_checkpoint_dir(&root, &checkpoint_id)?;
        restrict_directory_permissions(&checkpoint_dir);
        let before = match capture_snapshot(&root, &target_path, Some(&checkpoint_dir)) {
            Ok(value) => value,
            Err(error) => {
                remove_checkpoint_dir(&checkpoint_dir);
                return Err(error);
            }
        };
        let manifest = MutationCheckpointManifest {
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            checkpoint_kind: CHECKPOINT_KIND.to_string(),
            checkpoint_id,
            task_id: task_id.to_string(),
            action: action.to_string(),
            state: "prepared".to_string(),
            created_at: crate::now_ts_u64() as i64,
            target_path: target_path.clone(),
            missing_parent_paths: missing_parent_paths(&root, &target_path)?,
            before,
            after: Vec::new(),
            mutation_id: None,
        };
        if let Err(error) = write_manifest(&checkpoint_dir, &manifest) {
            remove_checkpoint_dir(&checkpoint_dir);
            return Err(error);
        }
        Ok(Self {
            root,
            checkpoint_dir,
            manifest,
        })
    }

    fn finish(&mut self) -> Result<String, String> {
        let after = match capture_snapshot(&self.root, &self.manifest.target_path, None) {
            Ok(value) => value,
            Err(error) => {
                return Err(self.restore_after_internal_failure(error));
            }
        };
        self.manifest.after = after;
        let changed = !snapshot_states_equal(&self.manifest.before, &self.manifest.after);
        self.manifest.state = if changed { "applied" } else { "no_op" }.to_string();
        self.manifest.mutation_id = Some(mutation_id(&self.manifest));
        if let Err(error) = write_manifest(&self.checkpoint_dir, &self.manifest) {
            return Err(self.restore_after_internal_failure(error));
        }
        encode_result(json!({
            "schema_version": 1,
            "source": "workspace_mutation",
            "status": "ok",
            "action": self.manifest.action,
            "message_key": "workspace.mutation.applied",
            "checkpoint_id": self.manifest.checkpoint_id,
            "mutation_id": self.manifest.mutation_id,
            "state": self.manifest.state,
            "target_path": self.manifest.target_path,
            "isolation_root": "workspace://current",
            "reversible": changed,
            "changed_files": if changed {
                vec![self.manifest.target_path.clone()]
            } else {
                Vec::<String>::new()
            },
            "before": public_snapshot_entries(&self.manifest.before),
            "after": public_snapshot_entries(&self.manifest.after),
            "artifact_refs": [
                {"kind": "workspace_mutation", "ref": format!("workspace_mutation:{}", self.manifest.mutation_id.as_deref().unwrap_or_default())},
                {"kind": "workspace_checkpoint", "ref": format!("workspace_checkpoint:{}", self.manifest.checkpoint_id)},
            ],
        }))
    }

    fn restore_before_state(&mut self) -> Result<(), String> {
        restore_snapshot(&self.root, &self.checkpoint_dir, &self.manifest)
    }

    fn restore_after_internal_failure(&mut self, failure: String) -> String {
        match self.restore_before_state() {
            Ok(()) => {
                remove_checkpoint_dir(&self.checkpoint_dir);
                failure
            }
            Err(restore_error) => mutation_error(
                "checkpoint_finalize_and_restore_failed",
                "workspace.mutation.restore_failed",
                json!({
                    "checkpoint_id": self.manifest.checkpoint_id,
                    "failure": failure,
                    "restore_error": restore_error,
                }),
            ),
        }
    }
}

fn capture_snapshot(
    root: &Path,
    target_path: &str,
    backup_root: Option<&Path>,
) -> Result<Vec<SnapshotEntry>, String> {
    let target = safe_target_from_relative(root, target_path)?;
    let mut entries = Vec::new();
    let mut budget = SnapshotBudget {
        entries: 0,
        bytes: 0,
    };
    capture_path(
        root,
        &target,
        target_path,
        backup_root,
        &mut entries,
        &mut budget,
    )?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn capture_path(
    root: &Path,
    path: &Path,
    relative: &str,
    backup_root: Option<&Path>,
    entries: &mut Vec<SnapshotEntry>,
    budget: &mut SnapshotBudget,
) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            entries.push(SnapshotEntry {
                path: relative.to_string(),
                kind: "missing".to_string(),
                sha256: None,
                size_bytes: None,
                backup_file: None,
            });
            return Ok(());
        }
        Err(error) => return Err(io_error("snapshot_inspection_failed", error)),
    };
    budget.entries += 1;
    if budget.entries > MAX_SNAPSHOT_ENTRIES {
        return Err(snapshot_limit_error(budget));
    }
    if metadata.file_type().is_symlink() {
        return Err(mutation_error(
            "snapshot_symlink_denied",
            "workspace.mutation.symlink_denied",
            json!({"path": relative}),
        ));
    }
    if metadata.is_file() {
        let bytes = fs::read(path).map_err(|error| io_error("snapshot_read_failed", error))?;
        budget.bytes = budget.bytes.saturating_add(bytes.len() as u64);
        if budget.bytes > MAX_SNAPSHOT_BYTES {
            return Err(snapshot_limit_error(budget));
        }
        let backup_file = if let Some(backup_root) = backup_root {
            let name = format!("before/{:04}.bin", entries.len());
            let backup = backup_root.join(&name);
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| io_error("snapshot_create_failed", error))?;
            }
            fs::write(&backup, &bytes).map_err(|error| io_error("snapshot_write_failed", error))?;
            Some(name)
        } else {
            None
        };
        entries.push(SnapshotEntry {
            path: relative.to_string(),
            kind: "file".to_string(),
            sha256: Some(sha256_label(&bytes)),
            size_bytes: Some(bytes.len() as u64),
            backup_file,
        });
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(mutation_error(
            "unsupported_target_type",
            "workspace.mutation.unsupported_target_type",
            json!({"path": relative}),
        ));
    }
    entries.push(SnapshotEntry {
        path: relative.to_string(),
        kind: "directory".to_string(),
        sha256: None,
        size_bytes: None,
        backup_file: None,
    });
    let mut children = fs::read_dir(path)
        .map_err(|error| io_error("snapshot_read_dir_failed", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error("snapshot_read_dir_failed", error))?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let child_path = child.path();
        if !child_path.starts_with(root) {
            return Err(mutation_error(
                "snapshot_outside_workspace",
                "workspace.mutation.outside_workspace",
                json!({"path": child_path.display().to_string()}),
            ));
        }
        let child_relative = child_path
            .strip_prefix(root)
            .map_err(|_| invalid_target_error(&child_path))?
            .to_string_lossy()
            .replace('\\', "/");
        capture_path(
            root,
            &child_path,
            &child_relative,
            backup_root,
            entries,
            budget,
        )?;
    }
    Ok(())
}

fn restore_snapshot(
    root: &Path,
    checkpoint_dir: &Path,
    manifest: &MutationCheckpointManifest,
) -> Result<(), String> {
    let target = safe_target_from_relative(root, &manifest.target_path)?;
    remove_existing_target(&target)?;
    let mut directories = manifest
        .before
        .iter()
        .filter(|entry| entry.kind == "directory")
        .collect::<Vec<_>>();
    directories.sort_by_key(|entry| path_depth(&entry.path));
    for entry in directories {
        let path = safe_target_from_relative(root, &entry.path)?;
        fs::create_dir_all(&path).map_err(|error| io_error("restore_create_failed", error))?;
    }
    for entry in manifest.before.iter().filter(|entry| entry.kind == "file") {
        let path = safe_target_from_relative(root, &entry.path)?;
        let backup_file = entry.backup_file.as_deref().ok_or_else(|| {
            mutation_error(
                "checkpoint_backup_missing",
                "workspace.mutation.backup_missing",
                json!({"path": entry.path}),
            )
        })?;
        let bytes = fs::read(checkpoint_dir.join(backup_file))
            .map_err(|error| io_error("restore_read_failed", error))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| io_error("restore_create_failed", error))?;
        }
        fs::write(path, bytes).map_err(|error| io_error("restore_write_failed", error))?;
    }
    for parent in manifest.missing_parent_paths.iter().rev() {
        let path = safe_target_from_relative(root, parent)?;
        match fs::remove_dir(&path) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(io_error("restore_parent_cleanup_failed", error)),
        }
    }
    Ok(())
}

fn remove_existing_target(target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(mutation_error(
            "restore_symlink_denied",
            "workspace.mutation.symlink_denied",
            json!({"path": target.display().to_string()}),
        )),
        Ok(metadata) if metadata.is_dir() => {
            fs::remove_dir_all(target).map_err(|error| io_error("restore_remove_failed", error))
        }
        Ok(_) => fs::remove_file(target).map_err(|error| io_error("restore_remove_failed", error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("restore_inspection_failed", error)),
    }
}

fn missing_parent_paths(root: &Path, target_path: &str) -> Result<Vec<String>, String> {
    let target = safe_target_from_relative(root, target_path)?;
    let mut paths = Vec::new();
    let mut current = target.parent();
    while let Some(path) = current {
        if path == root {
            break;
        }
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(mutation_error(
                    "snapshot_symlink_denied",
                    "workspace.mutation.symlink_denied",
                    json!({"path": path.display().to_string()}),
                ));
            }
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                paths.push(relative_path(root, path)?);
            }
            Err(error) => return Err(io_error("snapshot_inspection_failed", error)),
        }
        current = path.parent();
    }
    paths.reverse();
    Ok(paths)
}

fn safe_relative_target(root: &Path, target: &Path) -> Result<String, String> {
    if !target.starts_with(root) {
        return Err(invalid_target_error(target));
    }
    let relative = relative_path(root, target)?;
    let _ = safe_target_from_relative(root, &relative)?;
    Ok(relative)
}

fn safe_target_from_relative(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(invalid_target_error(path));
    }
    let mut target = root.to_path_buf();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) if value != ".git" && value != ".rustclaw" => {
                target.push(value);
                match fs::symlink_metadata(&target) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(mutation_error(
                            "snapshot_symlink_denied",
                            "workspace.mutation.symlink_denied",
                            json!({"path": relative}),
                        ));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(io_error("snapshot_inspection_failed", error)),
                }
            }
            std::path::Component::CurDir => {}
            _ => return Err(invalid_target_error(path)),
        }
    }
    Ok(target)
}

fn relative_path(root: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(root)
        .map_err(|_| invalid_target_error(path))
        .map(|value| value.to_string_lossy().replace('\\', "/"))
}

fn path_depth(path: &str) -> usize {
    Path::new(path).components().count()
}

fn snapshot_file_paths(entries: &[SnapshotEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.kind == "file")
        .map(|entry| entry.path.clone())
        .collect()
}

fn public_snapshot_entries(entries: &[SnapshotEntry]) -> Vec<Value> {
    entries
        .iter()
        .map(|entry| {
            json!({
                "path": entry.path,
                "kind": entry.kind,
                "sha256": entry.sha256,
                "size_bytes": entry.size_bytes,
            })
        })
        .collect()
}

fn snapshot_states_equal(left: &[SnapshotEntry], right: &[SnapshotEntry]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.path == right.path
                && left.kind == right.kind
                && left.sha256 == right.sha256
                && left.size_bytes == right.size_bytes
        })
}

fn mutation_id(manifest: &MutationCheckpointManifest) -> String {
    let digest = Sha256::digest(
        serde_json::to_vec(&json!({
            "action": manifest.action,
            "target_path": manifest.target_path,
            "before": public_snapshot_entries(&manifest.before),
            "after": public_snapshot_entries(&manifest.after),
        }))
        .unwrap_or_default(),
    );
    format!("sha256:{digest:x}")
}

fn sha256_label(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

fn write_manifest(
    checkpoint_dir: &Path,
    manifest: &MutationCheckpointManifest,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|error| {
        mutation_error(
            "checkpoint_encode_failed",
            "workspace.mutation.checkpoint_encode_failed",
            json!({"error_kind": format!("{:?}", error.classify())}),
        )
    })?;
    let temporary = checkpoint_dir.join("manifest.json.tmp");
    fs::write(&temporary, bytes).map_err(|error| io_error("checkpoint_write_failed", error))?;
    fs::rename(temporary, checkpoint_dir.join("manifest.json"))
        .map_err(|error| io_error("checkpoint_write_failed", error))
}

fn read_manifest_value(checkpoint_dir: &Path) -> Result<Value, String> {
    let bytes = fs::read(checkpoint_dir.join("manifest.json"))
        .map_err(|error| io_error("checkpoint_read_failed", error))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(mutation_error(
            "checkpoint_too_large",
            "workspace.mutation.checkpoint_too_large",
            json!({"manifest_bytes": bytes.len()}),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        mutation_error(
            "checkpoint_invalid",
            "workspace.mutation.checkpoint_invalid",
            json!({"error_kind": format!("{:?}", error.classify())}),
        )
    })
}

fn read_manifest(checkpoint_dir: &Path) -> Result<MutationCheckpointManifest, String> {
    let value = read_manifest_value(checkpoint_dir)?;
    let manifest: MutationCheckpointManifest = serde_json::from_value(value).map_err(|error| {
        mutation_error(
            "checkpoint_invalid",
            "workspace.mutation.checkpoint_invalid",
            json!({"error_kind": format!("{:?}", error.classify())}),
        )
    })?;
    if manifest.schema_version != CHECKPOINT_SCHEMA_VERSION
        || manifest.checkpoint_kind != CHECKPOINT_KIND
        || checkpoint_dir
            .file_name()
            .and_then(|value| value.to_str())
            .is_none_or(|value| manifest.checkpoint_id != value)
    {
        return Err(mutation_error(
            "checkpoint_invalid",
            "workspace.mutation.checkpoint_invalid",
            Value::Null,
        ));
    }
    Ok(manifest)
}

fn snapshot_limit_error(budget: &SnapshotBudget) -> String {
    mutation_error(
        "snapshot_limit_exceeded",
        "workspace.mutation.snapshot_limit_exceeded",
        json!({
            "entry_count": budget.entries,
            "max_entries": MAX_SNAPSHOT_ENTRIES,
            "bytes": budget.bytes,
            "max_bytes": MAX_SNAPSHOT_BYTES,
        }),
    )
}

fn invalid_target_error(path: &Path) -> String {
    mutation_error(
        "invalid_target_path",
        "workspace.mutation.invalid_target_path",
        json!({"path": path.display().to_string()}),
    )
}

fn io_error(error_code: &str, error: std::io::Error) -> String {
    mutation_error(
        error_code,
        "workspace.mutation.io_error",
        json!({"io_kind": format!("{:?}", error.kind())}),
    )
}

fn mutation_error(error_code: &str, message_key: &str, details: Value) -> String {
    super::builtin_error(
        "workspace_mutation",
        error_code,
        message_key,
        None,
        None,
        Some(json!({
            "error_code": error_code,
            "message_key": message_key,
            "details": details,
        })),
    )
}

fn encode_result(value: Value) -> Result<String, String> {
    serde_json::to_string(&value).map_err(|error| {
        mutation_error(
            "result_encode_failed",
            "workspace.mutation.result_encode_failed",
            json!({"error_kind": format!("{:?}", error.classify())}),
        )
    })
}

#[cfg(test)]
#[path = "builtin_workspace_mutation_tests.rs"]
mod tests;

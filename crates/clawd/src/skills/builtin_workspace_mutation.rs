use crate::runtime::host_filesystem::{HostAccessMode, HostFilesystemPolicy};
use rustix::fs::FileType;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::builtin_workspace_patch::{
    canonical_workspace_root, create_checkpoint_dir, resolve_checkpoint_dir, validate_checkpoint_id,
};

const CHECKPOINT_SCHEMA_VERSION: u32 = 2;
const CHECKPOINT_KIND: &str = "structured_mutation";
const MAX_SNAPSHOT_ENTRIES: usize = 4096;
const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CHECKPOINT_METADATA_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) enum MutationAfterShape {
    RegularFile,
    Directory,
    Missing,
}

impl MutationAfterShape {
    fn projected_entry(self, path: &str) -> SnapshotEntry {
        let (kind, sha256, size_bytes) = match self {
            Self::RegularFile => ("file", Some(sha256_label(&[])), Some(u64::MAX)),
            Self::Directory => ("directory", None, None),
            Self::Missing => ("missing", None, None),
        };
        SnapshotEntry {
            path: path.to_string(),
            kind: kind.into(),
            sha256,
            size_bytes,
            backup_file: None,
            unix_mode: (!matches!(self, Self::Missing))
                .then(|| HostAccessMode::try_from(0o777).expect("ordinary projected mode")),
        }
    }

    fn accepts(self, path: &str, entries: &[SnapshotEntry]) -> bool {
        let [entry] = entries else {
            return false;
        };
        if entry.path != path || entry.backup_file.is_some() {
            return false;
        }
        match self {
            Self::RegularFile => {
                entry.kind == "file"
                    && entry.sha256.is_some()
                    && entry.size_bytes.is_some()
                    && entry.unix_mode.is_some()
            }
            Self::Directory => {
                entry.kind == "directory"
                    && entry.sha256.is_none()
                    && entry.size_bytes.is_none()
                    && entry.unix_mode.is_some()
            }
            Self::Missing => {
                entry.kind == "missing"
                    && entry.sha256.is_none()
                    && entry.size_bytes.is_none()
                    && entry.unix_mode.is_none()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SnapshotEntry {
    path: String,
    kind: String,
    sha256: Option<String>,
    size_bytes: Option<u64>,
    backup_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unix_mode: Option<HostAccessMode>,
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
    max_entries: usize,
}

pub(super) struct StructuredMutationCheckpoint<'a> {
    filesystem: &'a HostFilesystemPolicy,
    root: PathBuf,
    checkpoint_dir: PathBuf,
    manifest: MutationCheckpointManifest,
    after_shape: MutationAfterShape,
}

pub(super) fn run_checkpointed_workspace_mutation_with_policy<F>(
    workspace_root: &Path,
    task_id: &str,
    action: &str,
    target: &Path,
    after_shape: MutationAfterShape,
    filesystem: &HostFilesystemPolicy,
    operation: F,
) -> Result<String, String>
where
    F: FnOnce() -> Result<(), String>,
{
    let mut checkpoint = StructuredMutationCheckpoint::prepare(
        workspace_root,
        task_id,
        action,
        target,
        after_shape,
        filesystem,
    )
    .map_err(pre_dispatch_replan_error)?;
    if let Err(operation_error) = operation() {
        if checkpoint.current_matches_before_state().unwrap_or(false) {
            checkpoint.remove_checkpoint();
            return Err(verified_no_effect_error(operation_error));
        }
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
        checkpoint.remove_checkpoint();
        return Err(verified_no_effect_error(operation_error));
    }
    checkpoint.finish()
}

pub(super) fn run_authorized_mutation_with_policy<F>(
    workspace_root: &Path,
    task_id: &str,
    action: &str,
    target: &Path,
    after_shape: MutationAfterShape,
    host_scope_authorized: bool,
    filesystem: &HostFilesystemPolicy,
    operation: F,
) -> Result<String, String>
where
    F: FnOnce() -> Result<(), String>,
{
    let root = canonical_workspace_root(workspace_root).map_err(pre_dispatch_error)?;
    if target.starts_with(&root) || target.starts_with(workspace_root) {
        return run_checkpointed_workspace_mutation_with_policy(
            workspace_root,
            task_id,
            action,
            target,
            after_shape,
            filesystem,
            operation,
        );
    }
    if !host_scope_authorized {
        return Err(pre_dispatch_replan_error(invalid_target_error(target)));
    }
    operation()?;
    encode_result(json!({
        "schema_version": 1,
        "source": "host_scope_mutation",
        "status": "ok",
        "action": action,
        "message_key": "host.mutation.applied",
        "task_id": task_id,
        "state": "applied",
        "target_path": target.display().to_string(),
        "isolation_root": "host://service-account",
        "authority_scope": "host_policy_grant",
        "reversible": false,
        "rollback_unavailable_reason": "outside_workspace_checkpoint_scope",
        "changed_files": [target.display().to_string()],
        "artifact_refs": [],
    }))
}

fn pre_dispatch_error(error: String) -> String {
    crate::skills::annotate_structured_skill_error_not_applied(
        &error,
        "pre_dispatch",
        Some(false),
        None,
    )
}

pub(super) fn pre_dispatch_replan_error(error: String) -> String {
    if crate::skills::parse_structured_skill_error(&error)
        .is_some_and(|parsed| parsed.error_code == "host_private_directory_access_denied")
    {
        return pre_dispatch_error(error);
    }
    crate::skills::annotate_structured_skill_error_not_applied(
        &error,
        "pre_dispatch",
        Some(true),
        Some("replan_arguments"),
    )
}

fn verified_no_effect_error(error: String) -> String {
    crate::skills::annotate_structured_skill_error_not_applied(
        &error,
        "execution_no_effect",
        None,
        None,
    )
}

pub(super) fn checkpoint_is_structured_mutation(
    checkpoint_dir: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<bool, String> {
    let value = read_manifest_value(checkpoint_dir, filesystem)?;
    Ok(value.get("checkpoint_kind").and_then(Value::as_str) == Some(CHECKPOINT_KIND))
}

pub(super) fn structured_mutation_diff_with_policy(
    workspace_root: &Path,
    checkpoint_id: &str,
    filesystem: &HostFilesystemPolicy,
) -> Result<String, String> {
    validate_checkpoint_id(checkpoint_id)?;
    let root = canonical_workspace_root(workspace_root)?;
    let checkpoint_dir = resolve_checkpoint_dir(&root, checkpoint_id, filesystem)?;
    let manifest = read_manifest(&checkpoint_dir, filesystem)?;
    capture_snapshot(&root, &manifest.target_path, None, filesystem)?;
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

pub(super) fn rewind_structured_mutation_with_policy(
    workspace_root: &Path,
    checkpoint_id: &str,
    filesystem: &HostFilesystemPolicy,
) -> Result<String, String> {
    validate_checkpoint_id(checkpoint_id)?;
    let root = canonical_workspace_root(workspace_root)?;
    let checkpoint_dir = resolve_checkpoint_dir(&root, checkpoint_id, filesystem)?;
    let mut manifest = read_manifest(&checkpoint_dir, filesystem)?;
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
    let current = capture_snapshot(&root, &manifest.target_path, None, filesystem)?;
    if !snapshot_matches_recorded_state(&manifest.after, &current, manifest.schema_version == 1) {
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
    restore_snapshot(&root, &checkpoint_dir, &manifest, filesystem)?;
    manifest.state = "rewound".to_string();
    write_manifest(&checkpoint_dir, &manifest, filesystem)?;
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
        "unix_mode_restoration": mode_restoration_coverage(&manifest.before),
        "artifact_refs": [
            {"kind": "workspace_checkpoint", "ref": format!("workspace_checkpoint:{checkpoint_id}")},
        ],
    }))
}

impl<'a> StructuredMutationCheckpoint<'a> {
    fn prepare(
        workspace_root: &Path,
        task_id: &str,
        action: &str,
        target: &Path,
        after_shape: MutationAfterShape,
        filesystem: &'a HostFilesystemPolicy,
    ) -> Result<Self, String> {
        let root = canonical_workspace_root(workspace_root)?;
        let target_path = safe_relative_target(workspace_root, &root, target)?;
        capture_snapshot(&root, &target_path, None, filesystem)?;
        let checkpoint_id = format!("mutation_{}", uuid::Uuid::new_v4().simple());
        let checkpoint_dir = create_checkpoint_dir(&root, &checkpoint_id, filesystem)?;
        let before = match capture_snapshot(&root, &target_path, Some(&checkpoint_dir), filesystem)
        {
            Ok(value) => value,
            Err(error) => {
                super::builtin_workspace_patch::remove_checkpoint_dir(
                    &root,
                    &checkpoint_dir,
                    filesystem,
                );
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
        if let Err(error) = validate_checkpoint_capacity(&manifest, after_shape)
            .and_then(|()| write_manifest(&checkpoint_dir, &manifest, filesystem))
        {
            super::builtin_workspace_patch::remove_checkpoint_dir(
                &root,
                &checkpoint_dir,
                filesystem,
            );
            return Err(error);
        }
        Ok(Self {
            filesystem,
            root,
            checkpoint_dir,
            manifest,
            after_shape,
        })
    }

    fn finish(&mut self) -> Result<String, String> {
        let after = capture_snapshot_with_entry_limit(
            &self.root,
            &self.manifest.target_path,
            None,
            self.filesystem,
            1,
        )
        .map_err(|error| self.unconfirmed("after_state_capture", error))?;
        if !self.after_shape.accepts(&self.manifest.target_path, &after) {
            return Err(self.unconfirmed("after_state_validation", mutation_error(
                "checkpoint_after_shape_mismatch",
                "workspace.mutation.after_shape_mismatch",
                json!({
                    "expected_kind": self.after_shape.projected_entry(&self.manifest.target_path).kind,
                    "observed_entry_count": after.len(),
                    "observed_root_kind": after.first().map(|entry| entry.kind.as_str()),
                }),
            )));
        }
        self.manifest.after = after;
        let changed = !snapshot_states_equal(&self.manifest.before, &self.manifest.after);
        self.manifest.state = if changed { "applied" } else { "no_op" }.to_string();
        self.manifest.mutation_id = Some(mutation_id(&self.manifest));
        if let Err(error) = write_manifest(&self.checkpoint_dir, &self.manifest, self.filesystem) {
            return Err(self.unconfirmed("commit", error));
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
        restore_snapshot(
            &self.root,
            &self.checkpoint_dir,
            &self.manifest,
            self.filesystem,
        )
    }

    fn current_matches_before_state(&self) -> Result<bool, String> {
        let current = capture_snapshot(
            &self.root,
            &self.manifest.target_path,
            None,
            self.filesystem,
        )?;
        Ok(snapshot_states_equal(&self.manifest.before, &current)
            && missing_parents_absent(&self.root, &self.manifest, self.filesystem)?)
    }

    fn unconfirmed(&self, phase: &str, cause: String) -> String {
        super::checkpoint_commit_unconfirmed_error(
            "workspace_mutation",
            "workspace.mutation.commit_unconfirmed",
            &self.manifest.task_id,
            &self.manifest.checkpoint_id,
            phase,
            cause,
        )
    }

    fn remove_checkpoint(&self) {
        super::builtin_workspace_patch::remove_checkpoint_dir(
            &self.root,
            &self.checkpoint_dir,
            self.filesystem,
        );
    }
}

fn capture_snapshot(
    root: &Path,
    target_path: &str,
    backup_root: Option<&Path>,
    filesystem: &HostFilesystemPolicy,
) -> Result<Vec<SnapshotEntry>, String> {
    capture_snapshot_with_entry_limit(
        root,
        target_path,
        backup_root,
        filesystem,
        MAX_SNAPSHOT_ENTRIES,
    )
}

fn capture_snapshot_with_entry_limit(
    root: &Path,
    target_path: &str,
    backup_root: Option<&Path>,
    filesystem: &HostFilesystemPolicy,
    max_entries: usize,
) -> Result<Vec<SnapshotEntry>, String> {
    let target = safe_target_from_relative(root, target_path)?;
    let mut entries = Vec::new();
    let mut budget = SnapshotBudget {
        entries: 0,
        bytes: 0,
        max_entries,
    };
    capture_path(
        root,
        &target,
        target_path,
        backup_root,
        &mut entries,
        &mut budget,
        filesystem,
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
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    let mut pending = vec![(path.to_path_buf(), relative.to_string())];
    while let Some((path, relative)) = pending.pop() {
        // Descendants obey the same path contract as their eventual rewind.
        safe_target_from_relative(root, &relative)?;
        budget.entries += 1;
        if budget.entries > budget.max_entries {
            return Err(snapshot_limit_error(budget));
        }
        let metadata = match filesystem.metadata(Some(root), &path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                entries.push(SnapshotEntry {
                    path: relative,
                    kind: "missing".to_string(),
                    sha256: None,
                    size_bytes: None,
                    backup_file: None,
                    unix_mode: None,
                });
                continue;
            }
            Err(error) => return Err(io_error("snapshot_inspection_failed", error)),
        };
        if metadata.file_type == FileType::Symlink {
            return Err(mutation_error(
                "snapshot_symlink_denied",
                "workspace.mutation.symlink_denied",
                json!({"path": relative}),
            ));
        }
        if metadata.file_type == FileType::RegularFile {
            if budget.bytes.saturating_add(metadata.size_bytes) > MAX_SNAPSHOT_BYTES {
                return Err(snapshot_limit_error(budget));
            }
            let (file, _) = filesystem
                .open_file(Some(root), &path)
                .map_err(|error| io_error("snapshot_read_failed", error))?;
            let unix_mode = HostAccessMode::capture(
                &file
                    .metadata()
                    .map_err(|error| io_error("snapshot_inspection_failed", error))?,
            )
            .map_err(|error| io_error("snapshot_permissions_unsupported", error))?;
            let mut bytes = Vec::new();
            file.take(MAX_SNAPSHOT_BYTES.saturating_sub(budget.bytes) + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| io_error("snapshot_read_failed", error))?;
            budget.bytes = budget.bytes.saturating_add(bytes.len() as u64);
            if budget.bytes > MAX_SNAPSHOT_BYTES {
                return Err(snapshot_limit_error(budget));
            }
            let backup_file = if let Some(backup_root) = backup_root {
                let name = format!("before/{:04}.bin", entries.len());
                let backup = backup_root.join(&name);
                filesystem
                    .atomic_write_file_with_options(Some(backup_root), &backup, &bytes, true)
                    .map_err(|error| io_error("snapshot_write_failed", error))?;
                Some(name)
            } else {
                None
            };
            entries.push(SnapshotEntry {
                path: relative,
                kind: "file".to_string(),
                sha256: Some(sha256_label(&bytes)),
                size_bytes: Some(bytes.len() as u64),
                backup_file,
                unix_mode: Some(unix_mode),
            });
            continue;
        }
        if metadata.file_type != FileType::Directory {
            return Err(mutation_error(
                "unsupported_target_type",
                "workspace.mutation.unsupported_target_type",
                json!({"path": relative}),
            ));
        }
        let directory = filesystem
            .open_directory(Some(root), &path)
            .map_err(|error| io_error("snapshot_read_dir_failed", error))?;
        let unix_mode = directory
            .access_mode()
            .map_err(|error| io_error("snapshot_permissions_unsupported", error))?;
        entries.push(SnapshotEntry {
            path: relative,
            kind: "directory".to_string(),
            sha256: None,
            size_bytes: None,
            backup_file: None,
            unix_mode: Some(unix_mode),
        });
        let remaining = budget
            .max_entries
            .saturating_sub(budget.entries)
            .saturating_sub(pending.len());
        // One extra entry proves overflow without allocating the whole directory.
        let mut children = directory
            .entries_iter()
            .map_err(|error| io_error("snapshot_read_dir_failed", error))?
            .take(remaining.saturating_add(1))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|error| io_error("snapshot_read_dir_failed", error))?;
        if children.len() > remaining {
            budget.entries = budget
                .entries
                .saturating_add(pending.len())
                .saturating_add(children.len());
            return Err(snapshot_limit_error(budget));
        }
        // Reverse pushes preserve depth-first lexical order and backup ordinals.
        children.sort_by(|left, right| right.name.cmp(&left.name));
        for child in children {
            let child_path = directory.canonical_path().join(child.name);
            if !child_path.starts_with(root) {
                return Err(mutation_error(
                    "snapshot_outside_workspace",
                    "workspace.mutation.outside_workspace",
                    json!({"path": child_path.display().to_string()}),
                ));
            }
            let child_relative = relative_path(root, &child_path)?;
            pending.push((child_path, child_relative));
        }
    }
    Ok(())
}

struct RestoreFile<'a> {
    target: PathBuf,
    backup: PathBuf,
    entry: &'a SnapshotEntry,
    expected_bytes: usize,
}

struct RestorePlan<'a> {
    target: PathBuf,
    files: Vec<RestoreFile<'a>>,
    directories: Vec<(PathBuf, &'a SnapshotEntry)>,
    missing_parents: Vec<PathBuf>,
}

fn restore_manifest_error(reason: &str, path: &str) -> String {
    mutation_error(
        "checkpoint_restore_invalid",
        "workspace.mutation.restore_invalid",
        json!({"reason":reason,"path":path}),
    )
}

fn backup_validation_error(path: &str, reason: &str, details: Value) -> String {
    mutation_error(
        "checkpoint_backup_invalid",
        "workspace.mutation.backup_invalid",
        json!({"path":path,"reason":reason,"details":details}),
    )
}

fn lexical_restore_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(restore_manifest_error("invalid_relative_path", relative));
    }
    let mut target = root.to_path_buf();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value)
                if value != ".git"
                    && !claw_core::workspace_state::is_known_workspace_state_dir_name(
                        &value.to_string_lossy(),
                    ) =>
            {
                target.push(value)
            }
            std::path::Component::CurDir => {}
            _ => return Err(restore_manifest_error("invalid_relative_path", relative)),
        }
    }
    if target == root {
        return Err(restore_manifest_error("root_target_denied", relative));
    }
    Ok(target)
}

fn plan_restore<'a>(
    root: &Path,
    checkpoint_dir: &Path,
    manifest: &'a MutationCheckpointManifest,
) -> Result<RestorePlan<'a>, String> {
    let target = lexical_restore_path(root, &manifest.target_path)?;
    safe_target_from_relative(root, &manifest.target_path)?;
    if manifest.before.is_empty() || manifest.before.len() > MAX_SNAPSHOT_ENTRIES {
        return Err(restore_manifest_error(
            "entry_count_limit",
            &manifest.target_path,
        ));
    }
    let mut entries = BTreeMap::new();
    for entry in &manifest.before {
        let path = lexical_restore_path(root, &entry.path)?;
        if !path.starts_with(&target) {
            return Err(restore_manifest_error("path_outside_target", &entry.path));
        }
        if entries.insert(path, entry).is_some() {
            return Err(restore_manifest_error("duplicate_path", &entry.path));
        }
    }
    let root_entry = entries
        .get(&target)
        .ok_or_else(|| restore_manifest_error("target_entry_missing", &manifest.target_path))?;
    if root_entry.kind != "directory" && entries.len() != 1 {
        return Err(restore_manifest_error(
            "non_directory_descendants",
            &manifest.target_path,
        ));
    }
    let mut files = Vec::new();
    let mut directories = Vec::new();
    let mut total_bytes = 0_u64;
    for (path, entry) in &entries {
        if entry.kind == "missing" && entry.unix_mode.is_some() {
            return Err(restore_manifest_error("missing_entry_mode", &entry.path));
        }
        if manifest.schema_version == CHECKPOINT_SCHEMA_VERSION
            && matches!(entry.kind.as_str(), "file" | "directory")
            && entry.unix_mode.is_none()
        {
            return Err(restore_manifest_error("native_mode_missing", &entry.path));
        }
        if path != &target {
            let parent_is_directory = path
                .parent()
                .and_then(|parent| entries.get(parent))
                .is_some_and(|entry| entry.kind == "directory");
            if !parent_is_directory {
                return Err(restore_manifest_error("parent_entry_missing", &entry.path));
            }
        }
        match entry.kind.as_str() {
            "file" => {
                let backup = entry.backup_file.as_deref().ok_or_else(|| {
                    mutation_error(
                        "checkpoint_backup_missing",
                        "workspace.mutation.backup_missing",
                        json!({"path":entry.path}),
                    )
                })?;
                let size = entry.size_bytes.ok_or_else(|| {
                    backup_validation_error(&entry.path, "size_missing", Value::Null)
                })?;
                total_bytes = total_bytes
                    .checked_add(size)
                    .filter(|total| *total <= MAX_SNAPSHOT_BYTES)
                    .ok_or_else(|| {
                        backup_validation_error(
                            &entry.path,
                            "byte_budget_exceeded",
                            json!({"max_bytes":MAX_SNAPSHOT_BYTES}),
                        )
                    })?;
                if entry.sha256.is_none() {
                    return Err(backup_validation_error(
                        &entry.path,
                        "hash_missing",
                        Value::Null,
                    ));
                }
                let expected_bytes = usize::try_from(size).map_err(|_| {
                    backup_validation_error(
                        &entry.path,
                        "size_unsupported",
                        json!({"size_bytes":size}),
                    )
                })?;
                files.push(RestoreFile {
                    target: path.clone(),
                    backup: lexical_restore_path(checkpoint_dir, backup)?,
                    entry: *entry,
                    expected_bytes,
                });
            }
            "directory" | "missing" => {
                if entry.sha256.is_some()
                    || entry.size_bytes.is_some()
                    || entry.backup_file.is_some()
                {
                    return Err(restore_manifest_error(
                        "non_file_payload_metadata",
                        &entry.path,
                    ));
                }
                if entry.kind == "directory" {
                    directories.push((path.clone(), *entry));
                }
            }
            _ => return Err(restore_manifest_error("unknown_entry_kind", &entry.path)),
        }
    }
    if manifest.missing_parent_paths.len() > MAX_SNAPSHOT_ENTRIES {
        return Err(restore_manifest_error(
            "parent_count_limit",
            &manifest.target_path,
        ));
    }
    let mut parents = BTreeSet::new();
    for relative in &manifest.missing_parent_paths {
        let parent = lexical_restore_path(root, relative)?;
        if parent == target || !target.starts_with(&parent) {
            return Err(restore_manifest_error("parent_outside_target", relative));
        }
        if !parents.insert(parent) {
            return Err(restore_manifest_error("duplicate_parent", relative));
        }
    }
    directories.sort_by_key(|(path, _)| path.components().count());
    let mut missing_parents = parents.into_iter().collect::<Vec<_>>();
    missing_parents.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    Ok(RestorePlan {
        target,
        files,
        directories,
        missing_parents,
    })
}

#[path = "builtin_workspace_mutation_restore.rs"]
mod restore;
use restore::restore_snapshot;

fn remove_existing_target(
    root: &Path,
    target: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    match filesystem.metadata(Some(root), target) {
        Ok(metadata) if metadata.file_type == FileType::Symlink => Err(mutation_error(
            "restore_symlink_denied",
            "workspace.mutation.symlink_denied",
            json!({"path": target.display().to_string()}),
        )),
        Ok(metadata) if metadata.file_type == FileType::Directory => filesystem
            .remove_path(Some(root), target, true)
            .map_err(|error| io_error("restore_remove_failed", error)),
        Ok(_) => filesystem
            .remove_path(Some(root), target, false)
            .map_err(|error| io_error("restore_remove_failed", error)),
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

fn missing_parents_absent(
    root: &Path,
    manifest: &MutationCheckpointManifest,
    filesystem: &HostFilesystemPolicy,
) -> Result<bool, String> {
    for relative in &manifest.missing_parent_paths {
        let path = safe_target_from_relative(root, relative)?;
        match filesystem.metadata(Some(root), &path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Ok(false),
            Err(error) => return Err(io_error("restore_parent_inspection_failed", error)),
        }
    }
    Ok(true)
}

fn safe_relative_target(
    configured_root: &Path,
    canonical_root: &Path,
    target: &Path,
) -> Result<String, String> {
    // macOS may expose the configured `/var` path through canonical `/private/var`.
    // Rebase only the trusted root alias; nested components are still inspected
    // without following symlinks by `safe_target_from_relative` below.
    let relative = target
        .strip_prefix(canonical_root)
        .or_else(|_| target.strip_prefix(configured_root))
        .map_err(|_| invalid_target_error(target))?
        .to_str()
        .ok_or_else(|| invalid_target_error(target))?
        .to_owned();
    let _ = safe_target_from_relative(canonical_root, &relative)?;
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
            std::path::Component::Normal(value)
                if value != ".git"
                    && !claw_core::workspace_state::is_known_workspace_state_dir_name(
                        &value.to_string_lossy(),
                    ) =>
            {
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
        .map_err(|_| invalid_target_error(path))?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid_target_error(path))
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
                "unix_mode": entry.unix_mode.map(u32::from),
            })
        })
        .collect()
}

fn snapshot_states_equal(left: &[SnapshotEntry], right: &[SnapshotEntry]) -> bool {
    snapshot_matches_recorded_state(left, right, false)
}

fn snapshot_matches_recorded_state(
    left: &[SnapshotEntry],
    right: &[SnapshotEntry],
    allow_unrecorded_modes: bool,
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.path == right.path
                && left.kind == right.kind
                && left.sha256 == right.sha256
                && left.size_bytes == right.size_bytes
                && (left.unix_mode == right.unix_mode
                    || (allow_unrecorded_modes && left.unix_mode.is_none()))
        })
}

fn mode_restoration_coverage(entries: &[SnapshotEntry]) -> &'static str {
    if entries
        .iter()
        .any(|entry| entry.kind != "missing" && entry.unix_mode.is_none())
    {
        "not_recorded"
    } else {
        "complete"
    }
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
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    let bytes = encode_manifest(manifest, "current_metadata")?;
    filesystem
        .atomic_write_file_with_options(
            Some(checkpoint_dir),
            &checkpoint_dir.join("manifest.json"),
            &bytes,
            false,
        )
        .map_err(|error| io_error("checkpoint_write_failed", error))
}

fn validate_checkpoint_capacity(
    manifest: &MutationCheckpointManifest,
    after_shape: MutationAfterShape,
) -> Result<(), String> {
    let mut projected = manifest.clone();
    projected.after = vec![after_shape.projected_entry(&manifest.target_path)];
    projected.mutation_id = Some(sha256_label(&[]));
    encode_manifest(&projected, "projected_metadata").map(|_| ())
}

fn encode_manifest(
    manifest: &MutationCheckpointManifest,
    metadata_scope: &str,
) -> Result<Vec<u8>, String> {
    use super::builtin_workspace_checkpoint_json::{encode_pretty, EncodeError};
    encode_pretty(manifest, MAX_CHECKPOINT_METADATA_BYTES).map_err(|error| match error {
        EncodeError::LimitExceeded { limit_bytes } => mutation_error(
            "checkpoint_too_large", "workspace.mutation.checkpoint_too_large",
            json!({"max_bytes":limit_bytes,"manifest_bytes_at_least":limit_bytes+1,"metadata_scope":metadata_scope}),
        ),
        EncodeError::Serialization(error) => mutation_error(
            "checkpoint_encode_failed", "workspace.mutation.checkpoint_encode_failed",
            json!({"serde_category":format!("{:?}",error.classify()),"metadata_scope":metadata_scope}),
        ),
    })
}

fn read_manifest_value(
    checkpoint_dir: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<Value, String> {
    let bytes = filesystem
        .read_file_bounded(
            Some(checkpoint_dir),
            &checkpoint_dir.join("manifest.json"),
            MAX_CHECKPOINT_METADATA_BYTES,
        )
        .map_err(|error| match error {
            BoundedFileReadError::Io(error) => io_error("checkpoint_read_failed", error),
            BoundedFileReadError::LimitExceeded {
                limit_bytes,
                observed_bytes_at_least,
            } => mutation_error(
                "checkpoint_too_large",
                "workspace.mutation.checkpoint_too_large",
                json!({"max_bytes":limit_bytes,"manifest_bytes_at_least":observed_bytes_at_least}),
            ),
        })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        mutation_error(
            "checkpoint_invalid",
            "workspace.mutation.checkpoint_invalid",
            json!({"error_code": format!("{:?}", error.classify())}),
        )
    })
}

fn read_manifest(
    checkpoint_dir: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<MutationCheckpointManifest, String> {
    let value = read_manifest_value(checkpoint_dir, filesystem)?;
    let manifest: MutationCheckpointManifest = serde_json::from_value(value).map_err(|error| {
        mutation_error(
            "checkpoint_invalid",
            "workspace.mutation.checkpoint_invalid",
            json!({"error_code": format!("{:?}", error.classify())}),
        )
    })?;
    if !matches!(manifest.schema_version, 1 | CHECKPOINT_SCHEMA_VERSION)
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
            "max_entries": budget.max_entries,
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
    let error_code = if HostFilesystemPolicy::is_private_access_denied(&error) {
        "host_private_directory_access_denied"
    } else {
        error_code
    };
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
            json!({"error_code": format!("{:?}", error.classify())}),
        )
    })
}

#[cfg(test)]
#[path = "builtin_workspace_mutation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "builtin_workspace_mutation_capacity_tests.rs"]
mod capacity_tests;

#[cfg(test)]
#[path = "builtin_workspace_mutation_restore_tests.rs"]
mod restore_tests;

#[cfg(test)]
#[path = "builtin_workspace_snapshot_enumeration_tests.rs"]
mod enumeration_tests;

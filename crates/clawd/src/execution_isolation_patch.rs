use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{bail, Context, Result};
use claw_core::skill_registry::CapabilityIsolationProfile;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::ExecutionIsolationPlan;

const MAX_CHILD_PATCH_BYTES: usize = 2 * 1024 * 1024;
const MAX_CHILD_PATCH_FILES: usize = 512;
const ISOLATION_MARKER_PATHSPEC: &str = ":(exclude).rustclaw-isolation.json";

#[derive(Debug)]
pub(crate) struct ValidatedChildPatchArtifact {
    pub(crate) plan: ExecutionIsolationPlan,
    pub(crate) patch: Vec<u8>,
    pub(crate) metadata: Value,
}

pub(crate) fn build_child_worktree_patch_artifact(plan: &ExecutionIsolationPlan) -> Result<Value> {
    if plan.profile != "local_worktree" || plan.creation_kind != "create_local_git_worktree" {
        bail!("child_patch_requires_local_worktree");
    }
    super::validate_existing_isolation(plan)?;
    let artifact_dir = super::isolation_artifact_dir(&plan.workspace_root);
    fs::create_dir_all(&artifact_dir).with_context(|| {
        format!(
            "create_child_patch_artifact_dir:{path}",
            path = artifact_dir.display()
        )
    })?;
    let index_path = temporary_index_path(&artifact_dir, &plan.task_key);
    let _index_guard = TempIndexGuard(index_path.clone());

    run_git_with_index(plan, &index_path, &["read-tree", "HEAD"])?;
    run_git_with_index(
        plan,
        &index_path,
        &["add", "-A", "--", ".", ISOLATION_MARKER_PATHSPEC],
    )?;
    let names_output = run_git_with_index(
        plan,
        &index_path,
        &[
            "diff",
            "--cached",
            "--name-only",
            "-z",
            "HEAD",
            "--",
            ".",
            ISOLATION_MARKER_PATHSPEC,
        ],
    )?;
    let changed_files = parse_nul_paths(&names_output.stdout)?;
    if changed_files.len() > MAX_CHILD_PATCH_FILES {
        bail!("child_worktree_patch_file_limit_exceeded");
    }
    let patch_output = run_git_with_index(
        plan,
        &index_path,
        &[
            "diff",
            "--cached",
            "--binary",
            "--no-ext-diff",
            "HEAD",
            "--",
            ".",
            ISOLATION_MARKER_PATHSPEC,
        ],
    )?;
    if patch_output.stdout.len() > MAX_CHILD_PATCH_BYTES {
        bail!("child_worktree_patch_byte_limit_exceeded");
    }
    let base_commit = run_git(plan, &["rev-parse", "HEAD"])?;
    let base_commit = String::from_utf8(base_commit.stdout)
        .context("child_worktree_base_commit_not_utf8")?
        .trim()
        .to_string();
    let precondition_hashes = base_precondition_hashes(plan, &changed_files)?;
    if patch_output.stdout.is_empty() {
        return Ok(json!({
            "schema_version": 1,
            "kind": "child_worktree_patch",
            "status": "empty",
            "base_commit": base_commit,
            "changed_file_count": 0,
            "changed_files": [],
            "patch_bytes": 0,
            "patch_ref": Value::Null,
            "artifact_path": Value::Null,
            "precondition_hashes": precondition_hashes,
            "apply_owner": "parent_agent",
            "apply_policy": "parent_review_required",
            "cleanup_ref": plan.cleanup_ref,
        }));
    }

    let patch_sha256 = format!("sha256:{:x}", Sha256::digest(&patch_output.stdout));
    let patch_ref = format!("child_worktree_patch:{patch_sha256}");
    let artifact_path = artifact_dir.join(format!("{}.patch", plan.task_key));
    write_private_atomic(&artifact_path, &patch_output.stdout)?;
    Ok(json!({
        "schema_version": 1,
        "kind": "child_worktree_patch",
        "status": "ready",
        "base_commit": base_commit,
        "changed_file_count": changed_files.len(),
        "changed_files": changed_files,
        "patch_bytes": patch_output.stdout.len(),
        "patch_sha256": patch_sha256,
        "patch_ref": patch_ref,
        "artifact_path": artifact_path.display().to_string(),
        "precondition_hashes": precondition_hashes,
        "apply_owner": "parent_agent",
        "apply_policy": "parent_review_required",
        "cleanup_ref": plan.cleanup_ref,
    }))
}

pub(crate) fn load_validated_child_worktree_patch_artifact(
    workspace_root: &Path,
    child_task_id: &str,
    metadata: &Value,
) -> Result<ValidatedChildPatchArtifact> {
    let plan = super::plan_execution_isolation(
        workspace_root,
        child_task_id,
        CapabilityIsolationProfile::LocalWorktree,
    )?;
    require_metadata_token(metadata, "kind", "child_worktree_patch")?;
    require_metadata_token(metadata, "apply_owner", "parent_agent")?;
    require_metadata_token(metadata, "apply_policy", "parent_review_required")?;
    let cleanup_ref = metadata
        .get("cleanup_ref")
        .and_then(Value::as_str)
        .context("child_patch_cleanup_ref_missing")?;
    if Some(cleanup_ref) != plan.cleanup_ref.as_deref() {
        bail!("child_patch_cleanup_ref_mismatch");
    }
    super::validate_existing_isolation(&plan)?;

    let base_commit = metadata
        .get("base_commit")
        .and_then(Value::as_str)
        .filter(|value| is_git_object_id(value))
        .context("child_patch_base_commit_invalid")?;
    let observed_child_head = run_git(&plan, &["rev-parse", "HEAD"])?;
    if String::from_utf8(observed_child_head.stdout)
        .context("child_patch_head_not_utf8")?
        .trim()
        != base_commit
    {
        bail!("child_patch_base_commit_mismatch");
    }
    validate_precondition_metadata(metadata)?;

    match metadata.get("status").and_then(Value::as_str) {
        Some("empty") => {
            if metadata.get("changed_file_count").and_then(Value::as_u64) != Some(0)
                || metadata.get("patch_bytes").and_then(Value::as_u64) != Some(0)
                || !metadata.get("patch_ref").is_none_or(Value::is_null)
                || !metadata.get("artifact_path").is_none_or(Value::is_null)
            {
                bail!("child_patch_empty_metadata_invalid");
            }
            Ok(ValidatedChildPatchArtifact {
                plan,
                patch: Vec::new(),
                metadata: metadata.clone(),
            })
        }
        Some("ready") => {
            let artifact_path = metadata
                .get("artifact_path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .context("child_patch_artifact_path_missing")?;
            let expected_path = super::isolation_artifact_dir(workspace_root)
                .join(format!("{}.patch", plan.task_key));
            if artifact_path != expected_path {
                bail!("child_patch_artifact_path_mismatch");
            }
            let file_metadata =
                fs::symlink_metadata(&artifact_path).context("child_patch_artifact_missing")?;
            if file_metadata.file_type().is_symlink() || !file_metadata.is_file() {
                bail!("child_patch_artifact_not_regular_file");
            }
            let patch = fs::read(&artifact_path).context("read_child_patch_artifact")?;
            if patch.is_empty() || patch.len() > MAX_CHILD_PATCH_BYTES {
                bail!("child_patch_artifact_size_invalid");
            }
            if metadata.get("patch_bytes").and_then(Value::as_u64) != Some(patch.len() as u64) {
                bail!("child_patch_artifact_byte_count_mismatch");
            }
            let patch_sha256 = format!("sha256:{:x}", Sha256::digest(&patch));
            if metadata.get("patch_sha256").and_then(Value::as_str) != Some(patch_sha256.as_str()) {
                bail!("child_patch_artifact_sha256_mismatch");
            }
            let patch_ref = format!("child_worktree_patch:{patch_sha256}");
            if metadata.get("patch_ref").and_then(Value::as_str) != Some(patch_ref.as_str()) {
                bail!("child_patch_artifact_ref_mismatch");
            }
            Ok(ValidatedChildPatchArtifact {
                plan,
                patch,
                metadata: metadata.clone(),
            })
        }
        _ => bail!("child_patch_artifact_status_invalid"),
    }
}

pub(crate) fn child_patch_base_is_parent_ancestor(
    workspace_root: &Path,
    base_commit: &str,
) -> Result<bool> {
    if !is_git_object_id(base_commit) {
        bail!("child_patch_base_commit_invalid");
    }
    let status = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(["merge-base", "--is-ancestor", base_commit, "HEAD"])
        .status()
        .context("spawn_child_patch_parent_merge_base")?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => bail!("child_patch_parent_merge_base_failed"),
    }
}

pub(crate) fn cleanup_child_worktree_artifacts(
    workspace_root: &Path,
    child_task_id: &str,
) -> Result<Value> {
    let plan = super::plan_execution_isolation(
        workspace_root,
        child_task_id,
        CapabilityIsolationProfile::LocalWorktree,
    )?;
    super::cleanup_execution_isolation(&plan)?;
    let artifact_path =
        super::isolation_artifact_dir(workspace_root).join(format!("{}.patch", plan.task_key));
    let artifact_removed = if artifact_path.exists() {
        fs::remove_file(&artifact_path).context("remove_child_patch_artifact")?;
        true
    } else {
        false
    };
    Ok(json!({
        "status": "complete",
        "cleanup_ref": plan.cleanup_ref,
        "worktree_removed": !plan.execution_root.exists(),
        "artifact_removed": artifact_removed,
    }))
}

fn base_precondition_hashes(
    plan: &ExecutionIsolationPlan,
    changed_files: &[String],
) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    for path in changed_files {
        let entry = run_git(plan, &["ls-tree", "-z", "HEAD", "--", path])?;
        if entry.stdout.is_empty() {
            hashes.insert(path.clone(), "missing".to_string());
            continue;
        }
        let record = entry
            .stdout
            .split(|byte| *byte == 0)
            .find(|item| !item.is_empty())
            .context("child_patch_base_tree_record_missing")?;
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("child_patch_base_tree_record_invalid")?;
        let (header, observed_path_with_separator) = record.split_at(separator);
        let observed_path = observed_path_with_separator
            .get(1..)
            .context("child_patch_base_tree_path_missing")?;
        if observed_path != path.as_bytes() {
            bail!("child_patch_base_tree_path_mismatch");
        }
        let header =
            std::str::from_utf8(header).context("child_patch_base_tree_header_not_utf8")?;
        let mut fields = header.split_ascii_whitespace();
        let _mode = fields
            .next()
            .context("child_patch_base_tree_mode_missing")?;
        if fields.next() != Some("blob") {
            bail!("child_patch_base_tree_entry_not_blob");
        }
        let object_id = fields
            .next()
            .filter(|value| is_git_object_id(value))
            .context("child_patch_base_tree_object_invalid")?;
        if fields.next().is_some() {
            bail!("child_patch_base_tree_header_invalid");
        }
        let blob = run_git(plan, &["cat-file", "blob", object_id])?;
        hashes.insert(
            path.clone(),
            format!("sha256:{:x}", Sha256::digest(&blob.stdout)),
        );
    }
    Ok(hashes)
}

fn validate_precondition_metadata(metadata: &Value) -> Result<()> {
    let changed_files = metadata
        .get("changed_files")
        .and_then(Value::as_array)
        .context("child_patch_changed_files_missing")?;
    if changed_files.len() > MAX_CHILD_PATCH_FILES
        || metadata.get("changed_file_count").and_then(Value::as_u64)
            != Some(changed_files.len() as u64)
    {
        bail!("child_patch_changed_file_count_invalid");
    }
    let preconditions = metadata
        .get("precondition_hashes")
        .and_then(Value::as_object)
        .context("child_patch_preconditions_missing")?;
    if preconditions.len() != changed_files.len() {
        bail!("child_patch_precondition_count_mismatch");
    }
    for path in changed_files {
        let path = path
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 1024)
            .context("child_patch_changed_file_invalid")?;
        let expected = preconditions
            .get(path)
            .and_then(Value::as_str)
            .context("child_patch_precondition_missing")?;
        if expected != "missing"
            && !expected.strip_prefix("sha256:").is_some_and(|value| {
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            bail!("child_patch_precondition_invalid");
        }
    }
    Ok(())
}

fn require_metadata_token(metadata: &Value, key: &str, expected: &str) -> Result<()> {
    if metadata.get(key).and_then(Value::as_str) != Some(expected) {
        bail!("child_patch_metadata_token_invalid:{key}");
    }
    Ok(())
}

fn is_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn run_git(plan: &ExecutionIsolationPlan, args: &[&str]) -> Result<Output> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&plan.execution_root)
        .args(args)
        .output()
        .context("spawn_child_worktree_git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("child_worktree_git_failed:{stderr}", stderr = stderr.trim());
    }
    Ok(output)
}

fn run_git_with_index(
    plan: &ExecutionIsolationPlan,
    index_path: &Path,
    args: &[&str],
) -> Result<Output> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&plan.execution_root)
        .env("GIT_INDEX_FILE", index_path)
        .args(args)
        .output()
        .context("spawn_child_worktree_index_git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "child_worktree_index_git_failed:{stderr}",
            stderr = stderr.trim()
        );
    }
    Ok(output)
}

fn parse_nul_paths(raw: &[u8]) -> Result<Vec<String>> {
    raw.split(|byte| *byte == 0)
        .filter(|item| !item.is_empty())
        .map(|item| {
            let path = String::from_utf8(item.to_vec()).context("child_patch_path_not_utf8")?;
            if path.is_empty() || path.len() > 1024 || path.starts_with('/') || path.contains('\0')
            {
                bail!("child_patch_path_invalid");
            }
            Ok(path)
        })
        .collect()
}

fn temporary_index_path(artifact_dir: &Path, task_key: &str) -> PathBuf {
    artifact_dir.join(format!(
        ".{task_key}.index.{}.{}",
        std::process::id(),
        crate::now_ts_u64()
    ))
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("child_patch_artifact_parent_missing")?;
    fs::create_dir_all(parent)?;
    let temp_path = path.with_extension(format!("patch.tmp.{}", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temp_path).with_context(|| {
        format!(
            "create_child_patch_artifact:{path}",
            path = temp_path.display()
        )
    })?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    fs::rename(&temp_path, path)
        .with_context(|| format!("commit_child_patch_artifact:{path}", path = path.display()))
}

struct TempIndexGuard(PathBuf);

impl Drop for TempIndexGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_file(PathBuf::from(format!("{}.lock", self.0.display())));
    }
}

#[cfg(test)]
#[path = "execution_isolation_patch_tests.rs"]
mod tests;

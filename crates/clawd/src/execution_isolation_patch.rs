use std::fs;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::ExecutionIsolationPlan;

const MAX_CHILD_PATCH_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHILD_PATCH_FILES: usize = 512;
const ISOLATION_MARKER_PATHSPEC: &str = ":(exclude).rustclaw-isolation.json";

pub(crate) fn build_child_worktree_patch_artifact(plan: &ExecutionIsolationPlan) -> Result<Value> {
    if plan.profile != "local_worktree" || plan.creation_kind != "create_local_git_worktree" {
        bail!("child_patch_requires_local_worktree");
    }
    super::validate_existing_isolation(plan)?;
    let artifact_dir = super::isolation_artifact_dir(&plan.workspace_root);
    fs::create_dir_all(&artifact_dir)
        .with_context(|| format!("create child patch artifact dir {}", artifact_dir.display()))?;
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
        "apply_owner": "parent_agent",
        "apply_policy": "parent_review_required",
        "cleanup_ref": plan.cleanup_ref,
    }))
}

fn run_git(plan: &ExecutionIsolationPlan, args: &[&str]) -> Result<Output> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&plan.execution_root)
        .args(args)
        .output()
        .context("spawn_child_worktree_git")?;
    if !output.status.success() {
        bail!(
            "child_worktree_git_failed:{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
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
        bail!(
            "child_worktree_index_git_failed:{}",
            String::from_utf8_lossy(&output.stderr).trim()
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
    let mut file = options
        .open(&temp_path)
        .with_context(|| format!("create child patch artifact {}", temp_path.display()))?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    fs::rename(&temp_path, path)
        .with_context(|| format!("commit child patch artifact {}", path.display()))
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

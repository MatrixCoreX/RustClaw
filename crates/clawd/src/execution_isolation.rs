#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use claw_core::skill_registry::CapabilityIsolationProfile;
use serde::Serialize;
use serde_json::{json, Value};

const ISOLATION_ROOT_DIR: &str = ".rustclaw";
const ISOLATION_DIR: &str = "isolation";
const TEMP_DIR: &str = "temp";
const WORKTREE_DIR: &str = "worktrees";
const MARKER_FILE: &str = ".rustclaw-isolation.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExecutionIsolationPlan {
    pub profile: String,
    pub task_key: String,
    pub workspace_root: PathBuf,
    pub execution_root: PathBuf,
    pub allocation_root: Option<PathBuf>,
    pub creation_kind: String,
    pub cleanup_ref: Option<String>,
    pub read_only: bool,
    pub remote: bool,
    pub requires_cleanup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExecutionIsolationRuntime {
    pub plan: ExecutionIsolationPlan,
    pub artifact_refs: Vec<Value>,
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub(crate) struct IsolationCleanupReport {
    pub removed: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

pub(crate) fn plan_execution_isolation(
    workspace_root: &Path,
    task_id: &str,
    profile: CapabilityIsolationProfile,
) -> Result<ExecutionIsolationPlan> {
    let task_key = isolation_task_key(task_id)?;
    let profile_token = profile.as_token().to_string();
    let base = isolation_base(workspace_root);

    let plan = match profile {
        CapabilityIsolationProfile::LocalCurrentWorkspace => ExecutionIsolationPlan {
            profile: profile_token,
            task_key,
            workspace_root: workspace_root.to_path_buf(),
            execution_root: workspace_root.to_path_buf(),
            allocation_root: None,
            creation_kind: "reuse_current_workspace".to_string(),
            cleanup_ref: None,
            read_only: false,
            remote: false,
            requires_cleanup: false,
        },
        CapabilityIsolationProfile::ReadOnly => ExecutionIsolationPlan {
            profile: profile_token,
            task_key,
            workspace_root: workspace_root.to_path_buf(),
            execution_root: workspace_root.to_path_buf(),
            allocation_root: None,
            creation_kind: "reuse_read_only_workspace".to_string(),
            cleanup_ref: None,
            read_only: true,
            remote: false,
            requires_cleanup: false,
        },
        CapabilityIsolationProfile::LocalTempWorkspace => {
            let allocation_root = base.join(TEMP_DIR).join(&task_key);
            let cleanup_ref = Some(format!("isolation:{TEMP_DIR}:{task_key}"));
            ExecutionIsolationPlan {
                profile: profile_token,
                task_key,
                workspace_root: workspace_root.to_path_buf(),
                execution_root: allocation_root.clone(),
                allocation_root: Some(allocation_root),
                creation_kind: "create_local_temp_workspace".to_string(),
                cleanup_ref,
                read_only: false,
                remote: false,
                requires_cleanup: true,
            }
        }
        CapabilityIsolationProfile::LocalWorktree => {
            let allocation_root = base.join(WORKTREE_DIR).join(&task_key);
            let cleanup_ref = Some(format!("isolation:{WORKTREE_DIR}:{task_key}"));
            ExecutionIsolationPlan {
                profile: profile_token,
                task_key,
                workspace_root: workspace_root.to_path_buf(),
                execution_root: allocation_root.clone(),
                allocation_root: Some(allocation_root),
                creation_kind: "create_local_git_worktree".to_string(),
                cleanup_ref,
                read_only: false,
                remote: false,
                requires_cleanup: true,
            }
        }
        CapabilityIsolationProfile::RemoteExecutor => ExecutionIsolationPlan {
            profile: profile_token,
            task_key,
            workspace_root: workspace_root.to_path_buf(),
            execution_root: workspace_root.to_path_buf(),
            allocation_root: None,
            creation_kind: "delegate_remote_executor".to_string(),
            cleanup_ref: None,
            read_only: false,
            remote: true,
            requires_cleanup: false,
        },
    };
    Ok(plan)
}

pub(crate) fn create_execution_isolation(
    plan: &ExecutionIsolationPlan,
    created_at_unix: u64,
) -> Result<ExecutionIsolationRuntime> {
    match plan.creation_kind.as_str() {
        "create_local_temp_workspace" => {
            ensure_safe_allocation_path(plan)?;
            fs::create_dir_all(&plan.execution_root).with_context(|| {
                format!("create isolation dir {}", plan.execution_root.display())
            })?;
            write_isolation_marker(plan, created_at_unix)?;
        }
        "create_local_git_worktree" => {
            ensure_safe_allocation_path(plan)?;
            create_git_worktree(plan)?;
            write_isolation_marker(plan, created_at_unix)?;
        }
        "reuse_current_workspace" | "reuse_read_only_workspace" | "delegate_remote_executor" => {}
        other => bail!("unknown_isolation_creation_kind:{other}"),
    }
    Ok(execution_isolation_runtime(plan, false))
}

pub(crate) fn create_or_reuse_execution_isolation(
    plan: &ExecutionIsolationPlan,
    created_at_unix: u64,
) -> Result<ExecutionIsolationRuntime> {
    if !plan.requires_cleanup || !plan.execution_root.exists() {
        return create_execution_isolation(plan, created_at_unix);
    }
    ensure_safe_allocation_path(plan)?;
    validate_existing_isolation(plan)?;
    Ok(execution_isolation_runtime(plan, true))
}

pub(crate) fn cleanup_execution_isolation(plan: &ExecutionIsolationPlan) -> Result<()> {
    if !plan.requires_cleanup {
        return Ok(());
    }
    ensure_safe_allocation_path(plan)?;
    match plan.creation_kind.as_str() {
        "create_local_git_worktree" => remove_git_worktree(plan),
        "create_local_temp_workspace" => remove_dir_if_exists(&plan.execution_root),
        other => bail!("unknown_isolation_creation_kind:{other}"),
    }
}

pub(crate) fn is_execution_isolation_root(path: &Path) -> bool {
    read_isolation_marker(path).is_some()
}

pub(crate) fn execution_isolation_root_profile(path: &Path) -> Option<String> {
    read_isolation_marker(path)?
        .get("profile")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) fn cleanup_abandoned_isolation_workspaces(
    workspace_root: &Path,
    now_unix: u64,
    older_than_seconds: u64,
) -> IsolationCleanupReport {
    let mut report = IsolationCleanupReport::default();
    cleanup_abandoned_family(
        workspace_root,
        TEMP_DIR,
        "create_local_temp_workspace",
        now_unix,
        older_than_seconds,
        &mut report,
    );
    cleanup_abandoned_family(
        workspace_root,
        WORKTREE_DIR,
        "create_local_git_worktree",
        now_unix,
        older_than_seconds,
        &mut report,
    );
    report
}

pub(crate) fn execution_isolation_artifact_ref(plan: &ExecutionIsolationPlan) -> Value {
    json!({
        "kind": "execution_isolation_workspace",
        "profile": plan.profile,
        "creation_kind": plan.creation_kind,
        "execution_root": plan.execution_root.display().to_string(),
        "artifact_path": plan.execution_root.display().to_string(),
        "cleanup_ref": plan.cleanup_ref,
        "read_only": plan.read_only,
        "remote": plan.remote,
        "requires_cleanup": plan.requires_cleanup,
    })
}

fn execution_isolation_runtime(
    plan: &ExecutionIsolationPlan,
    reused: bool,
) -> ExecutionIsolationRuntime {
    let mut artifact = execution_isolation_artifact_ref(plan);
    if let Some(obj) = artifact.as_object_mut() {
        obj.insert(
            "allocation_state".to_string(),
            json!(if reused { "reused" } else { "created" }),
        );
    }
    ExecutionIsolationRuntime {
        plan: plan.clone(),
        artifact_refs: vec![artifact],
        reused,
    }
}

pub(crate) fn isolation_profile_from_token(token: &str) -> Option<CapabilityIsolationProfile> {
    match token.trim() {
        "local_current_workspace" => Some(CapabilityIsolationProfile::LocalCurrentWorkspace),
        "local_worktree" => Some(CapabilityIsolationProfile::LocalWorktree),
        "local_temp_workspace" => Some(CapabilityIsolationProfile::LocalTempWorkspace),
        "remote_executor" => Some(CapabilityIsolationProfile::RemoteExecutor),
        "read_only" => Some(CapabilityIsolationProfile::ReadOnly),
        _ => None,
    }
}

fn isolation_base(workspace_root: &Path) -> PathBuf {
    workspace_root.join(ISOLATION_ROOT_DIR).join(ISOLATION_DIR)
}

fn isolation_task_key(task_id: &str) -> Result<String> {
    let mut out = String::new();
    for ch in task_id.trim().chars().take(96) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let out = out.trim_matches(['.', '-']).to_string();
    if out.is_empty()
        || out == ".."
        || !out.chars().any(|ch| ch.is_ascii_alphanumeric())
        || out.contains("..")
    {
        bail!("invalid_isolation_task_key");
    }
    Ok(out)
}

fn ensure_safe_allocation_path(plan: &ExecutionIsolationPlan) -> Result<()> {
    let base = isolation_base(&plan.workspace_root);
    let allocation_root = plan
        .allocation_root
        .as_ref()
        .context("missing_isolation_allocation_root")?;
    if !allocation_root.starts_with(&base) || allocation_root == &base {
        bail!("unsafe_isolation_allocation_path");
    }
    Ok(())
}

fn write_isolation_marker(plan: &ExecutionIsolationPlan, created_at_unix: u64) -> Result<()> {
    let marker = json!({
        "marker_kind": "rustclaw_execution_isolation",
        "task_key": plan.task_key,
        "profile": plan.profile,
        "creation_kind": plan.creation_kind,
        "created_at_unix": created_at_unix,
        "cleanup_ref": plan.cleanup_ref,
    });
    fs::write(
        plan.execution_root.join(MARKER_FILE),
        serde_json::to_vec_pretty(&marker)?,
    )
    .with_context(|| format!("write isolation marker {}", plan.execution_root.display()))
}

fn validate_existing_isolation(plan: &ExecutionIsolationPlan) -> Result<()> {
    let marker = read_isolation_marker(&plan.execution_root)
        .context("existing_isolation_marker_missing_or_invalid")?;
    for (field, expected) in [
        ("task_key", plan.task_key.as_str()),
        ("profile", plan.profile.as_str()),
        ("creation_kind", plan.creation_kind.as_str()),
    ] {
        if marker.get(field).and_then(Value::as_str) != Some(expected) {
            bail!("existing_isolation_contract_mismatch:{field}");
        }
    }
    if plan.creation_kind == "create_local_git_worktree"
        && !plan.execution_root.join(".git").exists()
    {
        bail!("existing_isolation_git_worktree_missing");
    }
    Ok(())
}

fn create_git_worktree(plan: &ExecutionIsolationPlan) -> Result<()> {
    if plan.execution_root.exists() {
        bail!("isolation_target_exists");
    }
    if let Some(parent) = plan.execution_root.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create worktree parent {}", parent.display()))?;
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(&plan.workspace_root)
        .args(["worktree", "add", "--detach"])
        .arg(&plan.execution_root)
        .arg("HEAD")
        .output()
        .context("spawn_git_worktree_add")?;
    if !output.status.success() {
        bail!(
            "git_worktree_add_failed:{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn remove_git_worktree(plan: &ExecutionIsolationPlan) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&plan.workspace_root)
        .args(["worktree", "remove", "--force"])
        .arg(&plan.execution_root)
        .output()
        .context("spawn_git_worktree_remove")?;
    if output.status.success() {
        return Ok(());
    }
    remove_dir_if_exists(&plan.execution_root)
}

fn remove_dir_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .with_context(|| format!("remove isolation dir {}", path.display()))?;
    }
    Ok(())
}

fn cleanup_abandoned_family(
    workspace_root: &Path,
    family: &str,
    creation_kind: &str,
    now_unix: u64,
    older_than_seconds: u64,
    report: &mut IsolationCleanupReport,
) {
    let family_root = isolation_base(workspace_root).join(family);
    let Ok(entries) = fs::read_dir(&family_root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            report.skipped += 1;
            continue;
        }
        let Some(marker) = read_isolation_marker(&path) else {
            report.skipped += 1;
            continue;
        };
        let created_at = marker
            .get("created_at_unix")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX);
        if now_unix.saturating_sub(created_at) < older_than_seconds {
            report.skipped += 1;
            continue;
        }
        let plan = ExecutionIsolationPlan {
            profile: marker
                .get("profile")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            task_key: marker
                .get("task_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            workspace_root: workspace_root.to_path_buf(),
            execution_root: path.clone(),
            allocation_root: Some(path),
            creation_kind: creation_kind.to_string(),
            cleanup_ref: marker
                .get("cleanup_ref")
                .and_then(Value::as_str)
                .map(str::to_string),
            read_only: false,
            remote: false,
            requires_cleanup: true,
        };
        match cleanup_execution_isolation(&plan) {
            Ok(()) => report.removed += 1,
            Err(err) => report.errors.push(err.to_string()),
        }
    }
}

fn read_isolation_marker(path: &Path) -> Option<Value> {
    let body = fs::read_to_string(path.join(MARKER_FILE)).ok()?;
    let marker: Value = serde_json::from_str(&body).ok()?;
    (marker.get("marker_kind").and_then(Value::as_str) == Some("rustclaw_execution_isolation"))
        .then_some(marker)
}

#[cfg(test)]
#[path = "execution_isolation_tests.rs"]
mod tests;

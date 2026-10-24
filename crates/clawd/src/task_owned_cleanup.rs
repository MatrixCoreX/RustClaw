//! Task-owned paths only; never derives deletion paths from skill output prose.
use std::path::{Component, Path, PathBuf};

use anyhow::ensure;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskOwnedCleanupPlan {
    pub(crate) task_id: String,
    pub(crate) durable_jobs: Vec<String>,
}

// Canonicalize the trusted workspace itself (e.g. macOS /tmp), but reject any
// symlink below it. Missing descendants are safe and make retries idempotent.
pub(crate) fn validate_owned_path(workspace: &Path, path: &Path) -> anyhow::Result<bool> {
    let relative = path.strip_prefix(workspace)?;
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "work_cleanup_path_invalid"
    );
    let mut cursor = workspace.canonicalize()?;
    for component in relative.components() {
        cursor.push(component);
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "work_cleanup_symlink_rejected"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}

fn task_roots(workspace: &Path, task_id: &str) -> anyhow::Result<Vec<PathBuf>> {
    let task = uuid::Uuid::parse_str(task_id)?;
    ensure!(task.to_string() == task_id, "work_cleanup_task_id_invalid");
    let artifacts = claw_core::workspace_state::workspace_artifacts_root(workspace);
    // These are host-owned task namespaces, not skill-private storage.
    Ok([
        "delivery",
        "skill-invocations",
        "task-storage",
        "async-process",
        "skill-output",
        "canonical-evidence",
        "model-evidence",
        "transcript-review",
        "transcript-fallback",
    ]
    .into_iter()
    .map(|namespace| artifacts.join(namespace).join(task_id))
    .collect())
}

pub(crate) fn prepare(workspace: &Path, task_id: &str) -> anyhow::Result<TaskOwnedCleanupPlan> {
    let roots = task_roots(workspace, task_id)?;
    for root in &roots {
        validate_owned_path(workspace, root)?;
    }
    let invocation_root = roots[1].clone();
    let async_root = claw_core::workspace_state::workspace_state_root(workspace).join("async_jobs");
    let mut durable_jobs = crate::task_result_retention::durable_job_ids(workspace, task_id)?;
    for id in &durable_jobs {
        let path = async_root.join(id);
        if !validate_owned_path(workspace, &path)? {
            continue;
        }
        let metadata = path.join("host_artifact_output_directory");
        if !validate_owned_path(workspace, &metadata)? {
            // Registration precedes launch. A crash before these pre-spawn
            // metadata/output files exist left an unstarted, task-owned job.
            ensure!(
                !path.join("pid").exists()
                    && !path.join("stdout").exists()
                    && !path.join("lease_ready").exists()
                    && !crate::task_result_retention::monitors::active(&path),
                "work_cleanup_job_binding_missing"
            );
            continue;
        }
        let artifact_dir = std::fs::read_to_string(&metadata)?;
        let artifact_dir = Path::new(artifact_dir.trim());
        ensure!(
            artifact_dir.starts_with(&invocation_root) && artifact_dir != invocation_root,
            "work_cleanup_job_binding_invalid"
        );
        validate_owned_path(workspace, artifact_dir)?;
        for name in ["job_kind", "exit_code", "pid", "process_command_marker"] {
            validate_owned_path(workspace, &path.join(name))?;
        }
        ensure!(
            std::fs::read_to_string(path.join("job_kind"))?.trim() == "skill_runner",
            "work_cleanup_job_kind_invalid"
        );
        ensure!(
            !crate::task_result_retention::monitors::active(&path),
            "work_cleanup_job_monitor_active"
        );
        let exit_recorded = std::fs::read_to_string(path.join("exit_code"))
            .ok()
            .is_some_and(|s| s.trim().parse::<i32>().is_ok());
        let identity = crate::local_process_job::read_pid(&path)
            .map(|pid| crate::local_process_job::process_identity_state(&path, pid))
            .unwrap_or(crate::local_process_job::ProcessIdentityState::Missing);
        validate_owned_path(workspace, &path.join("process_missing_observed_at"))?;
        let stable = !exit_recorded
            && crate::local_process_job::process_loss_is_stable(
                &path,
                identity,
                crate::now_ts_u64().min(i64::MAX as u64) as i64,
                5,
            );
        ensure!(
            terminal_process_is_stopped(exit_recorded, identity, stable),
            "work_cleanup_job_not_stopped"
        );
    }
    durable_jobs.sort();
    Ok(TaskOwnedCleanupPlan {
        task_id: task_id.to_string(),
        durable_jobs,
    })
}

fn terminal_process_is_stopped(
    exit_recorded: bool,
    identity: crate::local_process_job::ProcessIdentityState,
    loss_stable: bool,
) -> bool {
    matches!(
        identity,
        crate::local_process_job::ProcessIdentityState::Missing
            | crate::local_process_job::ProcessIdentityState::IdentityMismatch
    ) && (exit_recorded || loss_stable)
}

pub(crate) fn execute(workspace: &Path, plan: &TaskOwnedCleanupPlan) -> anyhow::Result<()> {
    let mut roots = task_roots(workspace, &plan.task_id)?;
    let async_root = claw_core::workspace_state::workspace_state_root(workspace).join("async_jobs");
    let registered_jobs = crate::task_result_retention::durable_job_ids(workspace, &plan.task_id)?;
    for id in &plan.durable_jobs {
        ensure!(registered_jobs.contains(id), "work_cleanup_job_not_owned");
        let parsed = uuid::Uuid::parse_str(id)?;
        ensure!(parsed.to_string() == *id, "work_cleanup_job_id_invalid");
        let path = async_root.join(id);
        ensure!(
            !crate::task_result_retention::monitors::active(&path),
            "work_cleanup_job_monitor_active"
        );
        roots.push(path);
    }
    // Validate the complete deletion set before deleting anything. remove_dir_all
    // does not follow descendant symlinks; ancestors and each root are rejected.
    for root in &roots {
        validate_owned_path(workspace, root)?;
    }
    for root in roots {
        match std::fs::remove_dir_all(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(parent) = root.parent() {
            if validate_owned_path(workspace, parent)? {
                std::fs::File::open(parent)?.sync_all()?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "task_owned_cleanup_tests.rs"]
mod tests;

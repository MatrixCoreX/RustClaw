//! Temporary skill storage for host-enrolled deliveries; ordinary storage is
//! unchanged. The descriptor still comes from the same storage resolver and
//! crosses the normal registry declaration, sandbox and permission checks.
use crate::skill_storage::{SkillStorageDescriptor, SkillStorageResolver};
use anyhow::ensure;
use serde_json::Value;

pub(crate) fn descriptor_for_task(
    state: &crate::AppState,
    task_id: &str,
    payload: &Value,
    descriptor: Option<SkillStorageDescriptor>,
) -> anyhow::Result<Option<SkillStorageDescriptor>> {
    if super::retains_shared_outcome(state, task_id, payload) {
        return Ok(descriptor);
    }
    let Some(descriptor) = descriptor else {
        return Ok(None);
    };
    ensure!(
        descriptor.storage_kind == "directory",
        "work_cleanup_storage_kind_unsupported"
    );
    let task = uuid::Uuid::parse_str(task_id)?;
    ensure!(task.to_string() == task_id, "work_cleanup_task_id_invalid");
    let root = claw_core::workspace_state::workspace_artifacts_root(&state.skill_rt.workspace_root)
        .join("task-storage")
        .join(task_id);
    super::files::validate_owned_path(&state.skill_rt.workspace_root, &root)?;
    super::files::validate_owned_path(
        &state.skill_rt.workspace_root,
        &root.join(&descriptor.skill_name),
    )?;
    let configured = root
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("work_cleanup_path_invalid"))?;
    let resolver = SkillStorageResolver::new(
        &state.skill_rt.workspace_root,
        configured,
        descriptor.database_busy_timeout_ms,
    )?;
    let scoped =
        resolver.directory_descriptor(&descriptor.skill_name, descriptor.schema_version)?;
    let scoped_path = scoped
        .directory_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("work_cleanup_path_invalid"))?;
    super::files::validate_owned_path(
        &state.skill_rt.workspace_root,
        std::path::Path::new(scoped_path),
    )?;
    Ok(Some(scoped))
}

#[cfg(test)]
#[path = "task_retention_storage_tests.rs"]
mod tests;

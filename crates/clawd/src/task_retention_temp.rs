//! Per-invocation temp environment for private host-enrolled deliveries only.
//! Values use the existing writable storage mapping; no new child permissions.
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use anyhow::ensure;
use serde_json::Value;

use crate::skill_storage::SkillStorageDescriptor;

pub(crate) const TEMP_VARIABLES: [&str; 3] = ["TMPDIR", "TMP", "TEMP"];
const TEMP_DIRECTORY: &str = "process-temp";

pub(crate) struct TaskTemporaryDirectory {
    workspace: PathBuf,
    host_directory: PathBuf,
}

pub(crate) fn prepare(
    state: &crate::AppState,
    task_id: &str,
    payload: &Value,
    descriptor: Option<&SkillStorageDescriptor>,
    environment_allowlist: &[String],
    fixed_environment: &BTreeMap<String, String>,
) -> anyhow::Result<Option<TaskTemporaryDirectory>> {
    if super::retains_shared_outcome(state, task_id, payload) {
        return Ok(None);
    }
    // Installed children clear inherited env. These receipt-bound fields must
    // actually forward all three names, without fixed package values winning.
    ensure!(
        TEMP_VARIABLES
            .iter()
            .all(|key| environment_allowlist.iter().any(|v| v == key)
                && !fixed_environment.contains_key(*key)),
        "work_cleanup_temp_environment_unavailable"
    );
    let descriptor =
        descriptor.ok_or_else(|| anyhow::anyhow!("work_cleanup_temp_storage_unavailable"))?;
    ensure!(
        descriptor.storage_kind == "directory",
        "work_cleanup_temp_storage_unavailable"
    );
    let task = uuid::Uuid::parse_str(task_id)?;
    ensure!(task.to_string() == task_id, "work_cleanup_task_id_invalid");
    let mut skill_components = Path::new(&descriptor.skill_name).components();
    ensure!(
        matches!(skill_components.next(), Some(Component::Normal(_)))
            && skill_components.next().is_none(),
        "work_cleanup_temp_storage_not_owned"
    );
    let storage = PathBuf::from(
        descriptor
            .directory_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("work_cleanup_temp_storage_unavailable"))?,
    );
    let expected =
        claw_core::workspace_state::workspace_artifacts_root(&state.skill_rt.workspace_root)
            .join("task-storage")
            .join(task_id)
            .join(&descriptor.skill_name);
    ensure!(storage == expected, "work_cleanup_temp_storage_not_owned");
    ensure!(
        super::files::validate_owned_path(&state.skill_rt.workspace_root, &storage)?,
        "work_cleanup_temp_storage_missing"
    );
    let temporary = storage.join(TEMP_DIRECTORY);
    super::files::validate_owned_path(&state.skill_rt.workspace_root, &temporary)?;
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&temporary) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    ensure!(
        super::files::validate_owned_path(&state.skill_rt.workspace_root, &temporary)?
            && temporary.is_dir(),
        "work_cleanup_temp_directory_invalid"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o700))?;
    }
    let plan = TaskTemporaryDirectory {
        workspace: state.skill_rt.workspace_root.clone(),
        host_directory: temporary,
    };
    plan.validate_writable()?;
    Ok(Some(plan))
}

impl TaskTemporaryDirectory {
    fn validate_writable(&self) -> anyhow::Result<()> {
        ensure!(
            super::files::validate_owned_path(&self.workspace, &self.host_directory)?
                && self.host_directory.is_dir(),
            "work_cleanup_temp_directory_invalid"
        );
        let probe = self
            .host_directory
            .join(format!(".host-write-probe-{}", uuid::Uuid::new_v4()));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        drop(file);
        std::fs::remove_file(probe)?;
        Ok(())
    }

    fn mapped_values(
        &self,
        sandbox_storage: Option<&Path>,
    ) -> anyhow::Result<BTreeMap<String, OsString>> {
        self.validate_writable()?;
        let mapped =
            sandbox_storage.ok_or_else(|| anyhow::anyhow!("work_cleanup_temp_mapping_missing"))?;
        ensure!(
            mapped.is_absolute()
                && mapped.parent().is_some()
                && mapped
                    .components()
                    .all(|c| matches!(c, Component::RootDir | Component::Normal(_))),
            "work_cleanup_temp_mapping_invalid"
        );
        let target = mapped.join(TEMP_DIRECTORY).into_os_string();
        Ok(TEMP_VARIABLES
            .into_iter()
            .map(|key| (key.to_string(), target.clone()))
            .collect())
    }
}

pub(crate) fn apply(
    command: &mut tokio::process::Command,
    plan: Option<&TaskTemporaryDirectory>,
    sandbox_storage: Option<&Path>,
) -> anyhow::Result<()> {
    if let Some(plan) = plan {
        command.envs(plan.mapped_values(sandbox_storage)?);
    }
    Ok(())
}

#[cfg(test)]
#[path = "task_retention_temp_tests.rs"]
mod tests;

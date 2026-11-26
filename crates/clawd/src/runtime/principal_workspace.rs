use crate::{AppState, ClaimedTask};
use anyhow::{ensure, Context};
use claw_core::channel_workspace::{private_directory as directory, WORKSPACES};
use std::path::{Path, PathBuf};

/// Recognize the host-selected data layout when reconstructing persisted worktree plans.
pub(crate) fn personal_base(path: &Path) -> Option<&Path> {
    path.ancestors().find(|ancestor| {
        ancestor.parent().is_some_and(|p| p.ends_with(WORKSPACES))
            && ancestor
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|name| {
                    name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
    })
}

impl AppState {
    pub(crate) fn granted_operator_config(&self, skill: &str) -> anyhow::Result<Option<PathBuf>> {
        if self.skill_rt.principal_workspace.is_none() {
            return Ok(None);
        }
        let Some(relative) = self.skill_rt.tools_policy.operator_skill_configs.get(skill) else {
            return Ok(None);
        };
        let relative = Path::new(relative);
        ensure!(
            relative.parent() == Some(Path::new("configs")) && relative.file_name().is_some(),
            "operator_config_grant_invalid"
        );
        let root = self.skill_rt.workspace_root.canonicalize()?;
        let path = root.join(relative);
        ensure!(
            path.canonicalize()? == path && path.is_file(),
            "operator_config_grant_invalid"
        );
        Ok(Some(path))
    }

    pub(crate) fn identity_files_root(
        &self,
        identity: &claw_core::types::AuthIdentity,
    ) -> anyhow::Result<PathBuf> {
        if identity.role.eq_ignore_ascii_case("admin") {
            return Ok(self.skill_rt.workspace_root.clone());
        }
        directory(&self.skill_rt.workspace_root, &identity.principal_id)
    }

    pub(crate) fn stored_task_files_root(&self, task_id: &str) -> anyhow::Result<PathBuf> {
        let db = self.core.db.get()?;
        let (key, owner): (Option<String>, Option<String>) = db.query_row(
            "SELECT user_key, principal_id FROM tasks WHERE task_id=?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let credential_present = key.is_some();
        if let Some(key) = key {
            let actor = match crate::repo::auth::resolve_auth_identity_on_connection(&db, &key) {
                Ok(actor) => actor,
                Err(error) => {
                    // Older record stores can have task owners without live credentials.
                    // They can use the isolated fallback, never the operator workspace.
                    let modern: bool = db.query_row(
                        "SELECT COUNT(*)=3 FROM sqlite_schema WHERE type='table' AND name IN ('auth_keys','credential_bindings','principals')",
                        [], |row| row.get(0),
                    )?;
                    if modern {
                        return Err(error);
                    }
                    None
                }
            };
            if let Some(actor) = actor {
                if owner
                    .as_ref()
                    .is_none_or(|owner| owner == &actor.principal_id)
                {
                    return self.identity_files_root(&actor);
                }
            }
        }
        directory(
            &self.skill_rt.workspace_root,
            &owner
                .filter(|_| credential_present)
                .unwrap_or_else(|| format!("legacy-task:{task_id}")),
        )
    }

    pub(crate) fn scope_task_workspace(&self, task: &ClaimedTask) -> anyhow::Result<Option<Self>> {
        let principal = if let Some(key) = task.user_key.as_deref() {
            let actor = crate::resolve_auth_identity_by_key(self, key)?
                .context("workspace_credential_inactive")?;
            if actor.role.eq_ignore_ascii_case("admin") {
                ensure!(
                    self.skill_rt.principal_workspace.is_none(),
                    "workspace_identity_changed"
                );
                return Ok(None);
            }
            actor.principal_id
        } else {
            format!("legacy-task:{}", task.task_id)
        };
        let root = directory(&self.skill_rt.workspace_root, &principal)?;
        if let Some(current) = &self.skill_rt.principal_workspace {
            ensure!(current.starts_with(&root), "workspace_identity_changed");
            return Ok(None);
        }
        let mut scoped = self.clone();
        scoped.skill_rt.host_filesystem = self.skill_rt.host_filesystem.scoped_to(&root)?.into();
        scoped.skill_rt.principal_workspace = Some(root.clone());
        scoped.skill_rt.default_locator_search_dir = root;
        Ok(Some(scoped))
    }
}

#[cfg(test)]
#[path = "principal_workspace_tests.rs"]
mod tests;

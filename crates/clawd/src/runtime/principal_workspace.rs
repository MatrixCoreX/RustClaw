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
            if let Some(actor) = crate::repo::auth::resolve_auth_identity_on_connection(&db, &key)?
            {
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
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn personal_skill_config_is_a_single_reviewed_file_and_rejects_links() {
        let tmp = tempfile::tempdir().unwrap();
        let mut host = AppState::test_default_with_fixture_provider();
        host.skill_rt.workspace_root = tmp.path().to_owned();
        host.skill_rt.principal_workspace = Some(tmp.path().join("own"));
        let configs = tmp.path().join("configs");
        std::fs::create_dir(&configs).unwrap();
        std::fs::write(configs.join("rss.toml"), "[rss]").unwrap();
        assert_eq!(
            host.granted_operator_config("rss_fetch").unwrap(),
            Some(configs.join("rss.toml").canonicalize().unwrap())
        );
        assert!(host.granted_operator_config("ungranted").unwrap().is_none());
        std::fs::remove_file(configs.join("rss.toml")).unwrap();
        std::fs::write(tmp.path().join("private"), "fixture").unwrap();
        symlink(tmp.path().join("private"), configs.join("rss.toml")).unwrap();
        assert!(host.granted_operator_config("rss_fetch").is_err());
    }

    #[test]
    fn authenticated_accounts_get_distinct_workspaces_with_stable_ownership() {
        let tmp = tempfile::tempdir().unwrap();
        let mut host = AppState::test_default_with_fixture_provider();
        host.skill_rt.workspace_root = tmp.path().to_owned();
        for (key, role) in [
            ("account-first", "user"),
            ("account-second", "guest"),
            ("account-admin", "admin"),
        ] {
            host.seed_test_auth_identity(key, role);
        }
        let task = |key: &str| ClaimedTask {
            claim_attempt: 1,
            task_id: "workspace-test".into(),
            user_id: 7,
            chat_id: 9,
            user_key: Some(key.into()),
            channel: "ui".into(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".into(),
            payload_json: "{}".into(),
        };
        let first = host
            .scope_task_workspace(&task("account-first"))
            .unwrap()
            .unwrap();
        let second = host
            .scope_task_workspace(&task("account-second"))
            .unwrap()
            .unwrap();
        let document = first.skill_rt.files_root().join("report.txt");
        std::fs::write(&document, "owned").unwrap();
        assert!(first
            .skill_rt
            .host_filesystem
            .open_file(Some(first.skill_rt.files_root()), &document)
            .is_ok());
        assert!(second
            .skill_rt
            .host_filesystem
            .open_file(None, &document)
            .is_err());
        assert!(first.scope_task_workspace(&task("account-second")).is_err());
        assert!(host
            .scope_task_workspace(&task("account-admin"))
            .unwrap()
            .is_none());
        assert!(host
            .scope_task_workspace(&task("account-no-longer-active"))
            .is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn command_sees_only_its_workspace_and_approved_runtime_files() {
        let tmp = tempfile::tempdir().unwrap();
        let first = directory(tmp.path(), "first").unwrap();
        let second = directory(tmp.path(), "second").unwrap();
        std::fs::write(second.join("report.txt"), "other").unwrap();
        let prepared = crate::process_sandbox::prepare_process_invocation(
            "/bin/sh",
            crate::process_sandbox::ProcessSandboxRequest {
                principal_readonly: Some(&[]),
                workspace_root: &first,
                execution_root: &first,
                mode: claw_core::config::ToolSandboxMode::DangerFull,
                backend: claw_core::config::ToolSandboxBackend::Auto,
                network: crate::process_sandbox::ProcessNetworkPolicy::Deny,
                additional_writable_paths: &[],
                host_private_directories: &[],
                browser_subprocess: false,
            },
        )
        .unwrap();
        let status = prepared
            .invocation
            .into_command()
            .args([
                "-c",
                "test ! -e \"$1\" && printf mine > report.txt",
                "check",
            ])
            .arg(second.join("report.txt"))
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(std::fs::read(first.join("report.txt")).unwrap(), b"mine");
        assert_eq!(std::fs::read(second.join("report.txt")).unwrap(), b"other");
    }

    #[test]
    fn workspace_is_stable_private_and_never_resolves_a_redirected_directory() {
        let root = tempfile::tempdir().unwrap();
        let first = directory(root.path(), "one").unwrap();
        let second = directory(root.path(), "../two").unwrap();
        assert_ne!(first, second);
        assert_eq!(first, directory(root.path(), "one").unwrap());
        assert_eq!(
            first.metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::write(first.join("a.txt"), b"one").unwrap();
        assert!(!second.join("a.txt").exists());
        std::fs::remove_dir(&second).unwrap();
        symlink(&first, &second).unwrap();
        assert!(directory(root.path(), "../two").is_err());
    }
}

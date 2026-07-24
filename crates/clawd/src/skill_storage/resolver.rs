use serde::Serialize;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct SkillStorageResolver {
    root: PathBuf,
    busy_timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SkillStorageDescriptor {
    pub(crate) schema_version: u32,
    pub(crate) skill_name: String,
    pub(crate) storage_kind: &'static str,
    pub(crate) database_path: String,
    pub(crate) database_busy_timeout_ms: u64,
}

impl SkillStorageResolver {
    pub(crate) fn new(
        workspace_root: &Path,
        configured_root: &str,
        busy_timeout_ms: u64,
    ) -> anyhow::Result<Self> {
        let configured = configured_root.trim();
        let configured = if configured.is_empty() {
            Path::new("data/skills")
        } else {
            Path::new(configured)
        };
        if configured.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        }) && !configured.is_absolute()
        {
            anyhow::bail!("skill_data_root contains an unsafe path component");
        }
        let root = if configured.is_absolute() {
            configured.to_path_buf()
        } else {
            workspace_root.join(configured)
        };
        std::fs::create_dir_all(&root)?;
        apply_private_directory_permissions(&root)?;
        Ok(Self {
            root,
            busy_timeout_ms: busy_timeout_ms.max(1),
        })
    }

    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rustclaw-skill-storage-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).expect("create test skill storage root");
        Self {
            root,
            busy_timeout_ms: 5_000,
        }
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn database_path(&self, skill_name: &str) -> anyhow::Result<PathBuf> {
        let path = self.resolved_database_path(skill_name)?;
        let directory = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("skill storage path has no parent"))?;
        std::fs::create_dir_all(&directory)?;
        apply_private_directory_permissions(&directory)?;
        Ok(path)
    }

    pub(crate) fn resolved_database_path(&self, skill_name: &str) -> anyhow::Result<PathBuf> {
        validate_skill_name(skill_name)?;
        Ok(self.root.join(skill_name).join("state.db"))
    }

    pub(crate) fn descriptor(&self, skill_name: &str) -> anyhow::Result<SkillStorageDescriptor> {
        let path = self.database_path(skill_name)?;
        Ok(SkillStorageDescriptor {
            schema_version: 1,
            skill_name: skill_name.to_string(),
            storage_kind: "sqlite",
            database_path: path.display().to_string(),
            database_busy_timeout_ms: self.busy_timeout_ms,
        })
    }
}

fn validate_skill_name(skill_name: &str) -> anyhow::Result<()> {
    if skill_name.is_empty()
        || skill_name.len() > 96
        || !skill_name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
        || skill_name.starts_with('.')
        || skill_name.ends_with('.')
        || skill_name.contains("..")
    {
        anyhow::bail!("invalid canonical skill storage name");
    }
    Ok(())
}

#[cfg(unix)]
fn apply_private_directory_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn apply_private_directory_permissions(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("skill-owned storage is unsupported on this platform")
}

#[cfg(test)]
#[path = "resolver_tests.rs"]
mod tests;

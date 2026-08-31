use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::{SkillSdkError, SkillSdkResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedPathKind {
    Any,
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathAuthority {
    outside_workspace_granted: bool,
}

impl PathAuthority {
    pub fn from_runner_context(context: Option<&Value>) -> Self {
        let Some(context) = context.and_then(Value::as_object) else {
            return Self::default();
        };
        let permissions = context.get("permissions").and_then(Value::as_object);
        let outside_workspace_granted = context.get("authority_scope").and_then(Value::as_str)
            == Some("host_policy_grant")
            && permissions
                .and_then(|value| value.get("allow_path_outside_workspace"))
                .and_then(Value::as_bool)
                == Some(true);
        Self {
            outside_workspace_granted,
        }
    }

    pub fn outside_workspace_granted(self) -> bool {
        self.outside_workspace_granted
    }
}

#[derive(Debug, Clone)]
pub struct SkillPathPolicy {
    workspace_root: PathBuf,
    authority: PathAuthority,
}

impl SkillPathPolicy {
    pub fn new(workspace_root: &Path, context: Option<&Value>) -> SkillSdkResult<Self> {
        let workspace_root = workspace_root.canonicalize().map_err(|error| {
            SkillSdkError::new("workspace_root_unavailable", error.to_string()).phase("path_policy")
        })?;
        if !workspace_root.is_dir() {
            return Err(SkillSdkError::new(
                "workspace_root_invalid",
                format!("path={} is not a directory", workspace_root.display()),
            )
            .phase("path_policy"));
        }
        Ok(Self {
            workspace_root,
            authority: PathAuthority::from_runner_context(context),
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn authority(&self) -> PathAuthority {
        self.authority
    }

    pub fn resolve_existing(
        &self,
        input: &str,
        expected: ExpectedPathKind,
    ) -> SkillSdkResult<PathBuf> {
        let candidate = self.lexical_candidate(input)?;
        let resolved = candidate.canonicalize().map_err(|error| {
            SkillSdkError::new(
                "path_not_found",
                format!("path={} error={error}", candidate.display()),
            )
            .phase("path_policy")
        })?;
        self.require_allowed(&resolved)?;
        match expected {
            ExpectedPathKind::Any if !resolved.is_file() && !resolved.is_dir() => {
                return Err(self.kind_error(&resolved, "regular_file_or_directory"));
            }
            ExpectedPathKind::Any => {}
            ExpectedPathKind::File if !resolved.is_file() => {
                return Err(self.kind_error(&resolved, "file"));
            }
            ExpectedPathKind::Directory if !resolved.is_dir() => {
                return Err(self.kind_error(&resolved, "directory"));
            }
            ExpectedPathKind::File | ExpectedPathKind::Directory => {}
        }
        Ok(resolved)
    }

    pub fn resolve_create_target(&self, input: &str) -> SkillSdkResult<PathBuf> {
        let candidate = self.lexical_candidate(input)?;
        if candidate
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(SkillSdkError::new(
                "path_target_symlink_forbidden",
                format!("path={}", candidate.display()),
            )
            .phase("path_policy"));
        }

        let mut existing_ancestor = candidate.as_path();
        while !existing_ancestor.exists() {
            existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
                SkillSdkError::new(
                    "path_parent_unavailable",
                    format!("path={}", candidate.display()),
                )
                .phase("path_policy")
            })?;
        }
        let canonical_ancestor = existing_ancestor.canonicalize().map_err(|error| {
            SkillSdkError::new("path_parent_unavailable", error.to_string()).phase("path_policy")
        })?;
        self.require_allowed(&canonical_ancestor)?;
        let suffix = candidate.strip_prefix(existing_ancestor).map_err(|_| {
            SkillSdkError::new("path_invalid", format!("path={}", candidate.display()))
                .phase("path_policy")
        })?;
        let resolved = if suffix.as_os_str().is_empty() {
            canonical_ancestor
        } else {
            canonical_ancestor.join(suffix)
        };
        self.require_allowed(&resolved)?;
        if resolved.exists() && !resolved.is_file() && !resolved.is_dir() {
            return Err(self.kind_error(&resolved, "regular_file_or_directory"));
        }
        Ok(resolved)
    }

    fn lexical_candidate(&self, input: &str) -> SkillSdkResult<PathBuf> {
        let input = input.trim();
        if input.is_empty() || input.len() > 4096 {
            return Err(
                SkillSdkError::new("path_invalid", "path is empty or oversized")
                    .phase("path_policy"),
            );
        }
        let path = Path::new(input);
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(SkillSdkError::new(
                "path_traversal_forbidden",
                "path must not contain parent traversal",
            )
            .phase("path_policy"));
        }
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        })
    }

    fn require_allowed(&self, path: &Path) -> SkillSdkResult<()> {
        if self.authority.outside_workspace_granted() || path.starts_with(&self.workspace_root) {
            return Ok(());
        }
        Err(SkillSdkError::new(
            "path_outside_workspace",
            format!(
                "path={} workspace_root={}",
                path.display(),
                self.workspace_root.display()
            ),
        )
        .phase("path_policy"))
    }

    fn kind_error(&self, path: &Path, expected: &str) -> SkillSdkError {
        SkillSdkError::new(
            "path_kind_mismatch",
            format!("path={} expected={expected}", path.display()),
        )
        .phase("path_policy")
    }
}

#[cfg(test)]
#[path = "path_policy_tests.rs"]
mod tests;

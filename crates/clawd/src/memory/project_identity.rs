use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ResolvedProjectIdentity {
    pub(crate) project_ref: String,
    pub(crate) locator_kind: &'static str,
}

pub(crate) fn resolve_project_identity(
    db: &Connection,
    workspace: &Path,
) -> anyhow::Result<ResolvedProjectIdentity> {
    super::super::repo::auth::ensure_principal_identity_schema(db)?;
    let (locator_kind, canonical_locator) = canonical_project_locator(workspace)?;
    let locator_digest = project_locator_digest(locator_kind, &canonical_locator);
    if let Some(project_ref) = project_ref_for_digest(db, &locator_digest)? {
        return Ok(ResolvedProjectIdentity {
            project_ref,
            locator_kind,
        });
    }
    let project_ref = format!("project_{}", uuid::Uuid::new_v4().simple());
    let now = crate::now_ts();
    db.execute(
        "INSERT INTO memory_project_identities (
            project_ref, locator_digest, canonical_locator, status, revision, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'active', 1, ?4, ?4)
         ON CONFLICT(locator_digest) DO UPDATE SET
            status = 'active',
            updated_at = excluded.updated_at",
        params![project_ref, locator_digest, canonical_locator, now],
    )?;
    let resolved = project_ref_for_digest(db, &locator_digest)?
        .ok_or_else(|| anyhow::anyhow!("memory_project_identity_create_failed"))?;
    Ok(ResolvedProjectIdentity {
        project_ref: resolved,
        locator_kind,
    })
}

pub(crate) fn link_project_path_alias(
    db: &Connection,
    project_ref: &str,
    workspace: &Path,
) -> anyhow::Result<()> {
    let project_exists = db
        .query_row(
            "SELECT 1 FROM memory_project_identities
             WHERE project_ref = ?1 AND status = 'active'",
            [project_ref],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    anyhow::ensure!(project_exists, "memory_project_identity_not_found");
    let (locator_kind, canonical_alias) = canonical_project_locator(workspace)?;
    let alias_digest = project_locator_digest(locator_kind, &canonical_alias);
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "UPDATE memory_project_identities
         SET status = 'unlinked', revision = revision + 1, updated_at = ?2
         WHERE locator_digest = ?1 AND project_ref != ?3 AND status = 'active'",
        params![alias_digest, crate::now_ts(), project_ref],
    )?;
    tx.execute(
        "INSERT INTO memory_project_aliases (
            alias_digest, project_ref, canonical_alias, created_at
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(alias_digest) DO UPDATE SET
            project_ref = excluded.project_ref,
            canonical_alias = excluded.canonical_alias",
        params![alias_digest, project_ref, canonical_alias, crate::now_ts()],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn unlink_project_path_alias(
    db: &Connection,
    project_ref: &str,
    workspace: &Path,
) -> anyhow::Result<bool> {
    let (locator_kind, canonical_alias) = canonical_project_locator(workspace)?;
    let alias_digest = project_locator_digest(locator_kind, &canonical_alias);
    Ok(db.execute(
        "DELETE FROM memory_project_aliases
         WHERE alias_digest = ?1 AND project_ref = ?2",
        params![alias_digest, project_ref],
    )? > 0)
}

fn project_ref_for_digest(db: &Connection, digest: &str) -> anyhow::Result<Option<String>> {
    db.query_row(
        "SELECT project_ref FROM (
            SELECT project_ref, 0 AS precedence
            FROM memory_project_aliases
            WHERE alias_digest = ?1
            UNION ALL
            SELECT project_ref, 1 AS precedence
            FROM memory_project_identities
            WHERE locator_digest = ?1 AND status = 'active'
         ) ORDER BY precedence ASC LIMIT 1",
        [digest],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn canonical_project_locator(workspace: &Path) -> anyhow::Result<(&'static str, String)> {
    let canonical_workspace = workspace.canonicalize().map_err(|error| {
        anyhow::anyhow!(
            "memory_project_workspace_unavailable:{}:{error}",
            workspace.display()
        )
    })?;
    if let Some(common_dir) = git_common_dir(&canonical_workspace)? {
        return Ok(("git_common_dir", common_dir.to_string_lossy().into_owned()));
    }
    Ok((
        "canonical_path",
        canonical_workspace.to_string_lossy().into_owned(),
    ))
}

fn git_common_dir(workspace: &Path) -> anyhow::Result<Option<PathBuf>> {
    for candidate in workspace.ancestors() {
        let marker = candidate.join(".git");
        if marker.is_dir() {
            return marker.canonicalize().map(Some).map_err(Into::into);
        }
        if !marker.is_file() {
            continue;
        }
        let marker_text = std::fs::read_to_string(&marker)?;
        let git_dir_raw = marker_text
            .trim()
            .strip_prefix("gitdir:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("memory_project_gitdir_marker_invalid"))?;
        let git_dir = if Path::new(git_dir_raw).is_absolute() {
            PathBuf::from(git_dir_raw)
        } else {
            candidate.join(git_dir_raw)
        }
        .canonicalize()?;
        let common_dir_marker = git_dir.join("commondir");
        if !common_dir_marker.is_file() {
            return Ok(Some(git_dir));
        }
        let common_dir_raw = std::fs::read_to_string(common_dir_marker)?;
        let common_dir_raw = common_dir_raw.trim();
        anyhow::ensure!(
            !common_dir_raw.is_empty(),
            "memory_project_commondir_marker_invalid"
        );
        let common_dir = if Path::new(common_dir_raw).is_absolute() {
            PathBuf::from(common_dir_raw)
        } else {
            git_dir.join(common_dir_raw)
        };
        return common_dir.canonicalize().map(Some).map_err(Into::into);
    }
    Ok(None)
}

fn project_locator_digest(kind: &str, locator: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"memory-project-locator-v1\0");
    digest.update(kind.as_bytes());
    digest.update(b"\0");
    digest.update(locator.as_bytes());
    format!("sha256:{:x}", digest.finalize())
}

#[cfg(test)]
#[path = "project_identity_tests.rs"]
mod tests;

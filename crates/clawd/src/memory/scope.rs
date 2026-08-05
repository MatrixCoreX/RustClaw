use std::path::Path;

use claw_core::types::AuthIdentity;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

const MIGRATION_ID: &str = "010_memory_scope_contract_v1";
const MIGRATION_MANIFEST: &str =
    include_str!("../../../../migrations/010_memory_scope_contract.sql");

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryScopeKind {
    Conversation,
    Principal,
    Project,
}

impl MemoryScopeKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Principal => "principal",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ResolvedMemoryScope {
    pub(crate) kind: MemoryScopeKind,
    pub(crate) scope_ref: String,
    pub(crate) principal_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ResolvedMemoryAccess {
    pub(crate) principal_id: String,
    pub(crate) principal_scope_ref: String,
    pub(crate) conversation_scope_ref: Option<String>,
    pub(crate) project_scope_ref: Option<String>,
}

impl ResolvedMemoryAccess {
    pub(crate) fn allows(&self, scope_kind: &str, scope_ref: &str) -> bool {
        match scope_kind {
            "principal" => scope_ref == self.principal_scope_ref,
            "conversation" => self.conversation_scope_ref.as_deref() == Some(scope_ref),
            "project" => self.project_scope_ref.as_deref() == Some(scope_ref),
            _ => false,
        }
    }
}

pub(crate) fn resolve_principal_scope(identity: &AuthIdentity) -> ResolvedMemoryScope {
    ResolvedMemoryScope {
        kind: MemoryScopeKind::Principal,
        scope_ref: identity.principal_id.clone(),
        principal_id: identity.principal_id.clone(),
    }
}

pub(crate) fn resolve_conversation_scope(
    identity: &AuthIdentity,
    conversation_id: &str,
) -> anyhow::Result<ResolvedMemoryScope> {
    let conversation_id = conversation_id.trim();
    anyhow::ensure!(
        !conversation_id.is_empty() && conversation_id.len() <= 256,
        "memory_scope_conversation_invalid"
    );
    let scope_ref = conversation_scope_ref(&identity.principal_id, conversation_id)?;
    Ok(ResolvedMemoryScope {
        kind: MemoryScopeKind::Conversation,
        scope_ref,
        principal_id: identity.principal_id.clone(),
    })
}

pub(crate) fn conversation_scope_ref(
    principal_id: &str,
    conversation_id: &str,
) -> anyhow::Result<String> {
    let principal_id = principal_id.trim();
    let conversation_id = conversation_id.trim();
    anyhow::ensure!(
        !principal_id.is_empty()
            && !conversation_id.is_empty()
            && principal_id.len() <= 256
            && conversation_id.len() <= 256,
        "memory_scope_conversation_invalid"
    );
    let mut digest = Sha256::new();
    digest.update(b"memory-conversation-scope-v1\0");
    digest.update(principal_id.as_bytes());
    digest.update(b"\0");
    digest.update(conversation_id.as_bytes());
    Ok(format!("conversation_{:x}", digest.finalize()))
}

pub(crate) fn resolve_memory_access(
    db: &Connection,
    principal_id: &str,
    conversation_id: Option<&str>,
    workspace: Option<&Path>,
) -> anyhow::Result<ResolvedMemoryAccess> {
    let principal_id = principal_id.trim();
    anyhow::ensure!(!principal_id.is_empty(), "memory_scope_principal_invalid");
    let conversation_scope_ref = conversation_id
        .map(|value| conversation_scope_ref(principal_id, value))
        .transpose()?;
    let project_scope_ref = workspace
        .map(|path| super::project_identity::resolve_project_identity(db, path))
        .transpose()?
        .map(|identity| identity.project_ref);
    Ok(ResolvedMemoryAccess {
        principal_id: principal_id.to_string(),
        principal_scope_ref: principal_id.to_string(),
        conversation_scope_ref,
        project_scope_ref,
    })
}

pub(crate) fn resolve_project_scope(
    db: &Connection,
    identity: &AuthIdentity,
    workspace: &Path,
) -> anyhow::Result<ResolvedMemoryScope> {
    let project = super::project_identity::resolve_project_identity(db, workspace)?;
    Ok(ResolvedMemoryScope {
        kind: MemoryScopeKind::Project,
        scope_ref: project.project_ref,
        principal_id: identity.principal_id.clone(),
    })
}

pub(crate) fn ensure_memory_scope_schema(db: &Connection) -> anyhow::Result<()> {
    crate::repo::ensure_principal_ownership_schema(db)?;
    if let Some(applied_digest) = migration_digest(db)? {
        anyhow::ensure!(
            applied_digest == manifest_digest(),
            "runtime_schema_migration_digest_mismatch:{MIGRATION_ID}"
        );
    }
    if db.is_autocommit() {
        let tx = db.unchecked_transaction()?;
        apply_migration(&tx)?;
        tx.commit()?;
    } else {
        apply_migration(db)?;
    }
    Ok(())
}

fn apply_migration(db: &Connection) -> anyhow::Result<()> {
    for table in [
        "memories",
        "long_term_memories",
        "user_preferences",
        "memory_facts",
    ] {
        if !table_exists(db, table)? {
            continue;
        }
        add_column(db, table, "memory_id", "TEXT")?;
        add_column(db, table, "scope_kind", "TEXT NOT NULL DEFAULT 'principal'")?;
        add_column(db, table, "scope_ref", "TEXT")?;
        add_column(
            db,
            table,
            "origin",
            "TEXT NOT NULL DEFAULT 'imported_legacy'",
        )?;
        add_column(db, table, "row_revision", "INTEGER NOT NULL DEFAULT 1")?;
        add_column(
            db,
            table,
            "legacy_scope_inferred",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        backfill_opaque_ids(db, table)?;
        backfill_scope(db, table)?;
        db.execute_batch(&format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_{table}_memory_id
             ON {table}(memory_id) WHERE memory_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_{table}_principal_scope
             ON {table}(principal_id, scope_kind, scope_ref)"
        ))?;
    }
    if table_exists(db, "memory_retrieval_index")? {
        add_column(
            db,
            "memory_retrieval_index",
            "scope_kind",
            "TEXT NOT NULL DEFAULT 'principal'",
        )?;
        add_column(db, "memory_retrieval_index", "scope_ref", "TEXT")?;
        backfill_scope(db, "memory_retrieval_index")?;
        db.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_memory_retrieval_principal_scope
             ON memory_retrieval_index(principal_id, scope_kind, scope_ref, updated_at_ts DESC);",
        )?;
    }
    db.execute(
        "INSERT INTO runtime_schema_migrations (migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3) ON CONFLICT(migration_id) DO NOTHING",
        params![MIGRATION_ID, manifest_digest(), crate::now_ts()],
    )?;
    Ok(())
}

fn backfill_opaque_ids(db: &Connection, table: &str) -> anyhow::Result<()> {
    let ids = {
        let mut stmt = db.prepare(&format!(
            "SELECT id FROM {table} WHERE memory_id IS NULL OR TRIM(memory_id) = '' ORDER BY id"
        ))?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for id in ids {
        db.execute(
            &format!("UPDATE {table} SET memory_id = ?2 WHERE id = ?1"),
            params![id, format!("memory_{}", uuid::Uuid::new_v4().simple())],
        )?;
    }
    Ok(())
}

fn backfill_scope(db: &Connection, table: &str) -> anyhow::Result<()> {
    if column_exists(db, table, "principal_id")? {
        db.execute(
            &format!(
                "UPDATE {table}
                 SET scope_kind = 'principal', scope_ref = principal_id
                 WHERE principal_id IS NOT NULL
                   AND (scope_ref IS NULL OR TRIM(scope_ref) = '' OR scope_kind = 'user')"
            ),
            [],
        )?;
    }
    Ok(())
}

fn add_column(db: &Connection, table: &str, column: &str, definition: &str) -> anyhow::Result<()> {
    if !column_exists(db, table, column)? {
        db.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

fn table_exists(db: &Connection, table: &str) -> anyhow::Result<bool> {
    db.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |_| Ok(true),
    )
    .optional()
    .map(|value| value.unwrap_or(false))
    .map_err(Into::into)
}

fn column_exists(db: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for current in columns {
        if current?.eq_ignore_ascii_case(column) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn table_has_scope_contract(db: &Connection, table: &str) -> anyhow::Result<bool> {
    Ok(column_exists(db, table, "principal_id")?
        && column_exists(db, table, "scope_kind")?
        && column_exists(db, table, "scope_ref")?)
}

fn migration_digest(db: &Connection) -> anyhow::Result<Option<String>> {
    db.query_row(
        "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
        [MIGRATION_ID],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn manifest_digest() -> String {
    format!("sha256:{:x}", Sha256::digest(MIGRATION_MANIFEST.as_bytes()))
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;

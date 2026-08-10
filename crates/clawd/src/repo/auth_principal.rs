use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const PRINCIPAL_MIGRATION_ID: &str = "008_memory_context_identity_settings_v1";
const PRINCIPAL_SCHEMA_SQL: &str =
    include_str!("../../../../migrations/008_memory_context_identity_settings.sql");

pub(crate) fn credential_digest(user_key: &str) -> String {
    let normalized = super::normalize_user_key(user_key);
    let mut digest = Sha256::new();
    digest.update(b"runtime-credential-binding-v1\0");
    digest.update(normalized.as_bytes());
    format!("sha256:{:x}", digest.finalize())
}

fn schema_digest() -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(PRINCIPAL_SCHEMA_SQL.as_bytes())
    )
}

pub(crate) fn ensure_principal_identity_schema(db: &Connection) -> anyhow::Result<()> {
    let applied_digest = migration_digest(db, PRINCIPAL_MIGRATION_ID)?;
    if let Some(applied_digest) = applied_digest.as_ref() {
        anyhow::ensure!(
            applied_digest == &schema_digest(),
            "runtime_schema_migration_digest_mismatch:{PRINCIPAL_MIGRATION_ID}"
        );
    }
    db.execute_batch(PRINCIPAL_SCHEMA_SQL)?;
    crate::ensure_column_exists(
        db,
        "auth_keys",
        "principal_id",
        "ALTER TABLE auth_keys ADD COLUMN principal_id TEXT",
    )?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_auth_keys_principal_id
         ON auth_keys(principal_id) WHERE principal_id IS NOT NULL;",
    )?;
    backfill_principals(db)?;
    if applied_digest.is_none() {
        seed_memory_onboarding_state(db)?;
    }
    db.execute(
        "INSERT INTO runtime_schema_migrations (migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(migration_id) DO NOTHING",
        params![PRINCIPAL_MIGRATION_ID, schema_digest(), crate::now_ts()],
    )?;
    Ok(())
}

fn seed_memory_onboarding_state(db: &Connection) -> anyhow::Result<()> {
    let auth_key_count: i64 =
        db.query_row("SELECT COUNT(*) FROM auth_keys", [], |row| row.get(0))?;
    let has_existing_memory = [
        "memories",
        "long_term_memories",
        "user_preferences",
        "memory_facts",
    ]
    .into_iter()
    .try_fold(false, |found, table| -> anyhow::Result<bool> {
        if found {
            return Ok(true);
        }
        let exists: bool = db
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !exists {
            return Ok(false);
        }
        let count: i64 = db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })?;
        Ok(count > 0)
    })?;
    let is_upgrade = auth_key_count > 0 || has_existing_memory;
    let now = crate::now_ts();
    db.execute(
        "INSERT INTO memory_onboarding_state (
            singleton_id, installation_class, status, created_at, updated_at
         ) VALUES (1, ?1, ?2, ?3, ?3)
         ON CONFLICT(singleton_id) DO NOTHING",
        params![
            if is_upgrade { "upgrade" } else { "new_install" },
            if is_upgrade {
                "upgrade_preserved"
            } else {
                "pending_choice"
            },
            now
        ],
    )?;
    if !is_upgrade {
        db.execute(
            "INSERT INTO memory_runtime_settings (
                setting_key, setting_scope, use_mode, generate_mode,
                external_context_policy, managed_deny_use, managed_deny_generate,
                revision, policy_digest, updated_at, updated_by_principal_id
             ) VALUES (
                'admin:default', 'admin', 'disabled', 'disabled', 'inherit', 0, 0,
                1, 'onboarding-pending-v1', ?1, NULL
             ) ON CONFLICT(setting_key) DO NOTHING",
            [now],
        )?;
    }
    Ok(())
}

fn migration_digest(db: &Connection, migration_id: &str) -> anyhow::Result<Option<String>> {
    let table_exists: bool = db
        .query_row(
            "SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'runtime_schema_migrations'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !table_exists {
        return Ok(None);
    }
    db.query_row(
        "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
        [migration_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn backfill_principals(db: &Connection) -> anyhow::Result<()> {
    let rows = {
        let mut stmt = db.prepare(
            "SELECT user_key, role, principal_id
             FROM auth_keys
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let mapped = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };
    for (user_key, role, existing_principal_id) in rows {
        let principal_id = existing_principal_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("principal_{}", uuid::Uuid::new_v4().simple()));
        ensure_principal_row(db, &principal_id, &role)?;
        db.execute(
            "UPDATE auth_keys SET principal_id = ?2 WHERE user_key = ?1",
            params![user_key, principal_id],
        )?;
        ensure_active_credential_binding(db, &user_key, &principal_id)?;
    }
    Ok(())
}

pub(crate) fn create_principal_for_auth_key(
    db: &Connection,
    user_key: &str,
    role: &str,
) -> anyhow::Result<String> {
    ensure_principal_identity_schema(db)?;
    if let Some(existing) = principal_id_for_user_key(db, user_key)? {
        return Ok(existing);
    }
    let principal_id = format!("principal_{}", uuid::Uuid::new_v4().simple());
    ensure_principal_row(db, &principal_id, role)?;
    db.execute(
        "UPDATE auth_keys SET principal_id = ?2 WHERE user_key = ?1",
        params![user_key, principal_id],
    )?;
    ensure_active_credential_binding(db, user_key, &principal_id)?;
    Ok(principal_id)
}

fn ensure_principal_row(db: &Connection, principal_id: &str, role: &str) -> anyhow::Result<()> {
    let now = crate::now_ts();
    db.execute(
        "INSERT INTO principals (
            principal_id, role, status, revision, created_at, updated_at
         ) VALUES (?1, ?2, 'active', 1, ?3, ?3)
         ON CONFLICT(principal_id) DO UPDATE SET
            role = excluded.role,
            updated_at = excluded.updated_at",
        params![principal_id, role, now],
    )?;
    Ok(())
}

fn ensure_active_credential_binding(
    db: &Connection,
    user_key: &str,
    principal_id: &str,
) -> anyhow::Result<()> {
    let digest = credential_digest(user_key);
    let binding_id = format!("binding_{}", uuid::Uuid::new_v4().simple());
    db.execute(
        "INSERT INTO credential_bindings (
            binding_id, credential_digest, principal_id, status, created_at, revoked_at
         ) VALUES (?1, ?2, ?3, 'active', ?4, NULL)
         ON CONFLICT(credential_digest) DO UPDATE SET
            principal_id = excluded.principal_id,
            status = 'active',
            revoked_at = NULL",
        params![binding_id, digest, principal_id, crate::now_ts()],
    )?;
    Ok(())
}

pub(crate) fn principal_id_for_user_key(
    db: &Connection,
    user_key: &str,
) -> anyhow::Result<Option<String>> {
    let bindings_exist: bool = db
        .query_row(
            "SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'credential_bindings'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !bindings_exist {
        return Ok(None);
    }
    let digest = credential_digest(user_key);
    db.query_row(
        "SELECT p.principal_id
         FROM credential_bindings b
         JOIN principals p ON p.principal_id = b.principal_id
         WHERE b.credential_digest = ?1
           AND b.status = 'active'
           AND p.status = 'active'
         LIMIT 1",
        [digest],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub(crate) fn rotate_credential_binding(
    db: &Connection,
    principal_id: &str,
    old_user_key: &str,
    new_user_key: &str,
) -> anyhow::Result<()> {
    db.execute(
        "UPDATE credential_bindings
         SET status = 'revoked', revoked_at = ?2
         WHERE credential_digest = ?1 AND principal_id = ?3",
        params![
            credential_digest(old_user_key),
            crate::now_ts(),
            principal_id
        ],
    )?;
    ensure_active_credential_binding(db, new_user_key, principal_id)
}

#[cfg(test)]
#[path = "auth_principal_tests.rs"]
mod tests;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

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
    with_identity_transaction(db, ensure_identity_schema_in_transaction)
}

// An existing transaction remains owned by its caller, which must propagate an
// error and roll it back. Standalone calls commit only after the entire action.
pub(crate) fn with_identity_transaction<T>(
    db: &Connection,
    action: impl FnOnce(&Connection) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction()?;
        let result = action(&tx)?;
        tx.commit()?;
        Ok(result)
    } else {
        action(db)
    }
}

fn ensure_identity_schema_in_transaction(db: &Connection) -> anyhow::Result<()> {
    let applied_digest = migration_digest(db, PRINCIPAL_MIGRATION_ID)?;
    if let Some(applied_digest) = applied_digest.as_ref() {
        anyhow::ensure!(
            applied_digest == &schema_digest(),
            "runtime_schema_migration_digest_mismatch:{PRINCIPAL_MIGRATION_ID}"
        );
    }
    if applied_digest.is_none() {
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
    }
    backfill_principals(db, applied_digest.is_none())?;
    if applied_digest.is_none() {
        seed_memory_onboarding_state(db)?;
        db.execute(
            "INSERT INTO runtime_schema_migrations (migration_id, schema_digest, applied_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(migration_id) DO NOTHING",
            params![PRINCIPAL_MIGRATION_ID, schema_digest(), crate::now_ts()],
        )?;
    }
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

struct PrincipalBackfill {
    user_key: String,
    role: String,
    previous_principal_id: Option<String>,
    principal_id: String,
    credential_digest: String,
    create_principal: bool,
    create_binding: bool,
    assign_owner: bool,
}

fn backfill_principals(db: &Connection, first_migration: bool) -> anyhow::Result<()> {
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
    let mut plans = Vec::new();
    let mut seen_credentials = HashSet::new();
    let mut new_principals = HashSet::new();
    // Validate the complete batch before writing: an import must not repair
    // another credential by reviving it, moving its owner or replacing state.
    for (user_key, role, previous_principal_id) in rows {
        let digest = credential_digest(&user_key);
        anyhow::ensure!(
            seen_credentials.insert(digest.clone()),
            "identity_credential_collision"
        );
        let binding_owner = credential_binding_owner(db, &digest)?;
        let existing_owner = previous_principal_id
            .as_ref()
            .filter(|value| !value.trim().is_empty());
        if let (Some(owner), Some(bound)) = (existing_owner, binding_owner.as_ref()) {
            anyhow::ensure!(owner == bound, "credential_binding_owner_conflict");
        }
        let known_owner = existing_owner.is_some() || binding_owner.is_some();
        let principal_id = existing_owner
            .cloned()
            .or_else(|| binding_owner.clone())
            .unwrap_or_else(|| format!("principal_{}", uuid::Uuid::new_v4().simple()));
        let exists = db
            .query_row(
                "SELECT 1 FROM principals WHERE principal_id = ?1",
                [&principal_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        anyhow::ensure!(
            exists || !known_owner || first_migration,
            "principal_identity_missing"
        );
        let create_principal = !exists && new_principals.insert(principal_id.clone());
        anyhow::ensure!(
            known_owner || (!exists && create_principal),
            "principal_identity_collision"
        );
        let create_binding = binding_owner.is_none();
        anyhow::ensure!(
            !create_binding || existing_owner.is_none() || first_migration,
            "credential_binding_missing"
        );
        let assign_owner = existing_owner.is_none();
        if create_principal || create_binding || assign_owner {
            plans.push(PrincipalBackfill {
                user_key,
                role,
                previous_principal_id,
                principal_id,
                credential_digest: digest,
                create_principal,
                create_binding,
                assign_owner,
            });
        }
    }
    for plan in plans {
        if plan.create_principal {
            ensure_principal_row(db, &plan.principal_id, &plan.role)?;
        }
        if plan.create_binding {
            db.execute(
                "INSERT INTO credential_bindings
                 (binding_id, credential_digest, principal_id, status, created_at, revoked_at)
                 VALUES (?1, ?2, ?3, 'active', ?4, NULL)",
                params![
                    format!("binding_{}", uuid::Uuid::new_v4().simple()),
                    plan.credential_digest,
                    plan.principal_id,
                    crate::now_ts()
                ],
            )?;
        }
        if plan.assign_owner {
            let changed = db.execute(
                "UPDATE auth_keys SET principal_id = ?2
                 WHERE user_key = ?1 AND principal_id IS ?3",
                params![plan.user_key, plan.principal_id, plan.previous_principal_id],
            )?;
            anyhow::ensure!(changed == 1, "principal_identity_owner_changed");
        }
    }
    Ok(())
}

pub(crate) fn create_principal_for_auth_key(
    db: &Connection,
    user_key: &str,
    role: &str,
) -> anyhow::Result<String> {
    with_identity_transaction(db, |db| {
        let stored_role = db
            .query_row(
                "SELECT role FROM auth_keys WHERE user_key = ?1",
                [user_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("auth_key_missing"))?;
        anyhow::ensure!(
            super::normalize_auth_key_role(&stored_role)? == super::normalize_auth_key_role(role)?,
            "auth_key_role_conflict"
        );
        ensure_principal_identity_schema(db)?;
        active_auth_key_identity(db, user_key)?
            .map(|(_, principal)| principal)
            .ok_or_else(|| anyhow::anyhow!("auth_identity_inactive"))
    })
}

fn ensure_principal_row(db: &Connection, principal_id: &str, role: &str) -> anyhow::Result<()> {
    let now = crate::now_ts();
    db.execute(
        "INSERT INTO principals (
            principal_id, role, status, revision, created_at, updated_at
         ) VALUES (?1, ?2, 'active', 1, ?3, ?3)",
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
    let changed = db.execute(
        "INSERT INTO credential_bindings (
            binding_id, credential_digest, principal_id, status, created_at, revoked_at
         ) VALUES (?1, ?2, ?3, 'active', ?4, NULL)
         ON CONFLICT(credential_digest) DO UPDATE SET
            status = 'active', revoked_at = NULL
         WHERE credential_bindings.principal_id = excluded.principal_id",
        params![binding_id, digest, principal_id, crate::now_ts()],
    )?;
    anyhow::ensure!(changed == 1, "credential_binding_owner_changed");
    Ok(())
}

fn credential_binding_owner(db: &Connection, digest: &str) -> anyhow::Result<Option<String>> {
    db.query_row(
        "SELECT principal_id FROM credential_bindings WHERE credential_digest = ?1",
        [digest],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn active_auth_key_identity(
    db: &Connection,
    user_key: &str,
) -> anyhow::Result<Option<(String, String)>> {
    db.query_row(
        "SELECT k.role, k.principal_id FROM auth_keys k
         JOIN principals p ON p.principal_id = k.principal_id
         JOIN credential_bindings b ON b.principal_id = k.principal_id
         WHERE k.user_key = ?1 AND k.enabled = 1
           AND b.credential_digest = ?2 AND b.status = 'active' AND p.status = 'active'",
        params![user_key, credential_digest(user_key)],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

/// Validate this account through the same live key/binding/principal read used
/// by authentication and return that validated key. This is not authorization
/// to act for another account.
pub(crate) fn require_active_principal_credential(
    db: &Connection,
    principal_id: &str,
) -> anyhow::Result<String> {
    active_principal_credential(db, principal_id)?
        .ok_or_else(|| anyhow::anyhow!("credential_binding_missing"))
}

/// Live projection metadata is not authorization to act for another account.
pub(crate) fn active_principal_credential(
    db: &Connection,
    principal_id: &str,
) -> anyhow::Result<Option<String>> {
    let mut statement =
        db.prepare("SELECT user_key FROM auth_keys WHERE principal_id = ?1 AND enabled = 1")?;
    let mut keys = statement.query([principal_id])?;
    while let Some(row) = keys.next()? {
        let key: String = row.get(0)?;
        if active_auth_key_identity(db, &key)?.is_some_and(|(_, owner)| owner == principal_id) {
            return Ok(Some(key));
        }
    }
    Ok(None)
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
    with_identity_transaction(db, |db| {
        let old_digest = credential_digest(old_user_key);
        let new_digest = credential_digest(new_user_key);
        anyhow::ensure!(
            old_digest != new_digest && !new_user_key.trim().is_empty(),
            "credential_rotation_requires_new_key"
        );
        if let Some(owner) = credential_binding_owner(db, &new_digest)? {
            anyhow::ensure!(owner == principal_id, "credential_binding_owner_conflict");
        }
        let source_owner = credential_binding_owner(db, &old_digest)?;
        anyhow::ensure!(
            source_owner.as_deref() == Some(principal_id),
            "credential_rotation_source_conflict"
        );
        let changed = db.execute(
            "UPDATE credential_bindings SET status = 'revoked', revoked_at = ?2
             WHERE credential_digest = ?1 AND principal_id = ?3 AND status = 'active'",
            params![old_digest, crate::now_ts(), principal_id],
        )?;
        anyhow::ensure!(changed == 1, "credential_rotation_source_inactive");
        ensure_active_credential_binding(db, new_user_key, principal_id)
    })
}

#[cfg(test)]
#[path = "auth_principal_tests.rs"]
mod tests;

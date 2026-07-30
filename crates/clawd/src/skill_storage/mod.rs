mod data_owners;
mod migration;
mod ownership;
mod resolver;
mod schema;

use crate::db_init::DbPool;
use claw_core::config::DatabaseConfig;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) use ownership::KbUserDataSnapshot;
pub(crate) use resolver::{SkillStorageDescriptor, SkillStorageResolver};

#[derive(Clone)]
pub(crate) struct SkillStorageRuntime {
    resolver: SkillStorageResolver,
    pools: HashMap<String, DbPool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkillStorageDataRemoval {
    pub(crate) data_present_before: bool,
    pub(crate) rows_deleted: usize,
    pub(crate) files_deleted: usize,
}

impl SkillStorageRuntime {
    pub(crate) fn initialize(
        workspace_root: &Path,
        config: &DatabaseConfig,
        main_pool: &DbPool,
    ) -> anyhow::Result<Self> {
        let resolver = SkillStorageResolver::new(
            workspace_root,
            &config.skill_data_root,
            config.busy_timeout_ms,
        )?;
        let pools = data_owners::owners()
            .iter()
            .map(|owner| {
                open_pool(
                    &resolver.database_path(owner.skill_name)?,
                    config.busy_timeout_ms,
                    config.audit_pool_max_size,
                    owner.ensure_schema,
                )
                .map(|pool| (owner.skill_name.to_string(), pool))
            })
            .collect::<anyhow::Result<HashMap<_, _>>>()?;
        let crypto = pools
            .get("crypto")
            .ok_or_else(|| anyhow::anyhow!("crypto storage owner unavailable"))?;
        let kb = pools
            .get("kb")
            .ok_or_else(|| anyhow::anyhow!("kb storage owner unavailable"))?;
        migration::migrate_legacy_crypto(main_pool, crypto)?;
        migration::migrate_legacy_kb_rows(main_pool, kb)?;
        Ok(Self { resolver, pools })
    }

    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        let resolver = SkillStorageResolver::test_default();
        let pools = data_owners::owners()
            .iter()
            .map(|owner| {
                (
                    owner.skill_name.to_string(),
                    memory_pool(owner.ensure_schema),
                )
            })
            .collect();
        Self { resolver, pools }
    }

    pub(crate) fn pool_for(&self, skill_name: &str) -> anyhow::Result<&DbPool> {
        self.pools
            .get(skill_name)
            .ok_or_else(|| anyhow::anyhow!("skill storage owner unavailable: {skill_name}"))
    }

    pub(crate) fn descriptor(
        &self,
        skill_name: &str,
        schema_version: u32,
    ) -> anyhow::Result<SkillStorageDescriptor> {
        self.resolver.descriptor(skill_name, schema_version)
    }

    pub(crate) fn take_kb_user_data(&self, user_key: &str) -> anyhow::Result<KbUserDataSnapshot> {
        ownership::take_user_data(self.pool_for("kb")?, Some(user_key))
    }

    pub(crate) fn take_all_kb_data(&self) -> anyhow::Result<KbUserDataSnapshot> {
        ownership::take_user_data(self.pool_for("kb")?, None)
    }

    pub(crate) fn restore_kb_data(&self, snapshot: &KbUserDataSnapshot) -> anyhow::Result<()> {
        ownership::restore_user_data(self.pool_for("kb")?, snapshot)
    }

    pub(crate) fn rebind_kb_user_key(
        &self,
        old_user_key: &str,
        new_user_key: &str,
    ) -> anyhow::Result<usize> {
        ownership::rebind_user_key(self.pool_for("kb")?, old_user_key, new_user_key)
    }

    pub(crate) fn data_state(&self, skill_name: &str) -> anyhow::Result<&'static str> {
        if let (Some(owner), Some(pool)) =
            (data_owners::owner(skill_name), self.pools.get(skill_name))
        {
            return owner.data_state(pool);
        }
        Ok(
            if self.resolver.resolved_database_path(skill_name)?.is_file() {
                "present"
            } else {
                "empty"
            },
        )
    }

    pub(crate) fn clear_skill_data(
        &self,
        skill_name: &str,
    ) -> anyhow::Result<SkillStorageDataRemoval> {
        if let (Some(owner), Some(pool)) =
            (data_owners::owner(skill_name), self.pools.get(skill_name))
        {
            return owner.clear(pool);
        }
        let database_path = self.resolver.resolved_database_path(skill_name)?;
        let mut files_deleted = 0usize;
        for path in sqlite_storage_files(&database_path) {
            if path.is_file() {
                std::fs::remove_file(&path)?;
                files_deleted += 1;
            }
        }
        if let Some(directory) = database_path.parent() {
            if directory.is_dir() && std::fs::read_dir(directory)?.next().is_none() {
                std::fs::remove_dir(directory)?;
            }
        }
        Ok(SkillStorageDataRemoval {
            data_present_before: files_deleted > 0,
            rows_deleted: 0,
            files_deleted,
        })
    }
}

fn sqlite_storage_files(database_path: &Path) -> [PathBuf; 3] {
    let file_name = database_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state.db");
    [
        database_path.to_path_buf(),
        database_path.with_file_name(format!("{file_name}-wal")),
        database_path.with_file_name(format!("{file_name}-shm")),
    ]
}

fn open_pool(
    path: &Path,
    busy_timeout_ms: u64,
    max_size: u32,
    ensure_schema: fn(&Connection) -> anyhow::Result<()>,
) -> anyhow::Result<DbPool> {
    let path = PathBuf::from(path);
    let manager = SqliteConnectionManager::file(&path).with_init(move |conn| {
        conn.busy_timeout(Duration::from_millis(busy_timeout_ms.max(1)))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    });
    let pool = Pool::builder()
        .max_size(max_size.max(2))
        .build(manager)
        .map_err(|error| anyhow::anyhow!("init skill storage pool: {error}"))?;
    let db = pool
        .get()
        .map_err(|error| anyhow::anyhow!("get skill storage connection: {error}"))?;
    ensure_schema(&db)?;
    drop(db);
    Ok(pool)
}

#[cfg(test)]
fn memory_pool(ensure_schema: fn(&Connection) -> anyhow::Result<()>) -> DbPool {
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .expect("build skill storage test pool");
    let db = pool.get().expect("get skill storage test connection");
    ensure_schema(&db).expect("initialize skill storage test schema");
    drop(db);
    pool
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

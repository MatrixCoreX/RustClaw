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
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) use ownership::KbUserDataSnapshot;
pub(crate) use resolver::{SkillStorageDescriptor, SkillStorageResolver};

#[derive(Clone)]
pub(crate) struct SkillStorageRuntime {
    resolver: SkillStorageResolver,
    crypto: DbPool,
    kb: DbPool,
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
        let crypto = open_pool(
            &resolver.database_path("crypto")?,
            config.busy_timeout_ms,
            config.audit_pool_max_size,
            schema::ensure_crypto_schema,
        )?;
        let kb = open_pool(
            &resolver.database_path("kb")?,
            config.busy_timeout_ms,
            config.audit_pool_max_size,
            schema::ensure_kb_schema,
        )?;
        migration::migrate_legacy_crypto(main_pool, &crypto)?;
        migration::migrate_legacy_kb_rows(main_pool, &kb)?;
        Ok(Self {
            resolver,
            crypto,
            kb,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        let resolver = SkillStorageResolver::test_default();
        let crypto = memory_pool(schema::ensure_crypto_schema);
        let kb = memory_pool(schema::ensure_kb_schema);
        Self {
            resolver,
            crypto,
            kb,
        }
    }

    pub(crate) fn crypto_pool(&self) -> &DbPool {
        &self.crypto
    }

    pub(crate) fn kb_pool(&self) -> &DbPool {
        &self.kb
    }

    pub(crate) fn descriptor(&self, skill_name: &str) -> anyhow::Result<SkillStorageDescriptor> {
        self.resolver.descriptor(skill_name)
    }

    pub(crate) fn take_kb_user_data(&self, user_key: &str) -> anyhow::Result<KbUserDataSnapshot> {
        ownership::take_user_data(&self.kb, Some(user_key))
    }

    pub(crate) fn take_all_kb_data(&self) -> anyhow::Result<KbUserDataSnapshot> {
        ownership::take_user_data(&self.kb, None)
    }

    pub(crate) fn restore_kb_data(&self, snapshot: &KbUserDataSnapshot) -> anyhow::Result<()> {
        ownership::restore_user_data(&self.kb, snapshot)
    }

    pub(crate) fn rebind_kb_user_key(
        &self,
        old_user_key: &str,
        new_user_key: &str,
    ) -> anyhow::Result<usize> {
        ownership::rebind_user_key(&self.kb, old_user_key, new_user_key)
    }

    pub(crate) fn data_state(&self, skill_name: &str) -> anyhow::Result<&'static str> {
        let rows = match skill_name {
            "crypto" => count_rows(&self.crypto, "exchange_api_credentials")?,
            "kb" => {
                count_rows(&self.kb, "kb_namespaces")?
                    + count_rows(&self.kb, "memory_retrieval_index")?
            }
            _ => {
                return Ok(
                    if self.resolver.resolved_database_path(skill_name)?.is_file() {
                        "present"
                    } else {
                        "empty"
                    },
                );
            }
        };
        Ok(if rows > 0 { "present" } else { "empty" })
    }

    pub(crate) fn clear_skill_data(
        &self,
        skill_name: &str,
    ) -> anyhow::Result<SkillStorageDataRemoval> {
        match skill_name {
            "crypto" => {
                let mut db = self
                    .crypto
                    .get()
                    .map_err(|error| anyhow::anyhow!("crypto storage pool: {error}"))?;
                let tx = db.transaction()?;
                let rows_deleted = tx.execute("DELETE FROM exchange_api_credentials", [])?;
                tx.commit()?;
                schema::integrity_check(&db, "crypto")?;
                Ok(SkillStorageDataRemoval {
                    data_present_before: rows_deleted > 0,
                    rows_deleted,
                    files_deleted: 0,
                })
            }
            "kb" => {
                let snapshot = self.take_all_kb_data()?;
                let rows_deleted = snapshot.row_count();
                Ok(SkillStorageDataRemoval {
                    data_present_before: rows_deleted > 0,
                    rows_deleted,
                    files_deleted: 0,
                })
            }
            _ => {
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
    }
}

fn count_rows(pool: &DbPool, table: &str) -> anyhow::Result<usize> {
    let db = pool
        .get()
        .map_err(|error| anyhow::anyhow!("skill storage pool: {error}"))?;
    let count = match table {
        "exchange_api_credentials" => {
            db.query_row("SELECT COUNT(*) FROM exchange_api_credentials", [], |row| {
                row.get::<_, i64>(0)
            })?
        }
        "kb_namespaces" => db.query_row("SELECT COUNT(*) FROM kb_namespaces", [], |row| {
            row.get::<_, i64>(0)
        })?,
        "memory_retrieval_index" => {
            db.query_row("SELECT COUNT(*) FROM memory_retrieval_index", [], |row| {
                row.get::<_, i64>(0)
            })?
        }
        _ => anyhow::bail!("unsupported skill storage table"),
    };
    usize::try_from(count).map_err(|_| anyhow::anyhow!("negative skill storage row count"))
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

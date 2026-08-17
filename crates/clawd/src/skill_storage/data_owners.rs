use crate::db_init::DbPool;
use rusqlite::Connection;

use super::{ownership, schema, SkillStorageDataRemoval};

pub(super) struct SkillStorageOwner {
    pub(super) skill_name: &'static str,
    pub(super) ensure_schema: fn(&Connection) -> anyhow::Result<()>,
    data_state: fn(&DbPool) -> anyhow::Result<&'static str>,
    clear: fn(&DbPool) -> anyhow::Result<SkillStorageDataRemoval>,
}

pub(super) fn owners() -> &'static [SkillStorageOwner] {
    static OWNERS: [SkillStorageOwner; 2] = [
        SkillStorageOwner {
            skill_name: "crypto",
            ensure_schema: schema::ensure_crypto_schema,
            data_state: crypto_data_state,
            clear: clear_crypto,
        },
        SkillStorageOwner {
            skill_name: "kb",
            ensure_schema: schema::ensure_kb_schema,
            data_state: kb_data_state,
            clear: clear_kb,
        },
    ];
    &OWNERS
}

pub(super) fn owner(skill_name: &str) -> Option<&'static SkillStorageOwner> {
    owners().iter().find(|owner| owner.skill_name == skill_name)
}

impl SkillStorageOwner {
    pub(super) fn data_state(&self, pool: &DbPool) -> anyhow::Result<&'static str> {
        (self.data_state)(pool)
    }

    pub(super) fn clear(&self, pool: &DbPool) -> anyhow::Result<SkillStorageDataRemoval> {
        (self.clear)(pool)
    }
}

fn crypto_data_state(pool: &DbPool) -> anyhow::Result<&'static str> {
    state_from_count(count(
        pool,
        "SELECT COUNT(*) FROM exchange_api_credentials",
    )?)
}

fn kb_data_state(pool: &DbPool) -> anyhow::Result<&'static str> {
    state_from_count(
        count(pool, "SELECT COUNT(*) FROM kb_namespaces")?
            + count(pool, "SELECT COUNT(*) FROM kb_documents")?
            + count(pool, "SELECT COUNT(*) FROM kb_chunks")?
            + count(pool, "SELECT COUNT(*) FROM kb_ingest_jobs")?
            + count(pool, "SELECT COUNT(*) FROM memory_retrieval_index")?,
    )
}

fn state_from_count(rows: usize) -> anyhow::Result<&'static str> {
    Ok(if rows > 0 { "present" } else { "empty" })
}

fn count(pool: &DbPool, query: &str) -> anyhow::Result<usize> {
    let db = pool
        .get()
        .map_err(|error| anyhow::anyhow!("skill storage pool: {error}"))?;
    let count = db.query_row(query, [], |row| row.get::<_, i64>(0))?;
    usize::try_from(count).map_err(|_| anyhow::anyhow!("negative skill storage row count"))
}

fn clear_crypto(pool: &DbPool) -> anyhow::Result<SkillStorageDataRemoval> {
    let mut db = pool
        .get()
        .map_err(|error| anyhow::anyhow!("skill storage pool: {error}"))?;
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

fn clear_kb(pool: &DbPool) -> anyhow::Result<SkillStorageDataRemoval> {
    let snapshot = ownership::take_user_data(pool, None)?;
    let rows_deleted = snapshot.row_count();
    Ok(SkillStorageDataRemoval {
        data_present_before: rows_deleted > 0,
        rows_deleted,
        files_deleted: 0,
    })
}

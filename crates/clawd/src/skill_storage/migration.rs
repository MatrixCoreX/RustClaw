use crate::db_init::DbPool;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

const CRYPTO_MIGRATION_ID: &str = "legacy-main-exchange-api-credentials-v1";
const KB_MIGRATION_ID: &str = "legacy-main-kb-retrieval-rows-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
struct LegacyCredential {
    user_key: String,
    exchange: String,
    api_key: String,
    api_secret: String,
    passphrase: Option<String>,
    enabled: i64,
    updated_at: String,
}

#[derive(Clone, Debug)]
struct LegacyKbRow {
    source_ref: String,
    user_id: i64,
    chat_id: i64,
    user_key: String,
    memory_kind: String,
    role: Option<String>,
    search_text: String,
    trigger_text: Option<String>,
    topic_tags: String,
    vector_json: String,
    embedding_model: String,
    embedding_dims: i64,
    embedding_version: String,
    metadata_json: String,
    salience: f64,
    success_state: String,
    tool_or_skill_name: Option<String>,
    created_at_ts: i64,
    updated_at_ts: i64,
}

pub(super) fn migrate_legacy_crypto(
    main_pool: &DbPool,
    crypto_pool: &DbPool,
) -> anyhow::Result<()> {
    let mut main = main_pool
        .get()
        .map_err(|error| anyhow::anyhow!("get main db for crypto migration: {error}"))?;
    if !table_exists(&main, "exchange_api_credentials")? {
        return Ok(());
    }
    let rows = {
        let mut stmt = main.prepare(
            "SELECT user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at
             FROM exchange_api_credentials
             ORDER BY user_key, exchange",
        )?;
        let mapped = stmt.query_map([], |row| {
            Ok(LegacyCredential {
                user_key: row.get(0)?,
                exchange: row.get(1)?,
                api_key: row.get(2)?,
                api_secret: row.get(3)?,
                passphrase: row.get(4)?,
                enabled: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };
    let digest = credential_digest(&rows);
    let mut crypto = crypto_pool
        .get()
        .map_err(|error| anyhow::anyhow!("get crypto db for migration: {error}"))?;
    {
        let tx = crypto.transaction()?;
        for row in &rows {
            tx.execute(
                "INSERT INTO exchange_api_credentials
                    (user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(user_key, exchange) DO NOTHING",
                params![
                    row.user_key,
                    row.exchange,
                    row.api_key,
                    row.api_secret,
                    row.passphrase,
                    row.enabled,
                    row.updated_at
                ],
            )?;
        }
        tx.commit()?;
    }
    verify_credentials(&crypto, &rows)?;
    record_checkpoint(
        &crypto,
        CRYPTO_MIGRATION_ID,
        "runtime_main.exchange_api_credentials",
        rows.len(),
        &digest,
    )?;

    let tx = main.transaction()?;
    tx.execute("DROP TABLE exchange_api_credentials", [])?;
    tx.commit()?;
    tracing::info!(
        migrated_rows = rows.len(),
        migration_id = CRYPTO_MIGRATION_ID,
        "migrated crypto credentials to skill-owned storage"
    );
    Ok(())
}

pub(super) fn migrate_legacy_kb_rows(main_pool: &DbPool, kb_pool: &DbPool) -> anyhow::Result<()> {
    let mut main = main_pool
        .get()
        .map_err(|error| anyhow::anyhow!("get main db for KB migration: {error}"))?;
    if !table_exists(&main, "memory_retrieval_index")? {
        return Ok(());
    }
    let rows = {
        let mut stmt = main.prepare(
            "SELECT id, COALESCE(source_ref, ''), user_id, chat_id,
                    COALESCE(user_key, ''), memory_kind, role, search_text,
                    trigger_text, topic_tags, vector_json, embedding_model,
                    embedding_dims, embedding_version, metadata_json, salience,
                    success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             FROM memory_retrieval_index
             WHERE source_kind = 'kb_doc'
             ORDER BY id",
        )?;
        let mapped = stmt.query_map([], |row| {
            let id = row.get::<_, i64>(0)?;
            let source_ref = row.get::<_, String>(1)?;
            Ok(LegacyKbRow {
                source_ref: if source_ref.trim().is_empty() {
                    format!("legacy-kb-row:{id}")
                } else {
                    source_ref
                },
                user_id: row.get(2)?,
                chat_id: row.get(3)?,
                user_key: row.get(4)?,
                memory_kind: row.get(5)?,
                role: row.get(6)?,
                search_text: row.get(7)?,
                trigger_text: row.get(8)?,
                topic_tags: row.get(9)?,
                vector_json: row.get(10)?,
                embedding_model: row.get(11)?,
                embedding_dims: row.get(12)?,
                embedding_version: row.get(13)?,
                metadata_json: row.get(14)?,
                salience: row.get(15)?,
                success_state: row.get(16)?,
                tool_or_skill_name: row.get(17)?,
                created_at_ts: row.get(18)?,
                updated_at_ts: row.get(19)?,
            })
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };
    if rows.is_empty() {
        return Ok(());
    }
    let digest = kb_digest(&rows);
    let mut kb = kb_pool
        .get()
        .map_err(|error| anyhow::anyhow!("get KB db for migration: {error}"))?;
    {
        let tx = kb.transaction()?;
        for row in &rows {
            tx.execute(
                "INSERT INTO memory_retrieval_index (
                    source_kind, source_memory_id, source_pref_key, source_ref,
                    user_id, chat_id, user_key, memory_kind, role, search_text,
                    trigger_text, topic_tags, vector_json, embedding_model,
                    embedding_dims, embedding_version, metadata_json, salience,
                    success_state, tool_or_skill_name, created_at_ts, updated_at_ts
                 ) VALUES (
                    'kb_doc', NULL, NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                    ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
                 )
                 ON CONFLICT(user_key, source_ref) DO NOTHING",
                params![
                    row.source_ref,
                    row.user_id,
                    row.chat_id,
                    row.user_key,
                    row.memory_kind,
                    row.role,
                    row.search_text,
                    row.trigger_text,
                    row.topic_tags,
                    row.vector_json,
                    row.embedding_model,
                    row.embedding_dims,
                    row.embedding_version,
                    row.metadata_json,
                    row.salience,
                    row.success_state,
                    row.tool_or_skill_name,
                    row.created_at_ts,
                    row.updated_at_ts
                ],
            )?;
        }
        tx.commit()?;
    }
    verify_kb_rows(&kb, &rows)?;
    rebuild_kb_fts(&kb)?;
    record_checkpoint(
        &kb,
        KB_MIGRATION_ID,
        "runtime_main.memory_retrieval_index[kb_doc]",
        rows.len(),
        &digest,
    )?;

    let tx = main.transaction()?;
    tx.execute(
        "DELETE FROM memory_retrieval_index WHERE source_kind = 'kb_doc'",
        [],
    )?;
    if table_exists(&tx, "memory_retrieval_index_fts")? {
        tx.execute(
            "DELETE FROM memory_retrieval_index_fts
             WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
            [],
        )?;
    }
    tx.commit()?;
    tracing::info!(
        migrated_rows = rows.len(),
        migration_id = KB_MIGRATION_ID,
        "migrated KB retrieval rows to skill-owned storage"
    );
    Ok(())
}

fn verify_credentials(
    crypto: &rusqlite::Connection,
    source: &[LegacyCredential],
) -> anyhow::Result<()> {
    for expected in source {
        let actual = crypto
            .query_row(
                "SELECT user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at
                 FROM exchange_api_credentials
                 WHERE user_key = ?1 AND exchange = ?2",
                params![expected.user_key, expected.exchange],
                |row| {
                    Ok(LegacyCredential {
                        user_key: row.get(0)?,
                        exchange: row.get(1)?,
                        api_key: row.get(2)?,
                        api_secret: row.get(3)?,
                        passphrase: row.get(4)?,
                        enabled: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            )
            .optional()?;
        if actual.as_ref() != Some(expected) {
            anyhow::bail!("crypto skill storage migration verification failed");
        }
    }
    Ok(())
}

fn verify_kb_rows(kb: &rusqlite::Connection, source: &[LegacyKbRow]) -> anyhow::Result<()> {
    for expected in source {
        let count: i64 = kb.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index
             WHERE source_kind = 'kb_doc' AND user_key = ?1 AND source_ref = ?2
               AND search_text = ?3",
            params![expected.user_key, expected.source_ref, expected.search_text],
            |row| row.get(0),
        )?;
        if count != 1 {
            anyhow::bail!("KB skill storage migration verification failed");
        }
    }
    Ok(())
}

fn rebuild_kb_fts(kb: &rusqlite::Connection) -> anyhow::Result<()> {
    if !table_exists(kb, "memory_retrieval_index_fts")? {
        return Ok(());
    }
    kb.execute("DELETE FROM memory_retrieval_index_fts", [])?;
    kb.execute(
        "INSERT INTO memory_retrieval_index_fts(rowid, search_text, topic_tags)
         SELECT id, search_text, topic_tags FROM memory_retrieval_index",
        [],
    )?;
    Ok(())
}

fn record_checkpoint(
    db: &rusqlite::Connection,
    migration_id: &str,
    source_identity: &str,
    source_rows: usize,
    digest: &str,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO skill_storage_migrations
            (migration_id, source_identity, source_rows, verified_digest)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(migration_id) DO UPDATE SET
             source_identity = excluded.source_identity,
             source_rows = excluded.source_rows,
             verified_digest = excluded.verified_digest,
             completed_at = CURRENT_TIMESTAMP",
        params![migration_id, source_identity, source_rows as i64, digest],
    )?;
    Ok(())
}

fn credential_digest(rows: &[LegacyCredential]) -> String {
    let mut digest = Sha256::new();
    for row in rows {
        update_digest(
            &mut digest,
            &[
                row.user_key.as_bytes(),
                row.exchange.as_bytes(),
                row.api_key.as_bytes(),
                row.api_secret.as_bytes(),
                row.passphrase.as_deref().unwrap_or("").as_bytes(),
                row.updated_at.as_bytes(),
            ],
        );
        digest.update(row.enabled.to_le_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn kb_digest(rows: &[LegacyKbRow]) -> String {
    let mut digest = Sha256::new();
    for row in rows {
        update_digest(
            &mut digest,
            &[
                row.user_key.as_bytes(),
                row.source_ref.as_bytes(),
                row.search_text.as_bytes(),
                row.metadata_json.as_bytes(),
            ],
        );
    }
    format!("{:x}", digest.finalize())
}

fn update_digest(digest: &mut Sha256, values: &[&[u8]]) {
    for value in values {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value);
    }
}

fn table_exists(db: &rusqlite::Connection, table_name: &str) -> anyhow::Result<bool> {
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type IN ('table', 'virtual table') AND name = ?1",
        params![table_name],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;

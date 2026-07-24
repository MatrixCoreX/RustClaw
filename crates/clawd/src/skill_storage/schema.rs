use rusqlite::Connection;

pub(super) const CRYPTO_SCHEMA_VERSION: i64 = 1;
pub(super) const KB_SCHEMA_VERSION: i64 = 1;

pub(super) fn ensure_crypto_schema(db: &Connection) -> anyhow::Result<()> {
    ensure_common_schema(db, "crypto", CRYPTO_SCHEMA_VERSION)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS exchange_api_credentials (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key    TEXT NOT NULL,
            exchange    TEXT NOT NULL,
            api_key     TEXT NOT NULL,
            api_secret  TEXT NOT NULL,
            passphrase  TEXT,
            enabled     INTEGER NOT NULL DEFAULT 1,
            updated_at  TEXT NOT NULL,
            UNIQUE(user_key, exchange)
        );
        CREATE INDEX IF NOT EXISTS idx_exchange_api_credentials_user_exchange
        ON exchange_api_credentials(user_key, exchange);",
    )?;
    integrity_check(db, "crypto")
}

pub(super) fn ensure_kb_schema(db: &Connection) -> anyhow::Result<()> {
    ensure_common_schema(db, "kb", KB_SCHEMA_VERSION)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS kb_namespaces (
            owner_user_key  TEXT NOT NULL,
            namespace       TEXT NOT NULL,
            payload_json    TEXT NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            PRIMARY KEY(owner_user_key, namespace)
        );
        CREATE INDEX IF NOT EXISTS idx_kb_namespaces_owner_updated
        ON kb_namespaces(owner_user_key, updated_at_epoch DESC);
        CREATE TABLE IF NOT EXISTS memory_retrieval_index (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            source_kind       TEXT NOT NULL,
            source_memory_id  INTEGER,
            source_pref_key   TEXT,
            source_ref        TEXT,
            user_id           INTEGER NOT NULL,
            chat_id           INTEGER NOT NULL,
            user_key          TEXT,
            memory_kind       TEXT NOT NULL,
            role              TEXT,
            search_text       TEXT NOT NULL,
            trigger_text      TEXT,
            topic_tags        TEXT NOT NULL DEFAULT '',
            vector_json       TEXT NOT NULL DEFAULT '[]',
            embedding_model   TEXT NOT NULL DEFAULT 'local-hash-v1',
            embedding_dims    INTEGER NOT NULL DEFAULT 24,
            embedding_version TEXT NOT NULL DEFAULT 'local-hash-v1',
            metadata_json     TEXT NOT NULL DEFAULT '{}',
            salience          REAL NOT NULL DEFAULT 0.5,
            success_state     TEXT NOT NULL DEFAULT 'neutral',
            tool_or_skill_name TEXT,
            created_at_ts     INTEGER NOT NULL DEFAULT 0,
            updated_at_ts     INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_kb_retrieval_source_identity
        ON memory_retrieval_index(user_key, source_ref);
        CREATE INDEX IF NOT EXISTS idx_kb_retrieval_scope_updated
        ON memory_retrieval_index(user_key, updated_at_ts DESC);",
    )?;
    let _ = db.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_retrieval_index_fts
         USING fts5(search_text, topic_tags);",
    );
    integrity_check(db, "kb")
}

fn ensure_common_schema(
    db: &Connection,
    skill_name: &str,
    schema_version: i64,
) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS skill_storage_metadata (
            skill_name      TEXT PRIMARY KEY,
            schema_version  INTEGER NOT NULL,
            updated_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS skill_storage_migrations (
            migration_id   TEXT PRIMARY KEY,
            source_identity TEXT NOT NULL,
            source_rows     INTEGER NOT NULL,
            verified_digest TEXT NOT NULL,
            completed_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )?;
    db.execute(
        "INSERT INTO skill_storage_metadata (skill_name, schema_version)
         VALUES (?1, ?2)
         ON CONFLICT(skill_name) DO UPDATE SET
             schema_version = excluded.schema_version,
             updated_at = CURRENT_TIMESTAMP",
        rusqlite::params![skill_name, schema_version],
    )?;
    Ok(())
}

pub(super) fn integrity_check(db: &Connection, skill_name: &str) -> anyhow::Result<()> {
    let result: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if result != "ok" {
        anyhow::bail!("{skill_name} skill storage integrity check failed");
    }
    Ok(())
}

use crate::db_init::DbPool;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub(crate) struct KbUserDataSnapshot {
    namespaces: Vec<NamespaceRow>,
    retrieval_rows: Vec<RetrievalRow>,
}

#[derive(Clone, Debug)]
struct NamespaceRow {
    owner_user_key: String,
    namespace: String,
    payload_json: String,
    updated_at_epoch: i64,
}

#[derive(Clone, Debug)]
struct RetrievalRow {
    source_ref: Option<String>,
    user_id: i64,
    chat_id: i64,
    user_key: Option<String>,
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

impl KbUserDataSnapshot {
    pub(crate) fn row_count(&self) -> usize {
        self.namespaces.len() + self.retrieval_rows.len()
    }

    fn rebind(mut self, old_user_key: &str, new_user_key: &str) -> anyhow::Result<Self> {
        for namespace in &mut self.namespaces {
            namespace.owner_user_key = new_user_key.to_string();
            let mut payload = serde_json::from_str::<Value>(&namespace.payload_json)?;
            let object = payload
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("KB namespace payload must be an object"))?;
            object.insert(
                "owner_user_key".to_string(),
                Value::String(new_user_key.to_string()),
            );
            namespace.payload_json = serde_json::to_string(&payload)?;
        }
        let old_prefix = format!("kb:{old_user_key}:");
        let new_prefix = format!("kb:{new_user_key}:");
        for row in &mut self.retrieval_rows {
            row.user_key = Some(new_user_key.to_string());
            if let Some(source_ref) = row.source_ref.as_mut() {
                if source_ref.starts_with(&old_prefix) {
                    *source_ref = format!("{new_prefix}{}", &source_ref[old_prefix.len()..]);
                }
            }
            if let Ok(mut metadata) = serde_json::from_str::<Value>(&row.metadata_json) {
                if let Some(object) = metadata.as_object_mut() {
                    object.insert(
                        "owner_user_key".to_string(),
                        Value::String(new_user_key.to_string()),
                    );
                }
                row.metadata_json = serde_json::to_string(&metadata)?;
            }
        }
        Ok(self)
    }
}

pub(super) fn take_user_data(
    pool: &DbPool,
    user_key: Option<&str>,
) -> anyhow::Result<KbUserDataSnapshot> {
    let mut db = pool
        .get()
        .map_err(|error| anyhow::anyhow!("KB storage pool: {error}"))?;
    let snapshot = select_snapshot(&db, user_key)?;
    let tx = db.transaction()?;
    match user_key {
        Some(user_key) => {
            tx.execute(
                "DELETE FROM kb_namespaces WHERE owner_user_key = ?1",
                params![user_key],
            )?;
            tx.execute(
                "DELETE FROM memory_retrieval_index WHERE user_key = ?1",
                params![user_key],
            )?;
        }
        None => {
            tx.execute("DELETE FROM kb_namespaces", [])?;
            tx.execute("DELETE FROM memory_retrieval_index", [])?;
        }
    }
    rebuild_fts(&tx)?;
    tx.commit()?;
    Ok(snapshot)
}

pub(super) fn restore_user_data(
    pool: &DbPool,
    snapshot: &KbUserDataSnapshot,
) -> anyhow::Result<()> {
    if snapshot.row_count() == 0 {
        return Ok(());
    }
    let mut db = pool
        .get()
        .map_err(|error| anyhow::anyhow!("KB storage pool: {error}"))?;
    let tx = db.transaction()?;
    for row in &snapshot.namespaces {
        tx.execute(
            "INSERT INTO kb_namespaces
                (owner_user_key, namespace, payload_json, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_user_key, namespace) DO UPDATE SET
                payload_json = excluded.payload_json,
                updated_at_epoch = excluded.updated_at_epoch",
            params![
                row.owner_user_key,
                row.namespace,
                row.payload_json,
                row.updated_at_epoch
            ],
        )?;
    }
    for row in &snapshot.retrieval_rows {
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
             ON CONFLICT(user_key, source_ref) DO UPDATE SET
                search_text = excluded.search_text,
                topic_tags = excluded.topic_tags,
                vector_json = excluded.vector_json,
                metadata_json = excluded.metadata_json,
                updated_at_ts = excluded.updated_at_ts",
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
    rebuild_fts(&tx)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn rebind_user_key(
    pool: &DbPool,
    old_user_key: &str,
    new_user_key: &str,
) -> anyhow::Result<usize> {
    let original = take_user_data(pool, Some(old_user_key))?;
    let count = original.row_count();
    if count == 0 {
        return Ok(0);
    }
    let rebound = match original.clone().rebind(old_user_key, new_user_key) {
        Ok(rebound) => rebound,
        Err(error) => {
            restore_user_data(pool, &original)?;
            return Err(error);
        }
    };
    if let Err(error) = restore_user_data(pool, &rebound) {
        restore_user_data(pool, &original)?;
        return Err(error);
    }
    Ok(count)
}

fn select_snapshot(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<KbUserDataSnapshot> {
    let namespaces = select_namespaces(db, user_key)?;
    let retrieval_rows = select_retrieval_rows(db, user_key)?;
    Ok(KbUserDataSnapshot {
        namespaces,
        retrieval_rows,
    })
}

fn select_namespaces(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<Vec<NamespaceRow>> {
    let sql = match user_key {
        Some(_) => {
            "SELECT owner_user_key, namespace, payload_json, updated_at_epoch
             FROM kb_namespaces WHERE owner_user_key = ?1 ORDER BY namespace"
        }
        None => {
            "SELECT owner_user_key, namespace, payload_json, updated_at_epoch
             FROM kb_namespaces ORDER BY owner_user_key, namespace"
        }
    };
    let mut stmt = db.prepare(sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(NamespaceRow {
            owner_user_key: row.get(0)?,
            namespace: row.get(1)?,
            payload_json: row.get(2)?,
            updated_at_epoch: row.get(3)?,
        })
    };
    let rows = match user_key {
        Some(user_key) => stmt.query_map(params![user_key], map)?,
        None => stmt.query_map([], map)?,
    };
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn select_retrieval_rows(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<Vec<RetrievalRow>> {
    let sql = match user_key {
        Some(_) => {
            "SELECT source_ref, user_id, chat_id, user_key, memory_kind, role,
                    search_text, trigger_text, topic_tags, vector_json,
                    embedding_model, embedding_dims, embedding_version,
                    metadata_json, salience, success_state, tool_or_skill_name,
                    created_at_ts, updated_at_ts
             FROM memory_retrieval_index WHERE user_key = ?1 ORDER BY id"
        }
        None => {
            "SELECT source_ref, user_id, chat_id, user_key, memory_kind, role,
                    search_text, trigger_text, topic_tags, vector_json,
                    embedding_model, embedding_dims, embedding_version,
                    metadata_json, salience, success_state, tool_or_skill_name,
                    created_at_ts, updated_at_ts
             FROM memory_retrieval_index ORDER BY id"
        }
    };
    let mut stmt = db.prepare(sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(RetrievalRow {
            source_ref: row.get(0)?,
            user_id: row.get(1)?,
            chat_id: row.get(2)?,
            user_key: row.get(3)?,
            memory_kind: row.get(4)?,
            role: row.get(5)?,
            search_text: row.get(6)?,
            trigger_text: row.get(7)?,
            topic_tags: row.get(8)?,
            vector_json: row.get(9)?,
            embedding_model: row.get(10)?,
            embedding_dims: row.get(11)?,
            embedding_version: row.get(12)?,
            metadata_json: row.get(13)?,
            salience: row.get(14)?,
            success_state: row.get(15)?,
            tool_or_skill_name: row.get(16)?,
            created_at_ts: row.get(17)?,
            updated_at_ts: row.get(18)?,
        })
    };
    let rows = match user_key {
        Some(user_key) => stmt.query_map(params![user_key], map)?,
        None => stmt.query_map([], map)?,
    };
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn rebuild_fts(db: &rusqlite::Connection) -> anyhow::Result<()> {
    let has_fts = db
        .query_row(
            "SELECT 1 FROM sqlite_master
             WHERE type='table' AND name='memory_retrieval_index_fts'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !has_fts {
        return Ok(());
    }
    db.execute("DELETE FROM memory_retrieval_index_fts", [])?;
    db.execute(
        "INSERT INTO memory_retrieval_index_fts(rowid, search_text, topic_tags)
         SELECT id, search_text, topic_tags FROM memory_retrieval_index",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;

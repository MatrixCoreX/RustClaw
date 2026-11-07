use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::memory) fn insert_index_row(
    db: &Connection,
    source_kind: &str,
    source_memory_id: Option<i64>,
    source_pref_key: Option<&str>,
    source_ref: Option<&str>,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    memory_kind: &str,
    role: Option<&str>,
    search_text: &str,
    trigger_text: Option<&str>,
    metadata_json: Option<&str>,
    salience: f32,
    success_state: &str,
    tool_or_skill_name: Option<&str>,
    ts: i64,
) -> anyhow::Result<()> {
    crate::memory::projection_transaction::atomic(db, |db| {
        let topic_tags = build_topic_tags(search_text);
        let vector_json = "[]";
        let embedding_spec = crate::memory::embedding::local_hash_embedding_spec();
        let metadata_json = metadata_json.unwrap_or("{}");
        if retrieval_scope_contract_available(db)? {
            let Some(principal_id) = crate::repo::auth::principal_id_for_user_key(db, user_key)?
            else {
                tracing::warn!(
                    source_kind,
                    source_memory_id,
                    source_pref_key,
                    "memory_retrieval_unowned_source_skipped"
                );
                return Ok(());
            };
            let mut metadata = serde_json::from_str::<serde_json::Value>(metadata_json)
                .map_err(|_| anyhow::anyhow!("memory_projection_metadata_invalid"))?;
            anyhow::ensure!(metadata.is_object(), "memory_projection_metadata_invalid");
            let requested_scope = metadata
                .get("scope_kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("principal");
            let (mut scope_kind, mut scope_ref) = match requested_scope {
                "project" | "conversation" => (
                    requested_scope.to_string(),
                    metadata
                        .get("scope_ref")
                        .or_else(|| metadata.get("project_ref"))
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string),
                ),
                _ => ("principal".to_string(), Some(principal_id.clone())),
            };
            anyhow::ensure!(scope_ref.is_some(), "memory_retrieval_scope_ref_required");
            let canonical_source = match source_kind {
                RETRIEVAL_SOURCE_MEMORY => Some(crate::memory::recall_policy::Source::Memory(
                    source_memory_id.ok_or_else(|| {
                        anyhow::anyhow!("memory_projection_source_identity_required")
                    })?,
                )),
                RETRIEVAL_SOURCE_PREFERENCE => {
                    Some(crate::memory::recall_policy::Source::Preference {
                        user_id,
                        chat_id,
                        key: source_pref_key.ok_or_else(|| {
                            anyhow::anyhow!("memory_projection_source_identity_required")
                        })?,
                        canonical_id: metadata
                            .get("canonical_memory_id")
                            .and_then(serde_json::Value::as_str),
                    })
                }
                RETRIEVAL_SOURCE_MEMORY_FACT => Some(crate::memory::recall_policy::Source::Fact(
                    metadata
                        .get("fact_id")
                        .and_then(serde_json::Value::as_i64)
                        .ok_or_else(|| {
                            anyhow::anyhow!("memory_projection_source_identity_required")
                        })?,
                )),
                _ => None,
            };
            if let Some(source) = canonical_source {
                let Some(scope) =
                    crate::memory::recall_policy::resolve(db, &principal_id, user_key, source)?
                else {
                    return Ok(());
                };
                scope_kind = scope.kind;
                scope_ref = Some(scope.reference.clone());
                metadata["scope_kind"] = serde_json::Value::String(scope_kind.clone());
                metadata["scope_ref"] = serde_json::Value::String(scope.reference);
            }
            let metadata_json = metadata.to_string();
            db.execute(
                "INSERT INTO memory_retrieval_index (
                source_kind, source_memory_id, source_pref_key, source_ref,
                user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
                memory_kind, role, search_text, trigger_text, topic_tags, vector_json,
                embedding_model, embedding_dims, embedding_version, metadata_json,
                salience, success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?24
             )",
                params![
                    source_kind,
                    source_memory_id,
                    source_pref_key,
                    source_ref,
                    user_id,
                    chat_id,
                    user_key,
                    principal_id,
                    scope_kind,
                    scope_ref,
                    memory_kind,
                    role,
                    search_text,
                    trigger_text,
                    topic_tags,
                    vector_json,
                    embedding_spec.model_id,
                    embedding_spec.dims as i64,
                    embedding_spec.version,
                    metadata_json,
                    salience,
                    success_state,
                    tool_or_skill_name,
                    ts,
                ],
            )?;
        } else {
            db.execute(
                "INSERT INTO memory_retrieval_index (
                source_kind, source_memory_id, source_pref_key, source_ref, user_id, chat_id,
                user_key, memory_kind, role, search_text, trigger_text, topic_tags, vector_json,
                embedding_model, embedding_dims, embedding_version, metadata_json,
                salience, success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?21
             )",
                params![
                    source_kind,
                    source_memory_id,
                    source_pref_key,
                    source_ref,
                    user_id,
                    chat_id,
                    user_key,
                    memory_kind,
                    role,
                    search_text,
                    trigger_text,
                    topic_tags,
                    vector_json,
                    embedding_spec.model_id,
                    embedding_spec.dims as i64,
                    embedding_spec.version,
                    metadata_json,
                    salience,
                    success_state,
                    tool_or_skill_name,
                    ts,
                ],
            )?;
        }
        let row_id = db.last_insert_rowid();
        let fts_present: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table'
         AND name='memory_retrieval_index_fts')",
            [],
            |row| row.get(0),
        )?;
        if fts_present {
            db.execute(
                "INSERT INTO memory_retrieval_index_fts(rowid, search_text, topic_tags)
             VALUES (?1, ?2, ?3)",
                params![row_id, search_text, topic_tags],
            )?;
        }
        if retrieval_scope_contract_available(db)? {
            let principal_id = crate::repo::auth::principal_id_for_user_key(db, user_key)?;
            let scope = db.query_row(
                "SELECT COALESCE(scope_kind, 'principal'), scope_ref
                 FROM memory_retrieval_index WHERE id = ?1",
                [row_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )?;
            crate::memory::vector_store::enqueue_retrieval_embedding(
                db,
                row_id,
                principal_id.as_deref(),
                &scope.0,
                scope.1.as_deref(),
                search_text,
            )?;
        }
        Ok(())
    })
}

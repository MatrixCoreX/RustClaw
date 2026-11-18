use super::*;

pub(crate) fn index_preference_entries(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    entries: &[(String, String, f32, String)],
    now_ts_i64: i64,
) -> anyhow::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    crate::memory::projection_transaction::atomic(db, |db| {
        for (pref_key, pref_value, confidence, source) in entries {
            let source_ref = retrieval_source_ref_for_preference(pref_key);
            if !crate::memory::recall_policy::prepare(
                db,
                user_key,
                crate::memory::recall_policy::Source::Preference {
                    user_id,
                    chat_id,
                    key: pref_key,
                    canonical_id: None,
                },
                pref_value,
            )? {
                continue;
            }
            let (scope_kind, scope_ref, canonical_id): (String, Option<String>, Option<String>) =
                db.query_row(
                    "SELECT COALESCE(scope_kind, 'principal'), scope_ref,memory_id
                 FROM user_preferences
                 WHERE user_key = ?1 AND pref_key = ?2 AND user_id=?3 AND chat_id=?4
                 ORDER BY updated_at_ts DESC, id DESC LIMIT 1",
                    params![user_key, pref_key, user_id, chat_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?
                .unwrap_or_else(|| ("principal".to_string(), None, None));
            let text = format!("Preference {pref_key}: {pref_value}");
            insert_index_row(
                db,
                RETRIEVAL_SOURCE_PREFERENCE,
                None,
                Some(pref_key),
                Some(&source_ref),
                user_id,
                chat_id,
                user_key,
                RETRIEVAL_KIND_SEMANTIC_FACT,
                None,
                &text,
                Some(pref_key),
                Some(&build_preference_metadata_json(
                    pref_key,
                    &scope_kind,
                    scope_ref.as_deref(),
                    canonical_id.as_deref(),
                )),
                *confidence,
                RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
                Some(source),
                now_ts_i64,
            )?;
        }
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn index_memory_row(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    source_memory_id: i64,
    role: &str,
    content: &str,
    memory_type: &str,
    salience: f32,
    is_instructional: bool,
    created_at_ts: i64,
) -> anyhow::Result<()> {
    crate::memory::projection_transaction::atomic(db, |db| {
        if !crate::memory::recall_policy::prepare(
            db,
            user_key,
            crate::memory::recall_policy::Source::Memory(source_memory_id),
            content,
        )? {
            return Ok(());
        }

        let cleaned = content.trim();
        if cleaned.is_empty() {
            return Ok(());
        }
        let search_text = cleaned
            .strip_prefix(LLM_SHORT_TERM_MEMORY_PREFIX)
            .unwrap_or(cleaned)
            .trim();
        if search_text.is_empty() {
            return Ok(());
        }
        if memory_type == MEMORY_TYPE_SAFETY_SIGNAL {
            return Ok(());
        }
        if role == MEMORY_ROLE_ASSISTANT
            && crate::memory::is_transient_assistant_context_text_basic(search_text)
        {
            return Ok(());
        }
        let source_ref = retrieval_source_ref_for_memory(source_memory_id);

        if memory_type == MEMORY_TYPE_UNFINISHED_GOAL {
            insert_index_row(
                db,
                RETRIEVAL_SOURCE_MEMORY,
                Some(source_memory_id),
                None,
                Some(&source_ref),
                user_id,
                chat_id,
                user_key,
                RETRIEVAL_KIND_UNFINISHED_GOAL,
                Some(role),
                search_text,
                None,
                Some(&build_chat_scope_metadata_json()),
                (salience + 0.18).clamp(0.0, 1.0),
                RETRIEVAL_SUCCESS_STATE_NEUTRAL,
                None,
                created_at_ts,
            )?;
            return Ok(());
        }

        if role == MEMORY_ROLE_ASSISTANT {
            insert_index_row(
                db,
                RETRIEVAL_SOURCE_MEMORY,
                Some(source_memory_id),
                None,
                Some(&source_ref),
                user_id,
                chat_id,
                user_key,
                RETRIEVAL_KIND_ASSISTANT_RESULT,
                Some(role),
                search_text,
                None,
                Some(&build_chat_scope_metadata_json()),
                (salience + 0.08).clamp(0.0, 1.0),
                RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
                None,
                created_at_ts,
            )?;
            return Ok(());
        }

        if role != MEMORY_ROLE_ASSISTANT {
            insert_index_row(
                db,
                RETRIEVAL_SOURCE_MEMORY,
                Some(source_memory_id),
                None,
                Some(&source_ref),
                user_id,
                chat_id,
                user_key,
                RETRIEVAL_KIND_EPISODIC_EVENT,
                Some(role),
                search_text,
                None,
                Some(&build_chat_scope_metadata_json()),
                salience,
                RETRIEVAL_SUCCESS_STATE_NEUTRAL,
                None,
                created_at_ts,
            )?;
        }

        if role == MEMORY_ROLE_USER && (is_instructional || search_text.chars().count() <= 240) {
            insert_index_row(
                db,
                RETRIEVAL_SOURCE_MEMORY,
                Some(source_memory_id),
                None,
                Some(&source_ref),
                user_id,
                chat_id,
                user_key,
                RETRIEVAL_KIND_TRIGGER_ANCHOR,
                Some(role),
                search_text,
                Some(search_text),
                Some(&build_chat_scope_metadata_json()),
                (salience + 0.08).clamp(0.0, 1.0),
                RETRIEVAL_SUCCESS_STATE_NEUTRAL,
                None,
                created_at_ts,
            )?;
        }

        Ok(())
    })
}

pub(crate) fn upsert_memory_fact_retrieval_row(
    db: &Connection,
    user_id: i64,
    user_key: &str,
    namespace: &str,
    fact_id: i64,
    text: &str,
    confidence: f32,
    ts: i64,
) -> anyhow::Result<()> {
    crate::memory::projection_transaction::atomic(db, |db| {
        let cleaned = text.trim();
        if cleaned.is_empty() {
            return Ok(());
        }
        let source_ref = retrieval_source_ref_for_memory_fact(fact_id);
        if !crate::memory::recall_policy::prepare(
            db,
            user_key,
            crate::memory::recall_policy::Source::Fact(fact_id),
            cleaned,
        )? {
            return Ok(());
        }
        let (scope_kind, scope_ref): (String, Option<String>) = db
            .query_row(
                "SELECT COALESCE(scope_kind, 'principal'), scope_ref
             FROM memory_facts WHERE id = ?1",
                [fact_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .unwrap_or_else(|| ("principal".to_string(), None));
        let metadata =
            build_memory_fact_metadata_json(namespace, fact_id, &scope_kind, scope_ref.as_deref());
        insert_index_row(
            db,
            RETRIEVAL_SOURCE_MEMORY_FACT,
            None,
            None,
            Some(&source_ref),
            user_id,
            0,
            user_key,
            RETRIEVAL_KIND_SEMANTIC_FACT,
            Some(MEMORY_ROLE_SYSTEM),
            cleaned,
            None,
            Some(&metadata),
            confidence.clamp(0.0, 1.0),
            RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
            Some(RETRIEVAL_PRODUCER_MEMORY_PIPELINE),
            ts,
        )?;
        Ok(())
    })
}

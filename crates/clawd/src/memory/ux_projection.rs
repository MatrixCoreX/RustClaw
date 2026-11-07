use super::*;
use crate::memory::{
    indexing, MEMORY_ROLE_SYSTEM, RETRIEVAL_KIND_SEMANTIC_FACT, RETRIEVAL_PRODUCER_MEMORY_PIPELINE,
    RETRIEVAL_SOURCE_MEMORY_FACT, RETRIEVAL_SOURCE_PREFERENCE, RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
};

/// Synchronize one committed-canonical candidate inside its caller's mutation transaction.
pub(super) fn refresh_owned_canonical(
    db: &Connection,
    principal_id: &str,
    memory_id: &str,
    now_ts: i64,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_retrieval_mutation_requires_transaction"
    );
    let memory = load_mutable_memory(db, principal_id, memory_id)?
        .ok_or_else(|| anyhow!("memory_not_found"))?;
    let key = memory
        .snapshot
        .get("user_key")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let user_id = snapshot_i64(&memory.snapshot, "user_id")?;
    let chat_id = snapshot_i64(&memory.snapshot, "chat_id")?;
    let content = snapshot_str(&memory.snapshot, "content")?;
    let salience = snapshot_f64(&memory.snapshot, "salience").unwrap_or(0.8) as f32;
    let (kind, reference, text, trigger, metadata, role, producer) = match memory.kind.as_str() {
        "fact" => {
            anyhow::ensure!(
                snapshot_str(&memory.snapshot, "status")? == "active",
                "memory_retrieval_canonical_not_active"
            );
            let id: i64 = db.query_row(
                "SELECT id FROM memory_facts WHERE memory_id=?1 AND principal_id=?2",
                params![memory_id, principal_id],
                |row| row.get(0),
            )?;
            (
                RETRIEVAL_SOURCE_MEMORY_FACT,
                crate::memory::retrieval_source_ref_for_memory_fact(id),
                content.to_owned(),
                None,
                indexing::build_memory_fact_metadata_json(
                    snapshot_str(&memory.snapshot, "namespace")?,
                    id,
                    &memory.scope_kind,
                    Some(&memory.scope_ref),
                ),
                Some(MEMORY_ROLE_SYSTEM),
                RETRIEVAL_PRODUCER_MEMORY_PIPELINE,
            )
        }
        "preference" => {
            let key = snapshot_str(&memory.snapshot, "item_key")?;
            (
                RETRIEVAL_SOURCE_PREFERENCE,
                crate::memory::retrieval_source_ref_for_preference(key),
                format!("Preference {key}: {content}"),
                Some(key),
                indexing::build_preference_metadata_json(
                    key,
                    &memory.scope_kind,
                    Some(&memory.scope_ref),
                ),
                None,
                snapshot_str(&memory.snapshot, "source")?,
            )
        }
        _ => anyhow::bail!("memory_correction_kind_unsupported"),
    };
    retrieval::remove_canonical(db, &memory)?;
    indexing::insert_index_row(
        db,
        kind,
        None,
        trigger,
        Some(&reference),
        user_id,
        if memory.kind == "fact" { 0 } else { chat_id },
        key,
        RETRIEVAL_KIND_SEMANTIC_FACT,
        role,
        &text,
        trigger,
        Some(&metadata),
        salience.clamp(0.0, 1.0),
        RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
        Some(producer),
        now_ts,
    )?;
    // The shared index writer may refuse an unavailable original source credential.
    // Never commit corrected canonical content without its exact owned projection.
    let index_id: i64 = db
        .query_row(
            "SELECT id FROM memory_retrieval_index WHERE principal_id=?1
         AND source_kind=?2 AND source_ref=?3 AND search_text=?4
           AND scope_kind=?5 AND scope_ref=?6",
            params![
                principal_id,
                kind,
                reference,
                text,
                memory.scope_kind,
                memory.scope_ref
            ],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("memory_retrieval_projection_owner_unavailable"))?;
    let fts_present: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table'
         AND name='memory_retrieval_index_fts')",
        [],
        |row| row.get(0),
    )?;
    if fts_present {
        let projected: Option<String> = db
            .query_row(
                "SELECT search_text FROM memory_retrieval_index_fts WHERE rowid=?1",
                [index_id],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            projected.as_deref() == Some(text.as_str()),
            "memory_retrieval_fts_projection_failed"
        );
    }
    Ok(())
}

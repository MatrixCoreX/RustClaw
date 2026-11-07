use rusqlite::{Transaction, TransactionBehavior};

use super::*;

pub(crate) fn correct_memory(
    db: &Connection,
    principal_id: &str,
    actor_principal_id: &str,
    memory_id: &str,
    request: &MemoryCorrectionRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryMutationResult> {
    require_memory_ux_schema(db)?;
    let content = correction_content(&request.content)?;
    let transaction = db.unchecked_transaction()?;
    let old = load_mutable_memory(&transaction, principal_id, memory_id)?
        .ok_or_else(|| anyhow!("memory_not_found"))?;
    anyhow::ensure!(
        old.revision == request.expected_revision,
        "memory_revision_conflict"
    );
    let result = apply_correction(&transaction, &old, actor_principal_id, content, now_ts)?;
    transaction.commit()?;
    Ok(result)
}

/// The caller owns revision validation and the transaction commit.
fn apply_correction(
    transaction: &Transaction<'_>,
    old: &MutableMemory,
    actor_principal_id: &str,
    content: &str,
    now_ts: i64,
) -> anyhow::Result<MemoryMutationResult> {
    let memory_id = old.id.as_str();
    let principal_id = old.principal_id.as_str();
    let expected_revision = old.revision;
    let replacement_id = format!("memory_{}", uuid::Uuid::new_v4().simple());
    match old.kind.as_str() {
        "fact" => {
            transaction.execute(
                "UPDATE memory_facts SET status = 'superseded', row_revision = row_revision + 1,
                    modified_at_ts = ?2, updated_at_ts = ?2
                 WHERE memory_id = ?1 AND principal_id = ?3 AND row_revision = ?4",
                params![memory_id, now_ts, principal_id, expected_revision],
            )?;
            transaction.execute(
                "INSERT INTO memory_facts(
                    memory_id, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
                    namespace, fact_key, fact_value, fact_text, confidence, source_kind,
                    source_ref, source_memory_ids_json, reason, created_at_ts, updated_at_ts,
                    conflict_group, safety_flag, status, origin, row_revision,
                    legacy_scope_inferred, supersedes_memory_id, trust_tier,
                    observed_at_ts, valid_from_ts, last_verified_at_ts, modified_at_ts
                 ) SELECT ?1, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
                          namespace, fact_key, ?2, ?2, 1.0, 'user_correction', ?3,
                          source_memory_ids_json, 'user_confirmed_correction', ?4, ?4,
                          conflict_group, 'normal', 'active', 'user_confirmed', 1, 0,
                          memory_id, 'user_confirmed', ?4, ?4, ?4, ?4
                   FROM memory_facts WHERE memory_id = ?3 AND principal_id = ?5",
                params![replacement_id, content, memory_id, now_ts, principal_id],
            )?;
            transaction.execute(
                "DELETE FROM memory_retrieval_index
                 WHERE source_kind = 'memory_fact' AND source_ref IN (
                    SELECT CAST(id AS TEXT) FROM memory_facts WHERE memory_id = ?1
                 )",
                [memory_id],
            )?;
        }
        "preference" => {
            transaction.execute(
                "UPDATE user_preferences SET pref_value = ?1, confidence = 1.0,
                    source = 'user_correction', origin = 'user_confirmed',
                    row_revision = row_revision + 1, modified_at_ts = ?2,
                    updated_at_ts = ?2, updated_at = ?3, supersedes_memory_id = memory_id,
                    memory_id = ?4
                 WHERE memory_id = ?5 AND principal_id = ?6 AND row_revision = ?7",
                params![
                    content,
                    now_ts,
                    now_ts.to_string(),
                    replacement_id,
                    memory_id,
                    principal_id,
                    expected_revision,
                ],
            )?;
        }
        _ => anyhow::bail!("memory_correction_kind_unsupported"),
    }
    let revision_id = append_revision(
        transaction,
        old,
        "correct",
        actor_principal_id,
        Some(&replacement_id),
        Some(now_ts + UNDO_GRACE_SECONDS),
        now_ts,
    )?;
    Ok(MemoryMutationResult {
        status: "corrected".to_string(),
        memory_id: memory_id.to_string(),
        replacement_memory_id: Some(replacement_id),
        revision: 1,
        revision_id: Some(revision_id),
        undo_until_ts: Some(now_ts + UNDO_GRACE_SECONDS),
    })
}

pub(crate) fn record_feedback(
    db: &Connection,
    principal_id: &str,
    memory_id: &str,
    request: &MemoryFeedbackRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryMutationResult> {
    require_memory_ux_schema(db)?;
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    let current = load_mutable_memory(&transaction, principal_id, memory_id)?
        .ok_or_else(|| anyhow!("memory_not_found"))?;
    anyhow::ensure!(
        current.revision == request.expected_revision,
        "memory_revision_conflict"
    );
    let corrected = if matches!(request.feedback_kind, MemoryFeedbackKind::Incorrect) {
        Some(correction_content(
            request
                .corrected_content
                .as_deref()
                .ok_or_else(|| anyhow!("memory_correction_content_required"))?,
        )?)
    } else {
        None
    };
    transaction.execute(
        "INSERT INTO memory_retrieval_feedback(
            feedback_id, principal_id, memory_id, feedback_kind, retrieval_event_ref,
            expected_revision, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            format!("memory_feedback_{}", uuid::Uuid::new_v4().simple()),
            principal_id,
            memory_id,
            request.feedback_kind.as_str(),
            request.retrieval_event_ref.as_deref(),
            request.expected_revision,
            now_ts,
        ],
    )?;
    let result = if let Some(content) = corrected {
        apply_correction(&transaction, &current, principal_id, content, now_ts)?
    } else {
        if matches!(request.feedback_kind, MemoryFeedbackKind::DoNotUse) {
            disable_memory_recall(&transaction, &current, now_ts)?;
        }
        MemoryMutationResult {
            status: request.feedback_kind.as_str().to_string(),
            memory_id: memory_id.to_string(),
            replacement_memory_id: None,
            revision: current.revision,
            revision_id: None,
            undo_until_ts: None,
        }
    };
    transaction.commit()?;
    Ok(result)
}

fn correction_content(content: &str) -> anyhow::Result<&str> {
    let content = content.trim();
    anyhow::ensure!(!content.is_empty(), "memory_correction_content_required");
    Ok(content)
}

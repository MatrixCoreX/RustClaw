use anyhow::anyhow;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

use super::{digest_text, MemoryListItem};

#[derive(Debug, PartialEq)]
pub(in crate::memory) enum Decision {
    Skipped,
    Duplicate,
    Accepted(Value),
}

/// Shared filtering preserves legacy policy when an explicit character limit is
/// supplied. Streamed imports use their advertised resource budget instead.
pub(in crate::memory) fn prepare(
    db: &Connection,
    principal_id: &str,
    item: &MemoryListItem,
    character_limit: Option<usize>,
) -> anyhow::Result<Decision> {
    if !matches!(item.kind.as_str(), "fact" | "preference" | "recent")
        || item.content.trim().is_empty()
        || character_limit.is_some_and(|limit| item.content.chars().count() > limit)
    {
        return Ok(Decision::Skipped);
    }
    let (safe_content, redacted) =
        crate::skill_output_artifact::sensitivity_aware_text_model_view(&item.content);
    if redacted || safe_content.trim().is_empty() {
        return Ok(Decision::Skipped);
    }
    let content_digest = digest_text(&safe_content);
    let exists: bool = db.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM memory_facts
             WHERE principal_id = ?1 AND status = 'active'
               AND (content_digest = ?2 OR fact_text = ?3)
            UNION ALL
            SELECT 1 FROM user_preferences
             WHERE principal_id = ?1
               AND (content_digest = ?2 OR pref_value = ?3)
        )",
        params![principal_id, content_digest, safe_content],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(Decision::Duplicate);
    }
    Ok(Decision::Accepted(json!({
        "kind": item.kind,
        "content": safe_content,
        "content_digest": content_digest,
        "source_memory_id": item.id,
        "source_scope_kind": item.scope_kind,
        "source_origin": item.origin,
    })))
}

/// The caller owns one whole-import transaction and growth/identity admission.
/// A single item must never accidentally become an independently committed import.
pub(in crate::memory) fn insert(
    db: &Connection,
    principal_id: &str,
    user_key: &str,
    import_id: &str,
    item: &Value,
    now_ts: i64,
) -> anyhow::Result<usize> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_import_item_requires_transaction"
    );
    let content = item["content"]
        .as_str()
        .ok_or_else(|| anyhow!("memory_import_preview_corrupt"))?;
    let content_digest = item["content_digest"]
        .as_str()
        .ok_or_else(|| anyhow!("memory_import_preview_corrupt"))?;
    let idempotency_key = format!("import:{content_digest}");
    anyhow::ensure!(
        matches!(
            item["kind"].as_str(),
            Some("fact" | "preference" | "recent")
        ) && !content.trim().is_empty()
            && digest_text(content) == content_digest,
        "memory_import_preview_corrupt"
    );
    let changed = if item["kind"].as_str() == Some("preference") {
        let item_key = format!(
            "imported_{}",
            &content_digest[7..23.min(content_digest.len())]
        );
        db.execute(
            "INSERT INTO user_preferences(
            memory_id, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
            pref_key, pref_value, confidence, source, updated_at, updated_at_ts,
            origin, row_revision, legacy_scope_inferred, content_digest,
            idempotency_key, source_refs_json, evidence_refs_json, trust_tier,
            sensitivity, last_verified_at_ts, modified_at_ts
         ) SELECT ?1, 0, 0, ?2, ?3, 'principal', ?3, ?4, ?5, 0.5,
                  'memory_import', ?6, ?7, 'imported_legacy', 1, 0, ?8, ?9,
                  '[]', '[]', 'imported_legacy', 'normal', ?7, ?7
           WHERE NOT EXISTS(
               SELECT 1 FROM user_preferences
                WHERE principal_id = ?3 AND content_digest = ?8
           )",
            params![
                format!("memory_{}", uuid::Uuid::new_v4().simple()),
                user_key,
                principal_id,
                item_key,
                content,
                now_ts.to_string(),
                now_ts,
                content_digest,
                idempotency_key,
            ],
        )?
    } else {
        let item_key = format!(
            "imported_{}",
            &content_digest[7..23.min(content_digest.len())]
        );
        db.execute(
            "INSERT INTO memory_facts(
            memory_id, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
            namespace, fact_key, fact_value, fact_text, confidence, source_kind,
            source_ref, source_memory_ids_json, reason, created_at_ts, updated_at_ts,
            safety_flag, status, origin, row_revision, legacy_scope_inferred,
            content_digest, idempotency_key, source_refs_json, evidence_refs_json,
            trust_tier, sensitivity, last_verified_at_ts, modified_at_ts
         ) VALUES (?1, 0, 0, ?2, ?3, 'principal', ?3, 'imported_memory', ?4,
                   ?5, ?5, 0.5, 'memory_import', ?6, '[]', 'import_preview_confirmed',
                   ?7, ?7, 'normal', 'active', 'imported_legacy', 1, 0, ?8, ?9,
                   '[]', '[]', 'imported_legacy', 'normal', ?7, ?7)
         ON CONFLICT(principal_id, scope_kind, scope_ref, idempotency_key)
         WHERE idempotency_key IS NOT NULL AND status = 'active' DO NOTHING",
            params![
                format!("memory_{}", uuid::Uuid::new_v4().simple()),
                user_key,
                principal_id,
                item_key,
                content,
                import_id,
                now_ts,
                content_digest,
                idempotency_key,
            ],
        )?
    };
    Ok(changed)
}

#[cfg(test)]
#[path = "ux_import_items_tests.rs"]
mod tests;

use anyhow::anyhow;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MIGRATION_ID: &str = "012_memory_ux_audit_v1";
const MIGRATION_SQL: &str = include_str!("../../../../migrations/012_memory_ux_audit.sql");
const UNDO_GRACE_SECONDS: i64 = 300;

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct MemoryListFilter {
    pub(crate) search: Option<String>,
    pub(crate) scope: Option<String>,
    pub(crate) origin: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) freshness: Option<String>,
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct MemoryListItem {
    pub(crate) id: String,
    pub(crate) revision: i64,
    pub(crate) kind: String,
    pub(crate) scope_kind: String,
    pub(crate) origin: String,
    pub(crate) status: String,
    pub(crate) content: String,
    pub(crate) source: String,
    pub(crate) evidence_available: bool,
    pub(crate) trust_tier: String,
    pub(crate) updated_at_ts: i64,
    pub(crate) expires_at_ts: Option<i64>,
    pub(crate) supersedes_memory_id: Option<String>,
    pub(crate) last_recalled_at_ts: Option<i64>,
    pub(crate) freshness: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryPageResult {
    pub(crate) schema_version: u32,
    pub(crate) items: Vec<MemoryListItem>,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
    pub(crate) total: usize,
    pub(crate) has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MemoryCorrectionRequest {
    pub(crate) expected_revision: i64,
    pub(crate) content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryFeedbackKind {
    Incorrect,
    Irrelevant,
    DoNotUse,
}

impl MemoryFeedbackKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Incorrect => "incorrect",
            Self::Irrelevant => "irrelevant",
            Self::DoNotUse => "do_not_use",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MemoryFeedbackRequest {
    pub(crate) expected_revision: i64,
    pub(crate) feedback_kind: MemoryFeedbackKind,
    pub(crate) retrieval_event_ref: Option<String>,
    pub(crate) corrected_content: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MemoryMutationRequest {
    pub(crate) expected_revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MemoryUndoRequest {
    pub(crate) revision_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryMutationResult {
    pub(crate) status: String,
    pub(crate) memory_id: String,
    pub(crate) replacement_memory_id: Option<String>,
    pub(crate) revision: i64,
    pub(crate) revision_id: Option<String>,
    pub(crate) undo_until_ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryClearPreview {
    pub(crate) schema_version: u32,
    pub(crate) mode: String,
    pub(crate) transcript_rows: i64,
    pub(crate) derived_rows: i64,
    pub(crate) pending_jobs: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MemoryBulkClearRequest {
    pub(crate) mode: String,
    pub(crate) expected_transcript_rows: i64,
    pub(crate) expected_derived_rows: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemoryExport {
    pub(crate) schema_version: u32,
    pub(crate) exported_at_ts: i64,
    pub(crate) scope_kind: String,
    pub(crate) items: Vec<MemoryListItem>,
    pub(crate) checksum: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MemoryMarkdownExport {
    pub(crate) schema_version: u32,
    pub(crate) exported_at_ts: i64,
    pub(crate) content_type: String,
    pub(crate) content: String,
    pub(crate) checksum: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MemoryImportPreviewRequest {
    pub(crate) export: MemoryExport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct MemoryImportPreview {
    pub(crate) schema_version: u32,
    pub(crate) import_id: String,
    pub(crate) payload_digest: String,
    pub(crate) accepted_items: usize,
    pub(crate) skipped_items: usize,
    pub(crate) duplicate_items: usize,
    pub(crate) trust_tier: String,
    pub(crate) scope_kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MemoryImportConfirmRequest {
    pub(crate) import_id: String,
    pub(crate) expected_payload_digest: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryImportResult {
    pub(crate) schema_version: u32,
    pub(crate) import_id: String,
    pub(crate) status: String,
    pub(crate) imported_items: usize,
    pub(crate) existing_items: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryConsistencyReport {
    pub(crate) schema_version: u32,
    pub(crate) deleted_canonical_rows: i64,
    pub(crate) deleted_retrieval_rows: i64,
    pub(crate) orphan_retrieval_rows: i64,
    pub(crate) stale_jobs: i64,
    pub(crate) repaired_rows: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RemoteMemoryDisclosure {
    pub(crate) schema_version: u32,
    pub(crate) consent_state: String,
    pub(crate) extraction_provider: String,
    pub(crate) extraction_model: String,
    pub(crate) consolidation_provider: String,
    pub(crate) consolidation_model: String,
    pub(crate) extraction_sends: Vec<String>,
    pub(crate) embedding_sends: Vec<String>,
    pub(crate) withdrawal_effect: String,
}

pub(crate) fn remote_memory_disclosure(
    state: &crate::AppState,
    principal_id: &str,
) -> anyhow::Result<RemoteMemoryDisclosure> {
    let db = state.core.db.get().map_err(|error| anyhow!(error))?;
    let settings = super::settings::resolve_principal_memory_settings(
        &db,
        principal_id,
        state.policy.memory.long_term_enabled,
    )?;
    let inherited = state.core.llm_providers.first();
    let inherited_provider = inherited
        .map(|provider| provider.config.provider_type.clone())
        .unwrap_or_else(|| "unavailable".to_string());
    let inherited_model = inherited
        .map(|provider| provider.config.model.clone())
        .unwrap_or_else(|| "unavailable".to_string());
    Ok(RemoteMemoryDisclosure {
        schema_version: 1,
        consent_state: settings.external_context_policy.as_str().to_string(),
        extraction_provider: nonempty_or(
            &state.policy.memory.extract_provider,
            &inherited_provider,
        ),
        extraction_model: nonempty_or(&state.policy.memory.extract_model, &inherited_model),
        consolidation_provider: nonempty_or(
            &state.policy.memory.consolidation_provider,
            &inherited_provider,
        ),
        consolidation_model: nonempty_or(
            &state.policy.memory.consolidation_model,
            &inherited_model,
        ),
        extraction_sends: vec![
            "eligible_user_excerpt".to_string(),
            "redacted_evidence_reference".to_string(),
        ],
        embedding_sends: if state.policy.memory.embedding_model.starts_with("local-") {
            Vec::new()
        } else {
            vec![
                "searchable_memory_projection".to_string(),
                "consented_query_text".to_string(),
            ]
        },
        withdrawal_effect: "stop_new_requests_cancel_jobs_remove_remote_profiles".to_string(),
    })
}

pub(crate) fn ensure_memory_ux_schema(db: &Connection) -> anyhow::Result<()> {
    super::jobs::ensure_memory_job_schema(db)?;
    if let Some(applied) = db
        .query_row(
            "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
            [MIGRATION_ID],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        anyhow::ensure!(
            applied == migration_digest(),
            "memory_ux_migration_digest_mismatch"
        );
    }
    db.execute_batch(MIGRATION_SQL)?;
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3) ON CONFLICT(migration_id) DO NOTHING",
        params![MIGRATION_ID, migration_digest(), crate::now_ts()],
    )?;
    Ok(())
}

pub(crate) fn list_memory_page(
    db: &Connection,
    principal_id: &str,
    filter: &MemoryListFilter,
    now_ts: i64,
) -> anyhow::Result<MemoryPageResult> {
    ensure_memory_ux_schema(db)?;
    let page = filter.page.unwrap_or(1).max(1);
    let page_size = filter.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1).saturating_mul(page_size);
    let search = filter.search.as_deref().unwrap_or("").trim();
    let scope = filter.scope.as_deref().unwrap_or("").trim();
    let origin = filter.origin.as_deref().unwrap_or("").trim();
    let kind = filter.kind.as_deref().unwrap_or("").trim();
    let status = filter.status.as_deref().unwrap_or("").trim();
    let freshness = filter.freshness.as_deref().unwrap_or("").trim();
    let stale_cutoff = now_ts.saturating_sub(30 * 86_400);
    let union = memory_union_sql();
    let filtered = format!(
        "SELECT * FROM ({union}) item
         WHERE principal_id = ?1
           AND (?2 = '' OR content LIKE '%' || ?2 || '%' ESCAPE '\\')
           AND (?3 = '' OR scope_kind = ?3)
           AND (?4 = '' OR origin = ?4)
           AND (?5 = '' OR kind = ?5)
           AND (?6 = '' OR status = ?6)
           AND (?7 = '' OR (?7 = 'stale' AND COALESCE(last_verified_at_ts, updated_at_ts) < ?8)
                        OR (?7 = 'fresh' AND COALESCE(last_verified_at_ts, updated_at_ts) >= ?8))"
    );
    let total = db.query_row(
        &format!("SELECT COUNT(*) FROM ({filtered})"),
        params![
            principal_id,
            search,
            scope,
            origin,
            kind,
            status,
            freshness,
            stale_cutoff
        ],
        |row| row.get::<_, i64>(0),
    )? as usize;
    let mut statement = db.prepare(&format!(
        "{filtered} ORDER BY updated_at_ts DESC, id DESC LIMIT ?9 OFFSET ?10"
    ))?;
    let rows = statement.query_map(
        params![
            principal_id,
            search,
            scope,
            origin,
            kind,
            status,
            freshness,
            stale_cutoff,
            page_size as i64,
            offset as i64,
        ],
        |row| {
            let updated_at_ts = row.get::<_, i64>(9)?;
            let last_verified_at_ts = row.get::<_, Option<i64>>(13)?;
            Ok(MemoryListItem {
                id: row.get(0)?,
                revision: row.get(1)?,
                kind: row.get(2)?,
                scope_kind: row.get(3)?,
                origin: row.get(4)?,
                status: row.get(5)?,
                content: row.get(6)?,
                source: row.get(7)?,
                evidence_available: row.get::<_, i64>(8)? != 0,
                trust_tier: row.get(14)?,
                updated_at_ts,
                expires_at_ts: row.get(10)?,
                supersedes_memory_id: row.get(11)?,
                last_recalled_at_ts: row.get(12)?,
                freshness: if last_verified_at_ts.unwrap_or(updated_at_ts) < stale_cutoff {
                    "stale".to_string()
                } else {
                    "fresh".to_string()
                },
            })
        },
    )?;
    let items = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(MemoryPageResult {
        schema_version: 1,
        items,
        page,
        page_size,
        total,
        has_more: offset.saturating_add(page_size) < total,
    })
}

pub(crate) fn correct_memory(
    db: &Connection,
    principal_id: &str,
    actor_principal_id: &str,
    memory_id: &str,
    request: &MemoryCorrectionRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryMutationResult> {
    ensure_memory_ux_schema(db)?;
    let content = request.content.trim();
    anyhow::ensure!(!content.is_empty(), "memory_correction_content_required");
    let transaction = db.unchecked_transaction()?;
    let old = load_mutable_memory(&transaction, principal_id, memory_id)?
        .ok_or_else(|| anyhow!("memory_not_found"))?;
    anyhow::ensure!(
        old.revision == request.expected_revision,
        "memory_revision_conflict"
    );
    let replacement_id = format!("memory_{}", uuid::Uuid::new_v4().simple());
    match old.kind.as_str() {
        "fact" => {
            transaction.execute(
                "UPDATE memory_facts SET status = 'superseded', row_revision = row_revision + 1,
                    modified_at_ts = ?2, updated_at_ts = ?2
                 WHERE memory_id = ?1 AND principal_id = ?3 AND row_revision = ?4",
                params![memory_id, now_ts, principal_id, request.expected_revision],
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
                    request.expected_revision,
                ],
            )?;
        }
        _ => anyhow::bail!("memory_correction_kind_unsupported"),
    }
    let revision_id = append_revision(
        &transaction,
        &old,
        "correct",
        actor_principal_id,
        Some(&replacement_id),
        Some(now_ts + UNDO_GRACE_SECONDS),
        now_ts,
    )?;
    transaction.commit()?;
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
    ensure_memory_ux_schema(db)?;
    let current = load_mutable_memory(db, principal_id, memory_id)?
        .ok_or_else(|| anyhow!("memory_not_found"))?;
    anyhow::ensure!(
        current.revision == request.expected_revision,
        "memory_revision_conflict"
    );
    db.execute(
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
    match request.feedback_kind {
        MemoryFeedbackKind::Incorrect => {
            let content = request
                .corrected_content
                .as_deref()
                .ok_or_else(|| anyhow!("memory_correction_content_required"))?;
            return correct_memory(
                db,
                principal_id,
                principal_id,
                memory_id,
                &MemoryCorrectionRequest {
                    expected_revision: request.expected_revision,
                    content: content.to_string(),
                },
                now_ts,
            );
        }
        MemoryFeedbackKind::DoNotUse => disable_memory_recall(db, &current, now_ts)?,
        MemoryFeedbackKind::Irrelevant => {}
    }
    Ok(MemoryMutationResult {
        status: request.feedback_kind.as_str().to_string(),
        memory_id: memory_id.to_string(),
        replacement_memory_id: None,
        revision: current.revision,
        revision_id: None,
        undo_until_ts: None,
    })
}

pub(crate) fn delete_memory_with_revision(
    db: &Connection,
    principal_id: &str,
    memory_id: &str,
    request: &MemoryMutationRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryMutationResult> {
    ensure_memory_ux_schema(db)?;
    let transaction = db.unchecked_transaction()?;
    let current = load_mutable_memory(&transaction, principal_id, memory_id)?
        .ok_or_else(|| anyhow!("memory_not_found"))?;
    anyhow::ensure!(
        current.revision == request.expected_revision,
        "memory_revision_conflict"
    );
    let revision_id = append_revision(
        &transaction,
        &current,
        "delete",
        principal_id,
        None,
        Some(now_ts + UNDO_GRACE_SECONDS),
        now_ts,
    )?;
    let raw_source_id = if current.kind == "recent" {
        transaction
            .query_row(
                "SELECT id FROM memories WHERE memory_id = ?1 AND principal_id = ?2",
                params![memory_id, principal_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
    } else {
        None
    };
    super::jobs::request_cancel_for_scope(&transaction, principal_id, Some(&current.scope_ref))?;
    if let Some(source_id) = raw_source_id {
        transaction.execute(
            "DELETE FROM memory_facts
             WHERE principal_id = ?1 AND EXISTS (
               SELECT 1 FROM json_each(memory_facts.source_memory_ids_json)
               WHERE CAST(value AS INTEGER) = ?2
             )",
            params![principal_id, source_id],
        )?;
        transaction.execute(
            "DELETE FROM memory_source_events WHERE principal_id = ?1 AND source_memory_id = ?2",
            params![principal_id, source_id],
        )?;
        transaction.execute(
            "UPDATE memory_evidence SET availability = 'purged', redacted_excerpt = NULL
             WHERE principal_id = ?1 AND source_type = 'memory' AND source_ref = ?2",
            params![principal_id, source_id.to_string()],
        )?;
        transaction.execute(
            "DELETE FROM long_term_memories WHERE principal_id = ?1 AND scope_ref = ?2",
            params![principal_id, current.scope_ref],
        )?;
    }
    match current.kind.as_str() {
        "fact" => {
            transaction.execute(
                "DELETE FROM memory_facts WHERE memory_id = ?1 AND principal_id = ?2",
                params![memory_id, principal_id],
            )?;
        }
        "preference" => {
            transaction.execute(
                "DELETE FROM user_preferences WHERE memory_id = ?1 AND principal_id = ?2",
                params![memory_id, principal_id],
            )?;
        }
        "recent" => {
            transaction.execute(
                "DELETE FROM memories WHERE memory_id = ?1 AND principal_id = ?2",
                params![memory_id, principal_id],
            )?;
        }
        _ => anyhow::bail!("memory_kind_unsupported"),
    }
    transaction.execute(
        "DELETE FROM memory_retrieval_index
         WHERE principal_id = ?1 AND (
            source_ref = ?2 OR source_memory_id = ?3
         )",
        params![principal_id, memory_id, raw_source_id],
    )?;
    let _ = transaction.execute(
        "DELETE FROM memory_retrieval_index_fts
         WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
        [],
    );
    transaction.execute(
        "INSERT INTO memory_privacy_purge_queue(
            purge_id, principal_id, memory_id, object_kind, purge_after_ts, status, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'grace', ?6)",
        params![
            format!("memory_purge_{}", uuid::Uuid::new_v4().simple()),
            principal_id,
            memory_id,
            current.kind,
            now_ts + UNDO_GRACE_SECONDS,
            now_ts,
        ],
    )?;
    transaction.commit()?;
    Ok(MemoryMutationResult {
        status: "deleted".to_string(),
        memory_id: memory_id.to_string(),
        replacement_memory_id: None,
        revision: request.expected_revision + 1,
        revision_id: Some(revision_id),
        undo_until_ts: Some(now_ts + UNDO_GRACE_SECONDS),
    })
}

pub(crate) fn undo_memory_mutation(
    db: &Connection,
    principal_id: &str,
    request: &MemoryUndoRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryMutationResult> {
    ensure_memory_ux_schema(db)?;
    let transaction = db.unchecked_transaction()?;
    let revision = transaction
        .query_row(
            "SELECT memory_id, object_kind, row_revision, operation,
                    previous_snapshot_json, replacement_memory_id, undo_expires_at_ts
             FROM memory_revisions
             WHERE revision_id = ?1 AND principal_id = ?2",
            params![request.revision_id, principal_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("memory_revision_not_found"))?;
    let (memory_id, object_kind, row_revision, operation, snapshot_json, replacement_id, expiry) =
        revision;
    anyhow::ensure!(
        expiry.is_some_and(|value| value >= now_ts),
        "memory_undo_expired"
    );
    let snapshot: Value = serde_json::from_str(&snapshot_json)?;
    anyhow::ensure!(
        snapshot.get("purged").and_then(Value::as_bool) != Some(true),
        "memory_undo_expired"
    );
    match operation.as_str() {
        "correct" => undo_correction(
            &transaction,
            principal_id,
            &memory_id,
            &object_kind,
            replacement_id.as_deref(),
            row_revision,
            &snapshot,
            now_ts,
        )?,
        "delete" => restore_deleted_memory(
            &transaction,
            principal_id,
            &memory_id,
            &object_kind,
            row_revision,
            &snapshot,
            now_ts,
        )?,
        _ => anyhow::bail!("memory_undo_operation_unsupported"),
    }
    transaction.execute(
        "UPDATE memory_privacy_purge_queue
         SET status = 'cancelled', completed_at_ts = ?3
         WHERE principal_id = ?1 AND memory_id = ?2 AND status = 'grace'",
        params![principal_id, memory_id, now_ts],
    )?;
    let restore_revision_id = format!("memory_revision_{}", uuid::Uuid::new_v4().simple());
    transaction.execute(
        "INSERT INTO memory_revisions(
            revision_id, memory_id, principal_id, scope_kind, scope_ref, object_kind,
            row_revision, operation, previous_snapshot_json, replacement_memory_id,
            actor_principal_id, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'restore',
                   '{\"schema_version\":1,\"restore\":true}', NULL, ?3, ?8)",
        params![
            restore_revision_id,
            memory_id,
            principal_id,
            snapshot_str(&snapshot, "scope_kind")?,
            snapshot_str(&snapshot, "scope_ref")?,
            object_kind,
            row_revision + 1,
            now_ts,
        ],
    )?;
    transaction.execute(
        "UPDATE memory_revisions SET undo_expires_at_ts = NULL WHERE revision_id = ?1",
        [request.revision_id.as_str()],
    )?;
    transaction.commit()?;
    Ok(MemoryMutationResult {
        status: "restored".to_string(),
        memory_id,
        replacement_memory_id: None,
        revision: row_revision + 2,
        revision_id: Some(restore_revision_id),
        undo_until_ts: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn undo_correction(
    db: &Connection,
    principal_id: &str,
    memory_id: &str,
    object_kind: &str,
    replacement_id: Option<&str>,
    old_revision: i64,
    snapshot: &Value,
    now_ts: i64,
) -> anyhow::Result<()> {
    let replacement_id = replacement_id.ok_or_else(|| anyhow!("memory_replacement_missing"))?;
    match object_kind {
        "fact" => {
            db.execute(
                "DELETE FROM memory_facts WHERE memory_id = ?1 AND principal_id = ?2",
                params![replacement_id, principal_id],
            )?;
            let updated = db.execute(
                "UPDATE memory_facts SET status = 'active', row_revision = ?3,
                    modified_at_ts = ?4, updated_at_ts = ?4
                 WHERE memory_id = ?1 AND principal_id = ?2 AND status = 'superseded'",
                params![memory_id, principal_id, old_revision + 2, now_ts],
            )?;
            anyhow::ensure!(updated == 1, "memory_undo_state_conflict");
        }
        "preference" => {
            let updated = db.execute(
                "UPDATE user_preferences SET memory_id = ?1, pref_value = ?2,
                    origin = ?3, row_revision = ?4, supersedes_memory_id = NULL,
                    modified_at_ts = ?5, updated_at_ts = ?5, updated_at = ?6
                 WHERE memory_id = ?7 AND principal_id = ?8",
                params![
                    memory_id,
                    snapshot_str(snapshot, "content")?,
                    snapshot_str(snapshot, "origin")?,
                    old_revision + 2,
                    now_ts,
                    now_ts.to_string(),
                    replacement_id,
                    principal_id,
                ],
            )?;
            anyhow::ensure!(updated == 1, "memory_undo_state_conflict");
        }
        _ => anyhow::bail!("memory_undo_kind_unsupported"),
    }
    Ok(())
}

fn restore_deleted_memory(
    db: &Connection,
    principal_id: &str,
    memory_id: &str,
    object_kind: &str,
    old_revision: i64,
    snapshot: &Value,
    now_ts: i64,
) -> anyhow::Result<()> {
    let scope_kind = snapshot_str(snapshot, "scope_kind")?;
    let scope_ref = snapshot_str(snapshot, "scope_ref")?;
    let content = snapshot_str(snapshot, "content")?;
    let origin = snapshot_str(snapshot, "origin")?;
    let user_id = snapshot_i64(snapshot, "user_id")?;
    let chat_id = snapshot_i64(snapshot, "chat_id")?;
    let user_key = snapshot
        .get("user_key")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match object_kind {
        "fact" => {
            db.execute(
                "INSERT INTO memory_facts(
                    memory_id, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
                    namespace, fact_key, fact_value, fact_text, confidence, source_kind,
                    source_ref, source_memory_ids_json, reason, created_at_ts, updated_at_ts,
                    safety_flag, status, origin, row_revision, legacy_scope_inferred,
                    source_refs_json, evidence_refs_json, trust_tier, sensitivity,
                    last_verified_at_ts, modified_at_ts
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11,
                           ?12, 'undo_restore', '[]', 'user_undo', ?13, ?14, ?15,
                           'active', ?16, ?17, 0, '[]', '[]', 'user_confirmed',
                           'normal', ?14, ?14)",
                params![
                    memory_id,
                    user_id,
                    chat_id,
                    user_key,
                    principal_id,
                    scope_kind,
                    scope_ref,
                    snapshot_str(snapshot, "namespace")?,
                    snapshot_str(snapshot, "item_key")?,
                    content,
                    snapshot_f64(snapshot, "salience").unwrap_or(1.0),
                    snapshot_str(snapshot, "source")?,
                    snapshot_i64(snapshot, "created_at_ts").unwrap_or(now_ts),
                    now_ts,
                    snapshot_str(snapshot, "safety_flag")?,
                    origin,
                    old_revision + 2,
                ],
            )?;
        }
        "preference" => {
            db.execute(
                "INSERT INTO user_preferences(
                    memory_id, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
                    pref_key, pref_value, confidence, source, updated_at, updated_at_ts,
                    origin, row_revision, legacy_scope_inferred, source_refs_json,
                    evidence_refs_json, trust_tier, sensitivity, last_verified_at_ts,
                    modified_at_ts
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                           ?13, ?14, ?15, 0, '[]', '[]', 'user_confirmed', 'normal',
                           ?13, ?13)",
                params![
                    memory_id,
                    user_id,
                    chat_id,
                    user_key,
                    principal_id,
                    scope_kind,
                    scope_ref,
                    snapshot_str(snapshot, "item_key")?,
                    content,
                    snapshot_f64(snapshot, "salience").unwrap_or(1.0),
                    snapshot_str(snapshot, "source")?,
                    now_ts.to_string(),
                    now_ts,
                    origin,
                    old_revision + 2,
                ],
            )?;
        }
        "recent" => {
            db.execute(
                "INSERT INTO memories(
                    memory_id, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
                    channel, external_chat_id, role, content, created_at, created_at_ts,
                    memory_type, salience, safety_flag, origin, row_revision,
                    legacy_scope_inferred
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                           ?12, ?13, ?14, ?15, ?16, ?17, ?18, 0)",
                params![
                    memory_id,
                    user_id,
                    chat_id,
                    user_key,
                    principal_id,
                    scope_kind,
                    scope_ref,
                    snapshot_str(snapshot, "channel")?,
                    snapshot.get("external_chat_id").and_then(Value::as_str),
                    snapshot_str(snapshot, "role")?,
                    content,
                    snapshot_str(snapshot, "created_at")?,
                    snapshot_i64(snapshot, "created_at_ts")?,
                    snapshot_str(snapshot, "memory_type")?,
                    snapshot_f64(snapshot, "salience").unwrap_or(0.5),
                    snapshot_str(snapshot, "safety_flag")?,
                    origin,
                    old_revision + 2,
                ],
            )?;
        }
        _ => anyhow::bail!("memory_undo_kind_unsupported"),
    }
    Ok(())
}

pub(crate) fn scrub_expired_deletion_grace(db: &Connection, now_ts: i64) -> anyhow::Result<usize> {
    ensure_memory_ux_schema(db)?;
    let transaction = db.unchecked_transaction()?;
    let revision_ids = {
        let mut statement = transaction.prepare(
            "SELECT r.revision_id
             FROM memory_revisions r
             JOIN memory_privacy_purge_queue p
               ON p.principal_id = r.principal_id AND p.memory_id = r.memory_id
             WHERE p.status = 'grace' AND p.purge_after_ts <= ?1
               AND r.operation = 'delete'",
        )?;
        let rows = statement
            .query_map([now_ts], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for revision_id in &revision_ids {
        transaction.execute(
            "UPDATE memory_revisions
             SET previous_snapshot_json = '{\"schema_version\":1,\"purged\":true}',
                 undo_expires_at_ts = NULL
             WHERE revision_id = ?1",
            [revision_id],
        )?;
    }
    transaction.execute(
        "UPDATE memory_privacy_purge_queue
         SET status = 'purged', completed_at_ts = ?1
         WHERE status = 'grace' AND purge_after_ts <= ?1",
        [now_ts],
    )?;
    transaction.commit()?;
    Ok(revision_ids.len())
}

pub(crate) fn clear_preview(
    db: &Connection,
    principal_id: &str,
    mode: &str,
) -> anyhow::Result<MemoryClearPreview> {
    ensure_memory_ux_schema(db)?;
    anyhow::ensure!(
        matches!(mode, "transcript" | "transcript_and_derived"),
        "memory_clear_mode_invalid"
    );
    let transcript_rows = db.query_row(
        "SELECT COUNT(*) FROM memories WHERE principal_id = ?1",
        [principal_id],
        |row| row.get(0),
    )?;
    let derived_rows = if mode == "transcript_and_derived" {
        db.query_row(
            "SELECT (SELECT COUNT(*) FROM memory_facts WHERE principal_id = ?1)
                  + (SELECT COUNT(*) FROM user_preferences WHERE principal_id = ?1)
                  + (SELECT COUNT(*) FROM long_term_memories WHERE principal_id = ?1)",
            [principal_id],
            |row| row.get(0),
        )?
    } else {
        0
    };
    let pending_jobs = db.query_row(
        "SELECT COUNT(*) FROM memory_jobs WHERE principal_id = ?1
           AND status IN ('queued', 'retry_wait', 'running')",
        [principal_id],
        |row| row.get(0),
    )?;
    Ok(MemoryClearPreview {
        schema_version: 1,
        mode: mode.to_string(),
        transcript_rows,
        derived_rows,
        pending_jobs,
    })
}

pub(crate) fn clear_with_mode(
    db: &Connection,
    principal_id: &str,
    request: &MemoryBulkClearRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryClearPreview> {
    ensure_memory_ux_schema(db)?;
    let preview = clear_preview(db, principal_id, &request.mode)?;
    anyhow::ensure!(
        preview.transcript_rows == request.expected_transcript_rows
            && preview.derived_rows == request.expected_derived_rows,
        "memory_clear_preview_conflict"
    );
    let transaction = db.unchecked_transaction()?;
    super::jobs::request_cancel_for_scope(&transaction, principal_id, None)?;
    transaction.execute(
        "DELETE FROM memory_retrieval_index WHERE principal_id = ?1",
        [principal_id],
    )?;
    transaction.execute(
        "DELETE FROM memory_source_events WHERE principal_id = ?1",
        [principal_id],
    )?;
    transaction.execute(
        "DELETE FROM memories WHERE principal_id = ?1",
        [principal_id],
    )?;
    if request.mode == "transcript_and_derived" {
        for table in [
            "memory_facts",
            "user_preferences",
            "long_term_memories",
            "memory_raw_candidates",
            "memory_evidence",
        ] {
            transaction.execute(
                &format!("DELETE FROM {table} WHERE principal_id = ?1"),
                [principal_id],
            )?;
        }
    }
    let _ = transaction.execute(
        "DELETE FROM memory_retrieval_index_fts
         WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
        [],
    );
    transaction.execute(
        "INSERT INTO memory_retention_ledger(
            ledger_id, principal_id, scope_kind, scope_ref, object_kind, object_count,
            object_digest, reason_code, actor_principal_id, created_at_ts
         ) VALUES (?1, ?2, 'principal', ?2, 'bulk_clear', ?3, ?4,
                   'user_confirmed_bulk_clear', ?2, ?5)",
        params![
            format!("memory_clear_{}", uuid::Uuid::new_v4().simple()),
            principal_id,
            preview.transcript_rows + preview.derived_rows,
            format!(
                "sha256:{:x}",
                Sha256::digest(
                    format!(
                        "{}:{}:{}",
                        request.mode, preview.transcript_rows, preview.derived_rows
                    )
                    .as_bytes()
                )
            ),
            now_ts,
        ],
    )?;
    transaction.commit()?;
    Ok(preview)
}

pub(crate) fn export_memory(
    db: &Connection,
    principal_id: &str,
    now_ts: i64,
) -> anyhow::Result<MemoryExport> {
    let page = list_memory_page(
        db,
        principal_id,
        &MemoryListFilter {
            page: Some(1),
            page_size: Some(100),
            ..MemoryListFilter::default()
        },
        now_ts,
    )?;
    let digest_input = serde_json::to_vec(&page.items)?;
    Ok(MemoryExport {
        schema_version: 1,
        exported_at_ts: now_ts,
        scope_kind: "principal".to_string(),
        items: page.items,
        checksum: format!("sha256:{:x}", Sha256::digest(digest_input)),
    })
}

pub(crate) fn export_memory_markdown(
    db: &Connection,
    principal_id: &str,
    now_ts: i64,
) -> anyhow::Result<MemoryMarkdownExport> {
    let export = export_memory(db, principal_id, now_ts)?;
    let mut content = String::from("# memory_export_v1\n\n");
    content.push_str(&format!("schema_version={}  \n", export.schema_version));
    content.push_str(&format!("exported_at_unix={}\n\n", export.exported_at_ts));
    for item in &export.items {
        content.push_str(&format!(
            "## {} · {}\n\n- scope_kind={}\n- origin={}\n- status={}\n- updated_at_unix={}\n- evidence_available={}\n\n{}\n\n",
            item.kind,
            item.id,
            item.scope_kind,
            item.origin,
            item.status,
            item.updated_at_ts,
            item.evidence_available,
            item.content.replace('\n', "  \n")
        ));
    }
    let checksum = format!("sha256:{:x}", Sha256::digest(content.as_bytes()));
    Ok(MemoryMarkdownExport {
        schema_version: 1,
        exported_at_ts: now_ts,
        content_type: "text/markdown; charset=utf-8".to_string(),
        content,
        checksum,
    })
}

pub(crate) fn preview_memory_import(
    db: &Connection,
    principal_id: &str,
    request: &MemoryImportPreviewRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryImportPreview> {
    ensure_memory_ux_schema(db)?;
    anyhow::ensure!(
        request.export.schema_version == 1,
        "memory_import_schema_unsupported"
    );
    anyhow::ensure!(
        request.export.items.len() <= 10_000,
        "memory_import_item_limit_exceeded"
    );
    let expected_checksum = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&request.export.items)?)
    );
    anyhow::ensure!(
        expected_checksum == request.export.checksum,
        "memory_import_checksum_invalid"
    );
    let mut accepted = Vec::new();
    let mut skipped_items = 0usize;
    let mut duplicate_items = 0usize;
    for item in &request.export.items {
        if !matches!(item.kind.as_str(), "fact" | "preference" | "recent")
            || item.content.trim().is_empty()
            || item.content.chars().count() > 32_768
        {
            skipped_items += 1;
            continue;
        }
        let (safe_content, redacted) =
            crate::skill_output_artifact::sensitivity_aware_text_model_view(&item.content);
        if redacted || safe_content.trim().is_empty() {
            skipped_items += 1;
            continue;
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
            duplicate_items += 1;
            continue;
        }
        accepted.push(json!({
            "kind": item.kind,
            "content": safe_content,
            "content_digest": content_digest,
            "source_memory_id": item.id,
            "source_scope_kind": item.scope_kind,
            "source_origin": item.origin,
        }));
    }
    let payload = json!({
        "schema_version": 1,
        "items": accepted,
        "skipped_items": skipped_items,
        "duplicate_items": duplicate_items,
        "trust_tier": "imported_legacy",
        "scope_kind": "principal",
    });
    let payload_digest = format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&payload)?));
    let existing = db
        .query_row(
            "SELECT import_id, preview_json, status
             FROM memory_import_sessions
             WHERE principal_id = ?1 AND payload_digest = ?2",
            params![principal_id, payload_digest],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let import_id = if let Some((import_id, _, status)) = existing {
        anyhow::ensure!(status != "confirmed", "memory_import_already_confirmed");
        import_id
    } else {
        let import_id = format!("memory_import_{}", uuid::Uuid::new_v4().simple());
        db.execute(
            "INSERT INTO memory_import_sessions(
                import_id, principal_id, scope_kind, scope_ref, payload_digest,
                preview_json, status, created_at_ts
             ) VALUES (?1, ?2, 'principal', ?2, ?3, ?4, 'preview', ?5)",
            params![
                import_id,
                principal_id,
                payload_digest,
                payload.to_string(),
                now_ts
            ],
        )?;
        import_id
    };
    Ok(MemoryImportPreview {
        schema_version: 1,
        import_id,
        payload_digest,
        accepted_items: payload["items"].as_array().map_or(0, Vec::len),
        skipped_items,
        duplicate_items,
        trust_tier: "imported_legacy".to_string(),
        scope_kind: "principal".to_string(),
    })
}

pub(crate) fn confirm_memory_import(
    db: &Connection,
    principal_id: &str,
    request: &MemoryImportConfirmRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryImportResult> {
    ensure_memory_ux_schema(db)?;
    let transaction = db.unchecked_transaction()?;
    let (payload_digest, preview_json, status) = transaction
        .query_row(
            "SELECT payload_digest, preview_json, status FROM memory_import_sessions
             WHERE import_id = ?1 AND principal_id = ?2",
            params![request.import_id, principal_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("memory_import_session_not_found"))?;
    anyhow::ensure!(status == "preview", "memory_import_state_conflict");
    anyhow::ensure!(
        payload_digest == request.expected_payload_digest,
        "memory_import_preview_conflict"
    );
    let payload: Value = serde_json::from_str(&preview_json)?;
    let computed_digest = format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&payload)?));
    anyhow::ensure!(
        computed_digest == payload_digest,
        "memory_import_preview_corrupt"
    );
    let user_key = transaction
        .query_row(
            "SELECT user_key FROM auth_keys
             WHERE principal_id = ?1 AND enabled = 1 ORDER BY created_at, user_key LIMIT 1",
            [principal_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_default();
    let items = payload["items"]
        .as_array()
        .ok_or_else(|| anyhow!("memory_import_preview_corrupt"))?;
    let mut imported_items = 0usize;
    let mut existing_items = 0usize;
    for item in items {
        let content = item["content"]
            .as_str()
            .ok_or_else(|| anyhow!("memory_import_preview_corrupt"))?;
        let content_digest = item["content_digest"]
            .as_str()
            .ok_or_else(|| anyhow!("memory_import_preview_corrupt"))?;
        let idempotency_key = format!("import:{content_digest}");
        let changed = if item["kind"].as_str() == Some("preference") {
            let item_key = format!(
                "imported_{}",
                &content_digest[7..23.min(content_digest.len())]
            );
            transaction.execute(
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
            transaction.execute(
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
                    request.import_id,
                    now_ts,
                    content_digest,
                    idempotency_key,
                ],
            )?
        };
        if changed == 1 {
            imported_items += 1;
        } else {
            existing_items += 1;
        }
    }
    transaction.execute(
        "UPDATE memory_import_sessions SET status = 'confirmed', confirmed_at_ts = ?3
         WHERE import_id = ?1 AND principal_id = ?2 AND status = 'preview'",
        params![request.import_id, principal_id, now_ts],
    )?;
    transaction.commit()?;
    Ok(MemoryImportResult {
        schema_version: 1,
        import_id: request.import_id.clone(),
        status: "confirmed".to_string(),
        imported_items,
        existing_items,
    })
}

pub(crate) fn check_memory_consistency(
    db: &Connection,
    principal_id: &str,
    repair: bool,
    now_ts: i64,
) -> anyhow::Result<MemoryConsistencyReport> {
    ensure_memory_ux_schema(db)?;
    let deleted_canonical_rows = db.query_row(
        "SELECT COUNT(*) FROM memory_revisions r
         WHERE r.principal_id = ?1 AND r.operation = 'delete'
           AND r.undo_expires_at_ts IS NOT NULL
           AND (EXISTS(SELECT 1 FROM memory_facts f WHERE f.memory_id = r.memory_id)
             OR EXISTS(SELECT 1 FROM user_preferences p WHERE p.memory_id = r.memory_id)
             OR EXISTS(SELECT 1 FROM memories m WHERE m.memory_id = r.memory_id))",
        [principal_id],
        |row| row.get::<_, i64>(0),
    )?;
    let deleted_retrieval_rows = db.query_row(
        "SELECT COUNT(*) FROM memory_retrieval_index i
         WHERE i.principal_id = ?1 AND EXISTS(
           SELECT 1 FROM memory_revisions r
            WHERE r.principal_id = i.principal_id AND r.operation = 'delete'
              AND (i.source_ref = r.memory_id)
         )",
        [principal_id],
        |row| row.get::<_, i64>(0),
    )?;
    let orphan_retrieval_rows = db.query_row(
        "SELECT COUNT(*) FROM memory_retrieval_index i
         WHERE i.principal_id = ?1
           AND ((i.source_kind = 'memory' AND i.source_memory_id IS NOT NULL AND NOT EXISTS(
                 SELECT 1 FROM memories m WHERE m.id = i.source_memory_id))
             OR (i.source_kind = 'memory_fact' AND NOT EXISTS(
                 SELECT 1 FROM memory_facts f WHERE CAST(f.id AS TEXT) = i.source_ref)))",
        [principal_id],
        |row| row.get::<_, i64>(0),
    )?;
    let stale_jobs = db.query_row(
        "SELECT COUNT(*) FROM memory_jobs j WHERE j.principal_id = ?1
           AND j.status IN ('queued', 'retry_wait', 'running')
           AND j.source_task_id IS NOT NULL
           AND NOT EXISTS(
             SELECT 1 FROM memory_source_events e
              WHERE e.principal_id = j.principal_id
                AND e.source_task_id = j.source_task_id
                AND e.source_sequence BETWEEN COALESCE(j.source_event_start, e.source_sequence)
                                          AND COALESCE(j.source_event_end, e.source_sequence)
           )",
        [principal_id],
        |row| row.get::<_, i64>(0),
    )?;
    let mut repaired_rows = 0i64;
    if repair {
        let transaction = db.unchecked_transaction()?;
        repaired_rows += transaction.execute(
            "DELETE FROM memory_retrieval_index
             WHERE principal_id = ?1
               AND ((source_kind = 'memory' AND source_memory_id IS NOT NULL AND NOT EXISTS(
                     SELECT 1 FROM memories m WHERE m.id = source_memory_id))
                 OR (source_kind = 'memory_fact' AND NOT EXISTS(
                     SELECT 1 FROM memory_facts f WHERE CAST(f.id AS TEXT) = source_ref)))",
            [principal_id],
        )? as i64;
        repaired_rows += transaction.execute(
            "UPDATE memory_jobs SET cancel_requested = 1, updated_at_ts = ?2
             WHERE principal_id = ?1 AND status IN ('queued', 'retry_wait', 'running')
               AND source_task_id IS NOT NULL
               AND NOT EXISTS(
                 SELECT 1 FROM memory_source_events e
                  WHERE e.principal_id = memory_jobs.principal_id
                    AND e.source_task_id = memory_jobs.source_task_id
                    AND e.source_sequence BETWEEN COALESCE(memory_jobs.source_event_start, e.source_sequence)
                                              AND COALESCE(memory_jobs.source_event_end, e.source_sequence)
               )",
            params![principal_id, now_ts],
        )? as i64;
        let _ = transaction.execute(
            "DELETE FROM memory_retrieval_index_fts
             WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
            [],
        );
        transaction.commit()?;
    }
    Ok(MemoryConsistencyReport {
        schema_version: 1,
        deleted_canonical_rows,
        deleted_retrieval_rows,
        orphan_retrieval_rows,
        stale_jobs,
        repaired_rows,
    })
}

fn memory_union_sql() -> &'static str {
    "SELECT memory_id AS id, row_revision AS revision, 'fact' AS kind, scope_kind,
            origin, status, fact_text AS content, source_kind AS source,
            CASE WHEN evidence_refs_json != '[]' THEN 1 ELSE 0 END AS evidence_available,
            COALESCE(modified_at_ts, updated_at_ts) AS updated_at_ts, expires_at_ts,
            supersedes_memory_id, last_recalled_at_ts, last_verified_at_ts, trust_tier,
            principal_id
       FROM memory_facts
     UNION ALL
     SELECT memory_id, row_revision, 'preference', scope_kind, origin, 'active',
            pref_key || ': ' || pref_value, source,
            CASE WHEN evidence_refs_json != '[]' THEN 1 ELSE 0 END,
            COALESCE(modified_at_ts, updated_at_ts), NULL, supersedes_memory_id,
            last_recalled_at_ts, last_verified_at_ts, trust_tier, principal_id
       FROM user_preferences
     UNION ALL
     SELECT memory_id, row_revision, 'recent', scope_kind, origin, 'active',
            CASE WHEN safety_flag = 'normal' THEN content ELSE '[content hidden]' END,
            role, 0, created_at_ts, NULL, NULL, NULL, created_at_ts, 'source_transcript',
            principal_id
       FROM memories"
}

#[derive(Debug)]
struct MutableMemory {
    id: String,
    principal_id: String,
    revision: i64,
    kind: String,
    scope_kind: String,
    scope_ref: String,
    snapshot: Value,
}

fn load_mutable_memory(
    db: &Connection,
    principal_id: &str,
    memory_id: &str,
) -> anyhow::Result<Option<MutableMemory>> {
    db.query_row(
        "SELECT id, revision, kind, scope_kind, scope_ref, content, origin, status,
                principal_id, user_id, chat_id, user_key, item_key, namespace, source,
                role, channel, external_chat_id, created_at, created_at_ts, safety_flag,
                memory_type, salience
         FROM (
            SELECT memory_id AS id, row_revision AS revision, 'fact' AS kind,
                   scope_kind, scope_ref, fact_text AS content, origin, status, principal_id,
                   user_id, chat_id, user_key, fact_key AS item_key, namespace,
                   source_kind AS source, '' AS role, '' AS channel,
                   NULL AS external_chat_id, CAST(created_at_ts AS TEXT) AS created_at,
                   created_at_ts, safety_flag, 'fact' AS memory_type, confidence AS salience
              FROM memory_facts
            UNION ALL
            SELECT memory_id, row_revision, 'preference', scope_kind, scope_ref,
                   pref_value, origin, 'active', principal_id, user_id, chat_id, user_key,
                   pref_key, 'user_profile', source, '', '', NULL, updated_at,
                   updated_at_ts, 'normal', 'preference', confidence
              FROM user_preferences
            UNION ALL
            SELECT memory_id, row_revision, 'recent', scope_kind, scope_ref,
                   content, origin, 'active', principal_id, user_id, chat_id, user_key,
                   '', 'recent', role, role, channel, external_chat_id, created_at,
                   created_at_ts, safety_flag, memory_type, salience
              FROM memories
         ) WHERE id = ?1 AND principal_id = ?2 LIMIT 1",
        params![memory_id, principal_id],
        |row| {
            let id = row.get::<_, String>(0)?;
            let revision = row.get::<_, i64>(1)?;
            let kind = row.get::<_, String>(2)?;
            let scope_kind = row.get::<_, String>(3)?;
            let scope_ref = row.get::<_, String>(4)?;
            let owner_principal_id = row.get::<_, String>(8)?;
            Ok(MutableMemory {
                snapshot: json!({
                    "schema_version": 1,
                    "id": id,
                    "revision": revision,
                    "kind": kind,
                    "scope_kind": scope_kind,
                    "content": row.get::<_, String>(5)?,
                    "origin": row.get::<_, String>(6)?,
                    "status": row.get::<_, String>(7)?,
                    "principal_id": owner_principal_id,
                    "scope_ref": scope_ref,
                    "user_id": row.get::<_, i64>(9)?,
                    "chat_id": row.get::<_, i64>(10)?,
                    "user_key": row.get::<_, Option<String>>(11)?,
                    "item_key": row.get::<_, String>(12)?,
                    "namespace": row.get::<_, String>(13)?,
                    "source": row.get::<_, String>(14)?,
                    "role": row.get::<_, String>(15)?,
                    "channel": row.get::<_, String>(16)?,
                    "external_chat_id": row.get::<_, Option<String>>(17)?,
                    "created_at": row.get::<_, String>(18)?,
                    "created_at_ts": row.get::<_, i64>(19)?,
                    "safety_flag": row.get::<_, String>(20)?,
                    "memory_type": row.get::<_, String>(21)?,
                    "salience": row.get::<_, f64>(22)?,
                }),
                id,
                principal_id: owner_principal_id,
                revision,
                kind,
                scope_kind,
                scope_ref,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn append_revision(
    db: &Connection,
    memory: &MutableMemory,
    operation: &str,
    actor_principal_id: &str,
    replacement_memory_id: Option<&str>,
    undo_expires_at_ts: Option<i64>,
    now_ts: i64,
) -> anyhow::Result<String> {
    let revision_id = format!("memory_revision_{}", uuid::Uuid::new_v4().simple());
    db.execute(
        "INSERT INTO memory_revisions(
            revision_id, memory_id, principal_id, scope_kind, scope_ref, object_kind,
            row_revision, operation, previous_snapshot_json, replacement_memory_id,
            undo_expires_at_ts, actor_principal_id, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            revision_id,
            memory.id,
            memory.principal_id,
            memory.scope_kind,
            memory.scope_ref,
            memory.kind,
            memory.revision,
            operation,
            memory.snapshot.to_string(),
            replacement_memory_id,
            undo_expires_at_ts,
            actor_principal_id,
            now_ts,
        ],
    )?;
    Ok(revision_id)
}

fn disable_memory_recall(
    db: &Connection,
    memory: &MutableMemory,
    now_ts: i64,
) -> anyhow::Result<()> {
    match memory.kind.as_str() {
        "fact" => {
            db.execute(
                "UPDATE memory_facts SET status = 'deleted', row_revision = row_revision + 1,
                    modified_at_ts = ?2 WHERE memory_id = ?1",
                params![memory.id, now_ts],
            )?;
        }
        "preference" => {
            db.execute(
                "DELETE FROM memory_retrieval_index WHERE source_kind = 'preference'
                   AND source_ref IN (SELECT pref_key FROM user_preferences WHERE memory_id = ?1)",
                [memory.id.as_str()],
            )?;
        }
        "recent" => {
            db.execute(
                "DELETE FROM memory_retrieval_index WHERE source_kind = 'memory'
                   AND source_memory_id IN (SELECT id FROM memories WHERE memory_id = ?1)",
                [memory.id.as_str()],
            )?;
        }
        _ => anyhow::bail!("memory_kind_unsupported"),
    }
    Ok(())
}

fn migration_digest() -> String {
    format!("sha256:{:x}", Sha256::digest(MIGRATION_SQL.as_bytes()))
}

fn digest_text(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn snapshot_str<'a>(snapshot: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    snapshot
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("memory_revision_snapshot_invalid:{key}"))
}

fn snapshot_i64(snapshot: &Value, key: &str) -> anyhow::Result<i64> {
    snapshot
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("memory_revision_snapshot_invalid:{key}"))
}

fn snapshot_f64(snapshot: &Value, key: &str) -> Option<f64> {
    snapshot.get(key).and_then(Value::as_f64)
}

fn nonempty_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
#[path = "ux_tests.rs"]
mod tests;

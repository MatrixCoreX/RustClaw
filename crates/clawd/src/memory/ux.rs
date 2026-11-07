use anyhow::anyhow;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MIGRATION_ID: &str = "012_memory_ux_audit_v1";
const MIGRATION_SQL: &str = include_str!("../../../../migrations/012_memory_ux_audit.sql");
const UNDO_GRACE_SECONDS: i64 = 300;

#[path = "ux_write.rs"]
mod writing;
pub(crate) use writing::atomic_mutation;
pub(super) use writing::GrowthAdmission;

#[path = "ux_clear.rs"]
mod clearing;
#[path = "ux_export.rs"]
mod exports;
pub(crate) use exports::{export_memory, export_memory_markdown};

#[path = "ux_import_items.rs"]
pub(super) mod import_items;

#[path = "ux_mutations.rs"]
mod mutations;
pub(crate) use mutations::{correct_memory, record_feedback};

#[path = "ux_retrieval.rs"]
mod retrieval;
pub(crate) use retrieval::check_memory_consistency;

#[path = "ux_projection.rs"]
mod projection;
pub(super) use projection::refresh_owned_canonical;

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
    db: &Connection,
    state: &crate::AppState,
    principal_id: &str,
) -> anyhow::Result<RemoteMemoryDisclosure> {
    let settings = super::settings::resolve_principal_memory_settings(
        db,
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
    super::import_upload::initialize_schema(db)?;
    Ok(())
}

pub(crate) fn require_memory_ux_schema(db: &Connection) -> anyhow::Result<()> {
    super::jobs::require_memory_job_schema(db)?;
    super::scope::require_applied_migration(db, MIGRATION_ID, &migration_digest())
}

pub(crate) fn list_memory_page(
    db: &Connection,
    principal_id: &str,
    filter: &MemoryListFilter,
    now_ts: i64,
) -> anyhow::Result<MemoryPageResult> {
    require_memory_ux_schema(db)?;
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
        |row| memory_list_item(row, stale_cutoff),
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

fn memory_list_item(
    row: &rusqlite::Row<'_>,
    stale_cutoff: i64,
) -> rusqlite::Result<MemoryListItem> {
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
}

pub(crate) fn delete_memory_with_revision(
    db: &Connection,
    principal_id: &str,
    memory_id: &str,
    request: &MemoryMutationRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryMutationResult> {
    require_memory_ux_schema(db)?;
    atomic_mutation(db, |transaction| {
        let current = load_mutable_memory(transaction, principal_id, memory_id)?
            .ok_or_else(|| anyhow!("memory_not_found"))?;
        anyhow::ensure!(
            current.revision == request.expected_revision,
            "memory_revision_conflict"
        );
        let revision_id = append_revision(
            transaction,
            &current,
            "delete",
            principal_id,
            None,
            Some(now_ts + UNDO_GRACE_SECONDS),
            now_ts,
        )?;
        let retrieval_identity = retrieval::identity(transaction, &current)?;
        let raw_source_id = retrieval_identity.source_memory_id;
        super::jobs::request_cancel_for_scope(transaction, principal_id, Some(&current.scope_ref))?;
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
        retrieval::remove_identity(transaction, principal_id, &retrieval_identity)?;
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
        Ok(MemoryMutationResult {
            status: "deleted".to_string(),
            memory_id: memory_id.to_string(),
            replacement_memory_id: None,
            revision: request.expected_revision + 1,
            revision_id: Some(revision_id),
            undo_until_ts: Some(now_ts + UNDO_GRACE_SECONDS),
        })
    })
}

pub(crate) fn undo_memory_mutation(
    db: &Connection,
    principal_id: &str,
    request: &MemoryUndoRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryMutationResult> {
    require_memory_ux_schema(db)?;
    atomic_mutation(db, |transaction| {
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
        let (
            memory_id,
            object_kind,
            row_revision,
            operation,
            snapshot_json,
            replacement_id,
            expiry,
        ) = revision;
        anyhow::ensure!(
            expiry.is_some_and(|value| value >= now_ts),
            "memory_undo_expired"
        );
        let snapshot: Value = serde_json::from_str(&snapshot_json)?;
        anyhow::ensure!(
            snapshot.get("purged").and_then(Value::as_bool) != Some(true),
            "memory_undo_expired"
        );
        let admission = match operation.as_str() {
            "correct" => undo_correction(
                transaction,
                principal_id,
                &memory_id,
                &object_kind,
                replacement_id.as_deref(),
                row_revision,
                &snapshot,
                now_ts,
            )?,
            "delete" => {
                let admission = writing::GrowthAdmission::begin(transaction)?;
                restore_deleted_memory(
                    transaction,
                    principal_id,
                    &memory_id,
                    &object_kind,
                    row_revision,
                    &snapshot,
                    now_ts,
                )?;
                if object_kind == "recent" {
                    writing::validate_restored_source_quota(transaction, principal_id, now_ts)?;
                }
                Some(admission)
            }
            _ => anyhow::bail!("memory_undo_operation_unsupported"),
        };
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
        if let Some(admission) = admission {
            admission.finish(transaction)?;
        }
        Ok(MemoryMutationResult {
            status: "restored".to_string(),
            memory_id,
            replacement_memory_id: None,
            revision: row_revision + 2,
            revision_id: Some(restore_revision_id),
            undo_until_ts: None,
        })
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
) -> anyhow::Result<Option<writing::GrowthAdmission>> {
    let replacement_id = replacement_id.ok_or_else(|| anyhow!("memory_replacement_missing"))?;
    let mut admission = None;
    if let Some(replacement) = load_mutable_memory(db, principal_id, replacement_id)? {
        let grows = object_kind == "preference"
            && snapshot_str(snapshot, "content")?.len()
                > snapshot_str(&replacement.snapshot, "content")?.len();
        if grows {
            admission = Some(writing::GrowthAdmission::begin(db)?);
        }
        retrieval::remove_canonical(db, &replacement)?;
    }
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
    projection::refresh_owned_canonical(db, principal_id, memory_id, now_ts)?;
    Ok(admission)
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
    require_memory_ux_schema(db)?;
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
    require_memory_ux_schema(db)?;
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
    require_memory_ux_schema(db)?;
    atomic_mutation(db, |transaction| {
        let preview = clear_preview(transaction, principal_id, &request.mode)?;
        anyhow::ensure!(
            preview.transcript_rows == request.expected_transcript_rows
                && preview.derived_rows == request.expected_derived_rows,
            "memory_clear_preview_conflict"
        );
        super::jobs::request_cancel_for_scope(transaction, principal_id, None)?;
        transaction.execute(
            "DELETE FROM memory_retrieval_index WHERE principal_id = ?1
         AND (?2 = 1 OR source_kind = ?3)",
            params![
                principal_id,
                request.mode == "transcript_and_derived",
                super::RETRIEVAL_SOURCE_MEMORY
            ],
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
        clearing::purge_private_clear_state(
            transaction,
            principal_id,
            request.mode == "transcript_and_derived",
            now_ts,
        )?;
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
        Ok(preview)
    })
}

pub(crate) fn preview_memory_import(
    db: &Connection,
    principal_id: &str,
    request: &MemoryImportPreviewRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryImportPreview> {
    require_memory_ux_schema(db)?;
    crate::repo::auth::require_active_principal_credential(db, principal_id)?;
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
        match import_items::prepare(db, principal_id, item, Some(32_768))? {
            import_items::Decision::Skipped => skipped_items += 1,
            import_items::Decision::Duplicate => duplicate_items += 1,
            import_items::Decision::Accepted(item) => accepted.push(item),
        }
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
    atomic_mutation(db, |transaction| {
        crate::repo::auth::require_active_principal_credential(transaction, principal_id)?;
        let existing = transaction
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
            let admission = if accepted.is_empty() {
                None
            } else {
                Some(writing::GrowthAdmission::begin(transaction)?)
            };
            let import_id = format!("memory_import_{}", uuid::Uuid::new_v4().simple());
            transaction.execute(
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
            if let Some(admission) = admission {
                admission.finish(transaction)?;
            }
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
    })
}

pub(crate) fn confirm_memory_import(
    db: &Connection,
    principal_id: &str,
    request: &MemoryImportConfirmRequest,
    now_ts: i64,
) -> anyhow::Result<MemoryImportResult> {
    require_memory_ux_schema(db)?;
    atomic_mutation(db, |transaction| {
        let user_key =
            crate::repo::auth::require_active_principal_credential(transaction, principal_id)?;
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
        if super::import_upload::is_staged(&payload) {
            return super::import_upload::confirm_staged(
                transaction,
                principal_id,
                &user_key,
                request,
                &payload,
                now_ts,
            );
        }
        let computed_digest = format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&payload)?));
        anyhow::ensure!(
            computed_digest == payload_digest,
            "memory_import_preview_corrupt"
        );
        let items = payload["items"]
            .as_array()
            .ok_or_else(|| anyhow!("memory_import_preview_corrupt"))?;
        let mut imported_items = 0usize;
        let mut existing_items = 0usize;
        let mut admission = None;
        // Capture once before the batch, not once per duplicate item. Keep a
        // failed prospective check dormant for a metadata-only confirmation.
        let mut prospective =
            (!items.is_empty()).then(|| writing::GrowthAdmission::begin(transaction));
        for item in items {
            let changed = import_items::insert(
                transaction,
                principal_id,
                &user_key,
                &request.import_id,
                item,
                now_ts,
            )?;
            if changed == 1 {
                // The insert may be a true idempotent no-op. Check admission
                // only for actual content growth; any rejection rolls it back.
                if let Some(prospective) = prospective.take() {
                    admission = Some(prospective?);
                }
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
        super::import_upload::mark_confirmed(
            transaction,
            principal_id,
            &request.import_id,
            now_ts,
        )?;
        if let Some(admission) = admission {
            admission.finish(transaction)?;
        }
        Ok(MemoryImportResult {
            schema_version: 1,
            import_id: request.import_id.clone(),
            status: "confirmed".to_string(),
            imported_items,
            existing_items,
        })
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
    .map_err(|error| match error {
        rusqlite::Error::InvalidColumnType(4, column, rusqlite::types::Type::Null)
            if column == "scope_ref" =>
        {
            // An unavailable canonical scope is an error, never a default scope.
            anyhow!("memory_retrieval_scope_ref_required")
        }
        other => other.into(),
    })
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
                    modified_at_ts = ?2 WHERE memory_id = ?1 AND principal_id = ?3",
                params![memory.id, now_ts, memory.principal_id],
            )?;
        }
        "preference" | "recent" => {}
        _ => anyhow::bail!("memory_kind_unsupported"),
    }
    retrieval::remove_canonical(db, memory)?;
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

#[cfg(test)]
#[path = "ux_boundary_test_fixtures.rs"]
mod boundary_test_fixtures;

#[cfg(test)]
#[path = "ux_boundary_tests.rs"]
mod boundary_tests;

#[cfg(test)]
#[path = "ux_clear_import_tests.rs"]
mod clear_import_tests;

#[cfg(test)]
#[path = "ux_clear_scope_tests.rs"]
mod clear_scope_tests;

#[cfg(test)]
#[path = "ux_pressure_read_tests.rs"]
mod pressure_read_tests;

#[cfg(test)]
#[path = "schema_readiness_tests.rs"]
mod readiness_tests;

#[cfg(test)]
#[path = "ux_import_credential_tests.rs"]
mod import_credential_tests;

#[cfg(test)]
#[path = "ux_admission_contract_tests.rs"]
mod admission_contract_tests;

#[cfg(test)]
#[path = "ux_budget_contract_tests.rs"]
mod budget_contract_tests;

#[cfg(test)]
#[path = "ux_growth_state_tests.rs"]
mod growth_state_tests;

#[cfg(test)]
#[path = "ux_budget_policy_tests.rs"]
mod budget_policy_tests;

#[cfg(test)]
#[path = "ux_growth_availability_tests.rs"]
mod growth_availability_tests;

#[cfg(test)]
#[path = "ux_feedback_tests.rs"]
mod feedback_tests;

#[cfg(test)]
#[path = "ux_recall_contract_tests.rs"]
mod recall_contract_tests;

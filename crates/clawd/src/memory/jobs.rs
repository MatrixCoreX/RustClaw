use std::time::Duration;

use anyhow::anyhow;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::{error, info, warn};

use super::eligibility::{
    build_turn_eligibility, MemoryEligibilityDisposition, MemorySourceCategory,
};

const MIGRATION_ID: &str = "011_durable_memory_jobs_v1";
const MIGRATION_SQL: &str = include_str!("../../../../migrations/011_durable_memory_jobs.sql");

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MemoryJobSnapshot {
    pub(crate) job_id: String,
    pub(crate) job_kind: String,
    pub(crate) principal_id: String,
    pub(crate) scope_kind: String,
    pub(crate) scope_ref: String,
    pub(crate) source_task_id: Option<String>,
    pub(crate) source_event_start: Option<i64>,
    pub(crate) source_event_end: Option<i64>,
    pub(crate) source_digest: String,
    pub(crate) settings_revision: i64,
    pub(crate) policy_digest: String,
    pub(crate) provider_name: String,
    pub(crate) provider_type: String,
    pub(crate) model_name: String,
    pub(crate) model_capability_digest: String,
    pub(crate) status: String,
    pub(crate) attempt: i64,
    pub(crate) checkpoint_json: String,
    pub(crate) cancel_requested: bool,
}

pub(crate) fn ensure_memory_job_schema(db: &Connection) -> anyhow::Result<()> {
    super::scope::ensure_memory_scope_schema(db)?;
    if let Some(applied) = migration_digest(db)? {
        anyhow::ensure!(
            applied == migration_manifest_digest(),
            "runtime_schema_migration_digest_mismatch:{MIGRATION_ID}"
        );
    }
    if db.is_autocommit() {
        let tx = db.unchecked_transaction()?;
        apply_migration(&tx)?;
        tx.commit()?;
    } else {
        apply_migration(db)?;
    }
    Ok(())
}

fn apply_migration(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(MIGRATION_SQL)?;
    for (table, columns) in [
        (
            "memory_facts",
            &[
                ("content_digest", "TEXT"),
                ("idempotency_key", "TEXT"),
                ("source_task_id", "TEXT"),
                ("source_event_start", "INTEGER"),
                ("source_event_end", "INTEGER"),
                ("source_refs_json", "TEXT NOT NULL DEFAULT '[]'"),
                ("evidence_refs_json", "TEXT NOT NULL DEFAULT '[]'"),
                ("generation_job_id", "TEXT"),
                ("trust_tier", "TEXT NOT NULL DEFAULT 'legacy_unreviewed'"),
                ("sensitivity", "TEXT NOT NULL DEFAULT 'normal'"),
                ("observed_at_ts", "INTEGER"),
                ("valid_from_ts", "INTEGER"),
                ("valid_to_ts", "INTEGER"),
                ("last_verified_at_ts", "INTEGER"),
                ("last_recalled_at_ts", "INTEGER"),
                ("volatility_class", "TEXT NOT NULL DEFAULT 'stable'"),
                ("source_timezone", "TEXT"),
                ("modified_at_ts", "INTEGER"),
                ("deleted_at_ts", "INTEGER"),
                ("supersedes_memory_id", "TEXT"),
            ][..],
        ),
        (
            "user_preferences",
            &[
                ("content_digest", "TEXT"),
                ("idempotency_key", "TEXT"),
                ("source_task_id", "TEXT"),
                ("source_refs_json", "TEXT NOT NULL DEFAULT '[]'"),
                ("evidence_refs_json", "TEXT NOT NULL DEFAULT '[]'"),
                ("generation_job_id", "TEXT"),
                ("trust_tier", "TEXT NOT NULL DEFAULT 'legacy_unreviewed'"),
                ("sensitivity", "TEXT NOT NULL DEFAULT 'normal'"),
                ("last_verified_at_ts", "INTEGER"),
                ("last_recalled_at_ts", "INTEGER"),
                ("modified_at_ts", "INTEGER"),
                ("deleted_at_ts", "INTEGER"),
                ("supersedes_memory_id", "TEXT"),
            ][..],
        ),
    ] {
        if !table_exists(db, table)? {
            continue;
        }
        for &(column, definition) in columns {
            add_column(db, table, column, definition)?;
        }
    }
    if table_exists(db, "memory_facts")? {
        db.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_facts_active_idempotency
             ON memory_facts(principal_id, scope_kind, scope_ref, idempotency_key)
             WHERE idempotency_key IS NOT NULL AND status = 'active';
             CREATE INDEX IF NOT EXISTS idx_memory_facts_generation_job
             ON memory_facts(generation_job_id);",
        )?;
    }
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3) ON CONFLICT(migration_id) DO NOTHING",
        params![MIGRATION_ID, migration_manifest_digest(), crate::now_ts()],
    )?;
    Ok(())
}

pub(crate) fn enqueue_turn_memory_jobs(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    user_source_memory_id: Option<i64>,
    assistant_source_memory_id: Option<i64>,
    force_consolidation: bool,
) -> anyhow::Result<Vec<String>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow!("memory_job_db_pool:{error}"))?;
    ensure_memory_job_schema(&db)?;
    let settings = super::settings::resolve_task_memory_settings(state, task)?
        .ok_or_else(|| anyhow!("memory_job_settings_unavailable"))?;
    if !settings.generate_memory {
        return Ok(Vec::new());
    }
    super::retention::ensure_principal_quota(
        &db,
        &state.policy.memory,
        &settings.target_principal_id,
        crate::now_ts_u64() as i64,
    )?;
    if !super::retention::automatic_generation_allowed(&db, &settings.target_principal_id)? {
        return Ok(Vec::new());
    }
    let payload = serde_json::from_str::<Value>(&task.payload_json).unwrap_or(Value::Null);
    let eligibility = build_turn_eligibility(task, &payload, &settings);
    if !eligibility.durable_candidate_allowed {
        return Ok(Vec::new());
    }
    let conversation_id = crate::conversation_state::task_conversation_id(task);
    let (scope_kind, scope_ref) = if let Some(conversation_id) = conversation_id.as_deref() {
        (
            "conversation",
            super::scope::conversation_scope_ref(&settings.target_principal_id, conversation_id)?,
        )
    } else {
        ("principal", settings.target_principal_id.clone())
    };
    let tx = db.unchecked_transaction()?;
    let job_ids = enqueue_prepared_turn_jobs(
        state,
        task,
        &settings,
        &eligibility,
        &scope_kind,
        &scope_ref,
        user_source_memory_id,
        assistant_source_memory_id,
        force_consolidation,
        &tx,
    )?;
    tx.commit()?;
    Ok(job_ids)
}

pub(crate) fn persist_turn_and_enqueue(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    user_content: &str,
    assistant_content: Option<&str>,
    force_consolidation: bool,
) -> anyhow::Result<Vec<String>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow!("memory_job_db_pool:{error}"))?;
    ensure_memory_job_schema(&db)?;
    let settings = super::settings::resolve_task_memory_settings(state, task)?;
    let tx = db.unchecked_transaction()?;
    let user_source_memory_id = super::insert_memory_with_id_in_connection(
        state,
        &tx,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &task.channel,
        task.external_chat_id.as_deref(),
        super::MEMORY_ROLE_USER,
        user_content,
        super::MemoryWriteKind::Default,
    )?;
    let assistant_source_memory_id = match assistant_content {
        Some(content) if !content.trim().is_empty() => super::insert_memory_with_id_in_connection(
            state,
            &tx,
            task.user_id,
            task.chat_id,
            task.user_key.as_deref(),
            &task.channel,
            task.external_chat_id.as_deref(),
            super::MEMORY_ROLE_ASSISTANT,
            content,
            super::MemoryWriteKind::AssistantOutcome,
        )?,
        _ => None,
    };
    let Some(settings) = settings.filter(|settings| settings.generate_memory) else {
        tx.commit()?;
        return Ok(Vec::new());
    };
    super::retention::ensure_principal_quota(
        &tx,
        &state.policy.memory,
        &settings.target_principal_id,
        crate::now_ts_u64() as i64,
    )?;
    if !super::retention::automatic_generation_allowed(&tx, &settings.target_principal_id)? {
        tx.commit()?;
        return Ok(Vec::new());
    }
    let payload = serde_json::from_str::<Value>(&task.payload_json).unwrap_or(Value::Null);
    let eligibility = build_turn_eligibility(task, &payload, &settings);
    if !eligibility.durable_candidate_allowed {
        tx.commit()?;
        return Ok(Vec::new());
    }
    let conversation_id = crate::conversation_state::task_conversation_id(task);
    let (scope_kind, scope_ref) = if let Some(conversation_id) = conversation_id.as_deref() {
        (
            "conversation",
            super::scope::conversation_scope_ref(&settings.target_principal_id, conversation_id)?,
        )
    } else {
        ("principal", settings.target_principal_id.clone())
    };
    let job_ids = enqueue_prepared_turn_jobs(
        state,
        task,
        &settings,
        &eligibility,
        scope_kind,
        &scope_ref,
        user_source_memory_id,
        assistant_source_memory_id,
        force_consolidation,
        &tx,
    )?;
    tx.commit()?;
    Ok(job_ids)
}

fn provider_snapshot(
    provider: Option<std::sync::Arc<crate::LlmProviderRuntime>>,
) -> (String, String, String, String) {
    provider
        .as_ref()
        .map(|provider| {
            let capabilities = provider.model_capabilities();
            let capability_json = json!({
                "native_tools": capabilities.native_tools,
                "parallel_tools": capabilities.parallel_tools,
                "structured_output": capabilities.structured_output,
                "streaming": capabilities.streaming,
                "reasoning": capabilities.reasoning,
                "vision": capabilities.vision,
                "prompt_cache": capabilities.prompt_cache,
            });
            (
                provider.config.name.clone(),
                provider.config.provider_type.clone(),
                provider.config.model.clone(),
                sha256_json(&capability_json),
            )
        })
        .unwrap_or_else(|| {
            (
                "unavailable".to_string(),
                "unavailable".to_string(),
                "unavailable".to_string(),
                sha256_bytes(b"provider-unavailable"),
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn enqueue_prepared_turn_jobs(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    settings: &super::settings::MemoryEffectiveSettings,
    eligibility: &super::eligibility::MemoryGenerationEligibility,
    scope_kind: &str,
    scope_ref: &str,
    user_source_memory_id: Option<i64>,
    assistant_source_memory_id: Option<i64>,
    force_consolidation: bool,
    tx: &Connection,
) -> anyhow::Result<Vec<String>> {
    let mut source_rows = Vec::new();
    for (sequence, source_memory_id, category, actor_kind) in [
        (
            1_i64,
            user_source_memory_id,
            MemorySourceCategory::UserAuthored,
            "user",
        ),
        (
            2_i64,
            assistant_source_memory_id,
            MemorySourceCategory::AssistantAuthored,
            "assistant",
        ),
    ] {
        let Some(source_memory_id) = source_memory_id else {
            continue;
        };
        let content: Option<String> = tx
            .query_row(
                "SELECT content FROM memories
                 WHERE id = ?1 AND principal_id = ?2",
                params![source_memory_id, settings.target_principal_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(content) = content else {
            continue;
        };
        let disposition = eligibility
            .items
            .iter()
            .find(|item| item.category == category)
            .map(|item| item.disposition)
            .unwrap_or(MemoryEligibilityDisposition::Excluded);
        let digest = sha256_bytes(content.as_bytes());
        let (_, redacted) =
            crate::skill_output_artifact::sensitivity_aware_text_model_view(&content);
        tx.execute(
            "INSERT INTO memory_source_events(
                event_id, source_task_id, source_sequence, source_memory_id, principal_id,
                conversation_scope_ref, source_category, actor_kind, eligibility,
                content_digest, sensitivity, evidence_ref, created_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(source_task_id, source_sequence) DO UPDATE SET
                source_memory_id = excluded.source_memory_id,
                content_digest = excluded.content_digest,
                sensitivity = excluded.sensitivity",
            params![
                format!("memory_event_{}", uuid::Uuid::new_v4().simple()),
                task.task_id,
                sequence,
                source_memory_id,
                settings.target_principal_id,
                if scope_kind == "conversation" {
                    Some(scope_ref)
                } else {
                    None
                },
                category.as_str(),
                actor_kind,
                disposition.as_str(),
                digest,
                if redacted {
                    "restricted_redacted"
                } else {
                    "normal"
                },
                format!("task:{}:memory:{source_memory_id}", task.task_id),
                crate::now_ts_u64() as i64,
            ],
        )?;
        source_rows.push((source_memory_id, digest));
    }
    if source_rows.is_empty() {
        return Ok(Vec::new());
    }
    if source_rows.len() < 2 {
        return Ok(Vec::new());
    }
    source_rows.sort_by_key(|(id, _)| *id);
    let source_start = source_rows.first().map(|(id, _)| *id).unwrap_or_default();
    let source_end = source_rows
        .last()
        .map(|(id, _)| *id)
        .unwrap_or(source_start);
    let source_digest = sha256_json(&json!({
        "source_task_id": task.task_id,
        "rows": source_rows,
        "eligibility_digest": eligibility.policy_digest,
    }));
    let eligibility_json = serde_json::to_string(&eligibility)?;
    let now = crate::now_ts_u64() as i64;
    let mut job_ids = Vec::new();
    let mut kinds = Vec::new();
    if state.policy.memory.enable_preference_extraction
        && state.policy.memory.llm_preference_fallback_enabled
    {
        kinds.push("extract");
    }
    if state.policy.memory.long_term_enabled || force_consolidation {
        kinds.push("consolidate");
    }
    for kind in kinds {
        let (provider_name, provider_type, model_name, capability_digest) =
            provider_snapshot_for_job(state, task, settings, kind)?;
        let job_id = format!("memory_job_{}", uuid::Uuid::new_v4().simple());
        tx.execute(
            "INSERT INTO memory_jobs(
                job_id, job_kind, principal_id, scope_kind, scope_ref, source_task_id,
                source_event_start, source_event_end, source_digest, eligibility_json,
                settings_revision, policy_digest, provider_name, provider_type, model_name,
                model_capability_digest, status, not_before_ts, checkpoint_json,
                created_at_ts, updated_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                       ?13, ?14, ?15, ?16, 'queued', ?17, '{}', ?18, ?18)
             ON CONFLICT(job_kind, principal_id, source_task_id, source_event_start,
                         source_event_end, policy_digest) DO NOTHING",
            params![
                job_id,
                kind,
                settings.target_principal_id,
                scope_kind,
                scope_ref,
                task.task_id,
                source_start,
                source_end,
                source_digest,
                eligibility_json,
                settings.revision,
                settings.policy_digest,
                provider_name,
                provider_type,
                model_name,
                capability_digest,
                now + state.policy.memory.background_idle_seconds.max(1) as i64,
                now,
            ],
        )?;
        if tx.changes() > 0 {
            job_ids.push(job_id);
        }
    }
    Ok(job_ids)
}

fn provider_snapshot_for_job(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    settings: &super::settings::MemoryEffectiveSettings,
    job_kind: &str,
) -> anyhow::Result<(String, String, String, String)> {
    let (configured_provider, configured_model) = match job_kind {
        "extract" => (
            state.policy.memory.extract_provider.trim(),
            state.policy.memory.extract_model.trim(),
        ),
        "consolidate" => (
            state.policy.memory.consolidation_provider.trim(),
            state.policy.memory.consolidation_model.trim(),
        ),
        _ => ("", ""),
    };
    if configured_provider.is_empty() && configured_model.is_empty() {
        return Ok(provider_snapshot(
            state.task_llm_providers(task).into_iter().next(),
        ));
    }
    anyhow::ensure!(
        !configured_provider.is_empty() && !configured_model.is_empty(),
        "memory_job_model_config_incomplete"
    );
    anyhow::ensure!(
        settings.external_context_policy == super::settings::ExternalContextPolicy::Allow,
        "memory_job_independent_provider_consent_required"
    );
    let provider = state
        .core
        .llm_providers
        .iter()
        .find(|runtime| {
            runtime.config.provider_type == configured_provider
                && runtime.config.model == configured_model
        })
        .cloned()
        .ok_or_else(|| anyhow!("memory_job_configured_provider_unavailable"))?;
    Ok(provider_snapshot(Some(provider)))
}

pub(crate) fn spawn_memory_job_workers(state: crate::AppState, concurrency: usize) {
    for worker_index in 0..concurrency.max(1) {
        let state = state.clone();
        tokio::spawn(async move {
            let worker_id = format!("{}:memory:{worker_index}", state.worker.worker_id);
            loop {
                match run_one_memory_job(&state, &worker_id).await {
                    Ok(true) => {}
                    Ok(false) => tokio::time::sleep(Duration::from_millis(750)).await,
                    Err(error) => {
                        error!(worker_id, error = %error, "memory_job_worker_tick_failed");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });
    }
}

pub(crate) async fn run_one_memory_job(
    state: &crate::AppState,
    worker_id: &str,
) -> anyhow::Result<bool> {
    let Some(job) = claim_next_job(state, worker_id)? else {
        return Ok(false);
    };
    let execution = execute_claimed_job(state, &job);
    tokio::pin!(execution);
    let heartbeat_seconds = (state.policy.memory.background_lease_seconds.max(15) / 3).clamp(5, 30);
    let result = loop {
        tokio::select! {
            result = &mut execution => break result,
            _ = tokio::time::sleep(Duration::from_secs(heartbeat_seconds)) => {
                if let Err(error) = renew_job_lease(state, &job.job_id, worker_id) {
                    break Err(error);
                }
            }
        }
    };
    match result {
        Ok(()) => complete_job(state, &job.job_id, worker_id)?,
        Err(error) => {
            warn!(job_id = job.job_id, error = %error, "memory_job_execution_failed");
            fail_or_retry_job(
                state,
                &job.job_id,
                worker_id,
                job.attempt,
                "memory_job_execution_failed",
            )?;
        }
    }
    Ok(true)
}

fn renew_job_lease(state: &crate::AppState, job_id: &str, worker_id: &str) -> anyhow::Result<()> {
    let db = state.core.db.get().map_err(|error| anyhow!(error))?;
    let now = crate::now_ts_u64() as i64;
    let changed = db.execute(
        "UPDATE memory_jobs
         SET lease_expires_at_ts = ?1, updated_at_ts = ?2
         WHERE job_id = ?3 AND status = 'running' AND lease_owner = ?4
           AND cancel_requested = 0",
        params![
            now.saturating_add(state.policy.memory.background_lease_seconds.max(15) as i64),
            now,
            job_id,
            worker_id,
        ],
    )?;
    anyhow::ensure!(changed == 1, "memory_job_heartbeat_lease_lost");
    Ok(())
}

fn claim_next_job(
    state: &crate::AppState,
    worker_id: &str,
) -> anyhow::Result<Option<MemoryJobSnapshot>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow!("memory_job_db_pool:{error}"))?;
    ensure_memory_job_schema(&db)?;
    let now = crate::now_ts_u64() as i64;
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "UPDATE memory_jobs
         SET status = 'retry_wait', lease_owner = NULL, lease_expires_at_ts = NULL,
             not_before_ts = ?1, error_code = 'lease_expired', retryable = 1,
             updated_at_ts = ?1
         WHERE status = 'running' AND lease_expires_at_ts IS NOT NULL
           AND lease_expires_at_ts <= ?1 AND cancel_requested = 0",
        [now],
    )?;
    tx.execute(
        "UPDATE memory_jobs
         SET status = 'cancelled', lease_owner = NULL, lease_expires_at_ts = NULL,
             error_code = 'cancel_requested', retryable = 0,
             updated_at_ts = ?1, finished_at_ts = ?1
         WHERE cancel_requested = 1 AND status IN ('queued', 'retry_wait')",
        [now],
    )?;
    let job_id: Option<String> = tx
        .query_row(
            "SELECT j.job_id
             FROM memory_jobs j
             WHERE j.status IN ('queued', 'retry_wait')
               AND j.cancel_requested = 0 AND j.not_before_ts <= ?1
             ORDER BY (
                SELECT COUNT(*) FROM memory_jobs active
                WHERE active.principal_id = j.principal_id AND active.status = 'running'
             ) ASC, j.not_before_ts ASC, j.created_at_ts ASC
             LIMIT 1",
            [now],
            |row| row.get(0),
        )
        .optional()?;
    let Some(job_id) = job_id else {
        tx.commit()?;
        return Ok(None);
    };
    let changed = tx.execute(
        "UPDATE memory_jobs
         SET status = 'running', lease_owner = ?2, lease_expires_at_ts = ?3,
             attempt = attempt + 1, error_code = NULL, retryable = 0,
             updated_at_ts = ?1
         WHERE job_id = ?4 AND status IN ('queued', 'retry_wait')
           AND cancel_requested = 0",
        params![
            now,
            worker_id,
            now + state.policy.memory.background_lease_seconds.max(15) as i64,
            job_id
        ],
    )?;
    if changed == 0 {
        tx.commit()?;
        return Ok(None);
    }
    let snapshot = load_job(&tx, &job_id)?.ok_or_else(|| anyhow!("memory_job_claim_lost"))?;
    tx.commit()?;
    Ok(Some(snapshot))
}

async fn execute_claimed_job(
    state: &crate::AppState,
    job: &MemoryJobSnapshot,
) -> anyhow::Result<()> {
    if job.cancel_requested {
        anyhow::bail!("memory_job_cancelled");
    }
    let task = load_source_task(state, job)?;
    let selected_provider = state
        .task_llm_providers(&task)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("memory_job_provider_unavailable"))?;
    let (_, _, selected_model, selected_capability_digest) =
        provider_snapshot(Some(selected_provider.clone()));
    anyhow::ensure!(
        selected_provider.config.name == job.provider_name
            && selected_model == job.model_name
            && selected_capability_digest == job.model_capability_digest,
        "memory_job_model_snapshot_mismatch"
    );
    let current = super::settings::resolve_task_memory_settings(state, &task)?
        .ok_or_else(|| anyhow!("memory_job_settings_unavailable"))?;
    anyhow::ensure!(current.generate_memory, "memory_job_generation_revoked");
    anyhow::ensure!(
        current.revision == job.settings_revision && current.policy_digest == job.policy_digest,
        "memory_job_settings_snapshot_changed"
    );
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow!("memory_job_db_pool:{error}"))?;
    let cancelled: bool = db.query_row(
        "SELECT cancel_requested != 0 FROM memory_jobs WHERE job_id = ?1",
        [&job.job_id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!cancelled, "memory_job_cancelled");
    let source_start = job
        .source_event_start
        .ok_or_else(|| anyhow!("memory_job_source_range_missing"))?;
    let source_end = job
        .source_event_end
        .ok_or_else(|| anyhow!("memory_job_source_range_missing"))?;
    db.execute(
        "UPDATE memory_jobs
         SET checkpoint_json = ?2, progress_current = 1, progress_total = 2,
             updated_at_ts = ?3
         WHERE job_id = ?1 AND status = 'running'",
        params![
            job.job_id,
            json!({
                "schema_version": 1,
                "phase": "source_range_verified",
                "source_event_start": source_start,
                "source_event_end": source_end,
                "source_digest": job.source_digest,
            })
            .to_string(),
            crate::now_ts_u64() as i64,
        ],
    )?;
    drop(db);
    match job.job_kind.as_str() {
        "extract" => {
            let content = load_user_source_bundle(state, job)?;
            if !content.trim().is_empty() {
                super::maybe_extract_memory_intent_with_llm(state, &task, &content).await?;
            }
        }
        "consolidate" => {
            super::service::maybe_refresh_long_term_summary_for_range(
                state,
                &task,
                true,
                Some((source_start, source_end)),
                Some(&job.job_id),
            )
            .await
            .map_err(anyhow::Error::msg)?;
        }
        _ => anyhow::bail!("memory_job_kind_not_implemented"),
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow!("memory_job_db_pool:{error}"))?;
    db.execute(
        "UPDATE memory_jobs
         SET checkpoint_json = ?2, progress_current = 2, progress_total = 2,
             updated_at_ts = ?3
         WHERE job_id = ?1 AND status = 'running'",
        params![
            job.job_id,
            json!({"schema_version": 1, "phase": "commit_verified"}).to_string(),
            crate::now_ts_u64() as i64,
        ],
    )?;
    Ok(())
}

fn load_user_source_bundle(
    state: &crate::AppState,
    job: &MemoryJobSnapshot,
) -> anyhow::Result<String> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow!("memory_job_db_pool:{error}"))?;
    let mut stmt = db.prepare(
        "SELECT m.content
         FROM memory_source_events e
         JOIN memories m ON m.id = e.source_memory_id
         WHERE e.source_task_id = ?1 AND e.principal_id = ?2
           AND e.source_category = 'user_authored' AND e.eligibility = 'candidate'
           AND m.id BETWEEN ?3 AND ?4
         ORDER BY e.source_sequence",
    )?;
    let rows = stmt.query_map(
        params![
            job.source_task_id,
            job.principal_id,
            job.source_event_start,
            job.source_event_end
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut out = Vec::new();
    for row in rows {
        let (safe, _) = crate::skill_output_artifact::sensitivity_aware_text_model_view(&row?);
        out.push(safe);
    }
    Ok(out.join("\n"))
}

fn load_source_task(
    state: &crate::AppState,
    job: &MemoryJobSnapshot,
) -> anyhow::Result<crate::ClaimedTask> {
    let source_task_id = job
        .source_task_id
        .as_deref()
        .ok_or_else(|| anyhow!("memory_job_source_task_missing"))?;
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow!("memory_job_db_pool:{error}"))?;
    db.query_row(
        "SELECT task_id, user_id, chat_id, user_key, channel, external_user_id,
                external_chat_id, kind, payload_json, principal_id
         FROM tasks WHERE task_id = ?1",
        [source_task_id],
        |row| {
            let principal_id = row.get::<_, Option<String>>(9)?;
            let mut task = crate::ClaimedTask {
                claim_attempt: 0,
                task_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                external_user_id: row.get(5)?,
                external_chat_id: row.get(6)?,
                kind: row.get(7)?,
                payload_json: row.get(8)?,
            };
            let mut payload =
                serde_json::from_str::<Value>(&task.payload_json).unwrap_or_else(|_| json!({}));
            if let Some(object) = payload.as_object_mut() {
                object.insert(
                    "_agent_model_selection".to_string(),
                    json!({
                        "schema_version": 1,
                        "provider": job.provider_type,
                        "model": job.model_name,
                        "authority": "server_validated_model_catalog",
                    }),
                );
            }
            task.payload_json = payload.to_string();
            Ok((task, principal_id))
        },
    )
    .optional()?
    .ok_or_else(|| anyhow!("memory_job_source_task_unavailable"))
    .and_then(|(task, principal_id)| {
        anyhow::ensure!(
            principal_id.as_deref() == Some(job.principal_id.as_str()),
            "memory_job_source_principal_mismatch"
        );
        Ok(task)
    })
}

fn complete_job(state: &crate::AppState, job_id: &str, worker_id: &str) -> anyhow::Result<()> {
    let db = state.core.db.get().map_err(|error| anyhow!(error))?;
    let now = crate::now_ts_u64() as i64;
    let changed = db.execute(
        "UPDATE memory_jobs
         SET status = CASE WHEN cancel_requested = 1 THEN 'cancelled' ELSE 'completed' END,
             lease_owner = NULL, lease_expires_at_ts = NULL, retryable = 0,
             updated_at_ts = ?1, finished_at_ts = ?1
         WHERE job_id = ?2 AND status = 'running' AND lease_owner = ?3",
        params![now, job_id, worker_id],
    )?;
    anyhow::ensure!(changed == 1, "memory_job_completion_lease_lost");
    db.execute(
        "UPDATE memory_principal_quotas
         SET used_background_cost_microunits = used_background_cost_microunits + 1000,
             updated_at_ts = ?2
         WHERE principal_id = (SELECT principal_id FROM memory_jobs WHERE job_id = ?1)",
        params![job_id, now],
    )?;
    info!(job_id, "memory_job_completed");
    Ok(())
}

fn fail_or_retry_job(
    state: &crate::AppState,
    job_id: &str,
    worker_id: &str,
    attempt: i64,
    error_code: &str,
) -> anyhow::Result<()> {
    let db = state.core.db.get().map_err(|error| anyhow!(error))?;
    let now = crate::now_ts_u64() as i64;
    let retryable = attempt < state.policy.memory.background_max_attempts.max(1) as i64;
    let backoff = 2_i64.saturating_pow(attempt.clamp(1, 8) as u32).min(300);
    let changed = db.execute(
        "UPDATE memory_jobs
         SET status = CASE WHEN cancel_requested = 1 THEN 'cancelled'
                           WHEN ?1 != 0 THEN 'retry_wait' ELSE 'failed' END,
             lease_owner = NULL, lease_expires_at_ts = NULL,
             not_before_ts = ?2, error_code = ?3, retryable = ?1,
             updated_at_ts = ?4,
             finished_at_ts = CASE WHEN cancel_requested = 1 OR ?1 = 0 THEN ?4 ELSE NULL END
         WHERE job_id = ?5 AND status = 'running' AND lease_owner = ?6",
        params![
            if retryable { 1 } else { 0 },
            now + backoff,
            error_code,
            now,
            job_id,
            worker_id,
        ],
    )?;
    anyhow::ensure!(changed == 1, "memory_job_failure_lease_lost");
    Ok(())
}

pub(crate) fn request_cancel_for_scope(
    db: &Connection,
    principal_id: &str,
    scope_ref: Option<&str>,
) -> anyhow::Result<usize> {
    let now = crate::now_ts_u64() as i64;
    let changed = if let Some(scope_ref) = scope_ref {
        db.execute(
            "UPDATE memory_jobs SET cancel_requested = 1, updated_at_ts = ?3
             WHERE principal_id = ?1 AND scope_ref = ?2
               AND status IN ('queued', 'retry_wait', 'running')",
            params![principal_id, scope_ref, now],
        )?
    } else {
        db.execute(
            "UPDATE memory_jobs SET cancel_requested = 1, updated_at_ts = ?2
             WHERE principal_id = ?1 AND status IN ('queued', 'retry_wait', 'running')",
            params![principal_id, now],
        )?
    };
    Ok(changed)
}

pub(crate) fn reconcile_missing_turn_jobs(state: &crate::AppState) -> anyhow::Result<usize> {
    let db = state.core.db.get().map_err(|error| anyhow!(error))?;
    ensure_memory_job_schema(&db)?;
    let mut stmt = db.prepare(
        "SELECT DISTINCT t.task_id
         FROM tasks t
         JOIN memories m ON m.principal_id = t.principal_id AND m.chat_id = t.chat_id
         WHERE t.status = 'success' AND t.kind = 'ask'
           AND m.created_at_ts >= COALESCE(CAST(t.updated_at AS INTEGER), 0) - 5
           AND m.created_at_ts <= COALESCE(CAST(t.updated_at AS INTEGER), 0) + 5
           AND NOT EXISTS (
             SELECT 1 FROM memory_jobs j WHERE j.source_task_id = t.task_id
           )
         ORDER BY t.id DESC LIMIT 100",
    )?;
    let task_ids = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    drop(db);
    let mut repaired = 0usize;
    for task_id in task_ids {
        let db = state.core.db.get().map_err(|error| anyhow!(error))?;
        let job = MemoryJobSnapshot {
            job_id: String::new(),
            job_kind: String::new(),
            principal_id: db.query_row(
                "SELECT principal_id FROM tasks WHERE task_id = ?1",
                [&task_id],
                |row| row.get::<_, String>(0),
            )?,
            scope_kind: String::new(),
            scope_ref: String::new(),
            source_task_id: Some(task_id.clone()),
            source_event_start: None,
            source_event_end: None,
            source_digest: String::new(),
            settings_revision: 0,
            policy_digest: String::new(),
            provider_name: String::new(),
            provider_type: String::new(),
            model_name: String::new(),
            model_capability_digest: String::new(),
            status: String::new(),
            attempt: 0,
            checkpoint_json: String::new(),
            cancel_requested: false,
        };
        drop(db);
        let task = load_source_task(state, &job)?;
        let db = state.core.db.get().map_err(|error| anyhow!(error))?;
        let ids = {
            let mut source_stmt = db.prepare(
                "SELECT id FROM memories
                 WHERE principal_id = ?1 AND chat_id = ?2
                 ORDER BY id DESC LIMIT 2",
            )?;
            let rows = source_stmt
                .query_map(params![job.principal_id, task.chat_id], |row| {
                    row.get::<_, i64>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        drop(db);
        let user = ids.last().copied();
        let assistant = ids.first().copied();
        if !enqueue_turn_memory_jobs(state, &task, user, assistant, false)?.is_empty() {
            repaired += 1;
        }
    }
    Ok(repaired)
}

fn load_job(db: &Connection, job_id: &str) -> anyhow::Result<Option<MemoryJobSnapshot>> {
    db.query_row(
        "SELECT job_id, job_kind, principal_id, scope_kind, scope_ref, source_task_id,
                source_event_start, source_event_end, source_digest, settings_revision,
                policy_digest, provider_name, provider_type, model_name,
                model_capability_digest, status, attempt, checkpoint_json, cancel_requested
         FROM memory_jobs WHERE job_id = ?1",
        [job_id],
        |row| {
            Ok(MemoryJobSnapshot {
                job_id: row.get(0)?,
                job_kind: row.get(1)?,
                principal_id: row.get(2)?,
                scope_kind: row.get(3)?,
                scope_ref: row.get(4)?,
                source_task_id: row.get(5)?,
                source_event_start: row.get(6)?,
                source_event_end: row.get(7)?,
                source_digest: row.get(8)?,
                settings_revision: row.get(9)?,
                policy_digest: row.get(10)?,
                provider_name: row.get(11)?,
                provider_type: row.get(12)?,
                model_name: row.get(13)?,
                model_capability_digest: row.get(14)?,
                status: row.get(15)?,
                attempt: row.get(16)?,
                checkpoint_json: row.get(17)?,
                cancel_requested: row.get::<_, i64>(18)? != 0,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn add_column(db: &Connection, table: &str, column: &str, definition: &str) -> anyhow::Result<()> {
    if !column_exists(db, table, column)? {
        db.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

fn table_exists(db: &Connection, table: &str) -> anyhow::Result<bool> {
    db.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |_| Ok(true),
    )
    .optional()
    .map(|value| value.unwrap_or(false))
    .map_err(Into::into)
}

fn column_exists(db: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for current in columns {
        if current?.eq_ignore_ascii_case(column) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn migration_digest(db: &Connection) -> anyhow::Result<Option<String>> {
    db.query_row(
        "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
        [MIGRATION_ID],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn migration_manifest_digest() -> String {
    sha256_bytes(MIGRATION_SQL.as_bytes())
}

fn sha256_json(value: &Value) -> String {
    sha256_bytes(&serde_json::to_vec(value).unwrap_or_default())
}

fn sha256_bytes(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;

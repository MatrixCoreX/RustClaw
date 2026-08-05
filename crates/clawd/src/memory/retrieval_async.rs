use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::vector_store::MemoryVectorIndex;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AsyncRecallTrace {
    pub(crate) schema_version: u32,
    pub(crate) profile_id: Option<String>,
    pub(crate) remote_outbound_count: u64,
    pub(crate) fallback_code: Option<String>,
    pub(crate) cache_hit: bool,
    pub(crate) query_cache_bytes: usize,
    pub(crate) query_cache_byte_limit: usize,
    pub(crate) query_cache_ttl_seconds: u64,
    pub(crate) candidate_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct AsyncRecallOutcome {
    pub(crate) recall: super::retrieval::IndexedRecall,
    pub(crate) trace: AsyncRecallTrace,
}

#[derive(Debug, Clone)]
struct CachedQueryVector {
    key: String,
    vector: Vec<f32>,
    expires_at_ts: i64,
    byte_size: usize,
}

#[derive(Debug, Default)]
struct QueryVectorCache {
    entries: HashMap<String, CachedQueryVector>,
    order: VecDeque<String>,
    total_bytes: usize,
}

fn query_cache() -> &'static Mutex<QueryVectorCache> {
    static CACHE: OnceLock<Mutex<QueryVectorCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(QueryVectorCache::default()))
}

pub(crate) async fn retrieve_for_task(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    anchor_prompt: &str,
) -> anyhow::Result<AsyncRecallOutcome> {
    let mut trace = AsyncRecallTrace {
        schema_version: 1,
        profile_id: None,
        remote_outbound_count: 0,
        fallback_code: None,
        cache_hit: false,
        query_cache_bytes: query_cache_bytes(),
        query_cache_byte_limit: state.policy.memory.embedding_query_cache_max_bytes,
        query_cache_ttl_seconds: state.policy.memory.embedding_query_cache_ttl_seconds,
        candidate_count: 0,
    };
    let settings = match super::settings::resolve_task_memory_settings(state, task)? {
        Some(settings) if settings.use_memory => settings,
        Some(_) => {
            trace.fallback_code = Some("memory_use_disabled".to_string());
            return Ok(AsyncRecallOutcome {
                recall: super::retrieval::IndexedRecall::default(),
                trace,
            });
        }
        None => {
            trace.fallback_code = Some("memory_settings_unavailable".to_string());
            return Ok(AsyncRecallOutcome {
                recall: super::retrieval::IndexedRecall::default(),
                trace,
            });
        }
    };
    let conversation_id = crate::conversation_state::task_conversation_id(task);
    let mut recall = super::retrieval::retrieve_indexed_memories_for_scope(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        conversation_id.as_deref(),
        anchor_prompt,
    )?;
    let profile = super::vector_store::configured_profile(&state.policy.memory)?;
    trace.profile_id = Some(profile.profile_id.clone());
    if profile.provider_kind != "remote_http" {
        trace.fallback_code = Some("local_profile_selected".to_string());
        return Ok(AsyncRecallOutcome { recall, trace });
    }
    if settings.external_context_policy != super::settings::ExternalContextPolicy::Allow {
        trace.fallback_code = Some("remote_embedding_consent_required".to_string());
        return Ok(AsyncRecallOutcome { recall, trace });
    }
    let (safe_query, redacted) =
        crate::skill_output_artifact::sensitivity_aware_text_model_view(anchor_prompt);
    if redacted {
        trace.fallback_code = Some("remote_embedding_sensitive_query_blocked".to_string());
        return Ok(AsyncRecallOutcome { recall, trace });
    }
    let db = state.core.db.get()?;
    super::vector_store::register_configured_profile(&db, &state.policy.memory)?;
    let access = super::scope::resolve_memory_access(
        &db,
        &settings.target_principal_id,
        conversation_id.as_deref(),
        Some(&state.skill_rt.workspace_root),
    )?;
    let eligible_rows: i64 = db.query_row(
        "SELECT COUNT(*) FROM memory_vector_rows
         WHERE principal_id = ?1 AND profile_id = ?2 AND generation = ?3
           AND status = 'active'
           AND ((scope_kind = 'principal' AND scope_ref = ?4)
             OR (scope_kind = 'conversation' AND ?5 IS NOT NULL AND scope_ref = ?5)
             OR (scope_kind = 'project' AND ?6 IS NOT NULL AND scope_ref = ?6))",
        rusqlite::params![
            access.principal_id,
            profile.profile_id,
            super::vector_store::active_generation_for_principal(
                &db,
                &access.principal_id,
                &profile.profile_id,
                profile.generation,
            )? as i64,
            access.principal_scope_ref,
            access.conversation_scope_ref,
            access.project_scope_ref,
        ],
        |row| row.get(0),
    )?;
    if eligible_rows == 0 {
        trace.fallback_code = Some("remote_embedding_profile_rows_unavailable".to_string());
        return Ok(AsyncRecallOutcome { recall, trace });
    }
    let now = crate::now_ts_u64() as i64;
    if super::embedding_jobs::provider_circuit_open(
        &db,
        &access.principal_id,
        &profile.profile_id,
        now,
    )? {
        trace.fallback_code = Some("memory_embedding_circuit_open".to_string());
        return Ok(AsyncRecallOutcome { recall, trace });
    }
    drop(db);

    let cache_key = query_cache_key(
        &settings.target_principal_id,
        &profile.profile_id,
        &settings.policy_digest,
        &safe_query,
    );
    let query_vector = if let Some(vector) = get_cached_vector(&cache_key, now) {
        trace.cache_hit = true;
        vector
    } else {
        let provider = match super::embedding::provider_for_profile(&profile, &state.policy.memory)
        {
            Ok(provider) => provider,
            Err(error) => {
                trace.fallback_code = Some(error.error_code.to_string());
                return Ok(AsyncRecallOutcome { recall, trace });
            }
        };
        let request = [super::embedding::EmbeddingRequestItem {
            request_item_id: format!("memory_query:{}", short_digest(cache_key.as_bytes())),
            text: safe_query,
        }];
        let provider_spec = provider.spec();
        if provider_spec.model_id != profile.model_name
            || provider_spec.dims != profile.dimensions
            || provider_spec.version != profile.profile_version
            || provider_spec.normalization != profile.normalization
        {
            trace.fallback_code = Some("memory_embedding_provider_profile_mismatch".to_string());
            return Ok(AsyncRecallOutcome { recall, trace });
        }
        trace.remote_outbound_count = 1;
        let response = match tokio::time::timeout(
            Duration::from_millis(state.policy.memory.embedding_query_timeout_ms.max(100)),
            provider.embed_batch(&request),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                record_query_provider_failure(
                    state,
                    &access.principal_id,
                    &profile.profile_id,
                    &error,
                );
                trace.fallback_code = Some(error.error_code.to_string());
                return Ok(AsyncRecallOutcome { recall, trace });
            }
            Err(_) => {
                record_query_provider_failure(
                    state,
                    &access.principal_id,
                    &profile.profile_id,
                    &super::embedding::EmbeddingProviderError {
                        error_code: "memory_embedding_query_timeout",
                        retryable: true,
                        retry_after_seconds: None,
                        status_code: None,
                    },
                );
                trace.fallback_code = Some("memory_embedding_query_timeout".to_string());
                return Ok(AsyncRecallOutcome { recall, trace });
            }
        };
        let Some(vector) = response.into_iter().next().map(|item| item.vector) else {
            trace.fallback_code = Some("memory_embedding_query_response_empty".to_string());
            return Ok(AsyncRecallOutcome { recall, trace });
        };
        if let Ok(db) = state.core.db.get() {
            let _ = super::embedding_jobs::reset_provider_circuit(
                &db,
                &access.principal_id,
                &profile.profile_id,
            );
        }
        put_cached_vector(
            cache_key.clone(),
            vector.clone(),
            now.saturating_add(state.policy.memory.embedding_query_cache_ttl_seconds as i64),
            state.policy.memory.embedding_query_cache_max_bytes,
        );
        trace.query_cache_bytes = query_cache_bytes();
        vector
    };
    let db = state.core.db.get()?;
    let neighbors = match super::vector_store::ExactSqliteVectorIndex.nearest(
        &db,
        &access,
        &profile,
        &query_vector,
        state.policy.memory.vector_candidate_limit.max(1),
    ) {
        Ok(neighbors) => neighbors,
        Err(_) => {
            trace.fallback_code = Some("memory_vector_query_failed".to_string());
            return Ok(AsyncRecallOutcome { recall, trace });
        }
    };
    trace.candidate_count = neighbors.len();
    let semantic = match super::retrieval::materialize_scoped_vector_neighbors(
        &db, state, &access, &neighbors,
    ) {
        Ok(semantic) => semantic,
        Err(_) => {
            trace.fallback_code = Some("memory_vector_materialization_failed".to_string());
            return Ok(AsyncRecallOutcome { recall, trace });
        }
    };
    merge_recall(&mut recall, semantic);
    Ok(AsyncRecallOutcome { recall, trace })
}

fn record_query_provider_failure(
    state: &crate::AppState,
    principal_id: &str,
    profile_id: &str,
    error: &super::embedding::EmbeddingProviderError,
) {
    if let Ok(db) = state.core.db.get() {
        let _ = super::embedding_jobs::record_provider_failure(
            &db,
            principal_id,
            profile_id,
            error,
            &state.policy.memory,
        );
    }
}

fn merge_recall(
    target: &mut super::retrieval::IndexedRecall,
    source: super::retrieval::IndexedRecall,
) {
    append_unique(&mut target.similar_triggers, source.similar_triggers);
    append_unique(&mut target.relevant_facts, source.relevant_facts);
    append_unique(&mut target.knowledge_docs, source.knowledge_docs);
    append_unique(
        &mut target.recent_related_events,
        source.recent_related_events,
    );
    append_unique(&mut target.assistant_results, source.assistant_results);
    append_unique(&mut target.unfinished_goals, source.unfinished_goals);
}

fn append_unique(
    target: &mut Vec<super::retrieval::RetrievedMemoryItem>,
    source: Vec<super::retrieval::RetrievedMemoryItem>,
) {
    for item in source {
        if !target.iter().any(|existing| existing.text == item.text) {
            target.push(item);
        }
    }
    target.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn get_cached_vector(key: &str, now: i64) -> Option<Vec<f32>> {
    let mut cache = query_cache().lock().ok()?;
    prune_query_cache(&mut cache, now);
    cache
        .entries
        .get(key)
        .filter(|entry| entry.key == key)
        .map(|entry| entry.vector.clone())
}

fn put_cached_vector(key: String, vector: Vec<f32>, expires_at_ts: i64, max_bytes: usize) {
    let Ok(mut cache) = query_cache().lock() else {
        return;
    };
    prune_query_cache(&mut cache, crate::now_ts_u64() as i64);
    if let Some(existing) = cache.entries.remove(&key) {
        cache.total_bytes = cache.total_bytes.saturating_sub(existing.byte_size);
        cache.order.retain(|existing_key| existing_key != &key);
    }
    let byte_size = key
        .len()
        .saturating_add(vector.len().saturating_mul(std::mem::size_of::<f32>()));
    if byte_size > max_bytes {
        return;
    }
    while cache.total_bytes.saturating_add(byte_size) > max_bytes {
        let Some(oldest) = cache.order.pop_front() else {
            break;
        };
        if let Some(removed) = cache.entries.remove(&oldest) {
            cache.total_bytes = cache.total_bytes.saturating_sub(removed.byte_size);
        }
    }
    cache.order.push_back(key.clone());
    cache.total_bytes = cache.total_bytes.saturating_add(byte_size);
    cache.entries.insert(
        key.clone(),
        CachedQueryVector {
            key,
            vector,
            expires_at_ts,
            byte_size,
        },
    );
}

fn prune_query_cache(cache: &mut QueryVectorCache, now: i64) {
    cache.entries.retain(|_, entry| entry.expires_at_ts > now);
    let live_keys = cache.entries.keys().cloned().collect::<HashSet<_>>();
    cache.order.retain(|key| live_keys.contains(key));
    cache.total_bytes = cache.entries.values().map(|entry| entry.byte_size).sum();
}

fn query_cache_bytes() -> usize {
    query_cache()
        .lock()
        .map(|cache| cache.total_bytes)
        .unwrap_or_default()
}

pub(crate) fn invalidate_principal_query_cache(principal_id: &str) {
    let prefix = format!("{}:", short_digest(principal_id.as_bytes()));
    if let Ok(mut cache) = query_cache().lock() {
        cache.entries.retain(|key, _| !key.starts_with(&prefix));
        cache.order.retain(|key| !key.starts_with(&prefix));
        cache.total_bytes = cache.entries.values().map(|entry| entry.byte_size).sum();
    }
}

fn query_cache_key(
    principal_id: &str,
    profile_id: &str,
    policy_digest: &str,
    query: &str,
) -> String {
    format!(
        "{}:{}:{}:{}",
        short_digest(principal_id.as_bytes()),
        short_digest(profile_id.as_bytes()),
        short_digest(policy_digest.as_bytes()),
        short_digest(query.as_bytes()),
    )
}

fn short_digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))[..20].to_string()
}

#[cfg(test)]
#[path = "retrieval_async_tests.rs"]
mod tests;

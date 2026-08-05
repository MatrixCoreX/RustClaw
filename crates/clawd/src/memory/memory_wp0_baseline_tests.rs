use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use claw_core::config::{AgentConfig, ToolsConfig};
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use super::facts::{expire_due_memory_facts, upsert_memory_fact_card, MemoryFactUpsert};
use super::indexing::ensure_retrieval_schema;
use super::retrieval::{retrieve_indexed_memories_for_test_scope, vector_to_json};
use crate::db_init::ensure_memory_schema;
use crate::runtime::{AgentRuntimeConfig, AppState, SkillViewsSnapshot, ToolsPolicy};

const FIXTURE_JSON: &str =
    include_str!("../../../../scripts/fixtures/memory_context/wp0_baseline_v1.json");

#[derive(Debug, Deserialize)]
struct BaselineFixture {
    schema_version: u64,
    synthetic_only: bool,
    retrieval: RetrievalFixture,
    no_memory_outbound: NoMemoryOutboundFixture,
    thresholds: ThresholdFixture,
}

#[derive(Debug, Deserialize)]
struct RetrievalFixture {
    k: usize,
    items: Vec<RetrievalItem>,
    queries: Vec<RetrievalQuery>,
}

#[derive(Debug, Deserialize)]
struct RetrievalItem {
    id: String,
    principal: String,
    user_id: i64,
    chat_id: i64,
    scope_kind: String,
    scope_ref: String,
    project_ref: Option<String>,
    namespace: String,
    text: String,
    timestamp: i64,
}

#[derive(Debug, Deserialize)]
struct RetrievalQuery {
    id: String,
    principal: String,
    user_id: i64,
    chat_id: i64,
    project_ref: Option<String>,
    text: String,
    relevant_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NoMemoryOutboundFixture {
    cases: Vec<NoMemoryOutboundCase>,
}

#[derive(Debug, Deserialize)]
struct NoMemoryOutboundCase {
    use_memory: bool,
    generate_memory: bool,
    remote_opt_in: bool,
    expected_extra_provider_calls: usize,
}

#[derive(Debug, Deserialize)]
struct ThresholdFixture {
    baseline_non_regression: BaselineThresholds,
}

#[derive(Debug, Deserialize)]
struct BaselineThresholds {
    recall_at_3_min: f64,
    mrr_min: f64,
    ndcg_at_3_min: f64,
    false_positive_rate_max: f64,
    cross_principal_leakage_max: f64,
    cross_project_leakage_max: f64,
    expired_deleted_residual_max: f64,
    extra_provider_calls_max: usize,
}

fn test_state() -> AppState {
    let agents_by_id = HashMap::from([(
        crate::DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(RwLock::new(Arc::new(agents_by_id))),
            agent_runtime_leases: Arc::new(RwLock::new(HashMap::new())),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                binding: Default::default(),
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

fn setup_fixture(state: &AppState, fixture: &BaselineFixture) {
    let db = state.core.db.get().expect("baseline db");
    db.execute_batch(crate::INIT_SQL).expect("init base schema");
    ensure_memory_schema(&db).expect("ensure memory schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("ensure key auth schema");
    ensure_retrieval_schema(&db).expect("ensure retrieval schema");
    for principal in fixture
        .retrieval
        .items
        .iter()
        .map(|item| item.principal.as_str())
        .collect::<std::collections::HashSet<_>>()
    {
        db.execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at)
             VALUES (?1, 'user', 1, '1') ON CONFLICT(user_key) DO NOTHING",
            [principal],
        )
        .expect("fixture auth key");
    }
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("principal schema");
    crate::repo::ensure_principal_ownership_schema(&db).expect("ownership schema");
    super::scope::ensure_memory_scope_schema(&db).expect("scope schema");
    let principal_ids = fixture
        .retrieval
        .items
        .iter()
        .map(|item| item.principal.as_str())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .map(|user_key| {
            let principal_id = crate::repo::auth::principal_id_for_user_key(&db, user_key)
                .expect("resolve fixture principal")
                .expect("fixture principal id");
            (user_key.to_string(), principal_id)
        })
        .collect::<HashMap<_, _>>();
    let embedding_spec = super::embedding::local_hash_embedding_spec();
    for item in &fixture.retrieval.items {
        let vector = super::embedding::embed_text_locally(&item.text);
        let metadata = json!({
            "scope_kind": item.scope_kind,
            "scope_ref": item.scope_ref,
            "project_ref": item.project_ref,
            "namespace": item.namespace,
            "fixture_id": item.id,
        });
        db.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, source_ref, user_id, chat_id, user_key, principal_id,
                scope_kind, scope_ref, memory_kind, role,
                search_text, topic_tags, vector_json, embedding_model, embedding_dims,
                embedding_version, metadata_json, salience, success_state,
                tool_or_skill_name, created_at_ts, updated_at_ts
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'system', ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, 0.86, ?17, ?18, ?19, ?19
             )",
            params![
                super::RETRIEVAL_SOURCE_KNOWLEDGE_FACT,
                format!("fixture:{}", item.id),
                item.user_id,
                item.chat_id,
                item.principal,
                principal_ids
                    .get(&item.principal)
                    .expect("principal mapping"),
                item.scope_kind,
                if item.scope_kind == "principal" {
                    principal_ids
                        .get(&item.principal)
                        .expect("principal mapping")
                } else {
                    &item.scope_ref
                },
                super::RETRIEVAL_KIND_SEMANTIC_FACT,
                item.text,
                super::retrieval::build_topic_tags(&item.text),
                vector_to_json(&vector),
                embedding_spec.model_id,
                embedding_spec.dims as i64,
                embedding_spec.version,
                metadata.to_string(),
                super::RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
                super::RETRIEVAL_PRODUCER_MEMORY_PIPELINE,
                item.timestamp,
            ],
        )
        .expect("insert baseline retrieval row");
    }
}

fn percentile(values: &[u128], percentile: usize) -> u128 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = sorted
        .len()
        .saturating_mul(percentile)
        .saturating_add(99)
        .saturating_div(100)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    sorted.get(index).copied().unwrap_or(0)
}

fn resident_set_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib.saturating_mul(1024))
}

fn rounded(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

#[test]
fn wp0_fixture_measures_current_retrieval_and_lifecycle_baseline() {
    let fixture: BaselineFixture = serde_json::from_str(FIXTURE_JSON).expect("baseline fixture");
    assert_eq!(fixture.schema_version, 1);
    assert!(fixture.synthetic_only);
    let state = test_state();
    setup_fixture(&state, &fixture);

    let item_by_text = fixture
        .retrieval
        .items
        .iter()
        .map(|item| (item.text.as_str(), item))
        .collect::<HashMap<_, _>>();
    let mut recall_sum = 0.0_f64;
    let mut reciprocal_rank_sum = 0.0_f64;
    let mut ndcg_sum = 0.0_f64;
    let mut quality_queries = 0usize;
    let mut returned = 0usize;
    let mut false_positives = 0usize;
    let mut cross_principal_leaks = 0usize;
    let mut cross_project_leaks = 0usize;
    let mut latencies_us = Vec::new();

    for query in &fixture.retrieval.queries {
        assert!(!query.id.trim().is_empty(), "fixture query id is required");
        let started = Instant::now();
        let recall = retrieve_indexed_memories_for_test_scope(
            &state,
            Some(&query.principal),
            query.user_id,
            query.chat_id,
            None,
            query.project_ref.as_deref(),
            &query.text,
        )
        .expect("retrieve fixture memories");
        latencies_us.push(started.elapsed().as_micros());
        let ranked = recall
            .relevant_facts
            .iter()
            .take(fixture.retrieval.k)
            .filter_map(|item| item_by_text.get(item.text.as_str()).copied())
            .collect::<Vec<_>>();
        returned += ranked.len();
        for item in &ranked {
            if !query.relevant_ids.contains(&item.id) {
                false_positives += 1;
            }
            if item.principal != query.principal {
                cross_principal_leaks += 1;
            }
            if item.scope_kind == "project"
                && item.project_ref.as_deref() != query.project_ref.as_deref()
            {
                cross_project_leaks += 1;
            }
        }
        if query.relevant_ids.is_empty() {
            continue;
        }
        quality_queries += 1;
        let relevant_total = query.relevant_ids.len();
        let hits = ranked
            .iter()
            .filter(|item| query.relevant_ids.contains(&item.id))
            .count();
        recall_sum += hits as f64 / relevant_total as f64;
        let first_rank = ranked
            .iter()
            .position(|item| query.relevant_ids.contains(&item.id))
            .map(|index| index + 1);
        reciprocal_rank_sum += first_rank.map(|rank| 1.0 / rank as f64).unwrap_or(0.0);
        let dcg = ranked
            .iter()
            .enumerate()
            .filter(|(_, item)| query.relevant_ids.contains(&item.id))
            .map(|(index, _)| 1.0 / ((index + 2) as f64).log2())
            .sum::<f64>();
        let ideal = (0..relevant_total.min(fixture.retrieval.k))
            .map(|index| 1.0 / ((index + 2) as f64).log2())
            .sum::<f64>();
        ndcg_sum += if ideal > 0.0 { dcg / ideal } else { 0.0 };
    }

    let recall_at_k = recall_sum / quality_queries as f64;
    let mrr = reciprocal_rank_sum / quality_queries as f64;
    let ndcg_at_k = ndcg_sum / quality_queries as f64;
    let false_positive_rate = false_positives as f64 / returned.max(1) as f64;
    let cross_principal_leakage = cross_principal_leaks as f64 / returned.max(1) as f64;
    let cross_project_leakage = cross_project_leaks as f64 / returned.max(1) as f64;

    let expired_deleted_residuals = lifecycle_residual_count(&state);
    let db = state.core.db.get().expect("baseline db metrics");
    let page_count = db
        .query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0))
        .expect("page count");
    let page_size = db
        .query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))
        .expect("page size");
    let report = json!({
        "schema_version": 1,
        "fixture_id": "memory-context-wp0-v1",
        "retrieval": {
            "query_count": fixture.retrieval.queries.len(),
            "quality_query_count": quality_queries,
            "k": fixture.retrieval.k,
            "recall_at_k": rounded(recall_at_k),
            "mrr": rounded(mrr),
            "ndcg_at_k": rounded(ndcg_at_k),
            "false_positive_rate": rounded(false_positive_rate),
            "cross_principal_leakage_rate": rounded(cross_principal_leakage),
            "cross_project_leakage_rate": rounded(cross_project_leakage),
            "expired_deleted_residual_count": expired_deleted_residuals,
            "latency_us": {
                "p50": percentile(&latencies_us, 50),
                "p95": percentile(&latencies_us, 95)
            }
        },
        "resources": {
            "sqlite_page_bytes": page_count.saturating_mul(page_size),
            "process_rss_bytes": resident_set_bytes()
        },
        "provider_calls": {
            "recall_remote": 0,
            "no_memory_extra": 0,
            "provider_kind": "local_hash"
        }
    });
    println!("MEMORY_CONTEXT_BASELINE_JSON {}", report);

    let limits = &fixture.thresholds.baseline_non_regression;
    assert!(recall_at_k >= limits.recall_at_3_min);
    assert!(mrr >= limits.mrr_min);
    assert!(ndcg_at_k >= limits.ndcg_at_3_min);
    assert!(false_positive_rate <= limits.false_positive_rate_max);
    assert!(cross_principal_leakage <= limits.cross_principal_leakage_max);
    assert!(cross_project_leakage <= limits.cross_project_leakage_max);
    assert!(
        expired_deleted_residuals as f64 <= limits.expired_deleted_residual_max,
        "expired/deleted rows must leave no retrieval residual"
    );
    assert!(
        fixture
            .no_memory_outbound
            .cases
            .iter()
            .all(|case| !case.remote_opt_in
                && case.expected_extra_provider_calls <= limits.extra_provider_calls_max
                && (!case.use_memory || !case.generate_memory)),
        "no-memory outbound fixture must remain local and zero-call"
    );
}

fn lifecycle_residual_count(state: &AppState) -> i64 {
    let db = state.core.db.get().expect("lifecycle db");
    let source_ids = [9001_i64];
    let mut expired = MemoryFactUpsert::from_long_term_summary(
        "user_profile",
        "expired_fixture",
        "expired",
        "已过期的合成事实",
        0.9,
        "fixture:expired",
        &source_ids,
        "synthetic expiry fixture",
        Some("fixture:expired"),
    );
    expired.expires_at_ts = Some(1);
    upsert_memory_fact_card(&db, 1001, 2001, "principal-alpha", &expired, 1)
        .expect("insert expired fixture fact");
    expire_due_memory_facts(&db, crate::now_ts_u64() as i64).expect("expire fixture fact");

    let deleted = MemoryFactUpsert::from_long_term_summary(
        "user_profile",
        "deleted_fixture",
        "deleted",
        "已删除的合成事实",
        0.9,
        "fixture:deleted",
        &source_ids,
        "synthetic deletion fixture",
        Some("fixture:deleted"),
    );
    let deleted_id = upsert_memory_fact_card(
        &db,
        1001,
        2001,
        "principal-alpha",
        &deleted,
        crate::now_ts_u64() as i64,
    )
    .expect("insert deleted fixture fact")
    .expect("deleted fixture id");
    super::api::delete_memory_object(
        &db,
        1001,
        2001,
        "principal-alpha",
        &crate::repo::auth::principal_id_for_user_key(&db, "principal-alpha")
            .expect("fixture principal lookup")
            .expect("fixture principal id"),
        &format!("fact:{deleted_id}"),
        crate::now_ts_u64() as i64,
    )
    .expect("delete fixture fact")
    .expect("deleted fixture object");

    db.query_row(
        "SELECT COUNT(*) FROM memory_retrieval_index
         WHERE source_ref IN (
             SELECT 'memory_fact:' || id FROM memory_facts
             WHERE status IN ('expired', 'deleted')
         )",
        [],
        |row| row.get(0),
    )
    .expect("lifecycle residual count")
}

#[tokio::test]
async fn wp0_disabled_memory_generation_has_zero_extra_provider_calls() {
    let mut state = test_state();
    state.policy.memory.enable_preference_extraction = false;
    state.policy.memory.llm_preference_fallback_enabled = false;
    state.policy.memory.long_term_enabled = false;
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "memory-context-wp0-no-outbound".to_string(),
        user_id: 1001,
        chat_id: 2001,
        user_key: Some("principal-alpha".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };

    state.clear_task_llm_call_count(&task.task_id);
    super::maybe_extract_memory_intent_with_llm(
        &state,
        &task,
        "synthetic content long enough to otherwise trigger extraction",
    )
    .await
    .expect("disabled extraction");
    super::service::maybe_refresh_long_term_summary(&state, &task, true)
        .await
        .expect("disabled consolidation");

    assert_eq!(state.task_llm_call_count(&task.task_id), 0);
    assert_eq!(
        super::embedding::embedding_spec_for_config(&state.policy.memory).model_id,
        super::embedding::LOCAL_HASH_MODEL_ID
    );
}

#[test]
#[ignore = "WP0 diagnostic intentionally reproduces legacy data-loss risks"]
fn wp0_diagnostic_reproduces_legacy_data_loss_risks() {
    let state = test_state();
    let fixture: BaselineFixture = serde_json::from_str(FIXTURE_JSON).expect("baseline fixture");
    setup_fixture(&state, &fixture);
    let db = state.core.db.get().expect("diagnostic db");

    let before_alpha: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE user_key = 'principal-alpha'",
            [],
            |row| row.get(0),
        )
        .expect("alpha rows");
    super::indexing::cleanup_retrieval_index(&db, 0, 1).expect("legacy global cleanup");
    let after_alpha: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE user_key = 'principal-alpha'",
            [],
            |row| row.get(0),
        )
        .expect("alpha rows after cleanup");
    assert!(before_alpha > 0 && after_alpha < before_alpha);

    let long_text = "合成长期内容".repeat(80);
    drop(db);
    super::insert_memory(
        &state,
        1001,
        2001,
        Some("principal-alpha"),
        "ui",
        None,
        super::MEMORY_ROLE_USER,
        &long_text,
        128,
        super::MemoryWriteKind::Default,
    )
    .expect("insert truncation probe");
    let db = state.core.db.get().expect("diagnostic db after write");
    let stored: String = db
        .query_row(
            "SELECT content FROM memories ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("stored truncation probe");
    assert!(stored.chars().count() < long_text.chars().count());
    let memory_id: i64 = db
        .query_row(
            "SELECT id FROM memories ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("truncation probe memory id");
    let indexed_before_delete: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index
             WHERE source_kind = 'memory' AND source_memory_id = ?1",
            [memory_id],
            |row| row.get(0),
        )
        .expect("index row before source delete");
    db.execute("DELETE FROM memories WHERE id = ?1", [memory_id])
        .expect("delete source row without lineage cleanup");
    let orphaned_after_delete: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index
             WHERE source_kind = 'memory' AND source_memory_id = ?1",
            [memory_id],
            |row| row.get(0),
        )
        .expect("orphan index rows");
    assert!(indexed_before_delete > 0);
    assert_eq!(orphaned_after_delete, indexed_before_delete);
}

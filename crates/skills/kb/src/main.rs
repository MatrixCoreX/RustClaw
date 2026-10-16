use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use skill_sdk::{SkillPathPolicy, SkillProgressEmitter};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_CHUNK_SIZE: usize = 1200;
const DEFAULT_CHUNK_OVERLAP: usize = 180;
const DEFAULT_TOP_K: usize = 5;
const SKILL_NAME: &str = "kb";
const MAX_NAMESPACE_CHARS: usize = 128;
const MAX_QUERY_CHARS: usize = 8_192;
const MAX_PATH_CHARS: usize = 4_096;
type KbProgressCallback<'a> = dyn FnMut(u64, u64) -> Result<()> + 'a;

mod ingest;
mod ingest_extract;
mod ingest_scan;
mod storage;

use ingest::{
    do_cancel_ingest, do_ingest_job_status, do_ingest_with_progress, do_reindex_with_progress,
    do_resume_ingest_with_progress, parse_ingest_paths,
};
#[cfg(test)]
use ingest::{do_ingest, do_reindex, do_resume_ingest, parse_ingest_args};
#[cfg(test)]
use ingest_scan::build_scan_targets;

#[derive(Debug, Deserialize)]
struct SkillRequest {
    request_id: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    context: Option<Value>,
    #[serde(default)]
    user_id: i64,
    #[serde(default)]
    chat_id: i64,
    #[serde(default)]
    user_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SkillStorageContext {
    schema_version: u32,
    skill_name: String,
    storage_kind: String,
    database_path: String,
    database_busy_timeout_ms: u64,
    database_resource_profile: String,
    #[serde(default)]
    database_cache_size_kib: Option<u32>,
    #[serde(default)]
    database_mmap_size_bytes: Option<u64>,
    #[serde(default)]
    database_temp_store: Option<String>,
}

#[derive(Debug, Serialize)]
struct SkillResponse {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Clone)]
struct KbRuntime {
    scope_user_key: String,
    workspace_root: PathBuf,
    storage_database_path: PathBuf,
    storage_busy_timeout_ms: u64,
    storage_resource_profile: String,
    storage_cache_size_kib: Option<u32>,
    storage_mmap_size_bytes: Option<u64>,
    storage_temp_store: Option<String>,
    path_policy: SkillPathPolicy,
}

impl KbRuntime {
    fn constrained_storage(&self) -> bool {
        self.storage_resource_profile == "constrained"
    }

    fn active_job_limit(&self) -> u64 {
        if self.constrained_storage() {
            1
        } else {
            4
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DocMeta {
    path: String,
    file_type: String,
    mtime_epoch: i64,
    size: u64,
    chunks: usize,
    #[serde(default)]
    content_sha256: String,
    #[serde(default = "default_parser_version")]
    parser_version: String,
    #[serde(default = "default_chunker_version")]
    chunker_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chunk {
    chunk_id: String,
    path: String,
    file_type: String,
    offset: usize,
    text: String,
    len_tokens: usize,
    mtime_epoch: i64,
    #[serde(default)]
    text_sha256: String,
    #[serde(default = "default_content_kind")]
    content_kind: String,
    #[serde(default)]
    location: ingest_extract::DocumentLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct NamespaceIndex {
    namespace: String,
    #[serde(default)]
    owner_user_key: String,
    updated_at_epoch: i64,
    next_chunk_seq: u64,
    #[serde(default)]
    revision: u64,
    #[serde(default = "default_parser_version")]
    parser_version: String,
    #[serde(default = "default_chunker_version")]
    chunker_version: String,
    #[serde(default = "default_embedding_version")]
    embedding_version: String,
    docs: HashMap<String, DocMeta>, // key: path
    chunks: Vec<Chunk>,
}

#[derive(Debug, Clone)]
struct SearchArgs {
    namespace: String,
    query: String,
    top_k: usize,
    path_prefix: Option<String>,
    file_type: Option<String>,
    time_from: Option<i64>,
    time_to: Option<i64>,
    min_score: f64,
}

#[derive(Debug, Clone, Serialize)]
struct SearchHit {
    schema_version: u8,
    evidence_id: String,
    source_ref: String,
    namespace: String,
    namespace_revision: u64,
    document_id: String,
    document_digest: String,
    chunk_id: String,
    chunk_ids: Vec<String>,
    path: String,
    file_type: String,
    location: Value,
    text: String,
    score: SearchScore,
    retrieval_modes: Vec<&'static str>,
    instruction_authority: &'static str,
    hit_terms: Vec<String>,
    metadata: Value,
}

#[derive(Debug, Clone, Serialize)]
struct SearchScore {
    #[serde(rename = "final")]
    final_score: f64,
    lexical: f64,
    semantic: f64,
    freshness: f64,
    authority: f64,
    rerank: f64,
}

#[derive(Debug, Clone)]
struct StatsArgs {
    namespace: Option<String>,
}

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<SkillRequest, _> = serde_json::from_str(&line);
        let response = match parsed {
            Ok(req) => {
                let mut progress = SkillProgressEmitter::new(&mut stdout, &req.request_id);
                emit_start_progress(&mut progress, &req)?;
                let mut ingest_progress = |current, total| {
                    progress
                        .emit_progress(
                            "kb.ingest.progress",
                            BTreeMap::new(),
                            Some(current),
                            Some(total),
                        )
                        .map_err(anyhow::Error::from)
                };
                execute_request(req, Some(&mut ingest_progress))
            }
            Err(err) => SkillResponse {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn emit_start_progress(
    progress: &mut SkillProgressEmitter<'_, impl Write>,
    request: &SkillRequest,
) -> Result<()> {
    let action = request
        .args
        .get("action")
        .and_then(Value::as_str)
        .filter(|action| {
            matches!(
                *action,
                "ingest"
                    | "search"
                    | "list_namespaces"
                    | "list_documents"
                    | "remove_documents"
                    | "delete_namespace"
                    | "reindex"
                    | "resume_ingest"
                    | "ingest_job_status"
                    | "cancel_ingest"
                    | "start_vector_reindex"
                    | "resume_vector_reindex"
                    | "vector_reindex_status"
                    | "cancel_vector_reindex"
                    | "stats"
            )
        })
        .unwrap_or("unknown");
    progress.emit_progress(
        "kb.operation.starting",
        BTreeMap::from([("action".to_string(), Value::String(action.to_string()))]),
        Some(0),
        Some(1),
    )?;
    Ok(())
}

fn execute_request(
    req: SkillRequest,
    mut progress: Option<&mut KbProgressCallback<'_>>,
) -> SkillResponse {
    let runtime = build_runtime_context(&req);
    let action = req
        .args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let result = runtime.and_then(|runtime| match action.as_str() {
        "ingest" => do_ingest_with_progress(&runtime, &req.args, progress.as_deref_mut()),
        "search" => do_search(&runtime, &req.args),
        "list_namespaces" => do_list_namespaces(&runtime),
        "list_documents" => do_list_documents(&runtime, &req.args),
        "remove_documents" => do_remove_documents(&runtime, &req.args),
        "delete_namespace" => do_delete_namespace(&runtime, &req.args),
        "reindex" => do_reindex_with_progress(&runtime, &req.args, progress.as_deref_mut()),
        "resume_ingest" => {
            do_resume_ingest_with_progress(&runtime, &req.args, progress.as_deref_mut())
        }
        "ingest_job_status" => do_ingest_job_status(&runtime, &req.args),
        "cancel_ingest" => do_cancel_ingest(&runtime, &req.args),
        "start_vector_reindex" => do_start_vector_reindex(&runtime, &req.args),
        "resume_vector_reindex" => do_resume_vector_reindex(&runtime, &req.args),
        "vector_reindex_status" => do_vector_reindex_status(&runtime, &req.args),
        "cancel_vector_reindex" => do_cancel_vector_reindex(&runtime, &req.args),
        "stats" => do_stats(&runtime, &req.args),
        _ => Err(anyhow!(
            "action must be ingest|search|list_namespaces|list_documents|remove_documents|delete_namespace|reindex|resume_ingest|ingest_job_status|cancel_ingest|start_vector_reindex|resume_vector_reindex|vector_reindex_status|cancel_vector_reindex|stats"
        )),
    });
    match result {
        Ok(extra) => SkillResponse {
            request_id: req.request_id,
            status: "ok".to_string(),
            text: extra.to_string(),
            extra: Some(extra),
            error_text: None,
        },
        Err(err) => SkillResponse {
            request_id: req.request_id,
            status: "error".to_string(),
            text: String::new(),
            extra: Some(
                err.downcast_ref::<ingest::CheckpointConflict>()
                    .map(ingest::CheckpointConflict::extra)
                    .or_else(|| {
                        err.downcast_ref::<storage::StoragePolicyError>()
                            .map(storage::StoragePolicyError::extra)
                    })
                    .unwrap_or_else(|| error_extra("execution_failed")),
            ),
            error_text: Some(err.to_string()),
        },
    }
}

fn error_extra(error_kind: &str) -> Value {
    json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_code": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn build_runtime_context(req: &SkillRequest) -> Result<KbRuntime> {
    let scope_user_key = req
        .user_key
        .as_deref()
        .or_else(|| {
            req.context
                .as_ref()
                .and_then(|ctx| ctx.get("user_key"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("anon:{}:{}", req.user_id, req.chat_id));
    let workspace_root = req
        .context
        .as_ref()
        .and_then(|ctx| ctx.get("workspace_root"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(workspace_root);
    let storage = req
        .context
        .as_ref()
        .and_then(|context| context.get("skill_storage"))
        .cloned()
        .ok_or_else(|| anyhow!("KB skill storage descriptor is required"))?;
    let storage: SkillStorageContext =
        serde_json::from_value(storage).context("KB skill storage descriptor is malformed")?;
    if storage.schema_version != storage::STORAGE_SCHEMA_VERSION
        || storage.skill_name != SKILL_NAME
        || storage.storage_kind != "sqlite"
        || !matches!(
            storage.database_resource_profile.as_str(),
            "standard" | "constrained"
        )
    {
        return Err(anyhow!("KB skill storage descriptor is invalid"));
    }
    let path_policy = SkillPathPolicy::new(&workspace_root, req.context.as_ref())
        .map_err(|error| anyhow!("{}: {}", error.code, error.detail))?;
    let runtime = KbRuntime {
        scope_user_key,
        workspace_root,
        storage_database_path: PathBuf::from(&storage.database_path),
        storage_busy_timeout_ms: storage.database_busy_timeout_ms.max(1),
        storage_resource_profile: storage.database_resource_profile,
        storage_cache_size_kib: storage.database_cache_size_kib,
        storage_mmap_size_bytes: storage.database_mmap_size_bytes,
        storage_temp_store: storage.database_temp_store,
        path_policy,
    };
    storage::initialize(&runtime)?;
    Ok(runtime)
}

fn do_search(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let search_started = Instant::now();
    let s = parse_search_args(args)?;
    if s.query.trim().is_empty() {
        return Err(anyhow!("query is required"));
    }
    let query_digest = sha256_hex(s.query.as_bytes());
    let query_projection = claw_core::retrieval_embedding::project_retrieval_tokens(
        &s.query,
        claw_core::retrieval_embedding::DEFAULT_RETRIEVAL_TOKEN_BUDGET,
    );
    let q_terms = query_projection.tokens.clone();
    let filter_count = search_filter_count(&s);
    if q_terms.is_empty() {
        return Ok(json!({
            "status":"ok",
            "hits":[],
            "summary":"no effective query terms",
            "stats":{
                "total_candidates":0,
                "retrieval_trace": search_trace(
                    &query_digest,
                    filter_count,
                    0,
                    0,
                    &[],
                    &query_projection.metadata,
                    None,
                    None,
                    SearchLatencyMs {
                        total: elapsed_ms(search_started),
                        ..SearchLatencyMs::default()
                    },
                    Some("query_projection_empty")
                )
            }
        }));
    }
    let normalized_path_prefix = s
        .path_prefix
        .as_deref()
        .map(|prefix| normalize_search_path_prefix(&runtime.workspace_root, prefix))
        .filter(|prefix| !prefix.is_empty());
    let embedding_started = Instant::now();
    let query_vector = claw_core::retrieval_embedding::embed_local_hash_with_context(
        &s.query,
        claw_core::retrieval_embedding::EmbeddingPurpose::Query,
        &runtime.scope_user_key,
    )?
    .vector;
    let embedding_ms = elapsed_ms(embedding_started);
    let candidate_started = Instant::now();
    let candidates = storage::load_search_candidates(
        runtime,
        &s.namespace,
        &q_terms,
        &query_vector,
        storage::SearchFilters {
            path_prefix: normalized_path_prefix.as_deref(),
            file_type: s.file_type.as_deref(),
            time_from: s.time_from,
            time_to: s.time_to,
        },
        s.top_k.saturating_mul(64).max(256),
    )
    .map_err(|_| anyhow!("namespace not found or unreadable: {}", s.namespace))?;
    let candidate_generation_ms = elapsed_ms(candidate_started);
    let storage::SearchCandidateSet {
        index,
        total_chunks,
        retrieval_mode,
        lexical_chunk_ids,
        semantic_scores,
        embedding_profile_id,
        vector_generation,
        vector_degradation,
        lexical_latency_ms,
        semantic_latency_ms,
        materialization_latency_ms,
        semantic_backend,
        semantic_rows_scored,
    } = candidates;
    let lexical_candidate_count = lexical_chunk_ids.len();
    let semantic_candidate_count = semantic_scores.len();
    let vector_maintenance = vector_maintenance_value(&s.namespace, vector_degradation.as_deref());
    let filtered_chunks = index.chunks.iter().collect::<Vec<_>>();
    let after_filters = filtered_chunks.len();
    let n_docs = filtered_chunks.len() as f64;
    if n_docs <= 0.0 {
        return Ok(json!({
            "action": "search",
            "status":"ok",
            "namespace": s.namespace,
            "namespace_revision": index.revision,
            "hits":[],
            "result_kind":"no_matching_evidence",
            "maintenance": vector_maintenance,
            "stats":{
                "total_candidates": total_chunks,
                "retrieval_candidates": index.chunks.len(),
                "after_filters": 0,
                "retrieval_mode": retrieval_mode,
                "embedding_profile_id": embedding_profile_id,
                "vector_generation": vector_generation,
                "vector_candidate_backend": semantic_backend,
                "vector_rows_scored": semantic_rows_scored,
                "degradation": vector_degradation,
                "retrieval_trace": search_trace(
                    &query_digest,
                    filter_count,
                    lexical_candidate_count,
                    semantic_candidate_count,
                    &[],
                    &query_projection.metadata,
                    Some(embedding_profile_id.as_str()),
                    vector_generation,
                    SearchLatencyMs {
                        embedding: embedding_ms,
                        fts: lexical_latency_ms,
                        vector: semantic_latency_ms,
                        materialization: materialization_latency_ms,
                        candidate_generation: candidate_generation_ms,
                        total: elapsed_ms(search_started),
                        ..SearchLatencyMs::default()
                    },
                    vector_degradation.as_deref()
                ),
            }
        }));
    }

    let rerank_started = Instant::now();
    let mut hits = vec![];
    for c in filtered_chunks {
        let tf = term_freq(&c.text);
        let mut hit_terms = vec![];
        let mut matched_occurrences = 0usize;
        for t in &q_terms {
            let occurrences = *tf.get(t).unwrap_or(&0);
            if occurrences == 0 {
                continue;
            }
            hit_terms.push(t.clone());
            matched_occurrences = matched_occurrences.saturating_add(occurrences);
        }
        hit_terms.sort();
        hit_terms.dedup();
        let lexical = if lexical_chunk_ids.contains(&c.chunk_id) && !hit_terms.is_empty() {
            let coverage = hit_terms.len() as f64 / q_terms.len().max(1) as f64;
            let saturation = 1.0 - (-(matched_occurrences as f64)).exp();
            (coverage * 0.85 + saturation * 0.15).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let semantic = semantic_scores
            .get(&c.chunk_id)
            .copied()
            .unwrap_or_default()
            .clamp(0.0, 1.0);
        if lexical <= 0.0 && semantic < 0.35 {
            continue;
        }
        let (final_score, retrieval_modes) = match (lexical > 0.0, semantic >= 0.35) {
            (true, true) => (
                stable_hybrid_score(lexical, semantic),
                vec!["lexical", "semantic"],
            ),
            (true, false) => (lexical, vec!["lexical"]),
            (false, true) => (calibrated_semantic_only_score(semantic), vec!["semantic"]),
            (false, false) => continue,
        };
        if final_score < s.min_score {
            continue;
        }
        let document_digest = index
            .docs
            .get(&c.path)
            .map(|document| document.content_sha256.clone())
            .unwrap_or_default();
        let document_id = document_digest.chars().take(16).collect::<String>();
        let path = public_source_path(&c.path);
        let source_ref = format!(
            "kb://{}/{}/{}",
            s.namespace,
            if document_id.is_empty() {
                "unknown"
            } else {
                &document_id
            },
            c.chunk_id
        );
        let location = retrieval_location(&path, c.offset, &c.location);
        hits.push(SearchHit {
            schema_version: 2,
            evidence_id: claw_core::retrieval_embedding::retrieval_evidence_id(
                &source_ref,
                &document_digest,
                index.revision,
            ),
            source_ref,
            namespace: s.namespace.clone(),
            namespace_revision: index.revision,
            document_id,
            document_digest,
            chunk_id: c.chunk_id.clone(),
            chunk_ids: vec![c.chunk_id.clone()],
            path: path.clone(),
            file_type: c.file_type.clone(),
            location: location.clone(),
            text: c.text.clone(),
            score: SearchScore {
                final_score: round_score(final_score),
                lexical: round_score(lexical),
                semantic: round_score(semantic),
                freshness: 0.0,
                authority: 1.0,
                rerank: round_score(final_score),
            },
            retrieval_modes,
            instruction_authority: "none",
            hit_terms,
            metadata: json!({
                "path": path,
                "file_type": c.file_type,
                "mtime_epoch": c.mtime_epoch,
                "chunk_id": c.chunk_id,
                "offset": c.offset,
                "content_kind": c.content_kind,
                "source_location": c.location,
            }),
        });
    }

    hits = merge_adjacent_search_hits(hits);
    hits.sort_by(|a, b| {
        b.score
            .final_score
            .partial_cmp(&a.score.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.source_ref.cmp(&b.source_ref))
    });
    hits = select_document_diverse_hits(hits, s.top_k);
    let rerank_ms = elapsed_ms(rerank_started);
    let selected_hit_ids = hits
        .iter()
        .map(|hit| hit.evidence_id.clone())
        .collect::<Vec<_>>();
    let retrieval_trace = search_trace(
        &query_digest,
        filter_count,
        lexical_candidate_count,
        semantic_candidate_count,
        &selected_hit_ids,
        &query_projection.metadata,
        Some(embedding_profile_id.as_str()),
        vector_generation,
        SearchLatencyMs {
            embedding: embedding_ms,
            fts: lexical_latency_ms,
            vector: semantic_latency_ms,
            materialization: materialization_latency_ms,
            candidate_generation: candidate_generation_ms,
            rerank: rerank_ms,
            total: elapsed_ms(search_started),
        },
        vector_degradation.as_deref(),
    );

    Ok(json!({
        "action": "search",
        "status":"ok",
        "namespace": s.namespace,
        "namespace_revision": index.revision,
        "parser_version": index.parser_version,
        "chunker_version": index.chunker_version,
        "embedding_version": index.embedding_version,
        "hits": hits,
        "result_kind": if hits.is_empty() { "no_matching_evidence" } else { "evidence_found" },
        "maintenance": vector_maintenance,
        "stats": {
            "corpus_chunks": total_chunks,
            "total_candidates": index.chunks.len(),
            "retrieval_candidates": index.chunks.len(),
            "after_filters": after_filters,
            "returned_hits": hits.len(),
            "top_k": s.top_k,
            "retrieval_mode": retrieval_mode,
            "embedding_profile_id": embedding_profile_id,
            "vector_generation": vector_generation,
            "vector_candidate_backend": semantic_backend,
            "vector_rows_scored": semantic_rows_scored,
            "degradation": vector_degradation,
            "retrieval_trace": retrieval_trace
        }
    }))
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct SearchLatencyMs {
    embedding: f64,
    fts: f64,
    vector: f64,
    materialization: f64,
    candidate_generation: f64,
    rerank: f64,
    total: f64,
}

fn elapsed_ms(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 1_000.0 * 1_000.0).round() / 1_000.0
}

fn search_filter_count(search: &SearchArgs) -> usize {
    [
        search.path_prefix.is_some(),
        search.file_type.is_some(),
        search.time_from.is_some(),
        search.time_to.is_some(),
        search.min_score > 0.0,
    ]
    .into_iter()
    .filter(|applied| *applied)
    .count()
}

#[allow(clippy::too_many_arguments)]
fn search_trace(
    query_digest: &str,
    filter_count: usize,
    lexical_candidate_count: usize,
    semantic_candidate_count: usize,
    selected_hit_ids: &[String],
    token_projection: &claw_core::retrieval_embedding::RetrievalTokenProjectionMetadata,
    embedding_profile_id: Option<&str>,
    vector_generation: Option<u64>,
    latency_ms: SearchLatencyMs,
    degradation: Option<&str>,
) -> Value {
    json!({
        "schema_version": 1,
        "query_digest": query_digest,
        "filter_count": filter_count,
        "candidate_count_by_source": {
            "lexical": lexical_candidate_count,
            "semantic": semantic_candidate_count,
        },
        "embedding_profile_id": embedding_profile_id,
        "vector_generation": vector_generation,
        "latency_ms": latency_ms,
        "selected_hit_ids": selected_hit_ids,
        "selected_hit_count": selected_hit_ids.len(),
        "token_projection": token_projection,
        "degradation": degradation,
        "no_answer": selected_hit_ids.is_empty(),
    })
}

fn vector_maintenance_value(namespace: &str, degradation: Option<&str>) -> Value {
    match degradation {
        Some(
            error_code @ ("embedding_projection_missing"
            | "embedding_projection_stale"
            | "ann_projection_stale"),
        ) => {
            json!({
                "required": true,
                "error_code": error_code,
                "message_key": format!("skill.kb.{error_code}"),
                "suggested_action": {
                    "action": "start_vector_reindex",
                    "namespace": namespace,
                }
            })
        }
        Some(error_code) => json!({
            "required": false,
            "error_code": error_code,
            "message_key": format!("skill.kb.{error_code}"),
            "suggested_action": null,
        }),
        None => json!({
            "required": false,
            "error_code": null,
            "message_key": null,
            "suggested_action": null,
        }),
    }
}

fn stable_hybrid_score(lexical: f64, semantic: f64) -> f64 {
    (lexical.clamp(0.0, 1.0) * 0.7 + semantic.clamp(0.0, 1.0) * 0.3).clamp(0.0, 1.0)
}

fn calibrated_semantic_only_score(semantic: f64) -> f64 {
    (semantic.clamp(0.0, 1.0) * 0.75).clamp(0.0, 1.0)
}

fn merge_adjacent_search_hits(mut hits: Vec<SearchHit>) -> Vec<SearchHit> {
    hits.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| hit_start_offset(left).cmp(&hit_start_offset(right)))
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    let mut merged: Vec<SearchHit> = Vec::with_capacity(hits.len());
    for mut hit in hits {
        let offset = hit_start_offset(&hit);
        if let Some(metadata) = hit.metadata.as_object_mut() {
            metadata.insert("chunk_ids".to_string(), json!(hit.chunk_ids));
        }
        if let Some(location) = hit.location.as_object_mut() {
            location.insert("end_offset".to_string(), json!(offset));
        }
        let can_merge = merged.last().is_some_and(|previous| {
            previous.document_id == hit.document_id
                && previous.path == hit.path
                && previous.chunk_ids.len() < 3
                && previous.metadata.get("content_kind") == hit.metadata.get("content_kind")
                && previous.metadata.get("source_location") == hit.metadata.get("source_location")
                && hit_end_offset(previous).saturating_add(1) == offset
                && previous.text.len().saturating_add(hit.text.len()) <= 16_384
        });
        if !can_merge {
            merged.push(hit);
            continue;
        }
        let previous = merged.last_mut().expect("adjacent hit exists");
        if !previous.text.ends_with('\n') {
            previous.text.push('\n');
        }
        previous.text.push_str(&hit.text);
        previous.chunk_ids.extend(hit.chunk_ids);
        previous.score.final_score = previous.score.final_score.max(hit.score.final_score);
        previous.score.lexical = previous.score.lexical.max(hit.score.lexical);
        previous.score.semantic = previous.score.semantic.max(hit.score.semantic);
        previous.score.freshness = previous.score.freshness.max(hit.score.freshness);
        previous.score.authority = previous.score.authority.min(hit.score.authority);
        previous.score.rerank = previous.score.rerank.max(hit.score.rerank);
        for mode in hit.retrieval_modes {
            if !previous.retrieval_modes.contains(&mode) {
                previous.retrieval_modes.push(mode);
            }
        }
        previous.hit_terms.extend(hit.hit_terms);
        previous.hit_terms.sort();
        previous.hit_terms.dedup();
        if let Some(location) = previous.location.as_object_mut() {
            location.insert("end_offset".to_string(), json!(offset));
        }
        if let Some(metadata) = previous.metadata.as_object_mut() {
            metadata.insert("chunk_ids".to_string(), json!(previous.chunk_ids));
            metadata.insert("merged_adjacent".to_string(), Value::Bool(true));
        }
        if let (Some(first), Some(last)) = (previous.chunk_ids.first(), previous.chunk_ids.last()) {
            previous.source_ref = format!(
                "kb://{}/{}/{}..{}",
                previous.namespace,
                if previous.document_id.is_empty() {
                    "unknown"
                } else {
                    &previous.document_id
                },
                first,
                last,
            );
            previous.evidence_id = claw_core::retrieval_embedding::retrieval_evidence_id(
                &previous.source_ref,
                &previous.document_digest,
                previous.namespace_revision,
            );
        }
    }
    merged
}

fn hit_start_offset(hit: &SearchHit) -> u64 {
    hit.location
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or_default()
}

fn hit_end_offset(hit: &SearchHit) -> u64 {
    hit.location
        .get("end_offset")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| hit_start_offset(hit))
}

fn select_document_diverse_hits(hits: Vec<SearchHit>, limit: usize) -> Vec<SearchHit> {
    let mut seen_documents = std::collections::HashSet::new();
    let mut first_per_document = Vec::new();
    let mut overflow = Vec::new();
    for hit in hits {
        let document_key = if hit.document_id.is_empty() {
            hit.path.clone()
        } else {
            hit.document_id.clone()
        };
        if seen_documents.insert(document_key) {
            first_per_document.push(hit);
        } else {
            overflow.push(hit);
        }
    }
    first_per_document.extend(overflow);
    first_per_document.truncate(limit.max(1));
    first_per_document
}

fn do_list_namespaces(runtime: &KbRuntime) -> Result<Value> {
    let mut namespaces = Vec::new();
    for index in storage::list_namespaces(runtime)? {
        namespaces.push(json!({
            "namespace": index.namespace,
            "docs": index.document_count,
            "chunks": index.chunk_count,
            "updated_at_epoch": index.updated_at_epoch,
            "namespace_revision": index.revision,
            "parser_version": index.parser_version,
            "chunker_version": index.chunker_version,
            "embedding_version": index.embedding_version,
            "active_vector_generation": index.active_vector_generation,
            "storage_kind": "sqlite"
        }));
    }
    namespaces.sort_by_key(|item| {
        std::cmp::Reverse(
            item.get("updated_at_epoch")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
        )
    });
    let namespace_count = namespaces.len();
    let names: Vec<Value> = namespaces
        .iter()
        .filter_map(|item| item.get("namespace").and_then(Value::as_str))
        .map(|namespace| json!(namespace))
        .collect();
    Ok(json!({
        "status": "ok",
        "namespaces": namespaces,
        "names": names,
        "count": namespace_count,
        "namespace_count": namespace_count,
        "summary": format!("found {} namespace(s)", namespace_count)
    }))
}

fn do_list_documents(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let namespace = required_namespace(args, "list_documents")?;
    let limit = match args.get("limit") {
        Some(value) => value
            .as_u64()
            .filter(|limit| (1..=500).contains(limit))
            .ok_or_else(|| anyhow!("list_documents limit must be between 1 and 500"))?
            as usize,
        None => 100,
    };
    if args
        .get("continuation")
        .is_some_and(|value| !value.is_object())
    {
        return Err(anyhow!("list_documents continuation must be an object"));
    }
    let after_path = args
        .pointer("/continuation/after_path")
        .or_else(|| args.get("after_path"))
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| {
                    !value.is_empty()
                        && value.chars().count() <= MAX_PATH_CHARS
                        && !value.chars().any(char::is_control)
                })
                .ok_or_else(|| anyhow!("list_documents cursor exceeds its allowed boundary"))
        })
        .transpose()?;
    let page = storage::list_documents(runtime, &namespace, after_path.as_deref(), limit)
        .map_err(|_| anyhow!("namespace not found or unreadable: {namespace}"))?;
    let documents = page
        .documents
        .iter()
        .map(|doc| {
            json!({
                "path": doc.path,
                "file_type": doc.file_type,
                "mtime_epoch": doc.mtime_epoch,
                "size": doc.size,
                "chunk_count": doc.chunks,
                "content_sha256": doc.content_sha256,
                "parser_version": doc.parser_version,
                "chunker_version": doc.chunker_version,
            })
        })
        .collect::<Vec<_>>();
    let returned_count = documents.len();
    let continuation = page
        .next_after_path
        .as_ref()
        .map(|path| json!({"after_path": path}));
    Ok(json!({
        "action": "list_documents",
        "status": "ok",
        "namespace": namespace,
        "namespace_revision": page.summary.revision,
        "documents": documents,
        "document_count": page.summary.document_count,
        "returned_count": returned_count,
        "chunk_count": page.summary.chunk_count,
        "has_more": page.has_more,
        "continuation": continuation,
        "summary": format!("found {} document(s)", page.summary.document_count),
    }))
}

fn do_remove_documents(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let namespace = required_namespace(args, "remove_documents")?;
    let requested = parse_ingest_paths(args)?;
    let mut index = storage::load_namespace(runtime, &namespace)
        .map_err(|_| anyhow!("namespace not found or unreadable: {namespace}"))?;
    let mut removed_paths = Vec::new();
    let mut missing_paths = Vec::new();
    for path in requested {
        let normalized = normalize_managed_document_path(&runtime.workspace_root, &path)?;
        if index.docs.contains_key(&normalized) {
            remove_doc_from_index(&mut index, &normalized);
            removed_paths.push(normalized);
        } else {
            missing_paths.push(normalized);
        }
    }
    index.updated_at_epoch = now_epoch();
    let persisted = storage::save_namespace(runtime, &index)?;
    let removed_count = removed_paths.len();
    Ok(json!({
        "action": "remove_documents",
        "status": "ok",
        "namespace": namespace,
        "namespace_revision": persisted.revision,
        "removed_paths": removed_paths,
        "removed_count": removed_count,
        "missing_paths": missing_paths,
        "remaining_documents": persisted.total_docs,
        "remaining_chunks": persisted.total_chunks,
        "idempotent_success": removed_count == 0,
    }))
}

fn do_delete_namespace(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let namespace = required_namespace(args, "delete_namespace")?;
    let removed = storage::delete_namespace(runtime, &namespace)?;
    Ok(json!({
        "action": "delete_namespace",
        "status": "ok",
        "namespace": namespace,
        "deleted": true,
        "cleanup_status": "cleaned",
        "removed_documents": removed.removed_docs,
        "removed_chunks": removed.removed_chunks,
    }))
}

fn do_start_vector_reindex(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let namespace = required_namespace(args, "start_vector_reindex")?;
    let batch_size = vector_reindex_batch_size(runtime, args)?;
    let status = storage::start_vector_reindex(runtime, &namespace)?;
    let status = if status.is_resumable() {
        storage::resume_vector_reindex(runtime, &namespace, status.generation, batch_size)?
    } else {
        status
    };
    Ok(vector_reindex_status_value(
        "start_vector_reindex",
        &status,
        batch_size,
    ))
}

fn do_resume_vector_reindex(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let namespace = required_namespace(args, "resume_vector_reindex")?;
    let generation = required_vector_generation(args, "resume_vector_reindex")?;
    let batch_size = vector_reindex_batch_size(runtime, args)?;
    let status = storage::resume_vector_reindex(runtime, &namespace, generation, batch_size)?;
    Ok(vector_reindex_status_value(
        "resume_vector_reindex",
        &status,
        batch_size,
    ))
}

fn do_vector_reindex_status(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let namespace = required_namespace(args, "vector_reindex_status")?;
    let generation = args
        .get("generation")
        .map(|value| {
            value
                .as_u64()
                .filter(|generation| *generation > 0)
                .ok_or_else(|| anyhow!("vector_reindex_status requires a positive generation"))
        })
        .transpose()?;
    let status = storage::load_vector_reindex_status(runtime, &namespace, generation)?;
    Ok(vector_reindex_status_value(
        "vector_reindex_status",
        &status,
        32,
    ))
}

fn do_cancel_vector_reindex(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let namespace = required_namespace(args, "cancel_vector_reindex")?;
    let generation = required_vector_generation(args, "cancel_vector_reindex")?;
    let status = storage::cancel_vector_reindex(runtime, &namespace, generation)?;
    Ok(vector_reindex_status_value(
        "cancel_vector_reindex",
        &status,
        32,
    ))
}

fn required_vector_generation(args: &Value, action: &str) -> Result<u64> {
    args.get("generation")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or_else(|| anyhow!("{action} requires a positive generation"))
}

fn vector_reindex_batch_size(runtime: &KbRuntime, args: &Value) -> Result<usize> {
    match args.get("batch_size") {
        None => Ok(if runtime.constrained_storage() { 8 } else { 32 }),
        Some(value) => value
            .as_u64()
            .filter(|value| (1..=256).contains(value))
            .map(|value| value as usize)
            .ok_or_else(|| anyhow!("batch_size must be an integer between 1 and 256")),
    }
}

fn vector_reindex_status_value(
    action: &str,
    status: &storage::VectorReindexStatus,
    batch_size: usize,
) -> Value {
    let complete = status.is_complete();
    let continuation = status.is_resumable().then(|| {
        json!({
            "action": "resume_vector_reindex",
            "namespace": status.namespace,
            "generation": status.generation,
            "batch_size": batch_size,
        })
    });
    let degradation = (status.state == "stale").then_some("embedding_projection_stale");
    json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "action": action,
        "status": "ok",
        "namespace": status.namespace,
        "namespace_revision": status.namespace_revision,
        "profile_id": status.profile_id,
        "generation": status.generation,
        "active_generation": status.active_generation,
        "generation_state": status.state,
        "job_status": if complete { "completed" } else if status.is_resumable() { "waiting" } else { status.state.as_str() },
        "complete": complete,
        "continuation": continuation,
        "degradation": degradation,
        "progress": {
            "indexed_rows": status.indexed_rows,
            "expected_rows": status.expected_rows,
        },
        "checkpoint": {
            "generation": status.generation,
            "cursor_chunk_id": status.cursor_chunk_id,
            "source_digest": status.source_digest,
            "projection_digest": status.projection_digest,
        },
        "created_at_epoch": status.created_at_epoch,
        "updated_at_epoch": status.updated_at_epoch,
    })
}

fn do_stats(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let stats = parse_stats_args(args)?;
    if let Some(namespace) = stats.namespace {
        let index = storage::list_documents(runtime, &namespace, None, 1)
            .with_context(|| format!("load namespace failed: {namespace}"))?;
        let index = index.summary;
        let document_count = index.document_count;
        let chunk_count = index.chunk_count;
        let file_types = storage::namespace_file_type_counts(runtime, &namespace)?;
        return Ok(json!({
            "action": "stats",
            "status": "ok",
            "namespace": namespace,
            "document_count": document_count,
            "chunk_count": chunk_count,
            "namespace_revision": index.revision,
            "parser_version": index.parser_version,
            "chunker_version": index.chunker_version,
            "embedding_version": index.embedding_version,
            "stats": {
                "docs": document_count,
                "chunks": chunk_count,
                "document_count": document_count,
                "chunk_count": chunk_count,
                "updated_at_epoch": index.updated_at_epoch,
                "file_types": file_types
            },
            "summary": "namespace stats ready"
        }));
    }
    let namespaces = do_list_namespaces(runtime)?;
    let count = namespaces
        .get("namespaces")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or_default();
    Ok(json!({
        "action": "stats",
        "status": "ok",
        "stats": {
            "namespace_count": count,
            "storage": storage::storage_summary(runtime)
        },
        "summary": format!("{} namespace(s) available", count)
    }))
}

fn parse_search_args(args: &Value) -> Result<SearchArgs> {
    let namespace = args
        .get("namespace")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("search requires namespace"))?
        .trim()
        .to_string();
    validate_namespace_value(&namespace)?;
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if query.is_empty()
        || query.chars().count() > MAX_QUERY_CHARS
        || query.chars().any(char::is_control)
    {
        return Err(anyhow!(
            "search query is empty or exceeds its allowed boundary"
        ));
    }
    let top_k = match args.get("top_k") {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| anyhow!("search top_k must be an integer"))?
            as usize,
        None => DEFAULT_TOP_K,
    };
    if !(1..=50).contains(&top_k) {
        return Err(anyhow!("search top_k must be between 1 and 50"));
    }
    let filters = args.get("filters");
    if filters.is_some_and(|value| !value.is_object()) {
        return Err(anyhow!("search filters must be an object"));
    }
    if filters.and_then(Value::as_object).is_some_and(|filters| {
        filters.keys().any(|key| {
            !matches!(
                key.as_str(),
                "path_prefix" | "file_type" | "time_from" | "time_to"
            )
        })
    }) {
        return Err(anyhow!("search filters contain unsupported fields"));
    }
    let path_prefix_value = filters
        .and_then(|f| f.get("path_prefix"))
        .or_else(|| args.get("path_prefix"));
    let path_prefix = path_prefix_value
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| {
                    !value.is_empty()
                        && value.chars().count() <= MAX_PATH_CHARS
                        && !value.chars().any(char::is_control)
                })
                .map(str::to_string)
                .ok_or_else(|| anyhow!("search path_prefix exceeds its allowed boundary"))
        })
        .transpose()?;
    let file_type_value = filters
        .and_then(|f| f.get("file_type"))
        .or_else(|| args.get("file_type"));
    let file_type = file_type_value
        .map(|value| {
            value
                .as_str()
                .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
                .filter(|value| {
                    !value.is_empty() && value.len() <= 32 && !value.chars().any(char::is_control)
                })
                .ok_or_else(|| anyhow!("search file_type exceeds its allowed boundary"))
        })
        .transpose()?;
    let time_from = filters
        .and_then(|f| f.get("time_from"))
        .or_else(|| args.get("time_from"))
        .map(|value| parse_epoch_value(value, "time_from"))
        .transpose()?;
    let time_to = filters
        .and_then(|f| f.get("time_to"))
        .or_else(|| args.get("time_to"))
        .map(|value| parse_epoch_value(value, "time_to"))
        .transpose()?;
    let min_score = match args.get("min_score") {
        Some(value) => value
            .as_f64()
            .ok_or_else(|| anyhow!("search min_score must be numeric"))?,
        None => 0.0,
    };
    if !min_score.is_finite() || !(0.0..=1.0).contains(&min_score) {
        return Err(anyhow!("search min_score must be between 0 and 1"));
    }
    if matches!((time_from, time_to), (Some(from), Some(to)) if from > to) {
        return Err(anyhow!("search time range is invalid"));
    }
    Ok(SearchArgs {
        namespace,
        query,
        top_k,
        path_prefix,
        file_type,
        time_from,
        time_to,
        min_score,
    })
}

fn parse_stats_args(args: &Value) -> Result<StatsArgs> {
    let namespace = args
        .get("namespace")
        .map(|value| {
            let namespace = value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("stats namespace must be a non-empty string"))?
                .to_string();
            validate_namespace_value(&namespace)?;
            Ok::<_, anyhow::Error>(namespace)
        })
        .transpose()?;
    Ok(StatsArgs { namespace })
}

fn required_namespace(args: &Value, action: &str) -> Result<String> {
    let namespace = args
        .get("namespace")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|namespace| !namespace.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{action} requires namespace"))?;
    validate_namespace_value(&namespace)?;
    Ok(namespace)
}

pub(crate) fn validate_namespace_value(namespace: &str) -> Result<()> {
    let chars = namespace.chars().count();
    if chars == 0 || chars > MAX_NAMESPACE_CHARS || namespace.chars().any(char::is_control) {
        return Err(anyhow!("namespace exceeds its allowed boundary"));
    }
    Ok(())
}

fn normalize_managed_document_path(workspace_root: &Path, raw: &str) -> Result<String> {
    let path = Path::new(raw.trim());
    if raw.trim().is_empty()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(anyhow!("managed document path is invalid"));
    }
    Ok(if path.is_absolute() {
        storage_path_for(path, workspace_root)
    } else {
        normalize_path_string(path)
    })
}

fn parse_chunker_setting(version: &str, index: usize) -> Option<usize> {
    version.split(':').nth(index)?.parse().ok()
}

fn round_score(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn public_source_path(raw: &str) -> String {
    let path = Path::new(raw);
    let unsafe_component = path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    });
    if path.is_absolute() || unsafe_component {
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document")
            .to_string();
    }
    raw.to_string()
}

fn term_freq(text: &str) -> HashMap<String, usize> {
    let mut tf = HashMap::new();
    for t in tokenize(text) {
        *tf.entry(t).or_insert(0) += 1;
    }
    tf
}

fn tokenize(text: &str) -> Vec<String> {
    tokenize_terms(text)
}

fn split_chunks(text: &str, chunk_size: usize, chunk_overlap: usize) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n");
    let mut sections = Vec::new();
    let mut current = String::new();
    for line in normalized.lines() {
        let trimmed = line.trim();
        let is_heading = trimmed.starts_with('#');
        let line_len = line.chars().count();
        let current_len = current.chars().count();
        if is_heading && !current.trim().is_empty() && current_len >= chunk_size / 3 {
            sections.push(current.trim().to_string());
            current.clear();
        }
        if current_len > 0 && current_len + line_len + 1 > chunk_size && !current.trim().is_empty()
        {
            sections.push(current.trim().to_string());
            let overlap_text = tail_chars(&current, chunk_overlap);
            current = overlap_text;
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
        if trimmed.is_empty()
            && !current.trim().is_empty()
            && current.chars().count() >= chunk_size / 2
        {
            sections.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        sections.push(current.trim().to_string());
    }
    let mut out = Vec::new();
    for section in sections {
        if section.chars().count() <= chunk_size {
            out.push(section);
            continue;
        }
        let chars = section.chars().collect::<Vec<_>>();
        let mut start = 0usize;
        while start < chars.len() {
            let hard_end = (start + chunk_size).min(chars.len());
            let end = preferred_chunk_boundary(&chars, start, hard_end, chunk_size);
            let chunk = chars[start..end]
                .iter()
                .collect::<String>()
                .trim()
                .to_string();
            if !chunk.is_empty() {
                out.push(chunk);
            }
            if end >= chars.len() {
                break;
            }
            start = end.saturating_sub(chunk_overlap.max(1));
        }
    }
    out
}

fn preferred_chunk_boundary(
    chars: &[char],
    start: usize,
    hard_end: usize,
    chunk_size: usize,
) -> usize {
    if hard_end >= chars.len() {
        return hard_end;
    }
    let minimum = start.saturating_add((chunk_size / 2).max(1));
    (minimum..hard_end)
        .rev()
        .find_map(|index| {
            matches!(
                chars[index],
                '\n' | '\r' | '。' | '！' | '？' | '.' | '!' | '?' | ';' | '；'
            )
            .then_some(index + 1)
        })
        .filter(|boundary| *boundary > start)
        .unwrap_or(hard_end)
}

fn tail_chars(text: &str, keep: usize) -> String {
    if keep == 0 {
        return String::new();
    }
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= keep {
        return text.trim().to_string();
    }
    chars[chars.len() - keep..]
        .iter()
        .collect::<String>()
        .trim()
        .to_string()
}

fn remove_doc_from_index(index: &mut NamespaceIndex, path: &str) {
    index.docs.remove(path);
    index.chunks.retain(|c| c.path != path);
}

fn document_is_current(
    doc: &DocMeta,
    content_sha256: &str,
    parser_version: &str,
    chunker_version: &str,
) -> bool {
    doc.content_sha256 == content_sha256
        && doc.parser_version == parser_version
        && doc.chunker_version == chunker_version
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn mtime_epoch(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn parse_epoch_value(v: &Value, field: &str) -> Result<i64> {
    if let Some(n) = v.as_i64() {
        return Ok(n);
    }
    v.as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| anyhow!("search {field} must be an epoch integer"))
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join("Cargo.toml").exists() && cur.join("crates").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

fn tokenize_terms(text: &str) -> Vec<String> {
    claw_core::retrieval_embedding::tokenize_retrieval_text(text)
}

fn storage_path_for(path: &Path, workspace_root: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(workspace_root) {
        return normalize_path_string(rel);
    }
    normalize_path_string(path)
}

fn normalize_path_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

fn normalize_search_path_prefix(workspace_root: &Path, raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        return storage_path_for(&path, workspace_root);
    }
    normalize_path_string(Path::new(trimmed))
}

fn workspace_root() -> PathBuf {
    if let Ok(root) = env::var("WORKSPACE_ROOT") {
        let path = PathBuf::from(root);
        if path.is_absolute() {
            return path;
        }
    }
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    find_workspace_root(&cwd).unwrap_or(cwd)
}

fn default_parser_version() -> String {
    "typed-content-v3".to_string()
}

fn default_content_kind() -> String {
    "plain_text".to_string()
}

fn default_chunker_version() -> String {
    chunker_version(DEFAULT_CHUNK_SIZE, DEFAULT_CHUNK_OVERLAP)
}

fn chunker_version(chunk_size: usize, chunk_overlap: usize) -> String {
    format!("structured-block-window-v3:{chunk_size}:{chunk_overlap}")
}

fn retrieval_location(
    path: &str,
    offset: usize,
    source: &ingest_extract::DocumentLocation,
) -> Value {
    let mut location = serde_json::to_value(source)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    location.insert("path".to_string(), json!(path));
    location.insert("offset".to_string(), json!(offset));
    Value::Object(location)
}

fn default_embedding_version() -> String {
    claw_core::retrieval_embedding::LOCAL_HASH_VERSION.to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

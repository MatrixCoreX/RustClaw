use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use skill_sdk::{SkillPathPolicy, SkillProgressFrame, SkillProgressKind};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CHUNK_SIZE: usize = 1200;
const DEFAULT_CHUNK_OVERLAP: usize = 180;
const DEFAULT_TOP_K: usize = 5;
const SKILL_NAME: &str = "kb";

mod ingest;
mod ingest_extract;
mod ingest_scan;
mod storage;

#[cfg(test)]
use ingest::parse_ingest_args;
use ingest::{
    do_cancel_ingest, do_ingest, do_ingest_job_status, do_reindex, do_resume_ingest,
    parse_ingest_paths,
};
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
    path_policy: SkillPathPolicy,
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
    chunk_id: String,
    path: String,
    file_type: String,
    offset: usize,
    text: String,
    score: f64,
    hit_terms: Vec<String>,
    score_reason: String,
    metadata: Value,
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
                emit_start_progress(&mut stdout, &req)?;
                execute_request(req)
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

fn emit_start_progress(stdout: &mut impl Write, request: &SkillRequest) -> Result<()> {
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
                    | "stats"
            )
        })
        .unwrap_or("unknown");
    let frame = SkillProgressFrame {
        schema_version: skill_sdk::SKILL_PROGRESS_FRAME_SCHEMA_VERSION,
        record_type: skill_sdk::SKILL_PROGRESS_FRAME_RECORD_TYPE.to_string(),
        request_id: request.request_id.clone(),
        sequence: 1,
        kind: SkillProgressKind::Progress,
        detail_key: "kb.operation.starting".to_string(),
        params: BTreeMap::from([("action".to_string(), Value::String(action.to_string()))]),
        current: Some(0),
        total: Some(1),
        reference: None,
    };
    writeln!(stdout, "{}", frame.to_line()?)?;
    stdout.flush()?;
    Ok(())
}

fn execute_request(req: SkillRequest) -> SkillResponse {
    let runtime = build_runtime_context(&req);
    let action = req
        .args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let result = runtime.and_then(|runtime| match action.as_str() {
        "ingest" => do_ingest(&runtime, &req.args),
        "search" => do_search(&runtime, &req.args),
        "list_namespaces" => do_list_namespaces(&runtime),
        "list_documents" => do_list_documents(&runtime, &req.args),
        "remove_documents" => do_remove_documents(&runtime, &req.args),
        "delete_namespace" => do_delete_namespace(&runtime, &req.args),
        "reindex" => do_reindex(&runtime, &req.args),
        "resume_ingest" => do_resume_ingest(&runtime, &req.args),
        "ingest_job_status" => do_ingest_job_status(&runtime, &req.args),
        "cancel_ingest" => do_cancel_ingest(&runtime, &req.args),
        "stats" => do_stats(&runtime, &req.args),
        _ => Err(anyhow!(
            "action must be ingest|search|list_namespaces|list_documents|remove_documents|delete_namespace|reindex|resume_ingest|ingest_job_status|cancel_ingest|stats"
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
            extra: Some(error_extra("execution_failed")),
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
    if storage.schema_version != 3
        || storage.skill_name != SKILL_NAME
        || storage.storage_kind != "sqlite"
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
        path_policy,
    };
    storage::initialize(&runtime)?;
    Ok(runtime)
}

fn do_search(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let s = parse_search_args(args)?;
    if s.query.trim().is_empty() {
        return Err(anyhow!("query is required"));
    }
    let q_terms = tokenize(&s.query);
    if q_terms.is_empty() {
        return Ok(
            json!({"status":"ok","hits":[],"summary":"no effective query terms","stats":{"total_candidates":0}}),
        );
    }
    let candidates = storage::load_search_candidates(
        runtime,
        &s.namespace,
        &q_terms,
        s.top_k.saturating_mul(64).max(256),
    )
    .map_err(|_| anyhow!("namespace not found or unreadable: {}", s.namespace))?;
    let index = candidates.index;
    let total_chunks = candidates.total_chunks;
    let retrieval_mode = candidates.retrieval_mode;

    let normalized_path_prefix = s
        .path_prefix
        .as_deref()
        .map(|prefix| normalize_search_path_prefix(&runtime.workspace_root, prefix))
        .filter(|prefix| !prefix.is_empty());
    let filtered_chunks = index
        .chunks
        .iter()
        .filter(|c| {
            pass_filters(
                c,
                normalized_path_prefix.as_deref(),
                s.file_type.as_deref(),
                s.time_from,
                s.time_to,
            )
        })
        .collect::<Vec<_>>();
    let after_filters = filtered_chunks.len();
    let n_docs = filtered_chunks.len() as f64;
    if n_docs <= 0.0 {
        return Ok(json!({
            "action": "search",
            "status":"ok",
            "namespace": s.namespace,
            "namespace_revision": index.revision,
            "hits":[],
            "summary":"no matching chunks under filters",
            "stats":{
                "total_candidates": total_chunks,
                "retrieval_candidates": index.chunks.len(),
                "after_filters": 0,
                "retrieval_mode": retrieval_mode,
            }
        }));
    }

    let avgdl = filtered_chunks
        .iter()
        .map(|c| c.len_tokens.max(1) as f64)
        .sum::<f64>()
        / n_docs;
    let df = build_df(&filtered_chunks);
    let k1 = 1.5;
    let b = 0.75;

    let mut hits = vec![];
    for c in filtered_chunks {
        let tf = term_freq(&c.text);
        let mut score = 0.0f64;
        let mut hit_terms = vec![];
        for t in &q_terms {
            let f = *tf.get(t).unwrap_or(&0) as f64;
            if f <= 0.0 {
                continue;
            }
            hit_terms.push(t.clone());
            let dfi = *df.get(t).unwrap_or(&0) as f64;
            let idf = ((n_docs - dfi + 0.5) / (dfi + 0.5) + 1.0).ln();
            let dl = c.len_tokens.max(1) as f64;
            let den = f + k1 * (1.0 - b + b * dl / avgdl.max(1.0));
            score += idf * (f * (k1 + 1.0)) / den.max(1e-9);
        }
        if score < s.min_score || hit_terms.is_empty() {
            continue;
        }
        hit_terms.sort();
        hit_terms.dedup();
        let score_reason = format!(
            "bm25 over {} terms; matched {}; dl={}; avgdl={:.1}",
            q_terms.len(),
            hit_terms.len(),
            c.len_tokens,
            avgdl
        );
        hits.push(SearchHit {
            chunk_id: c.chunk_id.clone(),
            path: c.path.clone(),
            file_type: c.file_type.clone(),
            offset: c.offset,
            text: c.text.clone(),
            score: (score * 1000.0).round() / 1000.0,
            hit_terms,
            score_reason,
            metadata: json!({
                "path": c.path,
                "file_type": c.file_type,
                "mtime_epoch": c.mtime_epoch,
                "chunk_id": c.chunk_id,
                "offset": c.offset
            }),
        });
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if hits.len() > s.top_k {
        hits.truncate(s.top_k);
    }

    Ok(json!({
        "action": "search",
        "status":"ok",
        "namespace": s.namespace,
        "namespace_revision": index.revision,
        "parser_version": index.parser_version,
        "chunker_version": index.chunker_version,
        "embedding_version": index.embedding_version,
        "hits": hits,
        "summary": format!("found {} hit(s) for query", hits.len()),
        "stats": {
            "total_candidates": total_chunks,
            "retrieval_candidates": index.chunks.len(),
            "after_filters": after_filters,
            "returned_hits": hits.len(),
            "top_k": s.top_k,
            "retrieval_mode": retrieval_mode
        }
    }))
}

fn do_list_namespaces(runtime: &KbRuntime) -> Result<Value> {
    let mut namespaces = Vec::new();
    for index in storage::list_namespaces(runtime)? {
        namespaces.push(json!({
            "namespace": index.namespace,
            "docs": index.docs.len(),
            "chunks": index.chunks.len(),
            "updated_at_epoch": index.updated_at_epoch,
            "namespace_revision": index.revision,
            "parser_version": index.parser_version,
            "chunker_version": index.chunker_version,
            "embedding_version": index.embedding_version,
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
    let index = storage::load_namespace(runtime, &namespace)
        .map_err(|_| anyhow!("namespace not found or unreadable: {namespace}"))?;
    let mut documents = index
        .docs
        .values()
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
    documents.sort_by(|left, right| {
        left.get("path")
            .and_then(Value::as_str)
            .cmp(&right.get("path").and_then(Value::as_str))
    });
    let document_count = documents.len();
    Ok(json!({
        "action": "list_documents",
        "status": "ok",
        "namespace": namespace,
        "namespace_revision": index.revision,
        "documents": documents,
        "document_count": document_count,
        "chunk_count": index.chunks.len(),
        "summary": format!("found {} document(s)", document_count),
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

fn do_stats(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let stats = parse_stats_args(args)?;
    if let Some(namespace) = stats.namespace {
        let index = storage::load_namespace(runtime, &namespace)
            .with_context(|| format!("load namespace failed: {namespace}"))?;
        let document_count = index.docs.len();
        let chunk_count = index.chunks.len();
        let file_types =
            index
                .docs
                .values()
                .fold(HashMap::<String, usize>::new(), |mut acc, doc| {
                    *acc.entry(doc.file_type.clone()).or_insert(0) += 1;
                    acc
                });
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
    if namespace.is_empty() {
        return Err(anyhow!("search requires namespace"));
    }
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let top_k = args
        .get("top_k")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_TOP_K)
        .clamp(1, 50);
    let filters = args.get("filters");
    let path_prefix = filters
        .and_then(|f| f.get("path_prefix"))
        .or_else(|| args.get("path_prefix"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let file_type = filters
        .and_then(|f| f.get("file_type"))
        .or_else(|| args.get("file_type"))
        .and_then(Value::as_str)
        .map(|s| s.trim_start_matches('.').to_ascii_lowercase());
    let time_from = filters
        .and_then(|f| f.get("time_from"))
        .or_else(|| args.get("time_from"))
        .and_then(parse_epoch_value);
    let time_to = filters
        .and_then(|f| f.get("time_to"))
        .or_else(|| args.get("time_to"))
        .and_then(parse_epoch_value);
    let min_score = args.get("min_score").and_then(Value::as_f64).unwrap_or(0.0);
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
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(StatsArgs { namespace })
}

fn required_namespace(args: &Value, action: &str) -> Result<String> {
    args.get("namespace")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|namespace| !namespace.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{action} requires namespace"))
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

fn pass_filters(
    c: &Chunk,
    path_prefix: Option<&str>,
    file_type: Option<&str>,
    time_from: Option<i64>,
    time_to: Option<i64>,
) -> bool {
    if let Some(prefix) = path_prefix {
        if !(c.path == prefix || c.path.starts_with(&format!("{prefix}/"))) {
            return false;
        }
    }
    if let Some(ft) = file_type {
        if c.file_type != ft {
            return false;
        }
    }
    if let Some(from) = time_from {
        if c.mtime_epoch < from {
            return false;
        }
    }
    if let Some(to) = time_to {
        if c.mtime_epoch > to {
            return false;
        }
    }
    true
}

fn build_df(chunks: &[&Chunk]) -> HashMap<String, usize> {
    let mut df: HashMap<String, usize> = HashMap::new();
    for c in chunks {
        let mut seen = HashSet::new();
        for t in tokenize(&c.text) {
            if seen.insert(t.clone()) {
                *df.entry(t).or_insert(0) += 1;
            }
        }
    }
    df
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
            let end = (start + chunk_size).min(chars.len());
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

fn parse_epoch_value(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    v.as_str().and_then(|s| s.parse::<i64>().ok())
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
    let lower = text.to_ascii_lowercase();
    let mut out = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_string())
        .collect::<Vec<_>>();
    let cjk = text
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .collect::<String>();
    let chars = cjk.chars().collect::<Vec<_>>();
    for window in chars.windows(2).take(32) {
        out.push(window.iter().collect::<String>());
    }
    out.sort();
    out.dedup();
    out
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
    "typed-content-v2".to_string()
}

fn default_chunker_version() -> String {
    chunker_version(DEFAULT_CHUNK_SIZE, DEFAULT_CHUNK_OVERLAP)
}

fn chunker_version(chunk_size: usize, chunk_overlap: usize) -> String {
    format!("heading-window-v1:{chunk_size}:{chunk_overlap}")
}

fn default_embedding_version() -> String {
    "local-hash-v1".to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

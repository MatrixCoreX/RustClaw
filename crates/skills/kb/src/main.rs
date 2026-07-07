use anyhow::{anyhow, Context, Result};
use claw_core::config::AppConfig;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_CHUNK_SIZE: usize = 1200;
const DEFAULT_CHUNK_OVERLAP: usize = 180;
const DEFAULT_TOP_K: usize = 5;
const DEFAULT_MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
const SKILL_NAME: &str = "kb";

#[derive(Debug, Deserialize)]
struct SkillRequest {
    request_id: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    context: Option<SkillContext>,
    #[serde(default)]
    user_id: i64,
    #[serde(default)]
    chat_id: i64,
    #[serde(default)]
    user_key: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct SkillContext {
    #[serde(default)]
    user_key: Option<String>,
    #[serde(default)]
    workspace_root: Option<String>,
    #[serde(default)]
    database_sqlite_path: Option<String>,
    #[serde(default)]
    database_busy_timeout_ms: Option<u64>,
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
    unified_index_db_path: Option<PathBuf>,
    unified_index_busy_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DocMeta {
    path: String,
    file_type: String,
    mtime_epoch: i64,
    size: u64,
    chunks: usize,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct NamespaceIndex {
    namespace: String,
    #[serde(default)]
    owner_user_key: String,
    updated_at_epoch: i64,
    next_chunk_seq: u64,
    docs: HashMap<String, DocMeta>, // key: path
    chunks: Vec<Chunk>,
}

#[derive(Debug, Clone)]
struct IngestArgs {
    namespace: String,
    paths: Vec<String>,
    chunk_size: usize,
    chunk_overlap: usize,
    overwrite: bool,
    file_types: HashSet<String>,
    max_file_size: u64,
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
            Ok(req) => execute_request(req),
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
        "stats" => do_stats(&runtime, &req.args),
        _ => Err(anyhow!(
            "action must be ingest|search|list_namespaces|stats"
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
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn build_runtime_context(req: &SkillRequest) -> Result<KbRuntime> {
    let scope_user_key = req
        .user_key
        .as_deref()
        .or_else(|| req.context.as_ref().and_then(|ctx| ctx.user_key.as_deref()))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("anon:{}:{}", req.user_id, req.chat_id));
    let workspace_root = req
        .context
        .as_ref()
        .and_then(|ctx| ctx.workspace_root.as_deref())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(workspace_root);
    let unified_index_db_path = req
        .context
        .as_ref()
        .and_then(|ctx| ctx.database_sqlite_path.as_deref())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace_root.join(path)
            }
        });
    Ok(KbRuntime {
        scope_user_key,
        workspace_root,
        unified_index_db_path,
        unified_index_busy_timeout_ms: req
            .context
            .as_ref()
            .and_then(|ctx| ctx.database_busy_timeout_ms),
    })
}

fn do_ingest(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let ingest = parse_ingest_args(args)?;
    let scan_targets = build_scan_targets(runtime, &ingest.paths)?;
    let mut index = if ingest.overwrite {
        NamespaceIndex {
            namespace: ingest.namespace.clone(),
            owner_user_key: runtime.scope_user_key.clone(),
            updated_at_epoch: now_epoch(),
            next_chunk_seq: 1,
            docs: HashMap::new(),
            chunks: vec![],
        }
    } else {
        let namespace_file = ns_file(runtime, &ingest.namespace);
        if namespace_file.exists() {
            load_namespace(runtime, &ingest.namespace)?
        } else {
            NamespaceIndex {
                namespace: ingest.namespace.clone(),
                owner_user_key: runtime.scope_user_key.clone(),
                updated_at_epoch: now_epoch(),
                next_chunk_seq: 1,
                docs: HashMap::new(),
                chunks: vec![],
            }
        }
    };
    index.owner_user_key = runtime.scope_user_key.clone();

    let all_files = collect_target_files(&scan_targets)?;
    let current_paths = all_files
        .iter()
        .map(|path| storage_path_for(path, &runtime.workspace_root))
        .collect::<HashSet<_>>();

    let mut warnings = vec![];
    let mut ingested_docs = 0usize;
    let mut skipped_files = 0usize;
    let mut removed_docs = 0usize;

    for file in all_files {
        let meta =
            fs::metadata(&file).with_context(|| format!("stat failed: {}", file.display()))?;
        if !meta.is_file() {
            continue;
        }
        let size = meta.len();
        let file_type = file
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !ingest.file_types.is_empty() && !ingest.file_types.contains(&file_type) {
            skipped_files += 1;
            continue;
        }
        if size > ingest.max_file_size {
            skipped_files += 1;
            warnings.push(format!(
                "skip large file {} ({} bytes > max_file_size {})",
                file.display(),
                size,
                ingest.max_file_size
            ));
            continue;
        }

        let path_str = storage_path_for(&file, &runtime.workspace_root);
        let legacy_absolute_path = normalize_path_string(&file);
        let mtime = mtime_epoch(&meta);
        let unchanged = index
            .docs
            .get(&path_str)
            .or_else(|| index.docs.get(&legacy_absolute_path))
            .map(|d| d.mtime_epoch == mtime && d.size == size)
            .unwrap_or(false);
        if unchanged && !ingest.overwrite {
            continue;
        }

        let before =
            index.docs.contains_key(&path_str) || index.docs.contains_key(&legacy_absolute_path);
        remove_doc_from_index(&mut index, &path_str);
        if legacy_absolute_path != path_str {
            remove_doc_from_index(&mut index, &legacy_absolute_path);
        }
        if before {
            removed_docs += 1;
        }

        let text = read_text_lossy(&file)?;
        if text.trim().is_empty() {
            skipped_files += 1;
            warnings.push(format!("skip empty text file {}", path_str));
            continue;
        }
        let chunks = split_chunks(&text, ingest.chunk_size, ingest.chunk_overlap);
        let mut chunk_count = 0usize;
        for (offset, chunk_text) in chunks.into_iter().enumerate() {
            let chunk_id = format!("{}-{}", ingest.namespace, index.next_chunk_seq);
            index.next_chunk_seq += 1;
            let len_tokens = tokenize(&chunk_text).len();
            index.chunks.push(Chunk {
                chunk_id,
                path: path_str.clone(),
                file_type: file_type.clone(),
                offset,
                text: chunk_text,
                len_tokens,
                mtime_epoch: mtime,
            });
            chunk_count += 1;
        }
        index.docs.insert(
            path_str.clone(),
            DocMeta {
                path: path_str,
                file_type,
                mtime_epoch: mtime,
                size,
                chunks: chunk_count,
            },
        );
        ingested_docs += 1;
    }

    if !ingest.overwrite {
        let stale_paths = index
            .docs
            .keys()
            .filter(|path| {
                !current_paths.contains(*path)
                    && path_matches_any_scan_target(Path::new(path), &scan_targets)
            })
            .cloned()
            .collect::<Vec<_>>();
        for path in stale_paths {
            remove_doc_from_index(&mut index, &path);
            removed_docs += 1;
        }
    }

    index.updated_at_epoch = now_epoch();
    save_namespace(runtime, &index)?;
    let (unified_index_synced, unified_index_rows) =
        match sync_namespace_to_unified_index(runtime, &index) {
            Ok(row_count) => (true, row_count),
            Err(err) => {
                warnings.push(format!("unified index sync failed: {err}"));
                return Err(anyhow!(warnings.join("; ")));
            }
        };
    let total_docs = index.docs.len();
    let total_chunks = index.chunks.len();
    let warnings_empty = warnings.is_empty();
    let effective_success = kb_ingest_effective_success(
        ingested_docs,
        total_docs,
        unified_index_synced,
        warnings_empty,
    );
    let idempotent_success = ingested_docs == 0 && effective_success;
    let result_kind = kb_ingest_result_kind(
        ingested_docs,
        total_docs,
        unified_index_synced,
        unified_index_rows,
        warnings_empty,
    );

    Ok(json!({
        "action": "ingest",
        "status":"ok",
        "effective_status": if effective_success { "ok" } else { "needs_attention" },
        "result_kind": result_kind,
        "effective_success": effective_success,
        "idempotent_success": idempotent_success,
        "namespace": ingest.namespace,
        "path": ingest.paths.first().cloned().unwrap_or_default(),
        "paths": ingest.paths,
        "summary": result_kind,
        "stats": {
            "ingested_docs": ingested_docs,
            "removed_docs": removed_docs,
            "total_docs": total_docs,
            "total_chunks": total_chunks,
            "skipped_files": skipped_files,
            "chunk_size": ingest.chunk_size,
            "chunk_overlap": ingest.chunk_overlap,
            "unified_index_synced": unified_index_synced,
            "unified_index_rows": unified_index_rows,
            "warnings": warnings
        }
    }))
}

fn kb_ingest_effective_success(
    ingested_docs: usize,
    total_docs: usize,
    unified_index_synced: bool,
    warnings_empty: bool,
) -> bool {
    warnings_empty && unified_index_synced && (ingested_docs > 0 || total_docs > 0)
}

fn kb_ingest_result_kind(
    ingested_docs: usize,
    total_docs: usize,
    unified_index_synced: bool,
    unified_index_rows: usize,
    warnings_empty: bool,
) -> &'static str {
    if ingested_docs > 0 {
        "updated"
    } else if warnings_empty && total_docs > 0 && unified_index_synced && unified_index_rows > 0 {
        "already_indexed"
    } else if total_docs > 0 {
        "no_new_documents"
    } else {
        "no_documents_indexed"
    }
}

fn do_search(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let s = parse_search_args(args)?;
    let index = load_namespace(runtime, &s.namespace)
        .map_err(|_| anyhow!("namespace not found or unreadable: {}", s.namespace))?;
    if s.query.trim().is_empty() {
        return Err(anyhow!("query is required"));
    }
    let q_terms = tokenize(&s.query);
    if q_terms.is_empty() {
        return Ok(
            json!({"status":"ok","hits":[],"summary":"no effective query terms","stats":{"total_candidates":0}}),
        );
    }

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
        return Ok(
            json!({"status":"ok","hits":[],"summary":"no matching chunks under filters","stats":{"total_candidates":0}}),
        );
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
        "hits": hits,
        "summary": format!("found {} hit(s) for query", hits.len()),
        "stats": {
            "total_candidates": index.chunks.len(),
            "after_filters": after_filters,
            "returned_hits": hits.len(),
            "top_k": s.top_k
        }
    }))
}

fn do_list_namespaces(runtime: &KbRuntime) -> Result<Value> {
    let mut namespaces = Vec::new();
    let root = kb_root(runtime);
    if !root.exists() {
        return Ok(json!({
            "status": "ok",
            "namespaces": [],
            "summary": "no namespace indexes found"
        }));
    }
    for entry in
        fs::read_dir(&root).with_context(|| format!("read_dir failed: {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)?;
        let index: NamespaceIndex = serde_json::from_str(&raw)?;
        if !index.owner_user_key.trim().is_empty()
            && index.owner_user_key != runtime.scope_user_key.as_str()
        {
            continue;
        }
        namespaces.push(json!({
            "namespace": index.namespace,
            "docs": index.docs.len(),
            "chunks": index.chunks.len(),
            "updated_at_epoch": index.updated_at_epoch,
            "path": path.display().to_string()
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

fn do_stats(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let stats = parse_stats_args(args)?;
    if let Some(namespace) = stats.namespace {
        let index = load_namespace(runtime, &namespace)
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
            "kb_root": kb_root(runtime).display().to_string()
        },
        "summary": format!("{} namespace(s) available", count)
    }))
}

fn parse_ingest_args(args: &Value) -> Result<IngestArgs> {
    let namespace = args
        .get("namespace")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("ingest requires namespace"))?
        .trim()
        .to_string();
    if namespace.is_empty() {
        return Err(anyhow!("ingest requires namespace"));
    }
    let paths = parse_ingest_paths(args)?;
    if paths.is_empty() {
        return Err(anyhow!("paths[] must not be empty"));
    }
    let chunk_size = args
        .get("chunk_size")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_CHUNK_SIZE)
        .clamp(200, 8000);
    let chunk_overlap = args
        .get("chunk_overlap")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_CHUNK_OVERLAP)
        .min(chunk_size / 3)
        .min(400);
    let overwrite = args
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let file_types = args
        .get("file_types")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim_start_matches('.').to_ascii_lowercase())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let max_file_size = args
        .get("max_file_size")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_FILE_SIZE);
    Ok(IngestArgs {
        namespace,
        paths,
        chunk_size,
        chunk_overlap,
        overwrite,
        file_types,
        max_file_size,
    })
}

fn parse_ingest_paths(args: &Value) -> Result<Vec<String>> {
    let mut paths = match args.get("paths") {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>(),
        Some(Value::String(path)) if !path.trim().is_empty() => vec![path.trim().to_string()],
        _ => Vec::new(),
    };
    if paths.is_empty() {
        if let Some(path) = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            paths.push(path.to_string());
        }
    }
    if paths.is_empty() {
        return Err(anyhow!("ingest requires paths[]"));
    }
    Ok(paths)
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

#[derive(Debug, Clone)]
struct ScanTarget {
    root: PathBuf,
    is_file: bool,
    storage_prefix: String,
}

fn build_scan_targets(runtime: &KbRuntime, raw_paths: &[String]) -> Result<Vec<ScanTarget>> {
    let mut out = Vec::new();
    for raw in raw_paths {
        let resolved = resolve_input_path(&runtime.workspace_root, raw);
        let canonical = fs::canonicalize(&resolved)
            .with_context(|| format!("path not found: {}", resolved.display()))?;
        let meta = fs::metadata(&canonical)
            .with_context(|| format!("stat failed: {}", canonical.display()))?;
        let storage_prefix = storage_path_for(&canonical, &runtime.workspace_root);
        out.push(ScanTarget {
            root: canonical,
            is_file: meta.is_file(),
            storage_prefix,
        });
    }
    Ok(out)
}

fn collect_target_files(targets: &[ScanTarget]) -> Result<Vec<PathBuf>> {
    let mut seen = HashSet::new();
    let mut visited_dirs = HashSet::new();
    let mut out = Vec::new();
    for target in targets {
        collect_files(&target.root, &mut out, &mut seen, &mut visited_dirs)?;
    }
    out.sort();
    Ok(out)
}

fn collect_files(
    path: &Path,
    out: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
    visited_dirs: &mut HashSet<PathBuf>,
) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("path not found: {}", path.display()));
    }
    if path.is_file() {
        let canonical = fs::canonicalize(path)
            .with_context(|| format!("canonicalize failed: {}", path.display()))?;
        if seen.insert(canonical.clone()) {
            out.push(canonical);
        }
        return Ok(());
    }
    let canonical_dir = fs::canonicalize(path)
        .with_context(|| format!("canonicalize failed: {}", path.display()))?;
    if !visited_dirs.insert(canonical_dir) {
        return Ok(());
    }
    for ent in fs::read_dir(path).with_context(|| format!("read_dir failed: {}", path.display()))? {
        let ent = ent?;
        let p = ent.path();
        if p.is_dir() {
            collect_files(&p, out, seen, visited_dirs)?;
        } else if p.is_file() {
            let canonical = fs::canonicalize(&p)
                .with_context(|| format!("canonicalize failed: {}", p.display()))?;
            if seen.insert(canonical.clone()) {
                out.push(canonical);
            }
        }
    }
    Ok(())
}

fn path_matches_any_scan_target(path: &Path, targets: &[ScanTarget]) -> bool {
    let stored = normalize_path_string(path);
    targets.iter().any(|target| {
        let absolute_match = if target.is_file {
            path == target.root
        } else {
            path.starts_with(&target.root)
        };
        if absolute_match {
            return true;
        }
        if target.is_file {
            return stored == target.storage_prefix;
        }
        if target.storage_prefix.is_empty() {
            return !Path::new(path).is_absolute();
        }
        stored == target.storage_prefix
            || stored.starts_with(&format!("{}/", target.storage_prefix))
    })
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

fn read_text_lossy(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let text = String::from_utf8(bytes.clone())
        .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).to_string());
    Ok(text)
}

fn remove_doc_from_index(index: &mut NamespaceIndex, path: &str) {
    index.docs.remove(path);
    index.chunks.retain(|c| c.path != path);
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

fn kb_root(runtime: &KbRuntime) -> PathBuf {
    if let Ok(p) = env::var("KB_ROOT") {
        let pb = PathBuf::from(p);
        if pb.is_absolute() {
            return pb
                .join("by_user")
                .join(storage_segment(&runtime.scope_user_key));
        }
        return runtime
            .workspace_root
            .join(pb)
            .join("by_user")
            .join(storage_segment(&runtime.scope_user_key));
    }
    runtime
        .workspace_root
        .join("data")
        .join("kb")
        .join("by_user")
        .join(storage_segment(&runtime.scope_user_key))
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

fn ns_file(runtime: &KbRuntime, namespace: &str) -> PathBuf {
    kb_root(runtime).join(format!("{}.json", storage_segment(namespace)))
}

fn storage_segment(input: &str) -> String {
    let preview = sanitize_fragment(input);
    format!("{preview}--{:016x}", stable_hash64(input.trim()))
}

fn sanitize_fragment(input: &str) -> String {
    let cleaned = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(24)
        .collect::<String>();
    if cleaned.trim_matches('_').is_empty() {
        "scope".to_string()
    } else {
        cleaned
    }
}

fn stable_hash64(input: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
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

fn resolve_input_path(workspace_root: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    }
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

fn load_namespace(runtime: &KbRuntime, namespace: &str) -> Result<NamespaceIndex> {
    let p = ns_file(runtime, namespace);
    let raw =
        fs::read_to_string(&p).with_context(|| format!("read index failed: {}", p.display()))?;
    let mut idx: NamespaceIndex =
        serde_json::from_str(&raw).with_context(|| "index json parse failed")?;
    if idx.namespace.is_empty() {
        idx.namespace = namespace.to_string();
    }
    if idx.owner_user_key.trim().is_empty() {
        idx.owner_user_key = runtime.scope_user_key.clone();
    }
    if idx.owner_user_key != runtime.scope_user_key.as_str() {
        return Err(anyhow!("namespace is owned by another user scope"));
    }
    Ok(idx)
}

fn save_namespace(runtime: &KbRuntime, index: &NamespaceIndex) -> Result<()> {
    let root = kb_root(runtime);
    fs::create_dir_all(&root).with_context(|| format!("mkdir failed: {}", root.display()))?;
    let p = ns_file(runtime, &index.namespace);
    let raw = serde_json::to_string_pretty(index)?;
    fs::write(&p, raw).with_context(|| format!("write index failed: {}", p.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
fn sync_namespace_to_unified_index(runtime: &KbRuntime, index: &NamespaceIndex) -> Result<usize> {
    let mut db = open_unified_index_db(runtime)?;
    ensure_retrieval_schema(&db)?;
    let source_ref_prefix = format!(
        "kb:{}:{}:",
        runtime.scope_user_key.trim(),
        index.namespace.trim()
    );
    db.execute(
        "DELETE FROM memory_retrieval_index
         WHERE source_kind = 'kb_doc' AND COALESCE(user_key, '') = ?1 AND source_ref LIKE ?2",
        params![
            runtime.scope_user_key.as_str(),
            format!("{source_ref_prefix}%")
        ],
    )?;
    let _ = db.execute(
        "DELETE FROM memory_retrieval_index_fts
         WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
        [],
    );
    let tx = db.transaction()?;
    let mut row_count = 0usize;
    for chunk in &index.chunks {
        let text = chunk.text.trim();
        if text.is_empty() {
            continue;
        }
        let metadata = json!({
            "scope_kind": "user",
            "owner_user_key": runtime.scope_user_key.as_str(),
            "namespace": index.namespace,
            "path": chunk.path,
            "file_type": chunk.file_type,
            "mtime_epoch": chunk.mtime_epoch,
            "chunk_id": chunk.chunk_id,
            "offset": chunk.offset,
        });
        let source_ref = format!(
            "kb:{}:{}:{}",
            runtime.scope_user_key.trim(),
            index.namespace.trim(),
            chunk.chunk_id.trim()
        );
        let topic_tags = build_topic_tags(text);
        let vector_json = vector_to_json(&embed_text_locally(text));
        let row_ts = if chunk.mtime_epoch > 0 {
            chunk.mtime_epoch
        } else {
            index.updated_at_epoch.max(now_epoch())
        };
        tx.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, source_memory_id, source_pref_key, source_ref, user_id, chat_id, user_key,
                memory_kind, role, search_text, trigger_text, topic_tags, vector_json, metadata_json,
                salience, success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             )
             VALUES (?1, NULL, NULL, ?2, 0, 0, ?3, ?4, NULL, ?5, NULL, ?6, ?7, ?8, ?9, 'succeeded', 'kb', ?10, ?10)",
            params![
                "kb_doc",
                &source_ref,
                runtime.scope_user_key.as_str(),
                "knowledge_doc",
                text,
                topic_tags,
                vector_json,
                metadata.to_string(),
                0.78_f32,
                row_ts,
            ],
        )?;
        let row_id = tx.last_insert_rowid();
        let _ = tx.execute(
            "INSERT INTO memory_retrieval_index_fts(rowid, search_text, topic_tags)
             VALUES (?1, ?2, ?3)",
            params![row_id, text, topic_tags],
        );
        row_count += 1;
    }
    tx.commit()?;
    Ok(row_count)
}

fn open_unified_index_db(runtime: &KbRuntime) -> Result<Connection> {
    let (db_path, busy_timeout_ms) = if let Some(path) = runtime.unified_index_db_path.clone() {
        (path, runtime.unified_index_busy_timeout_ms.unwrap_or(5_000))
    } else {
        let config_path = runtime.workspace_root.join("configs/config.toml");
        let config = AppConfig::load(
            config_path
                .to_str()
                .ok_or_else(|| anyhow!("invalid config path: {}", config_path.display()))?,
        )
        .with_context(|| format!("load config failed: {}", config_path.display()))?;
        let sqlite_path = PathBuf::from(&config.database.sqlite_path);
        let db_path = if sqlite_path.is_absolute() {
            sqlite_path
        } else {
            runtime.workspace_root.join(sqlite_path)
        };
        (db_path, config.database.busy_timeout_ms)
    };
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let db = Connection::open(&db_path)
        .with_context(|| format!("open unified index db failed: {}", db_path.display()))?;
    db.busy_timeout(Duration::from_millis(busy_timeout_ms))?;
    Ok(db)
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

fn ensure_retrieval_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_retrieval_index (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            source_kind       TEXT NOT NULL,
            source_memory_id  INTEGER,
            source_pref_key   TEXT,
            source_ref        TEXT,
            user_id           INTEGER NOT NULL,
            chat_id           INTEGER NOT NULL,
            user_key          TEXT,
            memory_kind       TEXT NOT NULL,
            role              TEXT,
            search_text       TEXT NOT NULL,
            trigger_text      TEXT,
            topic_tags        TEXT NOT NULL DEFAULT '',
            vector_json       TEXT NOT NULL DEFAULT '[]',
            metadata_json     TEXT NOT NULL DEFAULT '{}',
            salience          REAL NOT NULL DEFAULT 0.5,
            success_state     TEXT NOT NULL DEFAULT 'neutral',
            tool_or_skill_name TEXT,
            created_at_ts     INTEGER NOT NULL DEFAULT 0,
            updated_at_ts     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_memory_retrieval_scope_updated
        ON memory_retrieval_index(user_key, chat_id, updated_at_ts DESC);
        CREATE INDEX IF NOT EXISTS idx_memory_retrieval_scope_kind_updated
        ON memory_retrieval_index(user_key, chat_id, memory_kind, updated_at_ts DESC);
        CREATE INDEX IF NOT EXISTS idx_memory_retrieval_source_kind
        ON memory_retrieval_index(source_kind, updated_at_ts DESC);
        CREATE INDEX IF NOT EXISTS idx_memory_retrieval_source_ref
        ON memory_retrieval_index(source_ref);",
    )?;
    ensure_column_exists(
        db,
        "memory_retrieval_index",
        "source_ref",
        "ALTER TABLE memory_retrieval_index ADD COLUMN source_ref TEXT",
    )?;
    ensure_column_exists(
        db,
        "memory_retrieval_index",
        "metadata_json",
        "ALTER TABLE memory_retrieval_index ADD COLUMN metadata_json TEXT NOT NULL DEFAULT '{}'",
    )?;
    let _ = db.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_retrieval_index_fts
         USING fts5(search_text, topic_tags);",
    );
    Ok(())
}

fn ensure_column_exists(
    db: &Connection,
    table_name: &str,
    column_name: &str,
    alter_sql: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info({table_name})");
    let mut stmt = db.prepare(&pragma)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row?.eq_ignore_ascii_case(column_name) {
            return Ok(());
        }
    }
    db.execute(alter_sql, [])?;
    Ok(())
}

fn build_topic_tags(text: &str) -> String {
    tokenize_for_index(text)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
}

fn embed_text_locally(text: &str) -> Vec<f32> {
    const DIMS: usize = 24;
    let mut vec = vec![0.0_f32; DIMS];
    for token in tokenize_for_index(text) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.hash(&mut hasher);
        let idx = (hasher.finish() as usize) % DIMS;
        vec[idx] += 1.0;
    }
    normalize_vector(&mut vec);
    vec
}

fn vector_to_json(vec: &[f32]) -> String {
    serde_json::to_string(vec).unwrap_or_else(|_| "[]".to_string())
}

fn normalize_vector(vec: &mut [f32]) {
    let norm = vec
        .iter()
        .map(|v| (*v as f64) * (*v as f64))
        .sum::<f64>()
        .sqrt() as f32;
    if norm <= f32::EPSILON {
        return;
    }
    for item in vec {
        *item /= norm;
    }
}

fn tokenize_for_index(text: &str) -> Vec<String> {
    tokenize_terms(text)
}

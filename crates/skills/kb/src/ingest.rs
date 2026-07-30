use super::ingest_extract::{extract_document, ExtractOutcome};
use super::ingest_scan::{build_scan_targets, collect_target_files, path_matches_any_scan_target};
use super::{
    chunker_version, default_embedding_version, default_parser_version, document_is_current,
    mtime_epoch, now_epoch, parse_chunker_setting, remove_doc_from_index, sha256_hex, split_chunks,
    storage, storage_path_for, tokenize, Chunk, DocMeta, KbRuntime, NamespaceIndex,
};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use skill_sdk::ExpectedPathKind;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
const DEFAULT_FILES_PER_RUN: usize = 1_000;
const DEFAULT_BYTES_PER_RUN: u64 = 256 * 1024 * 1024;
const DEFAULT_CHUNKS_PER_RUN: usize = 100_000;
const DEFAULT_MAX_DEPTH: usize = 64;
const DEFAULT_RUN_SECONDS: u64 = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct IngestArgs {
    pub(super) namespace: String,
    pub(super) paths: Vec<String>,
    pub(super) chunk_size: usize,
    pub(super) chunk_overlap: usize,
    pub(super) overwrite: bool,
    pub(super) file_types: HashSet<String>,
    pub(super) max_file_size: u64,
    pub(super) max_files_per_run: usize,
    pub(super) max_bytes_per_run: u64,
    pub(super) max_chunks_per_run: usize,
    pub(super) max_depth: usize,
    pub(super) max_run_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct IngestJob {
    pub(super) job_id: String,
    pub(super) owner_user_key: String,
    pub(super) operation: String,
    pub(super) namespace: String,
    pub(super) status: String,
    pub(super) request: IngestArgs,
    pub(super) manifest: Vec<String>,
    pub(super) next_file_index: usize,
    pub(super) processed_files: usize,
    pub(super) processed_bytes: u64,
    pub(super) produced_chunks: usize,
    pub(super) ingested_docs: usize,
    pub(super) skipped_files: usize,
    pub(super) removed_docs: usize,
    pub(super) warnings: Vec<String>,
    pub(super) last_completed_path: Option<String>,
    pub(super) created_at_epoch: i64,
    pub(super) updated_at_epoch: i64,
}

pub(super) fn do_ingest(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    start_ingest_job(runtime, parse_ingest_args(args)?, "ingest")
}

pub(super) fn do_reindex(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let namespace = super::required_namespace(args, "reindex")?;
    let index = storage::load_namespace(runtime, &namespace)
        .map_err(|_| anyhow!("namespace not found or unreadable: {namespace}"))?;
    let mut paths = index.docs.keys().cloned().collect::<Vec<_>>();
    paths.sort();
    if paths.is_empty() {
        return Err(anyhow!("namespace has no documents to reindex"));
    }
    let mut request = json!({
        "namespace": namespace,
        "paths": paths,
        "overwrite": true,
        "chunk_size": parse_chunker_setting(&index.chunker_version, 1)
            .unwrap_or(super::DEFAULT_CHUNK_SIZE),
        "chunk_overlap": parse_chunker_setting(&index.chunker_version, 2)
            .unwrap_or(super::DEFAULT_CHUNK_OVERLAP),
    });
    for key in [
        "max_file_size",
        "max_files_per_run",
        "max_bytes_per_run",
        "max_chunks_per_run",
        "max_depth",
        "max_run_seconds",
    ] {
        if let Some(value) = args.get(key) {
            request[key] = value.clone();
        }
    }
    let mut result = start_ingest_job(runtime, parse_ingest_args(&request)?, "reindex")?;
    if let Some(object) = result.as_object_mut() {
        object.insert("reindexed_from_revision".to_string(), json!(index.revision));
    }
    Ok(result)
}

pub(super) fn do_resume_ingest(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let job_id = required_job_id(args, "resume_ingest")?;
    let job = storage::load_ingest_job(runtime, &job_id)?;
    if job.status == "cancelled" || job.status == "completed" {
        return Ok(job_status_value(&job));
    }
    let index = if storage::namespace_exists(runtime, &job.namespace)? {
        storage::load_namespace(runtime, &job.namespace)?
    } else {
        empty_namespace(runtime, &job.request)
    };
    process_job(runtime, job, index)
}

pub(super) fn do_ingest_job_status(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let job_id = required_job_id(args, "ingest_job_status")?;
    Ok(job_status_value(&storage::load_ingest_job(
        runtime, &job_id,
    )?))
}

pub(super) fn do_cancel_ingest(runtime: &KbRuntime, args: &Value) -> Result<Value> {
    let job_id = required_job_id(args, "cancel_ingest")?;
    let mut job = storage::load_ingest_job(runtime, &job_id)?;
    if job.status != "completed" {
        job.status = "cancelled".to_string();
        job.updated_at_epoch = now_epoch();
        storage::save_ingest_job(runtime, &job)?;
    }
    Ok(job_status_value(&job))
}

fn start_ingest_job(runtime: &KbRuntime, request: IngestArgs, operation: &str) -> Result<Value> {
    let targets = build_scan_targets(runtime, &request.paths)?;
    let scan = collect_target_files(&targets, request.max_depth)?;
    let now = now_epoch();
    let mut job = IngestJob {
        job_id: new_job_id(runtime, &request),
        owner_user_key: runtime.scope_user_key.clone(),
        operation: operation.to_string(),
        namespace: request.namespace.clone(),
        status: "running".to_string(),
        request,
        manifest: scan
            .files
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        next_file_index: 0,
        processed_files: 0,
        processed_bytes: 0,
        produced_chunks: 0,
        ingested_docs: 0,
        skipped_files: 0,
        removed_docs: 0,
        warnings: scan.warnings,
        last_completed_path: None,
        created_at_epoch: now,
        updated_at_epoch: now,
    };
    storage::save_ingest_job(runtime, &job)?;
    let index = if job.request.overwrite {
        empty_namespace(runtime, &job.request)
    } else if storage::namespace_exists(runtime, &job.namespace)? {
        storage::load_namespace(runtime, &job.namespace)?
    } else {
        empty_namespace(runtime, &job.request)
    };
    job.status = "running".to_string();
    process_job(runtime, job, index)
}

fn process_job(
    runtime: &KbRuntime,
    mut job: IngestJob,
    mut index: NamespaceIndex,
) -> Result<Value> {
    if job.owner_user_key != runtime.scope_user_key {
        return Err(anyhow!("ingest job owner mismatch"));
    }
    let scan_targets = build_scan_targets(runtime, &job.request.paths)?;
    let started = Instant::now();
    let mut run_files = 0usize;
    let mut run_bytes = 0u64;
    let mut run_chunks = 0usize;
    let mut run_ingested = 0usize;
    let mut run_skipped = 0usize;
    let mut run_removed = 0usize;

    while job.next_file_index < job.manifest.len() {
        if run_files > 0
            && (run_files >= job.request.max_files_per_run
                || run_bytes >= job.request.max_bytes_per_run
                || run_chunks >= job.request.max_chunks_per_run
                || started.elapsed().as_secs() >= job.request.max_run_seconds)
        {
            break;
        }
        let manifest_path = job.manifest[job.next_file_index].clone();
        let file = runtime
            .path_policy
            .resolve_existing(&manifest_path, ExpectedPathKind::File)
            .map_err(|error| anyhow!("{}: {}", error.code, error.detail))?;
        let meta =
            fs::metadata(&file).with_context(|| format!("stat failed: {}", file.display()))?;
        let size = meta.len();
        let file_type = file
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let path_str = storage_path_for(&file, &runtime.workspace_root);
        let legacy_absolute_path = super::normalize_path_string(&file);
        run_files += 1;
        run_bytes = run_bytes.saturating_add(size);
        job.processed_files += 1;
        job.processed_bytes = job.processed_bytes.saturating_add(size);
        job.next_file_index += 1;
        job.last_completed_path = Some(path_str.clone());

        if (!job.request.file_types.is_empty() && !job.request.file_types.contains(&file_type))
            || size > job.request.max_file_size
        {
            run_skipped += 1;
            job.skipped_files += 1;
            if size > job.request.max_file_size {
                job.warnings.push(format!(
                    "skip large file {path_str} ({size} bytes > max_file_size {})",
                    job.request.max_file_size
                ));
            }
            continue;
        }

        let bytes =
            fs::read(&file).with_context(|| format!("failed to read {}", file.display()))?;
        let content_sha256 = sha256_hex(&bytes);
        let extracted = match extract_document(&bytes, &file_type) {
            Ok(value) => value,
            Err(error) => {
                run_skipped += 1;
                job.skipped_files += 1;
                job.warnings
                    .push(format!("skip {path_str}: extraction failed: {error}"));
                continue;
            }
        };
        let (text, parser_version) = match extracted {
            ExtractOutcome::Text {
                text,
                parser_version,
            } => (text, parser_version),
            ExtractOutcome::Skip { reason } => {
                run_skipped += 1;
                job.skipped_files += 1;
                job.warnings.push(format!("skip {path_str}: {reason}"));
                continue;
            }
        };
        let chunker_version = chunker_version(job.request.chunk_size, job.request.chunk_overlap);
        let unchanged = index
            .docs
            .get(&path_str)
            .or_else(|| index.docs.get(&legacy_absolute_path))
            .map(|doc| document_is_current(doc, &content_sha256, &parser_version, &chunker_version))
            .unwrap_or(false);
        if unchanged && !job.request.overwrite {
            continue;
        }

        let replaced =
            index.docs.contains_key(&path_str) || index.docs.contains_key(&legacy_absolute_path);
        remove_doc_from_index(&mut index, &path_str);
        if legacy_absolute_path != path_str {
            remove_doc_from_index(&mut index, &legacy_absolute_path);
        }
        if replaced {
            run_removed += 1;
            job.removed_docs += 1;
        }

        let chunks = split_chunks(&text, job.request.chunk_size, job.request.chunk_overlap);
        let chunk_count = chunks.len();
        for (offset, chunk_text) in chunks.into_iter().enumerate() {
            let chunk_id = format!("{}-{}", job.namespace, index.next_chunk_seq);
            index.next_chunk_seq += 1;
            index.chunks.push(Chunk {
                chunk_id,
                path: path_str.clone(),
                file_type: file_type.clone(),
                offset,
                len_tokens: tokenize(&chunk_text).len(),
                text_sha256: sha256_hex(chunk_text.as_bytes()),
                text: chunk_text,
                mtime_epoch: mtime_epoch(&meta),
            });
        }
        index.docs.insert(
            path_str.clone(),
            DocMeta {
                path: path_str,
                file_type,
                mtime_epoch: mtime_epoch(&meta),
                size,
                chunks: chunk_count,
                content_sha256,
                parser_version,
                chunker_version,
            },
        );
        run_chunks += chunk_count;
        job.produced_chunks += chunk_count;
        run_ingested += 1;
        job.ingested_docs += 1;
    }

    let complete = job.next_file_index >= job.manifest.len();
    if complete && !job.request.overwrite {
        let current_paths = job
            .manifest
            .iter()
            .map(|path| storage_path_for(Path::new(path), &runtime.workspace_root))
            .collect::<HashSet<_>>();
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
            run_removed += 1;
            job.removed_docs += 1;
        }
    }

    index.updated_at_epoch = now_epoch();
    index.parser_version = default_parser_version();
    index.chunker_version = chunker_version(job.request.chunk_size, job.request.chunk_overlap);
    index.embedding_version = default_embedding_version();
    job.status = if complete { "completed" } else { "waiting" }.to_string();
    job.updated_at_epoch = now_epoch();
    let persisted = storage::save_namespace_and_job(runtime, &index, &job)?;
    Ok(job_result_value(
        &job,
        persisted,
        run_ingested,
        run_skipped,
        run_removed,
        run_files,
        run_bytes,
        run_chunks,
    ))
}

#[allow(clippy::too_many_arguments)]
fn job_result_value(
    job: &IngestJob,
    persisted: storage::SaveOutcome,
    run_ingested: usize,
    run_skipped: usize,
    run_removed: usize,
    run_files: usize,
    run_bytes: u64,
    run_chunks: usize,
) -> Value {
    let complete = job.status == "completed";
    let warnings_empty = job.warnings.is_empty();
    let effective_success =
        complete && warnings_empty && (job.ingested_docs > 0 || persisted.total_docs > 0);
    let idempotent_success = complete && job.ingested_docs == 0 && effective_success;
    let result_kind = if !complete {
        "continuation_required"
    } else if job.ingested_docs > 0 {
        "updated"
    } else if warnings_empty && persisted.total_docs > 0 && persisted.retrieval_rows > 0 {
        "already_indexed"
    } else if persisted.total_docs > 0 {
        "no_new_documents"
    } else {
        "no_documents_indexed"
    };
    let continuation = (!complete).then(|| {
        json!({
            "action": "resume_ingest",
            "job_id": job.job_id,
            "next_file_index": job.next_file_index,
        })
    });
    let budgets = json!({
        "max_files_per_run": job.request.max_files_per_run,
        "max_bytes_per_run": job.request.max_bytes_per_run,
        "max_chunks_per_run": job.request.max_chunks_per_run,
        "max_depth": job.request.max_depth,
        "max_run_seconds": job.request.max_run_seconds,
    });
    let stats = json!({
        "ingested_docs": run_ingested,
        "removed_docs": run_removed,
        "skipped_files": run_skipped,
        "processed_files": run_files,
        "processed_bytes": run_bytes,
        "produced_chunks": run_chunks,
        "job_processed_files": job.processed_files,
        "job_total_files": job.manifest.len(),
        "job_processed_bytes": job.processed_bytes,
        "job_produced_chunks": job.produced_chunks,
        "job_ingested_docs": job.ingested_docs,
        "job_skipped_files": job.skipped_files,
        "job_removed_docs": job.removed_docs,
        "total_docs": persisted.total_docs,
        "total_chunks": persisted.total_chunks,
        "chunk_size": job.request.chunk_size,
        "chunk_overlap": job.request.chunk_overlap,
        "retrieval_index_synced": true,
        "retrieval_index_rows": persisted.retrieval_rows,
        "last_completed_path": job.last_completed_path,
        "warnings": job.warnings,
        "budgets": budgets,
    });
    json!({
        "schema_version": 1,
        "source_skill": "kb",
        "action": job.operation,
        "status": "ok",
        "job_id": job.job_id,
        "job_status": job.status,
        "complete": complete,
        "continuation": continuation,
        "effective_status": if complete && effective_success { "ok" } else if complete { "needs_attention" } else { "in_progress" },
        "result_kind": result_kind,
        "effective_success": effective_success,
        "idempotent_success": idempotent_success,
        "namespace": job.namespace,
        "namespace_revision": persisted.revision,
        "parser_version": default_parser_version(),
        "chunker_version": chunker_version(job.request.chunk_size, job.request.chunk_overlap),
        "embedding_version": default_embedding_version(),
        "path": job.request.paths.first().cloned().unwrap_or_default(),
        "paths": job.request.paths,
        "summary": result_kind,
        "stats": stats,
    })
}

fn job_status_value(job: &IngestJob) -> Value {
    let complete = job.status == "completed";
    json!({
        "schema_version": 1,
        "source_skill": "kb",
        "action": "ingest_job_status",
        "status": "ok",
        "job_id": job.job_id,
        "job_status": job.status,
        "operation": job.operation,
        "namespace": job.namespace,
        "complete": complete,
        "continuation": (job.status == "waiting" || job.status == "running").then(|| json!({
            "action": "resume_ingest",
            "job_id": job.job_id,
            "next_file_index": job.next_file_index,
        })),
        "progress": {
            "processed_files": job.processed_files,
            "total_files": job.manifest.len(),
            "processed_bytes": job.processed_bytes,
            "produced_chunks": job.produced_chunks,
            "ingested_docs": job.ingested_docs,
            "skipped_files": job.skipped_files,
            "removed_docs": job.removed_docs,
            "last_completed_path": job.last_completed_path,
        },
        "warnings": job.warnings,
        "created_at_epoch": job.created_at_epoch,
        "updated_at_epoch": job.updated_at_epoch,
    })
}

fn empty_namespace(runtime: &KbRuntime, request: &IngestArgs) -> NamespaceIndex {
    NamespaceIndex {
        namespace: request.namespace.clone(),
        owner_user_key: runtime.scope_user_key.clone(),
        updated_at_epoch: now_epoch(),
        next_chunk_seq: 1,
        revision: 0,
        parser_version: default_parser_version(),
        chunker_version: chunker_version(request.chunk_size, request.chunk_overlap),
        embedding_version: default_embedding_version(),
        docs: HashMap::new(),
        chunks: Vec::new(),
    }
}

fn required_job_id(args: &Value, action: &str) -> Result<String> {
    args.get("job_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{action} requires job_id"))
}

fn new_job_id(runtime: &KbRuntime, request: &IngestArgs) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let seed = format!(
        "{}:{}:{}:{}:{}",
        runtime.scope_user_key,
        request.namespace,
        nanos,
        std::process::id(),
        request.paths.join("\n")
    );
    format!(
        "kbj-{}",
        &format!("{:x}", Sha256::digest(seed.as_bytes()))[..24]
    )
}

pub(super) fn parse_ingest_args(args: &Value) -> Result<IngestArgs> {
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
    let chunk_size = args
        .get("chunk_size")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(super::DEFAULT_CHUNK_SIZE)
        .clamp(200, 8_000);
    let chunk_overlap = args
        .get("chunk_overlap")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(super::DEFAULT_CHUNK_OVERLAP)
        .min(chunk_size / 3)
        .min(400);
    let file_types = args
        .get("file_types")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    Ok(IngestArgs {
        namespace,
        paths,
        chunk_size,
        chunk_overlap,
        overwrite: args
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        file_types,
        max_file_size: args
            .get("max_file_size")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_MAX_FILE_SIZE)
            .max(1),
        max_files_per_run: bounded_usize(
            args,
            "max_files_per_run",
            DEFAULT_FILES_PER_RUN,
            1_000_000,
        ),
        max_bytes_per_run: args
            .get("max_bytes_per_run")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_BYTES_PER_RUN)
            .clamp(1, 1024 * 1024 * 1024 * 1024),
        max_chunks_per_run: bounded_usize(
            args,
            "max_chunks_per_run",
            DEFAULT_CHUNKS_PER_RUN,
            10_000_000,
        ),
        max_depth: bounded_usize(args, "max_depth", DEFAULT_MAX_DEPTH, 256),
        max_run_seconds: args
            .get("max_run_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_RUN_SECONDS)
            .clamp(1, 300),
    })
}

fn bounded_usize(args: &Value, key: &str, default: usize, maximum: usize) -> usize {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(default)
        .clamp(1, maximum)
}

pub(super) fn parse_ingest_paths(args: &Value) -> Result<Vec<String>> {
    let mut paths = match args.get("paths") {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
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
            .filter(|value| !value.is_empty())
        {
            paths.push(path.to_string());
        }
    }
    if paths.is_empty() {
        return Err(anyhow!("ingest requires paths[]"));
    }
    Ok(paths)
}

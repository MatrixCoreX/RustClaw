use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CHUNK_SIZE: usize = 1200;
const DEFAULT_TOP_K: usize = 5;
const DEFAULT_MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;

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

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let req: Value = serde_json::from_str(&line).unwrap_or_else(|_| json!({"request_id":"unknown"}));
        let request_id = req
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let args = req.get("args").unwrap_or(&req);
        let action = args
            .get("action")
            .or_else(|| req.get("action"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();

        let text_payload = match action.as_str() {
            "ingest" => match do_ingest(args) {
                Ok(v) => v,
                Err(e) => json!({"status":"error","error_code":"INGEST_FAILED","error":e.to_string()}),
            },
            "search" => match do_search(args) {
                Ok(v) => v,
                Err(e) => json!({"status":"error","error_code":"SEARCH_FAILED","error":e.to_string(),"hits":[]}),
            },
            _ => json!({
                "status":"error",
                "error_code":"INVALID_ACTION",
                "error":"action must be ingest|search"
            }),
        };

        let out = json!({
            "request_id": request_id,
            "status": "ok",
            "text": serde_json::to_string(&text_payload)?,
            "error_text": Value::Null,
            "extra": { "action": action }
        });
        writeln!(stdout, "{}", serde_json::to_string(&out)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn do_ingest(args: &Value) -> Result<Value> {
    let ingest = parse_ingest_args(args)?;
    let mut index = if ingest.overwrite {
        NamespaceIndex {
            namespace: ingest.namespace.clone(),
            updated_at_epoch: now_epoch(),
            next_chunk_seq: 1,
            docs: HashMap::new(),
            chunks: vec![],
        }
    } else {
        load_namespace(&ingest.namespace).unwrap_or_default_with_ns(&ingest.namespace)
    };

    let mut all_files = vec![];
    for p in &ingest.paths {
        collect_files(Path::new(p), &mut all_files)?;
    }

    let mut warnings = vec![];
    let mut ingested_docs = 0usize;
    let mut skipped_files = 0usize;
    let mut removed_docs = 0usize;

    for file in all_files {
        let meta = fs::metadata(&file).with_context(|| format!("stat failed: {}", file.display()))?;
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

        let path_str = file.display().to_string();
        let mtime = mtime_epoch(&meta);
        let unchanged = index
            .docs
            .get(&path_str)
            .map(|d| d.mtime_epoch == mtime && d.size == size)
            .unwrap_or(false);
        if unchanged && !ingest.overwrite {
            continue;
        }

        let before = index.docs.contains_key(&path_str);
        remove_doc_from_index(&mut index, &path_str);
        if before {
            removed_docs += 1;
        }

        let text = read_text_lossy(&file)?;
        if text.trim().is_empty() {
            skipped_files += 1;
            warnings.push(format!("skip empty text file {}", path_str));
            continue;
        }
        let chunks = split_chunks(&text, ingest.chunk_size);
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

    index.updated_at_epoch = now_epoch();
    save_namespace(&index)?;

    Ok(json!({
        "status":"ok",
        "namespace": ingest.namespace,
        "summary": format!("ingest completed: {} docs updated", ingested_docs),
        "stats": {
            "ingested_docs": ingested_docs,
            "removed_docs": removed_docs,
            "total_docs": index.docs.len(),
            "total_chunks": index.chunks.len(),
            "skipped_files": skipped_files,
            "warnings": warnings
        }
    }))
}

fn do_search(args: &Value) -> Result<Value> {
    let s = parse_search_args(args)?;
    let index = load_namespace(&s.namespace)
        .map_err(|_| anyhow!("namespace not found or unreadable: {}", s.namespace))?;
    if s.query.trim().is_empty() {
        return Ok(json!({"status":"error","error_code":"INVALID_INPUT","error":"query is required","hits":[]}));
    }
    let q_terms = tokenize(&s.query);
    if q_terms.is_empty() {
        return Ok(json!({"status":"ok","hits":[],"summary":"no effective query terms","stats":{"total_candidates":0}}));
    }

    let filtered_chunks = index
        .chunks
        .iter()
        .filter(|c| pass_filters(c, &s))
        .collect::<Vec<_>>();
    let n_docs = filtered_chunks.len() as f64;
    if n_docs <= 0.0 {
        return Ok(json!({"status":"ok","hits":[],"summary":"no matching chunks under filters","stats":{"total_candidates":0}}));
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

    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    if hits.len() > s.top_k {
        hits.truncate(s.top_k);
    }

    Ok(json!({
        "status":"ok",
        "namespace": s.namespace,
        "hits": hits,
        "summary": format!("found {} hit(s) for query", hits.len()),
        "stats": {
            "total_candidates": index.chunks.len(),
            "after_filters": hits.len(),
            "top_k": s.top_k
        }
    }))
}

fn parse_ingest_args(args: &Value) -> Result<IngestArgs> {
    let namespace = args
        .get("namespace")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("ingest requires namespace"))?
        .to_string();
    let paths = args
        .get("paths")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("ingest requires paths[]"))?
        .iter()
        .filter_map(Value::as_str)
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Err(anyhow!("paths[] must not be empty"));
    }
    let chunk_size = args
        .get("chunk_size")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_CHUNK_SIZE)
        .clamp(200, 8000);
    let overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
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
        overwrite,
        file_types,
        max_file_size,
    })
}

fn parse_search_args(args: &Value) -> Result<SearchArgs> {
    let namespace = args
        .get("namespace")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("search requires namespace"))?
        .to_string();
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
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
    let min_score = args
        .get("min_score")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
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

fn pass_filters(c: &Chunk, s: &SearchArgs) -> bool {
    if let Some(prefix) = &s.path_prefix {
        if !c.path.starts_with(prefix) {
            return false;
        }
    }
    if let Some(ft) = &s.file_type {
        if &c.file_type != ft {
            return false;
        }
    }
    if let Some(from) = s.time_from {
        if c.mtime_epoch < from {
            return false;
        }
    }
    if let Some(to) = s.time_to {
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
    let re = Regex::new(r"[[:alnum:]_./-]{2,}").expect("regex");
    re.find_iter(&text.to_ascii_lowercase())
        .map(|m| m.as_str().to_string())
        .collect::<Vec<_>>()
}

fn collect_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("path not found: {}", path.display()));
    }
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    for ent in fs::read_dir(path).with_context(|| format!("read_dir failed: {}", path.display()))? {
        let ent = ent?;
        let p = ent.path();
        if p.is_dir() {
            collect_files(&p, out)?;
        } else if p.is_file() {
            out.push(p);
        }
    }
    Ok(())
}

fn split_chunks(text: &str, chunk_size: usize) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return vec![];
    }
    let mut out = vec![];
    let mut i = 0usize;
    while i < chars.len() {
        let end = (i + chunk_size).min(chars.len());
        let chunk = chars[i..end].iter().collect::<String>();
        out.push(chunk);
        i = end;
    }
    out
}

fn read_text_lossy(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| String::from_utf8_lossy(&bytes).to_string());
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

fn kb_root() -> PathBuf {
    if let Ok(p) = env::var("KB_ROOT") {
        let pb = PathBuf::from(p);
        if pb.is_absolute() {
            return pb;
        }
        return env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(pb);
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let base = find_workspace_root(&cwd).unwrap_or(cwd);
    base.join("data").join("kb")
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

fn ns_file(namespace: &str) -> PathBuf {
    kb_root().join(format!("{}.json", sanitize_ns(namespace)))
}

fn sanitize_ns(ns: &str) -> String {
    ns.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

trait DefaultWithNs {
    fn unwrap_or_default_with_ns(self, ns: &str) -> NamespaceIndex;
}
impl DefaultWithNs for Result<NamespaceIndex> {
    fn unwrap_or_default_with_ns(self, ns: &str) -> NamespaceIndex {
        self.unwrap_or(NamespaceIndex {
            namespace: ns.to_string(),
            updated_at_epoch: now_epoch(),
            next_chunk_seq: 1,
            docs: HashMap::new(),
            chunks: vec![],
        })
    }
}

fn load_namespace(namespace: &str) -> Result<NamespaceIndex> {
    let p = ns_file(namespace);
    let raw = fs::read_to_string(&p).with_context(|| format!("read index failed: {}", p.display()))?;
    let mut idx: NamespaceIndex = serde_json::from_str(&raw).with_context(|| "index json parse failed")?;
    if idx.namespace.is_empty() {
        idx.namespace = namespace.to_string();
    }
    Ok(idx)
}

fn save_namespace(index: &NamespaceIndex) -> Result<()> {
    let root = kb_root();
    fs::create_dir_all(&root).with_context(|| format!("mkdir failed: {}", root.display()))?;
    let p = ns_file(&index.namespace);
    let raw = serde_json::to_string_pretty(index)?;
    fs::write(&p, raw).with_context(|| format!("write index failed: {}", p.display()))?;
    Ok(())
}

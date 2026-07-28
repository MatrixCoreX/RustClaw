use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rustclaw_fs_discovery::{ripgrep_text_search, CaseMode, RipgrepTextRequest, TextPatternKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::grep_search::{find_matches, GrepOptions, PatternKind};

pub(super) struct ContentSearchOptions<'a> {
    pub(super) workspace_root: &'a Path,
    pub(super) search_root: &'a Path,
    pub(super) query: &'a str,
    pub(super) pattern_kind: PatternKind,
    pub(super) case_mode: CaseMode,
    pub(super) case_insensitive: bool,
    pub(super) multiline: bool,
    pub(super) context_before: usize,
    pub(super) context_after: usize,
    pub(super) max_line_chars: usize,
    pub(super) max_file_bytes: usize,
    pub(super) max_scan_bytes: usize,
    pub(super) max_matches: usize,
    pub(super) deadline: Option<Duration>,
}

pub(super) struct ContentSearchOutcome {
    pub(super) matches: Vec<Value>,
    pub(super) results: Vec<String>,
    pub(super) scanned_bytes: usize,
    pub(super) skipped_large_files: usize,
    pub(super) skipped_non_utf8_files: usize,
    pub(super) skipped_binary_files: usize,
    pub(super) skipped_encoding_counts_complete: bool,
    pub(super) scan_byte_limit_reached: bool,
    pub(super) match_limit_reached: bool,
    pub(super) backend: &'static str,
    pub(super) backend_version: Option<String>,
    pub(super) backend_fallback_reason: Option<String>,
    pub(super) backend_elapsed_ms: u64,
}

pub(super) fn search_content(
    paths: &[PathBuf],
    options: ContentSearchOptions<'_>,
) -> Result<ContentSearchOutcome, String> {
    let mut candidates = Vec::new();
    let mut scanned_bytes = 0usize;
    let mut skipped_large_files = 0usize;
    let mut scan_byte_limit_reached = false;
    for path in paths {
        let file_bytes = std::fs::metadata(path)
            .map(|metadata| metadata.len().min(usize::MAX as u64) as usize)
            .unwrap_or(0);
        if file_bytes > options.max_file_bytes {
            skipped_large_files = skipped_large_files.saturating_add(1);
            continue;
        }
        if scanned_bytes.saturating_add(file_bytes) > options.max_scan_bytes {
            scan_byte_limit_reached = true;
            break;
        }
        scanned_bytes = scanned_bytes.saturating_add(file_bytes);
        candidates.push(path.clone());
    }

    let fallback_reason = ripgrep_ineligible_reason(&candidates, &options);
    if fallback_reason.is_none() {
        match search_with_ripgrep(&candidates, &options) {
            Ok(mut outcome) => {
                outcome.scanned_bytes = scanned_bytes;
                outcome.skipped_large_files = skipped_large_files;
                outcome.scan_byte_limit_reached = scan_byte_limit_reached;
                return Ok(outcome);
            }
            Err(reason) => {
                return search_with_rust(
                    &candidates,
                    options,
                    scanned_bytes,
                    skipped_large_files,
                    scan_byte_limit_reached,
                    Some(reason),
                );
            }
        }
    }
    search_with_rust(
        &candidates,
        options,
        scanned_bytes,
        skipped_large_files,
        scan_byte_limit_reached,
        fallback_reason.map(str::to_string),
    )
}

fn ripgrep_ineligible_reason(
    candidates: &[PathBuf],
    options: &ContentSearchOptions<'_>,
) -> Option<&'static str> {
    if !options.search_root.is_dir() {
        return Some("known_file_uses_direct_rust_read");
    }
    if options.context_before > 0 || options.context_after > 0 {
        return Some("context_lines_require_rust_backend");
    }
    if candidates.is_empty() {
        return Some("no_candidate_files");
    }
    None
}

fn search_with_ripgrep(
    candidates: &[PathBuf],
    options: &ContentSearchOptions<'_>,
) -> Result<ContentSearchOutcome, String> {
    let report = ripgrep_text_search(&RipgrepTextRequest {
        workspace_root: options.workspace_root.to_path_buf(),
        root: options.search_root.to_path_buf(),
        paths: candidates.to_vec(),
        query: options.query.to_string(),
        pattern_kind: match options.pattern_kind {
            PatternKind::Literal => TextPatternKind::Literal,
            PatternKind::Regex => TextPatternKind::Regex,
        },
        case_mode: options.case_mode,
        multiline: options.multiline,
        max_matches: options.max_matches.saturating_add(1),
        max_output_bytes: 16 * 1024 * 1024,
        max_line_chars: options.max_line_chars,
        deadline: options.deadline,
        cancellation: None,
    })
    .map_err(|error| error.code().to_string())?;
    let started = Instant::now();
    let mut matches = report
        .matches
        .into_iter()
        .map(|matched| {
            let identity = file_identity(&options.workspace_root.join(&matched.path));
            let path = matched.path;
            json!({
                "path": path.clone(),
                "line": matched.line,
                "end_line": matched.end_line,
                "start_byte": matched.start_byte,
                "end_byte": matched.end_byte,
                "text": matched.text,
                "matched_text": matched.matched_text,
                "context_before": [],
                "context_after": [],
                "encoding": "utf-8",
                "binary": false,
                "range_handle": {
                    "path": path,
                    "start_byte": matched.start_byte,
                    "end_byte": matched.end_byte,
                    "file_identity": identity,
                },
            })
        })
        .collect::<Vec<_>>();
    let match_limit_reached = matches.len() > options.max_matches || report.output_truncated;
    matches.truncate(options.max_matches);
    let results = result_paths(&matches);
    Ok(ContentSearchOutcome {
        matches,
        results,
        scanned_bytes: 0,
        skipped_large_files: 0,
        skipped_non_utf8_files: 0,
        skipped_binary_files: 0,
        skipped_encoding_counts_complete: false,
        scan_byte_limit_reached: false,
        match_limit_reached,
        backend: "ripgrep",
        backend_version: report.backend.version,
        backend_fallback_reason: None,
        backend_elapsed_ms: report
            .backend
            .elapsed_ms
            .saturating_add(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
    })
}

fn search_with_rust(
    candidates: &[PathBuf],
    options: ContentSearchOptions<'_>,
    scanned_bytes: usize,
    skipped_large_files: usize,
    scan_byte_limit_reached: bool,
    fallback_reason: Option<String>,
) -> Result<ContentSearchOutcome, String> {
    let started = Instant::now();
    let grep_options = GrepOptions {
        query: options.query,
        pattern_kind: options.pattern_kind,
        case_insensitive: options.case_insensitive,
        multiline: options.multiline,
        context_before: options.context_before,
        context_after: options.context_after,
        max_line_chars: options.max_line_chars,
    };
    let mut matches = Vec::new();
    let mut skipped_non_utf8_files = 0usize;
    let mut skipped_binary_files = 0usize;
    for path in candidates {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if bytes.iter().take(8 * 1024).any(|byte| *byte == 0) {
            skipped_binary_files = skipped_binary_files.saturating_add(1);
            continue;
        }
        let file_sha256 = format!("{:x}", Sha256::digest(&bytes));
        let Ok(text) = String::from_utf8(bytes) else {
            skipped_non_utf8_files = skipped_non_utf8_files.saturating_add(1);
            continue;
        };
        let rel = crate::workspace_traversal::to_rel(options.workspace_root, path);
        for matched in find_matches(&text, grep_options)? {
            let mut value = serde_json::to_value(matched).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("path".to_string(), Value::String(rel.clone()));
                object.insert("encoding".to_string(), Value::String("utf-8".to_string()));
                object.insert("binary".to_string(), Value::Bool(false));
                object.insert(
                    "range_handle".to_string(),
                    json!({
                        "path": rel,
                        "start_byte": object.get("start_byte").cloned().unwrap_or(Value::Null),
                        "end_byte": object.get("end_byte").cloned().unwrap_or(Value::Null),
                        "file_sha256": file_sha256,
                    }),
                );
            }
            matches.push(value);
            if matches.len() > options.max_matches {
                break;
            }
        }
        if matches.len() > options.max_matches {
            break;
        }
    }
    let match_limit_reached = matches.len() > options.max_matches;
    matches.truncate(options.max_matches);
    matches.sort_by(match_order);
    let results = result_paths(&matches);
    Ok(ContentSearchOutcome {
        matches,
        results,
        scanned_bytes,
        skipped_large_files,
        skipped_non_utf8_files,
        skipped_binary_files,
        skipped_encoding_counts_complete: true,
        scan_byte_limit_reached,
        match_limit_reached,
        backend: "rust",
        backend_version: None,
        backend_fallback_reason: fallback_reason,
        backend_elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

fn match_order(left: &Value, right: &Value) -> std::cmp::Ordering {
    left.get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .cmp(
            right
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .then_with(|| {
            left.get("start_byte")
                .and_then(Value::as_u64)
                .cmp(&right.get("start_byte").and_then(Value::as_u64))
        })
}

fn result_paths(matches: &[Value]) -> Vec<String> {
    let mut results = matches
        .iter()
        .filter_map(|item| item.get("path").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    results.sort();
    results.dedup();
    results
}

fn file_identity(path: &Path) -> Value {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Value::Null;
    };
    let modified_unix_ns = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos().to_string());
    json!({"size_bytes": metadata.len(), "modified_unix_ns": modified_unix_ns})
}

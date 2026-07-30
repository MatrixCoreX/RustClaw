#![recursion_limit = "256"]

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::{Duration, SystemTime};

use fs_discovery::{
    fuzzy_name_score, BackendPreference, CaseMode, Completeness, DiscoverySelector, MatchMode,
    TargetKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod content_backend;
mod grep_search;
mod image_search;
mod result_pagination;
mod snapshot_cache;
mod workspace_traversal;

use content_backend::{search_content, ContentSearchOptions};
use grep_search::{
    find_matches, GrepOptions, PatternKind, MAX_CONTEXT_LINES, MAX_REGEX_PATTERN_BYTES,
};
use image_search::{collect_images, default_image_extensions, directory_counts, ImageEntry};
use result_pagination::{
    cursor_from_args, cursor_snapshot_identity, encode_scan_continuation, paginate, query_sha256,
    scan_offset_from_args,
};
use snapshot_cache::{render_cached, render_missing_snapshot, SnapshotCache};
use workspace_traversal::{
    resolve_path, to_rel, walk_collect_selected, workspace_root, ScanLimits, WalkStats,
};

const SKILL_NAME: &str = "fs_search";
const MAX_RESULT_SNAPSHOT_ITEMS: usize = 100_000;
const MAX_GREP_SNAPSHOT_MATCHES: usize = 20_000;
const DEFAULT_GREP_MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_GREP_MAX_SCAN_BYTES: usize = 64 * 1024 * 1024;
const MAX_GREP_FILE_BYTES: usize = 64 * 1024 * 1024;
const MAX_GREP_SCAN_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

fn scan_limits_from_args(obj: &serde_json::Map<String, Value>) -> ScanLimits {
    let max_depth = obj
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).clamp(1, 256));
    #[cfg(test)]
    let hard_entry_limit = obj
        .get("__test_hard_entry_limit")
        .and_then(Value::as_u64)
        .map(|value| (value as usize).max(1))
        .unwrap_or(fs_discovery::DEFAULT_HARD_ENTRY_LIMIT);
    #[cfg(not(test))]
    let hard_entry_limit = fs_discovery::DEFAULT_HARD_ENTRY_LIMIT;
    let timeout_seconds = std::env::var("SKILL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(30)
        .max(1);
    ScanLimits {
        max_depth,
        start_after_entries: 0,
        hard_entry_limit,
        include_hidden: obj
            .get("include_hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        respect_ignore: obj
            .get("respect_ignore")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        deadline: Some(Duration::from_secs(
            timeout_seconds.saturating_sub(1).max(1),
        )),
        backend: {
            #[cfg(test)]
            {
                match obj.get("__test_backend").and_then(Value::as_str) {
                    Some("rust") => BackendPreference::Rust,
                    Some("ripgrep") => BackendPreference::Ripgrep,
                    _ => BackendPreference::Auto,
                }
            }
            #[cfg(not(test))]
            {
                BackendPreference::Auto
            }
        },
        allow_path_outside_workspace: false,
    }
}

fn context_allows_path_outside_workspace(context: Option<&Value>) -> bool {
    context.is_some_and(|context| {
        context
            .pointer("/permissions/allow_path_outside_workspace")
            .or_else(|| context.get("allow_path_outside_workspace"))
            .and_then(Value::as_bool)
            == Some(true)
    })
}

fn effective_completeness(
    stats: &WalkStats,
    additional_hard_limit: bool,
    stale_snapshot: bool,
) -> Completeness {
    if stale_snapshot {
        return Completeness::StaleSnapshot;
    }
    if additional_hard_limit && stats.completeness.is_complete() {
        return Completeness::PartialHardLimit;
    }
    stats.completeness
}

fn scan_report(stats: &WalkStats, completeness: Completeness) -> Value {
    json!({
        "completeness": completeness.as_str(),
        "visited_files": stats.visited_files,
        "visited_directories": stats.visited_directories,
        "visited_entries": stats.visited_files.saturating_add(stats.visited_directories),
        "skipped_ignored": stats.skipped_ignored,
        "skipped_hidden": stats.skipped_hidden,
        "skipped_symlinks": stats.skipped_symlinks,
        "permission_denied": stats.permission_denied,
        "skipped_counts_complete": stats.skipped_counts_complete,
        "backend": stats.backend.as_str(),
        "backend_version": stats.backend_version,
        "backend_fallback_reason": stats.backend_fallback_reason,
        "backend_elapsed_ms": stats.backend_elapsed_ms,
        "traversal_start": stats.traversal_start,
        "traversal_next": stats.traversal_next,
    })
}

fn case_mode_from_args(
    obj: &serde_json::Map<String, Value>,
) -> Result<(CaseMode, &'static str), String> {
    if bool_arg(obj, "case_insensitive") || bool_arg(obj, "ignore_case") {
        return Ok((CaseMode::Insensitive, "insensitive"));
    }
    match obj
        .get("case_mode")
        .and_then(Value::as_str)
        .unwrap_or("smart")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "smart" => Ok((CaseMode::Smart, "smart")),
        "sensitive" | "case_sensitive" => Ok((CaseMode::Sensitive, "sensitive")),
        "insensitive" | "ignore_case" => Ok((CaseMode::Insensitive, "insensitive")),
        _ => Err("case_mode_unsupported".to_string()),
    }
}

fn match_mode_from_args(
    obj: &serde_json::Map<String, Value>,
) -> Result<(MatchMode, &'static str), String> {
    if bool_arg(obj, "exact") || bool_arg(obj, "exact_name") {
        return Ok((MatchMode::Exact, "exact"));
    }
    match obj
        .get("match_mode")
        .or_else(|| obj.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("contains")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "exact" | "basename_exact" | "name_exact" => Ok((MatchMode::Exact, "exact")),
        "prefix" | "starts_with" => Ok((MatchMode::StartsWith, "prefix")),
        "suffix" | "ends_with" => Ok((MatchMode::EndsWith, "suffix")),
        "contains" => Ok((MatchMode::Contains, "contains")),
        "fuzzy" | "approximate" | "typo_tolerant" => Ok((MatchMode::Fuzzy, "fuzzy")),
        "glob" => Ok((MatchMode::Glob, "glob")),
        _ => Err("match_mode_unsupported".to_string()),
    }
}

fn glob_values_from_args(obj: &serde_json::Map<String, Value>) -> Vec<String> {
    string_values_from_args(obj, &["glob", "globs"])
}

fn pattern_kind_from_args(
    obj: &serde_json::Map<String, Value>,
) -> Result<(PatternKind, &'static str), String> {
    match obj
        .get("pattern_kind")
        .and_then(Value::as_str)
        .unwrap_or("literal")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "literal" | "fixed" => Ok((PatternKind::Literal, "literal")),
        "regex" | "regexp" => Ok((PatternKind::Regex, "regex")),
        _ => Err("pattern_kind_unsupported".to_string()),
    }
}

fn output_mode_from_args(obj: &serde_json::Map<String, Value>) -> Result<&'static str, String> {
    match obj
        .get("output_mode")
        .and_then(Value::as_str)
        .unwrap_or("content")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "content" | "matches" => Ok("content"),
        "paths" | "files" => Ok("paths"),
        "count" | "counts" => Ok("count"),
        _ => Err("output_mode_unsupported".to_string()),
    }
}

fn smart_case_is_insensitive(mode: CaseMode, pattern: &str) -> bool {
    match mode {
        CaseMode::Sensitive => false,
        CaseMode::Insensitive => true,
        CaseMode::Smart => !pattern.chars().any(char::is_uppercase),
    }
}

fn target_kind_selector(value: &str) -> TargetKind {
    match value {
        "file" => TargetKind::File,
        "dir" => TargetKind::Directory,
        _ => TargetKind::Any,
    }
}

fn effective_policy(scan_limits: ScanLimits) -> Value {
    json!({
        "include_hidden": scan_limits.include_hidden,
        "respect_ignore": scan_limits.respect_ignore,
        "max_depth": scan_limits.max_depth,
        "traversal_start": scan_limits.start_after_entries,
        "follow_symlinks": false,
    })
}

fn continuation_from_page(
    page: &Value,
    completeness: Completeness,
    stats: &WalkStats,
    query_sha256: &str,
) -> Value {
    if completeness == Completeness::StaleSnapshot {
        return json!({
            "kind": "new_snapshot",
            "safe_to_continue": true,
            "reason_code": "stale_snapshot",
        });
    }
    if let Some(cursor) = page.get("next_cursor").and_then(Value::as_str) {
        return json!({
            "kind": "next_page",
            "safe_to_continue": true,
            "cursor": cursor,
        });
    }
    if !completeness.is_complete() {
        if let Some(offset) = stats.traversal_next {
            return json!({
                "kind": "scan_frontier",
                "safe_to_continue": true,
                "reason_code": completeness.as_str(),
                "token": encode_scan_continuation(query_sha256, offset),
                "traversal_offset": offset,
            });
        }
        return json!({
            "kind": "narrow_search",
            "safe_to_continue": true,
            "reason_code": completeness.as_str(),
        });
    }
    Value::Null
}

fn normalize_locator_shape(text: &str) -> String {
    text.trim()
        .chars()
        .map(|ch| match ch {
            '／' | '＼' => '/',
            '－' => '-',
            '＿' => '_',
            '．' => '.',
            '（' => '(',
            '）' => ')',
            '【' => '[',
            '】' => ']',
            '｛' => '{',
            '｝' => '}',
            '　' => ' ',
            _ => ch,
        })
        .collect::<String>()
}

fn string_values_from_args(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for key in keys {
        let Some(value) = obj.get(*key) else {
            continue;
        };
        if let Some(raw) = value.as_str() {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        } else if let Some(items) = value.as_array() {
            for item in items {
                let Some(raw) = item.as_str() else {
                    continue;
                };
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
        }
    }
    out
}

fn extension_values_from_args(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for raw in string_values_from_args(obj, keys) {
        for part in raw.split(|ch: char| matches!(ch, ',' | ';' | '|')) {
            let normalized = part
                .trim()
                .trim_start_matches('.')
                .trim()
                .to_ascii_lowercase();
            if !normalized.is_empty() && !out.iter().any(|existing| existing == &normalized) {
                out.push(normalized);
            }
        }
    }
    out
}

fn expand_name_pattern_preserving_case(raw: &str) -> Vec<String> {
    let normalized = normalize_locator_shape(raw);
    let stripped = normalized.trim_matches(|ch: char| {
        ch == '*' || ch == '?' || ch == '"' || ch == '\'' || ch.is_whitespace()
    });
    let alternation_source =
        if let (Some(start), Some(end)) = (stripped.find('('), stripped.rfind(')')) {
            if end > start {
                &stripped[start + 1..end]
            } else {
                stripped
            }
        } else {
            stripped
        };
    let mut out = Vec::new();
    for part in alternation_source.split('|') {
        let term = part.trim_matches(|ch: char| {
            ch == '*'
                || ch == '?'
                || ch == '('
                || ch == ')'
                || ch == '['
                || ch == ']'
                || ch == '{'
                || ch == '}'
                || ch == '"'
                || ch == '\''
                || ch.is_whitespace()
        });
        let term = strip_glob_wildcards(term);
        if !term.is_empty() {
            out.push(term);
        }
    }
    if out.is_empty() && !stripped.is_empty() {
        let stripped = strip_glob_wildcards(stripped);
        if !stripped.is_empty() {
            out.push(stripped);
        }
    }
    out
}

fn typed_name_patterns_from_args(
    obj: &serde_json::Map<String, Value>,
    match_mode: MatchMode,
    required: bool,
) -> Result<Vec<String>, String> {
    let raw_patterns = string_values_from_args(
        obj,
        &[
            "pattern",
            "patterns",
            "name",
            "names",
            "entry_name",
            "entry_names",
            "keyword",
            "keywords",
            "query",
            "queries",
        ],
    );
    let patterns = if match_mode == MatchMode::Glob {
        raw_patterns
    } else {
        raw_patterns
            .iter()
            .flat_map(|raw| expand_name_pattern_preserving_case(raw))
            .filter(|pattern| !pattern.is_empty())
            .collect::<Vec<_>>()
    };
    if required && patterns.is_empty() {
        Err("pattern is required".to_string())
    } else {
        Ok(patterns)
    }
}

fn typed_file_patterns_from_args(
    obj: &serde_json::Map<String, Value>,
    match_mode: MatchMode,
) -> Vec<String> {
    let raw_patterns = string_values_from_args(
        obj,
        &[
            "pattern",
            "patterns",
            "name",
            "names",
            "filename",
            "filenames",
            "file_pattern",
            "file_patterns",
        ],
    );
    if match_mode == MatchMode::Glob {
        raw_patterns
    } else {
        raw_patterns
            .iter()
            .flat_map(|raw| expand_name_pattern_preserving_case(raw))
            .filter(|pattern| !pattern.is_empty())
            .collect()
    }
}

fn strip_glob_wildcards(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(ch, '*' | '?'))
        .collect::<String>()
}

fn pattern_extension(pattern: &str) -> Option<String> {
    Path::new(pattern)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
}

fn bool_arg(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn normalize_target_kind(value: &str) -> &str {
    match value {
        "files" => "file",
        "dirs" | "directory" | "directories" | "folder" | "folders" => "dir",
        "file" | "dir" | "any" => value,
        _ => "any",
    }
}

fn entry_sort_mode(obj: &serde_json::Map<String, Value>) -> Result<&str, String> {
    let mode = obj.get("sort_by").and_then(Value::as_str).unwrap_or("name");
    match mode {
        "name" | "name_desc" | "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc" => Ok(mode),
        _ => Err("sort_by_unsupported".to_string()),
    }
}

fn sort_entry_results(results: &mut [String], root: &Path, mode: &str) {
    match mode {
        "name_desc" => results.sort_by(|left, right| right.cmp(left)),
        "mtime_desc" | "mtime_asc" => {
            let modified = |path: &str| {
                std::fs::metadata(root.join(path))
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH)
            };
            results.sort_by(|left, right| {
                let time_order = if mode == "mtime_desc" {
                    modified(right).cmp(&modified(left))
                } else {
                    modified(left).cmp(&modified(right))
                };
                time_order.then_with(|| left.cmp(right))
            });
        }
        "size_desc" | "size_asc" => {
            let size = |path: &str| {
                std::fs::metadata(root.join(path))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0)
            };
            results.sort_by(|left, right| {
                let size_order = if mode == "size_desc" {
                    size(right).cmp(&size(left))
                } else {
                    size(left).cmp(&size(right))
                };
                size_order.then_with(|| left.cmp(right))
            });
        }
        _ => results.sort(),
    }
}

fn sort_fuzzy_results(results: &mut [String], patterns: &[String], case_mode: CaseMode) {
    let score = |path: &str| {
        let name = Path::new(path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(path);
        patterns
            .iter()
            .filter_map(|pattern| fuzzy_name_score(name, pattern, case_mode))
            .min()
            .unwrap_or(usize::MAX)
    };
    results.sort_by(|left, right| score(left).cmp(&score(right)).then_with(|| left.cmp(right)));
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute_with_context(req.args, req.context.as_ref()) {
                Ok(extra) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text: extra.to_string(),
                    extra: Some(extra),
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    extra: Some(error_extra(execution_error_kind(&err))),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_extra(error_kind: &str) -> Value {
    json!({
        "schema_version": 2,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_code": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn execution_error_kind(error: &str) -> &str {
    match error {
        "multiline_query_invalid"
        | "query_empty"
        | "sort_by_unsupported"
        | "invalid_cursor"
        | "cursor_query_mismatch"
        | "cursor_out_of_range" => error,
        _ => "execution_failed",
    }
}

#[cfg(test)]
fn execute(args: Value) -> Result<Value, String> {
    execute_with_context(args, None)
}

fn execute_with_context(args: Value, context: Option<&Value>) -> Result<Value, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| {
            if obj.get("pattern").is_some() {
                "find_name".to_string()
            } else if obj.get("ext").is_some() {
                "find_ext".to_string()
            } else if obj.get("query").is_some() {
                "grep_text".to_string()
            } else {
                // Sensible fallback for broad scan requests.
                "find_images".to_string()
            }
        });
    let max_results = obj
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .clamp(1, 1000) as usize;
    let cursor = cursor_from_args(obj)?;
    let query_sha256 = query_sha256(obj);

    let root = workspace_root();
    let allow_path_outside_workspace = context_allows_path_outside_workspace(context);
    let search_root = resolve_path(
        &root,
        obj.get("root")
            .or_else(|| obj.get("path"))
            .or_else(|| obj.get("dir"))
            .and_then(|v| v.as_str())
            .unwrap_or("."),
        allow_path_outside_workspace,
    )?;
    let mut scan_limits = scan_limits_from_args(obj);
    scan_limits.start_after_entries = scan_offset_from_args(obj, &query_sha256)?;
    scan_limits.allow_path_outside_workspace = allow_path_outside_workspace;
    let snapshot_cache = SnapshotCache::from_context(context)?;
    if let (Some(cache), Some((cursor_query, cursor_snapshot))) =
        (snapshot_cache.as_ref(), cursor_snapshot_identity(&cursor)?)
    {
        if cursor_query != query_sha256 {
            return Err("cursor_query_mismatch".to_string());
        }
        match cache.load(&action, &query_sha256, &cursor_snapshot)? {
            Some(cached) => {
                return render_cached(cached, &cursor, max_results, &query_sha256);
            }
            None => {
                return Ok(render_missing_snapshot(
                    &action,
                    &query_sha256,
                    &cursor_snapshot,
                    max_results,
                ));
            }
        }
    }

    let mut results = Vec::new();
    match action.as_str() {
        "find_name" => {
            let (match_mode, match_mode_label) = match_mode_from_args(obj)?;
            let (case_mode, case_mode_label) = case_mode_from_args(obj)?;
            let globs = glob_values_from_args(obj);
            let pattern_norms = typed_name_patterns_from_args(obj, match_mode, globs.is_empty())?;
            let exact_name = match_mode == MatchMode::Exact;
            let sort_by = entry_sort_mode(obj)?;
            let target_kind = obj
                .get("target_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("any")
                .to_ascii_lowercase();
            let target_kind = if bool_arg(obj, "files_only") || bool_arg(obj, "file_only") {
                "file"
            } else if bool_arg(obj, "dirs_only")
                || bool_arg(obj, "directories_only")
                || bool_arg(obj, "folders_only")
            {
                "dir"
            } else {
                normalize_target_kind(&target_kind)
            };
            let mut selector = DiscoverySelector {
                patterns: pattern_norms.clone(),
                globs: globs.clone(),
                extensions: pattern_norms
                    .iter()
                    .filter_map(|pattern| pattern_extension(pattern))
                    .collect(),
                target_kind: target_kind_selector(target_kind),
                match_mode,
                case_mode,
            };
            if match_mode == MatchMode::Glob {
                selector.extensions.clear();
            }
            let mut collect = |p: &Path| {
                results.push(to_rel(&root, p));
                results.len() > MAX_RESULT_SNAPSHOT_ITEMS
            };
            let stats = walk_collect_selected(&search_root, scan_limits, selector, &mut collect)?;
            let result_limit_reached = results.len() > MAX_RESULT_SNAPSHOT_ITEMS;
            results.truncate(MAX_RESULT_SNAPSHOT_ITEMS);
            let fuzzy_relevance = match_mode == MatchMode::Fuzzy && !obj.contains_key("sort_by");
            if fuzzy_relevance {
                sort_fuzzy_results(&mut results, &pattern_norms, case_mode);
            } else {
                sort_entry_results(&mut results, &root, sort_by);
            }
            results.dedup();
            let page = paginate(
                &results,
                &cursor,
                max_results,
                stats.limit_reached || result_limit_reached,
                &query_sha256,
            )?;
            let completeness =
                effective_completeness(&stats, result_limit_reached, page.stale_snapshot);
            let continuation =
                continuation_from_page(&page.metadata, completeness, &stats, &query_sha256);
            let primary_items = results.iter().cloned().map(Value::String).collect();
            let response = json!({
                "schema_version": 2,
                "source_skill": SKILL_NAME,
                "status": "ok",
                "action": "find_name",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "patterns": pattern_norms,
                "globs": globs,
                "exact": exact_name,
                "match_mode": match_mode_label,
                "case_mode": case_mode_label,
                "sort_by": if fuzzy_relevance { "relevance" } else { sort_by },
                "count": page.returned_count,
                "returned_count": page.returned_count,
                "total_count": page.total_count,
                "known_match_count": page.total_count,
                "total_count_is_complete": completeness.is_complete(),
                "completeness": completeness.as_str(),
                "has_more": page.has_more,
                "result_limit": max_results,
                "truncated": page.has_more,
                "effective_policy": effective_policy(scan_limits),
                "scan": scan_report(&stats, completeness),
                "continuation": continuation,
                "snapshot_sha256": page.snapshot_sha256,
                "page": page.metadata,
                "results": page.items,
            });
            Ok(finalize_with_cache(
                response,
                snapshot_cache.as_ref(),
                "find_name",
                &query_sha256,
                &search_root,
                &root,
                primary_items,
            ))
        }
        "find_ext" => {
            let (match_mode, match_mode_label) = match_mode_from_args(obj)?;
            let (case_mode, case_mode_label) = case_mode_from_args(obj)?;
            let sort_by = entry_sort_mode(obj)?;
            let exts = extension_values_from_args(
                obj,
                &[
                    "ext",
                    "extension",
                    "extensions",
                    "ext_filter",
                    "file_extension",
                    "file_extensions",
                ],
            );
            if exts.is_empty() {
                return Err("ext is required".to_string());
            }
            let pattern_norms = typed_name_patterns_from_args(obj, match_mode, false)?;
            let globs = glob_values_from_args(obj);
            let selector = DiscoverySelector {
                patterns: pattern_norms.clone(),
                globs: globs.clone(),
                extensions: exts.clone(),
                target_kind: TargetKind::File,
                match_mode,
                case_mode,
            };
            let stats = walk_collect_selected(&search_root, scan_limits, selector, &mut |p| {
                results.push(to_rel(&root, p));
                results.len() > MAX_RESULT_SNAPSHOT_ITEMS
            })?;
            let result_limit_reached = results.len() > MAX_RESULT_SNAPSHOT_ITEMS;
            results.truncate(MAX_RESULT_SNAPSHOT_ITEMS);
            let fuzzy_relevance = match_mode == MatchMode::Fuzzy
                && !pattern_norms.is_empty()
                && !obj.contains_key("sort_by");
            if fuzzy_relevance {
                sort_fuzzy_results(&mut results, &pattern_norms, case_mode);
            } else {
                sort_entry_results(&mut results, &root, sort_by);
            }
            results.dedup();
            let page = paginate(
                &results,
                &cursor,
                max_results,
                stats.limit_reached || result_limit_reached,
                &query_sha256,
            )?;
            let completeness =
                effective_completeness(&stats, result_limit_reached, page.stale_snapshot);
            let continuation =
                continuation_from_page(&page.metadata, completeness, &stats, &query_sha256);
            let ext = exts.first().cloned().unwrap_or_default();
            let primary_items = results.iter().cloned().map(Value::String).collect();
            let response = json!({
                "schema_version": 2,
                "source_skill": SKILL_NAME,
                "status": "ok",
                "action": "find_ext",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "ext": ext,
                "exts": exts,
                "patterns": pattern_norms,
                "globs": globs,
                "match_mode": match_mode_label,
                "case_mode": case_mode_label,
                "sort_by": if fuzzy_relevance { "relevance" } else { sort_by },
                "count": page.returned_count,
                "returned_count": page.returned_count,
                "total_count": page.total_count,
                "known_match_count": page.total_count,
                "total_count_is_complete": completeness.is_complete(),
                "completeness": completeness.as_str(),
                "has_more": page.has_more,
                "result_limit": max_results,
                "truncated": page.has_more,
                "effective_policy": effective_policy(scan_limits),
                "scan": scan_report(&stats, completeness),
                "continuation": continuation,
                "snapshot_sha256": page.snapshot_sha256,
                "page": page.metadata,
                "results": page.items,
            });
            Ok(finalize_with_cache(
                response,
                snapshot_cache.as_ref(),
                "find_ext",
                &query_sha256,
                &search_root,
                &root,
                primary_items,
            ))
        }
        "grep_text" => {
            let query = obj
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "query is required".to_string())?;
            if query.is_empty() {
                return Err("query_empty".to_string());
            }
            let (case_mode, case_mode_label) = case_mode_from_args(obj)?;
            let case_insensitive = smart_case_is_insensitive(case_mode, query);
            let (pattern_kind, pattern_kind_label) = pattern_kind_from_args(obj)?;
            if pattern_kind == PatternKind::Regex && query.len() > MAX_REGEX_PATTERN_BYTES {
                return Err("regex_pattern_too_large".to_string());
            }
            let output_mode = output_mode_from_args(obj)?;
            let multiline = bool_arg(obj, "multiline");
            let (file_match_mode, _) = match obj.get("file_match_mode") {
                Some(value) => {
                    let mut file_args = obj.clone();
                    file_args.insert("match_mode".to_string(), value.clone());
                    match_mode_from_args(&file_args)?
                }
                None => (MatchMode::Contains, "contains"),
            };
            let pattern_norms = typed_file_patterns_from_args(obj, file_match_mode);
            let globs = glob_values_from_args(obj);
            let context_before = obj
                .get("context_before")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(MAX_CONTEXT_LINES as u64) as usize;
            let context_after = obj
                .get("context_after")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(MAX_CONTEXT_LINES as u64) as usize;
            let max_line_chars = obj
                .get("max_line_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(240)
                .clamp(40, 2000) as usize;
            let max_file_bytes = obj
                .get("max_file_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_GREP_MAX_FILE_BYTES as u64)
                .clamp(1, MAX_GREP_FILE_BYTES as u64) as usize;
            let max_scan_bytes = obj
                .get("max_scan_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_GREP_MAX_SCAN_BYTES as u64)
                .clamp(1, MAX_GREP_SCAN_BYTES as u64) as usize;
            let grep_options = GrepOptions {
                query,
                pattern_kind,
                case_insensitive,
                multiline,
                context_before,
                context_after,
                max_line_chars,
            };
            find_matches("pattern preflight", grep_options)?;
            let selector = DiscoverySelector {
                patterns: pattern_norms.clone(),
                globs: globs.clone(),
                extensions: Vec::new(),
                target_kind: TargetKind::File,
                match_mode: file_match_mode,
                case_mode,
            };
            let mut candidate_paths = Vec::new();
            let stats = walk_collect_selected(&search_root, scan_limits, selector, &mut |path| {
                candidate_paths.push(path.to_path_buf());
                false
            })?;
            let outcome = search_content(
                &candidate_paths,
                ContentSearchOptions {
                    workspace_root: &root,
                    search_root: &search_root,
                    query,
                    pattern_kind,
                    case_mode,
                    case_insensitive,
                    multiline,
                    context_before,
                    context_after,
                    max_line_chars,
                    max_file_bytes,
                    max_scan_bytes,
                    max_matches: MAX_GREP_SNAPSHOT_MATCHES,
                    deadline: scan_limits.deadline,
                },
            )?;
            let matches = outcome.matches;
            let results = outcome.results;
            let content_scan_truncated = stats.limit_reached
                || outcome.match_limit_reached
                || outcome.scan_byte_limit_reached
                || outcome.skipped_large_files > 0;
            let match_page = paginate(
                &matches,
                &cursor,
                max_results,
                content_scan_truncated,
                &query_sha256,
            )?;
            let path_page = paginate(
                &results,
                &cursor,
                max_results,
                content_scan_truncated,
                &query_sha256,
            )?;
            let mut page_result_paths = if output_mode == "content" {
                match_page
                    .items
                    .iter()
                    .filter_map(|item| item.get("path").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            } else if output_mode == "paths" {
                path_page.items.clone()
            } else {
                Vec::new()
            };
            page_result_paths.sort();
            page_result_paths.dedup();
            let mut page_metadata = if output_mode == "paths" {
                path_page.metadata.clone()
            } else {
                match_page.metadata.clone()
            };
            if output_mode == "count" {
                if let Some(metadata) = page_metadata.as_object_mut() {
                    metadata.insert("returned_count".to_string(), json!(0));
                    metadata.insert("has_more".to_string(), Value::Bool(false));
                    metadata.insert("next_cursor".to_string(), Value::Null);
                    metadata.insert("previous_cursor".to_string(), Value::Null);
                    metadata.insert("legacy_next_offset".to_string(), Value::Null);
                }
            }
            let truncated = if output_mode == "paths" {
                path_page.has_more
            } else if output_mode == "count" {
                content_scan_truncated
            } else {
                match_page.has_more
            };
            let snapshot_sha256 = if output_mode == "paths" {
                path_page.snapshot_sha256.clone()
            } else {
                match_page.snapshot_sha256.clone()
            };
            let (known_match_count, selected_has_more, selected_stale_snapshot) =
                if output_mode == "paths" {
                    (
                        path_page.total_count,
                        path_page.has_more,
                        path_page.stale_snapshot,
                    )
                } else if output_mode == "count" {
                    (match_page.total_count, false, match_page.stale_snapshot)
                } else {
                    (
                        match_page.total_count,
                        match_page.has_more,
                        match_page.stale_snapshot,
                    )
                };
            let completeness =
                effective_completeness(&stats, content_scan_truncated, selected_stale_snapshot);
            let continuation = if output_mode == "count" && completeness.is_complete() {
                Value::Null
            } else {
                continuation_from_page(&page_metadata, completeness, &stats, &query_sha256)
            };
            let observation_bytes = serde_json::to_vec(&page_result_paths)
                .map(|value| value.len())
                .unwrap_or(0)
                .saturating_add(if output_mode == "content" {
                    serde_json::to_vec(&match_page.items)
                        .map(|value| value.len())
                        .unwrap_or(0)
                } else {
                    0
                });
            let primary_items = if output_mode == "paths" {
                results.iter().cloned().map(Value::String).collect()
            } else {
                matches.clone()
            };
            let response = json!({
                "schema_version": 2,
                "source_skill": SKILL_NAME,
                "status": "ok",
                "action": "grep_text",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "query": query,
                "pattern_kind": pattern_kind_label,
                "output_mode": output_mode,
                "case_mode": case_mode_label,
                "case_insensitive": case_insensitive,
                "multiline": multiline,
                "context_before": context_before,
                "context_after": context_after,
                "patterns": pattern_norms,
                "globs": globs,
                "count": if output_mode == "count" { matches.len() } else { page_result_paths.len() },
                "total_file_count": results.len(),
                "match_count": match_page.returned_count,
                "total_match_count": match_page.total_count,
                "known_match_count": known_match_count,
                "total_count_is_complete": completeness.is_complete(),
                "completeness": completeness.as_str(),
                "has_more": selected_has_more,
                "results": page_result_paths,
                "matches": if output_mode == "content" { match_page.items } else { Vec::<Value>::new() },
                "name_fallback_used": false,
                "scanned_bytes": outcome.scanned_bytes,
                "max_file_bytes": max_file_bytes,
                "max_scan_bytes": max_scan_bytes,
                "scan_byte_limit_reached": outcome.scan_byte_limit_reached,
                "skipped_large_files": outcome.skipped_large_files,
                "skipped_non_utf8_files": outcome.skipped_non_utf8_files,
                "skipped_binary_files": outcome.skipped_binary_files,
                "skipped_encoding_counts_complete": outcome.skipped_encoding_counts_complete,
                "content_backend": outcome.backend,
                "content_backend_version": outcome.backend_version,
                "content_backend_fallback_reason": outcome.backend_fallback_reason,
                "content_backend_elapsed_ms": outcome.backend_elapsed_ms,
                "result_limit": max_results,
                "truncated": truncated,
                "effective_policy": effective_policy(scan_limits),
                "scan": scan_report(&stats, completeness),
                "continuation": continuation,
                "snapshot_sha256": snapshot_sha256,
                "page": page_metadata,
                "observation_bytes": observation_bytes,
            });
            Ok(finalize_with_cache(
                response,
                snapshot_cache.as_ref(),
                "grep_text",
                &query_sha256,
                &search_root,
                &root,
                primary_items,
            ))
        }
        "find_images" | "images" | "image_search" => {
            let max_dirs = obj
                .get("max_dirs")
                .and_then(|v| v.as_u64())
                .unwrap_or(200)
                .clamp(1, 2000) as usize;
            let exts: Vec<String> = obj
                .get("exts")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.trim_start_matches('.').to_ascii_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(default_image_extensions);
            let snapshot = collect_images(
                &root,
                &search_root,
                scan_limits,
                &exts,
                MAX_RESULT_SNAPSHOT_ITEMS,
            )?;
            let page = paginate(
                &snapshot.entries,
                &cursor,
                max_results,
                snapshot.scan_truncated,
                &query_sha256,
            )?;
            let completeness = effective_completeness(
                &snapshot.stats,
                snapshot.scan_truncated,
                page.stale_snapshot,
            );
            let continuation = continuation_from_page(
                &page.metadata,
                completeness,
                &snapshot.stats,
                &query_sha256,
            );
            let results = page
                .items
                .iter()
                .map(|entry: &ImageEntry| entry.path.clone())
                .collect::<Vec<_>>();
            let (directories_by_count, directories_truncated) =
                directory_counts(&snapshot.entries, max_dirs);
            let primary_items = snapshot
                .entries
                .iter()
                .map(|entry| serde_json::to_value(entry).unwrap_or(Value::Null))
                .collect();
            let response = json!({
                "schema_version": 2,
                "source_skill": SKILL_NAME,
                "status": "ok",
                "action": "find_images",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "extensions": exts,
                "count": page.returned_count,
                "returned_count": page.returned_count,
                "total_count": page.total_count,
                "known_match_count": page.total_count,
                "total_count_is_complete": completeness.is_complete(),
                "completeness": completeness.as_str(),
                "has_more": page.has_more,
                "results": results,
                "images": page.items,
                "directories_by_count": directories_by_count,
                "directories_truncated": directories_truncated,
                "truncated": page.has_more,
                "effective_policy": effective_policy(scan_limits),
                "scan": scan_report(&snapshot.stats, completeness),
                "continuation": continuation,
                "snapshot_sha256": page.snapshot_sha256,
                "page": page.metadata,
            });
            return Ok(finalize_with_cache(
                response,
                snapshot_cache.as_ref(),
                "find_images",
                &query_sha256,
                &search_root,
                &root,
                primary_items,
            ));
        }
        _ => {
            return Err(
                "unsupported action; use find_name|find_ext|grep_text|find_images".to_string(),
            )
        }
    }
}

fn finalize_with_cache(
    mut response: Value,
    cache: Option<&SnapshotCache>,
    action: &str,
    query_sha256: &str,
    search_root: &Path,
    workspace_root: &Path,
    primary_items: Vec<Value>,
) -> Value {
    let snapshot_sha256 = response
        .get("snapshot_sha256")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let cache_status = cache
        .map(|cache| {
            cache
                .store(
                    action,
                    query_sha256,
                    snapshot_sha256,
                    search_root,
                    workspace_root,
                    &response,
                    &primary_items,
                )
                .unwrap_or("store_failed")
        })
        .unwrap_or("disabled");
    if let Some(object) = response.as_object_mut() {
        object.insert("cache_reused".to_string(), Value::Bool(false));
        object.insert(
            "cache_status".to_string(),
            Value::String(cache_status.to_string()),
        );
        if let Some(scan) = object.get_mut("scan").and_then(Value::as_object_mut) {
            scan.insert("cache_reused".to_string(), Value::Bool(false));
            scan.insert(
                "cache_status".to_string(),
                Value::String(cache_status.to_string()),
            );
        }
    }
    response
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

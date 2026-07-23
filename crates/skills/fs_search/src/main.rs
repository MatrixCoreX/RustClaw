use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod result_pagination;
mod workspace_traversal;

use result_pagination::{cursor_from_args, paginate};
use workspace_traversal::{
    path_kind, resolve_path, to_rel, walk_collect, walk_collect_dirs, walk_collect_nodes,
    workspace_root, ScanLimits,
};

const SKILL_NAME: &str = "fs_search";
const MAX_RESULT_SNAPSHOT_ITEMS: usize = 100_000;
const MAX_GREP_SNAPSHOT_MATCHES: usize = 20_000;

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

fn parse_env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
}

fn scan_limits_from_env() -> ScanLimits {
    let max_depth = parse_env_usize("RUSTCLAW_FS_SEARCH_MAX_DEPTH")
        .or_else(|| parse_env_usize("RUSTCLAW_LOCATOR_SCAN_MAX_DEPTH").map(|v| v.max(8)))
        .unwrap_or(8)
        .max(1);
    let max_files = parse_env_usize("RUSTCLAW_FS_SEARCH_MAX_FILES")
        .or_else(|| parse_env_usize("RUSTCLAW_LOCATOR_SCAN_MAX_FILES").map(|v| v.max(20_000)))
        .unwrap_or(20_000)
        .max(1);
    ScanLimits {
        max_depth,
        max_files,
    }
}

fn scan_limits_from_args(obj: &serde_json::Map<String, Value>) -> ScanLimits {
    let defaults = scan_limits_from_env();
    let max_depth = obj
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).clamp(1, 64))
        .unwrap_or(defaults.max_depth);
    let max_files = obj
        .get("max_files")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).clamp(1, 500_000))
        .unwrap_or(defaults.max_files);
    ScanLimits {
        max_depth,
        max_files,
    }
}

fn normalize_locator_text(text: &str) -> String {
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
        .to_lowercase()
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

fn expand_name_pattern(raw: &str) -> Vec<String> {
    let normalized = normalize_locator_text(raw);
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

fn strip_glob_wildcards(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(ch, '*' | '?'))
        .collect::<String>()
}

fn pattern_stem(pattern: &str) -> Option<&str> {
    let path = Path::new(pattern);
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty() && *stem != pattern)
}

fn pattern_extension(pattern: &str) -> Option<String> {
    Path::new(pattern)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
}

fn path_extension_matches_pattern(path: &Path, pattern_norm: &str) -> bool {
    let Some(pattern_ext) = pattern_extension(pattern_norm) else {
        return true;
    };
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| ext == pattern_ext)
}

fn name_matches_pattern(name_norm: &str, pattern_norm: &str) -> bool {
    if name_norm.contains(pattern_norm) {
        return true;
    }
    pattern_stem(pattern_norm).is_some_and(|stem| name_norm.contains(stem))
}

fn path_name_matches_pattern(
    path: &Path,
    name_norm: &str,
    pattern_norm: &str,
    exact: bool,
) -> bool {
    if exact {
        return name_norm == pattern_norm;
    }
    path_extension_matches_pattern(path, pattern_norm)
        && name_matches_pattern(name_norm, pattern_norm)
}

fn name_patterns_from_args(obj: &serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
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
    if raw_patterns.is_empty() {
        return Err("pattern is required".to_string());
    }
    let patterns = raw_patterns
        .iter()
        .flat_map(|raw| expand_name_pattern(raw))
        .filter(|pattern| !pattern.is_empty())
        .collect::<Vec<_>>();
    if patterns.is_empty() {
        return Err("pattern is required".to_string());
    }
    Ok(patterns)
}

fn optional_name_patterns_from_args(obj: &serde_json::Map<String, Value>) -> Vec<String> {
    string_values_from_args(
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
    )
    .iter()
    .flat_map(|raw| expand_name_pattern(raw))
    .filter(|pattern| !pattern.is_empty())
    .collect::<Vec<_>>()
}

fn optional_file_patterns_from_args(obj: &serde_json::Map<String, Value>) -> Vec<String> {
    string_values_from_args(
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
    )
    .iter()
    .flat_map(|raw| expand_name_pattern(raw))
    .filter(|pattern| !pattern.is_empty())
    .collect::<Vec<_>>()
}

fn grep_text_name_fallback_matches(
    workspace_root: &Path,
    search_root: &Path,
    query: &str,
    scan_limits: ScanLimits,
    snapshot_limit: usize,
) -> Result<(Vec<String>, Vec<String>, bool), String> {
    let patterns = expand_name_pattern(query)
        .into_iter()
        .filter(|pattern| !pattern.is_empty())
        .collect::<Vec<_>>();
    if patterns.is_empty() {
        return Ok((Vec::new(), Vec::new(), false));
    }
    let mut results = Vec::new();
    let stats = walk_collect_nodes(search_root, scan_limits, &mut |p| {
        let name = p
            .file_name()
            .map(|s| normalize_locator_text(&s.to_string_lossy()))
            .unwrap_or_default();
        if patterns
            .iter()
            .any(|pattern_norm| path_name_matches_pattern(p, &name, pattern_norm, false))
        {
            results.push(to_rel(workspace_root, p));
        }
        results.len() > snapshot_limit
    })?;
    let result_limit_reached = results.len() > snapshot_limit;
    results.truncate(snapshot_limit);
    results.sort();
    results.dedup();
    Ok((
        patterns,
        results,
        stats.limit_reached || result_limit_reached,
    ))
}

fn bool_arg(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn exact_name_match_requested(obj: &serde_json::Map<String, Value>) -> bool {
    if bool_arg(obj, "exact") || bool_arg(obj, "exact_name") {
        return true;
    }
    obj.get("match_mode")
        .or_else(|| obj.get("mode"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "exact" | "basename_exact" | "name_exact"
            )
        })
}

fn normalize_target_kind(value: &str) -> &str {
    match value {
        "files" => "file",
        "dirs" | "directory" | "directories" | "folder" | "folders" => "dir",
        "file" | "dir" | "any" => value,
        _ => "any",
    }
}

fn line_matches_query(line: &str, query: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        let line_folded = line.to_lowercase();
        let query_folded = query.to_lowercase();
        return line_folded.contains(&query_folded)
            || ordered_wildcard_query_matches(&line_folded, &query_folded);
    }
    line.contains(query) || ordered_wildcard_query_matches(line, query)
}

fn ordered_wildcard_query_matches(line: &str, query: &str) -> bool {
    if !query.contains(".*") {
        return false;
    }
    let parts = query
        .split(".*")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return false;
    }
    let mut rest = line;
    for part in parts {
        let Some(idx) = rest.find(part) else {
            return false;
        };
        rest = &rest[idx + part.len()..];
    }
    true
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
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
                    extra: Some(error_extra("execution_failed")),
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
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn execute(args: Value) -> Result<Value, String> {
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
    let cursor = cursor_from_args(obj);

    let root = workspace_root();
    let search_root = resolve_path(
        &root,
        obj.get("root")
            .or_else(|| obj.get("path"))
            .or_else(|| obj.get("dir"))
            .and_then(|v| v.as_str())
            .unwrap_or("."),
    )?;
    let scan_limits = scan_limits_from_args(obj);

    let mut results = Vec::new();
    match action.as_str() {
        "find_name" => {
            let pattern_norms = name_patterns_from_args(obj)?;
            let exact_name = exact_name_match_requested(obj);
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
            let mut collect = |p: &Path| {
                let name = p
                    .file_name()
                    .map(|s| normalize_locator_text(&s.to_string_lossy()))
                    .unwrap_or_default();
                if !pattern_norms.iter().any(|pattern_norm| {
                    path_name_matches_pattern(p, &name, pattern_norm, exact_name)
                }) {
                    return false;
                }
                let kind = path_kind(p);
                if target_kind == "any" || target_kind == kind {
                    results.push(to_rel(&root, p));
                }
                results.len() > MAX_RESULT_SNAPSHOT_ITEMS
            };
            let stats = if target_kind == "dir" {
                walk_collect_dirs(&search_root, scan_limits, &mut collect)?
            } else {
                walk_collect_nodes(&search_root, scan_limits, &mut collect)?
            };
            let result_limit_reached = results.len() > MAX_RESULT_SNAPSHOT_ITEMS;
            results.truncate(MAX_RESULT_SNAPSHOT_ITEMS);
            results.sort();
            results.dedup();
            let page = paginate(
                &results,
                cursor,
                max_results,
                stats.limit_reached || result_limit_reached,
            );
            Ok(json!({
                "action": "find_name",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "patterns": pattern_norms,
                "exact": exact_name,
                "count": page.returned_count,
                "returned_count": page.returned_count,
                "total_count": page.total_count,
                "result_limit": max_results,
                "truncated": page.has_more,
                "snapshot_sha256": page.snapshot_sha256,
                "page": page.metadata,
                "results": page.items,
            }))
        }
        "find_ext" => {
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
            let pattern_norms = optional_name_patterns_from_args(obj);
            let stats = walk_collect(&search_root, scan_limits, &mut |p| {
                let got = p
                    .extension()
                    .map(|s| s.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                let name = p
                    .file_name()
                    .map(|s| normalize_locator_text(&s.to_string_lossy()))
                    .unwrap_or_default();
                let name_matches = pattern_norms.is_empty()
                    || pattern_norms
                        .iter()
                        .any(|pattern_norm| name_matches_pattern(&name, pattern_norm));
                if exts.iter().any(|ext| ext == &got) && name_matches {
                    results.push(to_rel(&root, p));
                }
                results.len() > MAX_RESULT_SNAPSHOT_ITEMS
            })?;
            let result_limit_reached = results.len() > MAX_RESULT_SNAPSHOT_ITEMS;
            results.truncate(MAX_RESULT_SNAPSHOT_ITEMS);
            results.sort();
            results.dedup();
            let page = paginate(
                &results,
                cursor,
                max_results,
                stats.limit_reached || result_limit_reached,
            );
            let ext = exts.first().cloned().unwrap_or_default();
            Ok(json!({
                "action": "find_ext",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "ext": ext,
                "exts": exts,
                "patterns": pattern_norms,
                "count": page.returned_count,
                "returned_count": page.returned_count,
                "total_count": page.total_count,
                "result_limit": max_results,
                "truncated": page.has_more,
                "snapshot_sha256": page.snapshot_sha256,
                "page": page.metadata,
                "results": page.items,
            }))
        }
        "grep_text" => {
            let query = obj
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "query is required".to_string())?;
            let case_insensitive =
                bool_arg(obj, "case_insensitive") || bool_arg(obj, "ignore_case");
            let pattern_norms = optional_file_patterns_from_args(obj);
            let max_line_chars = obj
                .get("max_line_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(240)
                .clamp(40, 2000) as usize;
            let mut matches = Vec::new();
            let stats = walk_collect(&search_root, scan_limits, &mut |p| {
                if !pattern_norms.is_empty() {
                    let name = p
                        .file_name()
                        .map(|s| normalize_locator_text(&s.to_string_lossy()))
                        .unwrap_or_default();
                    if !pattern_norms.iter().any(|pattern_norm| {
                        path_name_matches_pattern(p, &name, pattern_norm, false)
                    }) {
                        return false;
                    }
                }
                if let Ok(text) = std::fs::read_to_string(p) {
                    let rel = to_rel(&root, p);
                    let mut file_matched = false;
                    for (idx, line) in text.lines().enumerate() {
                        if line_matches_query(line, query, case_insensitive) {
                            if !file_matched {
                                results.push(rel.clone());
                                file_matched = true;
                            }
                            matches.push(json!({
                                "path": rel.clone(),
                                "line": idx + 1,
                                "text": truncate_chars(line.trim(), max_line_chars),
                            }));
                            if matches.len() > MAX_GREP_SNAPSHOT_MATCHES {
                                return true;
                            }
                        }
                    }
                }
                matches.len() > MAX_GREP_SNAPSHOT_MATCHES
            })?;
            let match_limit_reached = matches.len() > MAX_GREP_SNAPSHOT_MATCHES;
            matches.truncate(MAX_GREP_SNAPSHOT_MATCHES);
            matches.sort_by(|left, right| {
                let left_path = left.get("path").and_then(Value::as_str).unwrap_or_default();
                let right_path = right
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                left_path.cmp(right_path).then_with(|| {
                    left.get("line")
                        .and_then(Value::as_u64)
                        .cmp(&right.get("line").and_then(Value::as_u64))
                })
            });
            results.sort();
            results.dedup();
            let content_scan_truncated = stats.limit_reached || match_limit_reached;
            let (name_patterns, name_results, name_scan_truncated) = if results.is_empty() {
                grep_text_name_fallback_matches(
                    &root,
                    &search_root,
                    query,
                    scan_limits,
                    MAX_RESULT_SNAPSHOT_ITEMS,
                )?
            } else {
                (Vec::new(), Vec::new(), false)
            };
            let match_page = paginate(&matches, cursor, max_results, content_scan_truncated);
            let name_page = paginate(&name_results, cursor, max_results, name_scan_truncated);
            let page_result_paths = match_page
                .items
                .iter()
                .filter_map(|item| item.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>();
            let mut page_result_paths = page_result_paths;
            page_result_paths.sort();
            page_result_paths.dedup();
            let use_name_page = matches.is_empty();
            let page_metadata = if use_name_page {
                name_page.metadata
            } else {
                match_page.metadata
            };
            let truncated = if use_name_page {
                name_page.has_more
            } else {
                match_page.has_more
            };
            let snapshot_sha256 = if use_name_page {
                name_page.snapshot_sha256
            } else {
                match_page.snapshot_sha256
            };
            Ok(json!({
                "action": "grep_text",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "query": query,
                "case_insensitive": case_insensitive,
                "patterns": pattern_norms,
                "count": page_result_paths.len(),
                "total_file_count": results.len(),
                "match_count": match_page.returned_count,
                "total_match_count": match_page.total_count,
                "results": page_result_paths,
                "matches": match_page.items,
                "name_patterns": name_patterns,
                "name_count": name_page.returned_count,
                "total_name_count": name_page.total_count,
                "name_results": name_page.items,
                "result_limit": max_results,
                "truncated": truncated,
                "snapshot_sha256": snapshot_sha256,
                "page": page_metadata,
            }))
        }
        "find_images" | "images" | "image_search" => {
            let mut files = Vec::new();
            let mut dir_counts: HashMap<String, usize> = HashMap::new();
            let max_files = obj
                .get("max_files")
                .and_then(|v| v.as_u64())
                .unwrap_or(20000)
                .min(200000) as usize;
            let max_dirs = obj
                .get("max_dirs")
                .and_then(|v| v.as_u64())
                .unwrap_or(200)
                .min(2000) as usize;
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
                .unwrap_or_else(|| {
                    vec![
                        "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "svg", "ico",
                        "heic", "heif",
                    ]
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect()
                });

            walk_collect(&search_root, scan_limits, &mut |p| {
                let ext = p
                    .extension()
                    .map(|s| s.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if exts.iter().any(|e| e == &ext) {
                    files.push(to_rel(&root, p));
                    let rel = to_rel(&root, p);
                    let dir = std::path::Path::new(&rel)
                        .parent()
                        .map(|d| d.to_string_lossy().to_string())
                        .unwrap_or_else(|| ".".to_string());
                    *dir_counts.entry(dir).or_insert(0) += 1;
                }
                files.len() >= max_files
            })?;

            let mut dir_items: Vec<(String, usize)> = dir_counts.into_iter().collect();
            dir_items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            if dir_items.len() > max_dirs {
                dir_items.truncate(max_dirs);
            }
            let directories_by_count: Vec<Value> = dir_items
                .into_iter()
                .map(|(dir, count)| json!({"dir": dir, "count": count}))
                .collect();
            return Ok(json!({
                "action": "find_images",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "count": files.len(),
                "results": files,
                "directories_by_count": directories_by_count,
            }));
        }
        _ => {
            return Err(
                "unsupported action; use find_name|find_ext|grep_text|find_images".to_string(),
            )
        }
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

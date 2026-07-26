use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod grep_search;
mod image_search;
mod result_pagination;
mod workspace_traversal;

use grep_search::{find_matches, GrepOptions, MAX_CONTEXT_LINES};
use image_search::{collect_images, default_image_extensions, directory_counts, ImageEntry};
use result_pagination::{cursor_from_args, paginate};
use workspace_traversal::{
    path_kind, resolve_path, to_rel, walk_collect, walk_collect_dirs, walk_collect_nodes,
    workspace_root, ScanLimits,
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
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn execution_error_kind(error: &str) -> &str {
    match error {
        "multiline_query_invalid" | "query_empty" | "sort_by_unsupported" => error,
        _ => "execution_failed",
    }
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
            sort_entry_results(&mut results, &root, sort_by);
            results.dedup();
            let page = paginate(
                &results,
                cursor,
                max_results,
                stats.limit_reached || result_limit_reached,
            );
            Ok(json!({
                "schema_version": 1,
                "source_skill": SKILL_NAME,
                "status": "ok",
                "action": "find_name",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "patterns": pattern_norms,
                "exact": exact_name,
                "sort_by": sort_by,
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
            sort_entry_results(&mut results, &root, sort_by);
            results.dedup();
            let page = paginate(
                &results,
                cursor,
                max_results,
                stats.limit_reached || result_limit_reached,
            );
            let ext = exts.first().cloned().unwrap_or_default();
            Ok(json!({
                "schema_version": 1,
                "source_skill": SKILL_NAME,
                "status": "ok",
                "action": "find_ext",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "ext": ext,
                "exts": exts,
                "patterns": pattern_norms,
                "sort_by": sort_by,
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
            if query.is_empty() {
                return Err("query_empty".to_string());
            }
            let case_insensitive =
                bool_arg(obj, "case_insensitive") || bool_arg(obj, "ignore_case");
            let multiline = bool_arg(obj, "multiline");
            let pattern_norms = optional_file_patterns_from_args(obj);
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
            let mut matches = Vec::new();
            let mut scanned_bytes = 0usize;
            let mut skipped_large_files = 0usize;
            let mut skipped_non_utf8_files = 0usize;
            let mut scan_byte_limit_reached = false;
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
                let file_bytes = std::fs::metadata(p)
                    .map(|metadata| metadata.len().min(usize::MAX as u64) as usize)
                    .unwrap_or(0);
                if file_bytes > max_file_bytes {
                    skipped_large_files = skipped_large_files.saturating_add(1);
                    return false;
                }
                if scanned_bytes.saturating_add(file_bytes) > max_scan_bytes {
                    scan_byte_limit_reached = true;
                    return true;
                }
                if let Ok(bytes) = std::fs::read(p) {
                    scanned_bytes = scanned_bytes.saturating_add(bytes.len());
                    let Ok(text) = String::from_utf8(bytes) else {
                        skipped_non_utf8_files = skipped_non_utf8_files.saturating_add(1);
                        return false;
                    };
                    let rel = to_rel(&root, p);
                    let options = GrepOptions {
                        query,
                        case_insensitive,
                        multiline,
                        context_before,
                        context_after,
                        max_line_chars,
                    };
                    let file_matches = match find_matches(&text, options) {
                        Ok(file_matches) => file_matches,
                        Err(_) => return false,
                    };
                    if !file_matches.is_empty() {
                        results.push(rel.clone());
                    }
                    for matched in file_matches {
                        let mut value = serde_json::to_value(matched).unwrap_or_default();
                        if let Some(object) = value.as_object_mut() {
                            object.insert("path".to_string(), Value::String(rel.clone()));
                        }
                        matches.push(value);
                        if matches.len() > MAX_GREP_SNAPSHOT_MATCHES {
                            return true;
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
            let content_scan_truncated = stats.limit_reached
                || match_limit_reached
                || scan_byte_limit_reached
                || skipped_large_files > 0;
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
                "schema_version": 1,
                "source_skill": SKILL_NAME,
                "status": "ok",
                "action": "grep_text",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "query": query,
                "case_insensitive": case_insensitive,
                "multiline": multiline,
                "context_before": context_before,
                "context_after": context_after,
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
                "scanned_bytes": scanned_bytes,
                "max_file_bytes": max_file_bytes,
                "max_scan_bytes": max_scan_bytes,
                "scan_byte_limit_reached": scan_byte_limit_reached,
                "skipped_large_files": skipped_large_files,
                "skipped_non_utf8_files": skipped_non_utf8_files,
                "result_limit": max_results,
                "truncated": truncated,
                "snapshot_sha256": snapshot_sha256,
                "page": page_metadata,
            }))
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
                cursor,
                max_results,
                snapshot.scan_truncated,
            );
            let results = page
                .items
                .iter()
                .map(|entry: &ImageEntry| entry.path.clone())
                .collect::<Vec<_>>();
            let (directories_by_count, directories_truncated) =
                directory_counts(&snapshot.entries, max_dirs);
            return Ok(json!({
                "schema_version": 1,
                "source_skill": SKILL_NAME,
                "status": "ok",
                "action": "find_images",
                "root": to_rel(&root, &search_root),
                "workspace_root": root.display().to_string(),
                "extensions": exts,
                "count": page.returned_count,
                "returned_count": page.returned_count,
                "total_count": page.total_count,
                "results": results,
                "images": page.items,
                "directories_by_count": directories_by_count,
                "directories_truncated": directories_truncated,
                "truncated": page.has_more,
                "snapshot_sha256": page.snapshot_sha256,
                "page": page.metadata,
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

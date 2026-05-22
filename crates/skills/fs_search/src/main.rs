use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

#[derive(Debug, Clone, Copy)]
struct ScanLimits {
    max_depth: usize,
    max_files: usize,
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

fn skip_default_scan_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".venv" | "venv" | "__pycache__"
    )
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
    max_results: usize,
) -> Result<(Vec<String>, Vec<String>), String> {
    let patterns = expand_name_pattern(query)
        .into_iter()
        .filter(|pattern| !pattern.is_empty())
        .collect::<Vec<_>>();
    if patterns.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut results = Vec::new();
    walk_collect_nodes(search_root, scan_limits, &mut |p| {
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
        results.len() >= max_results
    })?;
    Ok((patterns, results))
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

fn line_matches_query(line: &str, query: &str) -> bool {
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
                    extra: None,
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: None,
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
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
        .min(1000) as usize;

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
                let kind = if p.is_dir() {
                    "dir"
                } else if p.is_file() {
                    "file"
                } else {
                    "other"
                };
                if target_kind == "any" || target_kind == kind {
                    results.push(to_rel(&root, p));
                }
                results.len() >= max_results
            };
            if target_kind == "dir" {
                walk_collect_dirs(&search_root, scan_limits, &mut collect)?;
            } else {
                walk_collect_nodes(&search_root, scan_limits, &mut collect)?;
            }
            Ok(json!({
                "action": "find_name",
                "root": to_rel(&root, &search_root),
                "patterns": pattern_norms,
                "exact": exact_name,
                "count": results.len(),
                "results": results,
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
            walk_collect(&search_root, scan_limits, &mut |p| {
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
                results.len() >= max_results
            })?;
            let ext = exts.first().cloned().unwrap_or_default();
            Ok(json!({
                "action": "find_ext",
                "root": to_rel(&root, &search_root),
                "ext": ext,
                "exts": exts,
                "patterns": pattern_norms,
                "count": results.len(),
                "results": results,
            }))
        }
        "grep_text" => {
            let query = obj
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "query is required".to_string())?;
            let pattern_norms = optional_file_patterns_from_args(obj);
            let max_line_chars = obj
                .get("max_line_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(240)
                .clamp(40, 2000) as usize;
            let mut matches = Vec::new();
            walk_collect(&search_root, scan_limits, &mut |p| {
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
                        if line_matches_query(line, query) {
                            if !file_matched {
                                results.push(rel.clone());
                                file_matched = true;
                            }
                            matches.push(json!({
                                "path": rel.clone(),
                                "line": idx + 1,
                                "text": truncate_chars(line.trim(), max_line_chars),
                            }));
                            if matches.len() >= max_results {
                                return true;
                            }
                        }
                    }
                }
                matches.len() >= max_results
            })?;
            let (name_patterns, name_results) = if results.is_empty() {
                grep_text_name_fallback_matches(
                    &root,
                    &search_root,
                    query,
                    scan_limits,
                    max_results,
                )?
            } else {
                (Vec::new(), Vec::new())
            };
            Ok(json!({
                "action": "grep_text",
                "root": to_rel(&root, &search_root),
                "query": query,
                "patterns": pattern_norms,
                "count": results.len(),
                "match_count": matches.len(),
                "results": results,
                "matches": matches,
                "name_patterns": name_patterns,
                "name_count": name_results.len(),
                "name_results": name_results,
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

fn walk_collect(
    path: &Path,
    limits: ScanLimits,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    let mut scanned_files = 0usize;
    let mut stop = false;
    walk_collect_inner(path, 0, limits, &mut scanned_files, &mut stop, f)
}

fn walk_collect_inner(
    path: &Path,
    depth: usize,
    limits: ScanLimits,
    scanned_files: &mut usize,
    stop: &mut bool,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    if *stop {
        return Ok(());
    }
    if path.is_file() {
        if *scanned_files >= limits.max_files {
            return Ok(());
        }
        *scanned_files += 1;
        if f(path) {
            *stop = true;
        }
        return Ok(());
    }
    if depth > limits.max_depth {
        return Ok(());
    }
    let iter = std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))?;
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let p = entry.path();
        if p.is_dir() {
            if skip_default_scan_dir(&p) {
                continue;
            }
            dirs.push(p);
        } else {
            files.push(p);
        }
    }
    files.sort();
    dirs.sort();
    for p in files {
        if *scanned_files >= limits.max_files {
            return Ok(());
        }
        *scanned_files += 1;
        if f(&p) {
            *stop = true;
            return Ok(());
        }
    }
    for p in dirs {
        if *stop {
            return Ok(());
        }
        if depth < limits.max_depth {
            walk_collect_inner(&p, depth + 1, limits, scanned_files, stop, f)?;
        }
    }
    Ok(())
}

fn walk_collect_nodes(
    path: &Path,
    limits: ScanLimits,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    let mut scanned_files = 0usize;
    let mut stop = false;
    walk_collect_nodes_inner(path, 0, limits, &mut scanned_files, &mut stop, f)
}

fn walk_collect_dirs(
    path: &Path,
    limits: ScanLimits,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    let mut scanned_dirs = 0usize;
    let mut stop = false;
    walk_collect_dirs_inner(path, 0, limits, &mut scanned_dirs, &mut stop, f)
}

fn walk_collect_dirs_inner(
    path: &Path,
    depth: usize,
    limits: ScanLimits,
    scanned_dirs: &mut usize,
    stop: &mut bool,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    if *stop || !path.is_dir() {
        return Ok(());
    }
    if *scanned_dirs >= limits.max_files {
        return Ok(());
    }
    *scanned_dirs += 1;
    if f(path) {
        *stop = true;
        return Ok(());
    }
    if depth >= limits.max_depth {
        return Ok(());
    }
    let iter = std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))?;
    let mut dirs = Vec::new();
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let p = entry.path();
        if p.is_dir() {
            if skip_default_scan_dir(&p) {
                continue;
            }
            dirs.push(p);
        }
    }
    dirs.sort();
    for p in dirs {
        if *stop {
            return Ok(());
        }
        walk_collect_dirs_inner(&p, depth + 1, limits, scanned_dirs, stop, f)?;
    }
    Ok(())
}

fn walk_collect_nodes_inner(
    path: &Path,
    depth: usize,
    limits: ScanLimits,
    scanned_files: &mut usize,
    stop: &mut bool,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    if *stop {
        return Ok(());
    }
    if path.is_file() {
        if *scanned_files >= limits.max_files {
            return Ok(());
        }
        *scanned_files += 1;
        if f(path) {
            *stop = true;
        }
        return Ok(());
    }
    if path.is_dir() && f(path) {
        *stop = true;
        return Ok(());
    }
    if depth > limits.max_depth {
        return Ok(());
    }
    let iter = std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))?;
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let p = entry.path();
        if p.is_dir() {
            if skip_default_scan_dir(&p) {
                continue;
            }
            dirs.push(p);
        } else {
            files.push(p);
        }
    }
    files.sort();
    dirs.sort();
    for p in files {
        if *scanned_files >= limits.max_files {
            return Ok(());
        }
        *scanned_files += 1;
        if f(&p) {
            *stop = true;
            return Ok(());
        }
    }
    for p in dirs {
        if *stop {
            return Ok(());
        }
        if depth < limits.max_depth {
            walk_collect_nodes_inner(&p, depth + 1, limits, scanned_files, stop, f)?;
        }
    }
    Ok(())
}

fn to_rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn resolve_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let raw = Path::new(input);
    let mut normalized = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::ParentDir => return Err("path with '..' is not allowed".to_string()),
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if raw.is_absolute() {
        return Ok(normalized);
    }
    Ok(workspace_root.join(normalized))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rustclaw-fs-search-{name}-{}", std::process::id()))
    }

    #[test]
    fn find_name_reaches_nested_prompt_paths_with_explicit_depth() {
        let root = unique_temp_dir("nested-prompt");
        let nested = root.join("prompts/layers/overlays");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        std::fs::write(nested.join("intent_normalizer_prompt.md"), "# prompt\n")
            .expect("write prompt file");

        let out = execute(json!({
            "action": "find_name",
            "pattern": "intent_normalizer_prompt",
            "root": root.to_string_lossy().to_string(),
            "max_depth": 8,
            "max_results": 10
        }))
        .expect("find_name succeeds");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array");
        assert!(
            results.iter().any(|v| v.as_str().is_some_and(
                |s| s.ends_with("prompts/layers/overlays/intent_normalizer_prompt.md")
            )),
            "results={results:?}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_name_directory_target_ignores_unrelated_file_budget() {
        let root = unique_temp_dir("dir-budget");
        let noisy = root.join("a_many_files");
        let target = root.join("z_parent/bundle_src");
        std::fs::create_dir_all(&noisy).expect("create noisy dir");
        std::fs::create_dir_all(&target).expect("create target dir");
        std::fs::write(root.join("z_parent/readme.txt"), "nearby file\n")
            .expect("write sibling file");
        for idx in 0..8 {
            std::fs::write(noisy.join(format!("noise_{idx}.txt")), "noise\n")
                .expect("write noise file");
        }

        let out = execute(json!({
            "action": "find_name",
            "pattern": "bundle_src",
            "root": root.to_string_lossy().to_string(),
            "target_kind": "directory",
            "max_depth": 4,
            "max_files": 4,
            "max_results": 5
        }))
        .expect("find_name succeeds");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array");
        assert!(
            results.iter().any(|v| v
                .as_str()
                .is_some_and(|s| s.ends_with("z_parent/bundle_src"))),
            "results={results:?}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_name_accepts_multiple_patterns_and_file_filter_alias() {
        let root = unique_temp_dir("multi-pattern");
        std::fs::create_dir_all(root.join("audio_dir")).expect("create audio dir");
        std::fs::write(root.join("audio.toml"), "").expect("write audio config");
        std::fs::write(root.join("image.toml"), "").expect("write image config");
        std::fs::write(root.join("stock.toml"), "").expect("write unrelated config");

        let out = execute(json!({
            "action": "find_name",
            "patterns": ["*audio*", "*image*"],
            "files_only": true,
            "root": root.to_string_lossy().to_string(),
            "max_depth": 2,
            "max_results": 10
        }))
        .expect("find_name succeeds with patterns");

        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(results.iter().any(|path| path.ends_with("audio.toml")));
        assert!(results.iter().any(|path| path.ends_with("image.toml")));
        assert!(!results.iter().any(|path| path.ends_with("audio_dir")));
        assert!(!results.iter().any(|path| path.ends_with("stock.toml")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_name_accepts_entry_name_alias() {
        let root = unique_temp_dir("entry-name-alias");
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        std::fs::write(nested.join("config.ini"), "").expect("write config");
        std::fs::write(root.join("config.txt"), "").expect("write sibling");

        let out = execute(json!({
            "action": "find_name",
            "entry_name": "config.ini",
            "target_kind": "file",
            "root": root.to_string_lossy().to_string(),
            "max_depth": 3,
            "max_results": 10
        }))
        .expect("find_name succeeds with entry_name alias");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 1);
        assert!(results[0].ends_with("nested/config.ini"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_name_exact_basename_avoids_stem_contains_match() {
        let root = unique_temp_dir("exact-basename");
        let _ = std::fs::remove_dir_all(&root);
        let exact_dir = root.join("case_only");
        let fuzzy_dir = root.join("fuzzy_top3");
        std::fs::create_dir_all(&exact_dir).expect("create exact dir");
        std::fs::create_dir_all(&fuzzy_dir).expect("create fuzzy dir");
        std::fs::write(exact_dir.join("Report.MD"), "").expect("write exact report");
        std::fs::write(fuzzy_dir.join("abcd_report.md"), "").expect("write fuzzy report");

        let out = execute(json!({
            "action": "find_name",
            "pattern": "Report.MD",
            "exact": true,
            "target_kind": "file",
            "root": root.to_string_lossy().to_string(),
            "max_depth": 3,
            "max_results": 10
        }))
        .expect("find_name succeeds with exact basename");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 1);
        assert!(results[0].ends_with("case_only/Report.MD"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_name_checks_shallow_files_before_deep_scan_budget() {
        let root = unique_temp_dir("shallow-before-deep");
        let deep = root.join("aaa_deep");
        std::fs::create_dir_all(&deep).expect("create deep dir");
        for idx in 0..8 {
            std::fs::write(deep.join(format!("noise-{idx}.txt")), "").expect("write noise");
        }
        std::fs::write(root.join("start-all-bin.sh"), "#!/usr/bin/env bash\n")
            .expect("write shallow script");

        let out = execute(json!({
            "action": "find_name",
            "pattern": "start-all-bin.sh",
            "root": root.to_string_lossy().to_string(),
            "max_depth": 8,
            "max_files": 1,
            "max_results": 10
        }))
        .expect("find_name succeeds");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array");
        assert!(results
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.ends_with("start-all-bin.sh"))));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_name_expands_simple_alternation_pattern() {
        let root = unique_temp_dir("alternation-pattern");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(root.join("speech.toml"), "").expect("write speech config");
        std::fs::write(root.join("photo.toml"), "").expect("write photo config");

        let out = execute(json!({
            "action": "find_name",
            "pattern": "*(speech|photo)*",
            "files_only": true,
            "root": root.to_string_lossy().to_string(),
            "max_depth": 1,
            "max_results": 10
        }))
        .expect("find_name succeeds with alternation");

        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(results.iter().any(|path| path.ends_with("speech.toml")));
        assert!(results.iter().any(|path| path.ends_with("photo.toml")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_ext_respects_optional_name_pattern() {
        let root = unique_temp_dir("find-ext-pattern");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(
            root.join("execution_intent_routing_repair_plan_20260509.md"),
            "",
        )
        .expect("write target plan");
        std::fs::write(root.join("builtin_skill_capability_governance_plan.md"), "")
            .expect("write unrelated plan");
        std::fs::write(root.join("execution_intent_trace.txt"), "").expect("write non-md file");

        let out = execute(json!({
            "action": "find_ext",
            "ext": "md",
            "pattern": "*execution_intent*.md",
            "root": root.to_string_lossy().to_string(),
            "max_depth": 1,
            "max_results": 10
        }))
        .expect("find_ext succeeds with pattern");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 1);
        assert!(results[0].ends_with("execution_intent_routing_repair_plan_20260509.md"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_ext_accepts_extension_alias_array_and_pattern() {
        let root = unique_temp_dir("find-ext-alias-array");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(root.join("clawd.log.md"), "").expect("write md target");
        std::fs::write(root.join("agent-log.txt"), "").expect("write txt target");
        std::fs::write(root.join("agent-log.toml"), "").expect("write non-target extension");
        std::fs::write(root.join("notes.md"), "").expect("write non-target name");

        let out = execute(json!({
            "action": "find_ext",
            "ext_filter": ["md", ".txt"],
            "query": "log",
            "root": root.to_string_lossy().to_string(),
            "max_depth": 1,
            "max_results": 10
        }))
        .expect("find_ext succeeds with extension aliases");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(2));
        let exts = out
            .get("exts")
            .and_then(Value::as_array)
            .expect("exts array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(exts, vec!["md", "txt"]);
        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(results.iter().any(|path| path.ends_with("clawd.log.md")));
        assert!(results.iter().any(|path| path.ends_with("agent-log.txt")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_ext_respects_max_results_across_subdirectories() {
        let root = unique_temp_dir("find-ext-max-results");
        for dir in ["a", "b", "c"] {
            std::fs::create_dir_all(root.join(dir)).expect("create nested dir");
            std::fs::write(root.join(dir).join(format!("{dir}.toml")), "").expect("write config");
        }

        let out = execute(json!({
            "action": "find_ext",
            "ext": "toml",
            "root": root.to_string_lossy().to_string(),
            "max_depth": 2,
            "max_results": 2
        }))
        .expect("find_ext succeeds");

        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array");
        assert_eq!(out.get("count").and_then(Value::as_u64), Some(2));
        assert_eq!(results.len(), 2, "results={results:?}");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn grep_text_returns_matching_lines_for_known_file_root() {
        let root = unique_temp_dir("grep-text-lines");
        std::fs::create_dir_all(&root).expect("create root");
        let file = root.join("sample.rs");
        std::fs::write(
            &file,
            "fn unrelated() {}\nif step_type == \"run_cmd\" {\n    normalize_run_cmd_call();\n}\n",
        )
        .expect("write sample file");

        let out = execute(json!({
            "action": "grep_text",
            "query": "run_cmd",
            "root": file.to_string_lossy().to_string(),
            "max_results": 10
        }))
        .expect("grep_text succeeds");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        assert_eq!(out.get("match_count").and_then(Value::as_u64), Some(2));
        let matches = out
            .get("matches")
            .and_then(Value::as_array)
            .expect("matches array");
        assert_eq!(matches[0].get("line").and_then(Value::as_u64), Some(2));
        assert!(matches[0]
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("step_type") && text.contains("run_cmd")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn grep_text_accepts_ordered_wildcard_query() {
        let root = unique_temp_dir("grep-text-ordered-wildcard");
        std::fs::create_dir_all(&root).expect("create root");
        let file = root.join("sample.rs");
        std::fs::write(&file, "if step_type == \"run_cmd\" {\n}\n").expect("write sample file");

        let out = execute(json!({
            "action": "grep_text",
            "query": "type.*run_cmd",
            "path": file.to_string_lossy().to_string(),
            "max_results": 10
        }))
        .expect("grep_text succeeds with ordered wildcard query");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        assert_eq!(out.get("match_count").and_then(Value::as_u64), Some(1));
        let matches = out
            .get("matches")
            .and_then(Value::as_array)
            .expect("matches array");
        assert!(matches[0]
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("step_type") && text.contains("run_cmd")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn grep_text_accepts_path_alias_for_search_root() {
        let root = unique_temp_dir("grep-text-path-alias");
        std::fs::create_dir_all(&root).expect("create root");
        let file = root.join("sample.rs");
        std::fs::write(&file, "if step_type == \"run_cmd\" {}\n").expect("write sample file");

        let out = execute(json!({
            "action": "grep_text",
            "query": "run_cmd",
            "path": file.to_string_lossy().to_string(),
            "max_results": 10
        }))
        .expect("grep_text succeeds with path alias");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        let root_value = out.get("root").and_then(Value::as_str).unwrap_or_default();
        assert!(
            root_value.ends_with("sample.rs"),
            "root should reflect the path alias target, got {root_value:?}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn grep_text_filters_by_file_pattern() {
        let root = unique_temp_dir("grep-text-file-pattern");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(
            root.join("prompt_utils.rs"),
            "if step_type == \"run_cmd\" {}\n",
        )
        .expect("write target");
        std::fs::write(root.join("other.rs"), "if step_type == \"run_cmd\" {}\n")
            .expect("write sibling");

        let out = execute(json!({
            "action": "grep_text",
            "query": "run_cmd",
            "pattern": "prompt_utils.rs",
            "root": root.to_string_lossy().to_string(),
            "max_results": 10
        }))
        .expect("grep_text succeeds with file pattern");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 1);
        assert!(results[0].ends_with("prompt_utils.rs"));
        assert!(out
            .get("patterns")
            .and_then(Value::as_array)
            .is_some_and(|patterns| patterns
                .iter()
                .any(|item| item.as_str() == Some("prompt_utils.rs"))));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn grep_text_surfaces_name_matches_when_content_has_no_hits() {
        let root = unique_temp_dir("grep-text-name-fallback");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(root.join("my_abcd.txt"), "content without target\n").expect("write target");
        std::fs::write(root.join("other.txt"), "content without target\n").expect("write other");

        let out = execute(json!({
            "action": "grep_text",
            "query": "abcd",
            "root": root.to_string_lossy().to_string(),
            "max_depth": 1,
            "max_results": 10
        }))
        .expect("grep_text succeeds");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(0));
        assert_eq!(out.get("match_count").and_then(Value::as_u64), Some(0));
        assert_eq!(out.get("name_count").and_then(Value::as_u64), Some(1));
        let name_results = out
            .get("name_results")
            .and_then(Value::as_array)
            .expect("name_results array");
        assert!(name_results
            .iter()
            .any(|v| v.as_str().is_some_and(|path| path.ends_with("my_abcd.txt"))));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_name_pattern_with_extension_filters_extension() {
        let root = unique_temp_dir("find-name-ext-pattern");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(
            root.join("execution_intent_routing_repair_plan_20260509.md"),
            "",
        )
        .expect("write md target");
        std::fs::write(root.join("execution_intent_route_trace_cases.txt"), "")
            .expect("write txt sibling");

        let out = execute(json!({
            "action": "find_name",
            "pattern": "*execution_intent*.md",
            "target_kind": "file",
            "root": root.to_string_lossy().to_string(),
            "max_depth": 1,
            "max_results": 10
        }))
        .expect("find_name succeeds with extension pattern");

        assert_eq!(out.get("count").and_then(Value::as_u64), Some(1));
        let results = out
            .get("results")
            .and_then(Value::as_array)
            .expect("results array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 1);
        assert!(results[0].ends_with("execution_intent_routing_repair_plan_20260509.md"));

        let _ = std::fs::remove_dir_all(root);
    }
}

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

fn path_name_matches_pattern(path: &Path, name_norm: &str, pattern_norm: &str) -> bool {
    path_extension_matches_pattern(path, pattern_norm)
        && name_matches_pattern(name_norm, pattern_norm)
}

fn name_patterns_from_args(obj: &serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
    let raw_patterns = string_values_from_args(
        obj,
        &[
            "pattern", "patterns", "name", "names", "keyword", "keywords", "query", "queries",
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
            "pattern", "patterns", "name", "names", "keyword", "keywords", "query", "queries",
        ],
    )
    .iter()
    .flat_map(|raw| expand_name_pattern(raw))
    .filter(|pattern| !pattern.is_empty())
    .collect::<Vec<_>>()
}

fn bool_arg(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(false)
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
        obj.get("root").and_then(|v| v.as_str()).unwrap_or("."),
    )?;
    let scan_limits = scan_limits_from_args(obj);

    let mut results = Vec::new();
    match action.as_str() {
        "find_name" => {
            let pattern_norms = name_patterns_from_args(obj)?;
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
                target_kind.as_str()
            };
            walk_collect_nodes(&search_root, scan_limits, &mut |p| {
                let name = p
                    .file_name()
                    .map(|s| normalize_locator_text(&s.to_string_lossy()))
                    .unwrap_or_default();
                if !pattern_norms
                    .iter()
                    .any(|pattern_norm| path_name_matches_pattern(p, &name, pattern_norm))
                {
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
            })?;
            Ok(json!({
                "action": "find_name",
                "root": to_rel(&root, &search_root),
                "patterns": pattern_norms,
                "count": results.len(),
                "results": results,
            }))
        }
        "find_ext" => {
            let ext = obj
                .get("ext")
                .or_else(|| obj.get("extension"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "ext is required".to_string())?
                .trim_start_matches('.')
                .to_ascii_lowercase();
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
                if got == ext && name_matches {
                    results.push(to_rel(&root, p));
                }
                results.len() >= max_results
            })?;
            Ok(json!({
                "action": "find_ext",
                "root": to_rel(&root, &search_root),
                "ext": ext,
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
            walk_collect(&search_root, scan_limits, &mut |p| {
                if let Ok(text) = std::fs::read_to_string(p) {
                    if text.contains(query) {
                        results.push(to_rel(&root, p));
                    }
                }
                results.len() >= max_results
            })?;
            Ok(json!({
                "action": "grep_text",
                "root": to_rel(&root, &search_root),
                "query": query,
                "count": results.len(),
                "results": results,
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
    walk_collect_inner(path, 0, limits, &mut scanned_files, f)
}

fn walk_collect_inner(
    path: &Path,
    depth: usize,
    limits: ScanLimits,
    scanned_files: &mut usize,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    if path.is_file() {
        if *scanned_files >= limits.max_files {
            return Ok(());
        }
        *scanned_files += 1;
        let _ = f(path);
        return Ok(());
    }
    if depth > limits.max_depth {
        return Ok(());
    }
    let iter = std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))?;
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let p = entry.path();
        if p.is_dir() {
            if depth < limits.max_depth {
                walk_collect_inner(&p, depth + 1, limits, scanned_files, f)?;
            }
        } else {
            if *scanned_files >= limits.max_files {
                return Ok(());
            }
            *scanned_files += 1;
            if f(&p) {
                return Ok(());
            }
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
    walk_collect_nodes_inner(path, 0, limits, &mut scanned_files, f)
}

fn walk_collect_nodes_inner(
    path: &Path,
    depth: usize,
    limits: ScanLimits,
    scanned_files: &mut usize,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    if path.is_file() {
        if *scanned_files >= limits.max_files {
            return Ok(());
        }
        *scanned_files += 1;
        let _ = f(path);
        return Ok(());
    }
    if path.is_dir() && f(path) {
        return Ok(());
    }
    if depth > limits.max_depth {
        return Ok(());
    }
    let iter = std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))?;
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let p = entry.path();
        if p.is_dir() {
            if depth < limits.max_depth {
                walk_collect_nodes_inner(&p, depth + 1, limits, scanned_files, f)?;
            }
        } else {
            if *scanned_files >= limits.max_files {
                return Ok(());
            }
            *scanned_files += 1;
            if f(&p) {
                return Ok(());
            }
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

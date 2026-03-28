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
    let max_depth = parse_env_usize("RUSTCLAW_LOCATOR_SCAN_MAX_DEPTH")
        .unwrap_or(6)
        .max(1);
    let max_files = parse_env_usize("RUSTCLAW_LOCATOR_SCAN_MAX_FILES")
        .unwrap_or(6000)
        .max(1);
    ScanLimits {
        max_depth,
        max_files,
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
    let scan_limits = scan_limits_from_env();

    let mut results = Vec::new();
    match action.as_str() {
        "find_name" => {
            let pattern = obj
                .get("pattern")
                .or_else(|| obj.get("name"))
                .or_else(|| obj.get("keyword"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "pattern is required".to_string())?
                .to_ascii_lowercase();
            let target_kind = obj
                .get("target_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("any")
                .to_ascii_lowercase();
            walk_collect_nodes(&search_root, scan_limits, &mut |p| {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if !name.contains(&pattern) {
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
            walk_collect(&search_root, scan_limits, &mut |p| {
                let got = p
                    .extension()
                    .map(|s| s.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if got == ext {
                    results.push(to_rel(&root, p));
                }
                results.len() >= max_results
            })?;
            Ok(json!({
                "action": "find_ext",
                "root": to_rel(&root, &search_root),
                "ext": ext,
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

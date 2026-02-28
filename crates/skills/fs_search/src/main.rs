use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    error_text: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
                Ok(text) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn execute(args: Value) -> Result<String, String> {
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
            walk_collect(&search_root, &mut |p| {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if name.contains(&pattern) {
                    results.push(to_rel(&root, p));
                }
                results.len() >= max_results
            })?;
        }
        "find_ext" => {
            let ext = obj
                .get("ext")
                .or_else(|| obj.get("extension"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "ext is required".to_string())?
                .trim_start_matches('.')
                .to_ascii_lowercase();
            walk_collect(&search_root, &mut |p| {
                let got = p
                    .extension()
                    .map(|s| s.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if got == ext {
                    results.push(to_rel(&root, p));
                }
                results.len() >= max_results
            })?;
        }
        "grep_text" => {
            let query = obj
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "query is required".to_string())?;
            walk_collect(&search_root, &mut |p| {
                if let Ok(text) = std::fs::read_to_string(p) {
                    if text.contains(query) {
                        results.push(to_rel(&root, p));
                    }
                }
                results.len() >= max_results
            })?;
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

            walk_collect(&search_root, &mut |p| {
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
            let mut out = Vec::new();
            out.push(format!("total_images={}", files.len()));
            out.push("directories_by_count:".to_string());
            for (dir, cnt) in dir_items {
                out.push(format!("{cnt}\t{dir}"));
            }
            return Ok(out.join("\n"));
        }
        _ => return Err("unsupported action; use find_name|find_ext|grep_text|find_images".to_string()),
    }
    Ok(results.join("\n"))
}

fn walk_collect(path: &Path, f: &mut dyn FnMut(&Path) -> bool) -> Result<(), String> {
    if path.is_file() {
        let _ = f(path);
        return Ok(());
    }
    let iter = std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))?;
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let p = entry.path();
        if p.is_dir() {
            walk_collect(&p, f)?;
        } else if f(&p) {
            return Ok(());
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
}

fn resolve_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let base = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        workspace_root.join(input)
    };
    if base.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("path with '..' is not allowed".to_string());
    }
    if !base.starts_with(workspace_root) {
        return Err("path is outside workspace".to_string());
    }
    Ok(base)
}

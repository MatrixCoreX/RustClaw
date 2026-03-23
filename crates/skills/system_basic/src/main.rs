use std::cmp::Ordering;
use std::ffi::OsStr;
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

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
            Ok(req) => handle(req),
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

fn handle(req: Req) -> Resp {
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let result = execute_action(&workspace_root, req.args);
    match result {
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
    }
}

fn execute_action(workspace_root: &Path, args: Value) -> Result<String, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("info")
        .to_ascii_lowercase();

    match action.as_str() {
        "info" => system_info(workspace_root),
        "inventory_dir" => inventory_dir(workspace_root, obj),
        "count_inventory" => count_inventory(workspace_root, obj),
        "workspace_glance" => workspace_glance(workspace_root, obj),
        "tree_summary" => tree_summary(workspace_root, obj),
        "dir_compare" => dir_compare(workspace_root, obj),
        "extract_field" => extract_field(workspace_root, obj),
        "extract_fields" => extract_fields(workspace_root, obj),
        "structured_keys" => structured_keys(workspace_root, obj),
        "find_path" => find_path(workspace_root, obj),
        "read_range" => read_range(workspace_root, obj),
        "compare_paths" => compare_paths(workspace_root, obj),
        "path_batch_facts" => path_batch_facts(workspace_root, obj),
        "diagnose_runtime" => diagnose_runtime(workspace_root, obj),
        other => Err(format!(
            "unknown action: {other}; allowed: info|inventory_dir|count_inventory|workspace_glance|tree_summary|dir_compare|extract_field|extract_fields|structured_keys|find_path|read_range|compare_paths|path_batch_facts|diagnose_runtime"
        )),
    }
}

fn system_info(workspace_root: &Path) -> Result<String, String> {
    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "unknown".to_string())
        .trim()
        .to_string();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let uptime = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(|v| v.to_string()))
        .unwrap_or_else(|| "-".to_string());
    let mem = memory_rss_bytes().unwrap_or(0);
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "-".to_string());
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "-".to_string());
    let pid = std::process::id();

    Ok(json!({
        "hostname": hostname,
        "now_ts": now,
        "uptime_seconds": uptime,
        "process_rss_bytes": mem,
        "pid": pid,
        "cwd": cwd,
        "workspace_root": workspace_root.display().to_string(),
        "current_exe": exe,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    })
    .to_string())
}

fn memory_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

fn inventory_dir(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let path = obj.get("path").and_then(Value::as_str).unwrap_or(".");
    let real = resolve_path(workspace_root, path)?;
    let include_hidden = bool_arg(obj, "include_hidden", false);
    let files_only = bool_arg(obj, "files_only", false);
    let dirs_only = bool_arg(obj, "dirs_only", false);
    let names_only = bool_arg(obj, "names_only", false);
    let max_entries = u64_arg(obj, "max_entries", 200).clamp(1, 1000) as usize;
    let sort_by = obj
        .get("sort_by")
        .and_then(Value::as_str)
        .unwrap_or("name")
        .to_ascii_lowercase();
    let ext_filters = ext_filters(obj);

    let mut entries = Vec::new();
    let iter = std::fs::read_dir(&real).map_err(|err| format!("read_dir failed: {err}"))?;
    for item in iter {
        let item = item.map_err(|err| format!("dir entry failed: {err}"))?;
        let entry_path = item.path();
        let file_name = item.file_name().to_string_lossy().to_string();
        let is_hidden = file_name.starts_with('.');
        if !include_hidden && is_hidden {
            continue;
        }
        let meta = item
            .metadata()
            .map_err(|err| format!("metadata failed for {}: {err}", entry_path.display()))?;
        let kind = if meta.is_dir() {
            "dir"
        } else if meta.is_file() {
            "file"
        } else {
            "other"
        };
        if files_only && kind != "file" {
            continue;
        }
        if dirs_only && kind != "dir" {
            continue;
        }
        if !ext_filters.is_empty() && kind == "file" {
            let ext = entry_path
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            if !ext_filters.iter().any(|f| f == &ext) {
                continue;
            }
        }
        let rel = to_rel(workspace_root, &entry_path);
        let modified_ts = meta
            .modified()
            .ok()
            .and_then(system_time_to_ts)
            .unwrap_or(0);
        entries.push(json!({
            "name": file_name,
            "path": rel,
            "kind": kind,
            "hidden": is_hidden,
            "size_bytes": if meta.is_file() { meta.len() } else { 0 },
            "modified_ts": modified_ts,
        }));
    }

    sort_inventory_entries(&mut entries, &sort_by);
    if entries.len() > max_entries {
        entries.truncate(max_entries);
    }

    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    let mut hidden_count = 0usize;
    let mut names = Vec::new();
    for entry in &entries {
        if entry.get("kind").and_then(Value::as_str) == Some("file") {
            file_count += 1;
        } else if entry.get("kind").and_then(Value::as_str) == Some("dir") {
            dir_count += 1;
        }
        if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
            hidden_count += 1;
        }
        if let Some(name) = entry.get("name").and_then(Value::as_str) {
            names.push(name.to_string());
        }
    }

    Ok(json!({
        "action": "inventory_dir",
        "path": path,
        "resolved_path": real.display().to_string(),
        "include_hidden": include_hidden,
        "files_only": files_only,
        "dirs_only": dirs_only,
        "names_only": names_only,
        "sort_by": sort_by,
        "ext_filter": ext_filters,
        "counts": {
            "files": file_count,
            "dirs": dir_count,
            "total": entries.len(),
            "hidden": hidden_count,
        },
        "has_hidden": hidden_count > 0,
        "names": names,
        "entries": if names_only { Value::Array(Vec::new()) } else { Value::Array(entries) },
    })
    .to_string())
}

fn count_inventory(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let path = obj.get("path").and_then(Value::as_str).unwrap_or(".");
    let real = resolve_path(workspace_root, path)?;
    let include_hidden = bool_arg(obj, "include_hidden", false);
    let recursive = bool_arg(obj, "recursive", false);
    let count_files = bool_arg(obj, "count_files", true);
    let count_dirs = bool_arg(obj, "count_dirs", true);
    let ext_filters = ext_filters(obj);
    let kind_filter = obj
        .get("kind_filter")
        .and_then(Value::as_str)
        .unwrap_or("any")
        .to_ascii_lowercase();

    let mut counts = CountSummary::default();
    walk_inventory(&real, recursive, &mut |entry_path, meta, depth| {
        if depth == 0 {
            return Ok(false);
        }
        let Some(name) = entry_path.file_name().and_then(OsStr::to_str) else {
            return Ok(false);
        };
        let hidden = name.starts_with('.');
        if !include_hidden && hidden {
            return Ok(meta.is_dir() && recursive);
        }
        let is_file = meta.is_file();
        let is_dir = meta.is_dir();
        if is_file {
            let ext = entry_path
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            if !ext_filters.is_empty() && !ext_filters.iter().any(|f| f == &ext) {
                return Ok(false);
            }
            if matches!(kind_filter.as_str(), "any" | "file") && count_files {
                counts.files += 1;
                counts.total += 1;
                counts.total_size_bytes += meta.len();
                if hidden {
                    counts.hidden += 1;
                }
                if !ext.is_empty() {
                    *counts.by_extension.entry(ext).or_insert(0) += 1;
                }
            }
        } else if is_dir {
            if matches!(kind_filter.as_str(), "any" | "dir") && count_dirs {
                counts.dirs += 1;
                counts.total += 1;
                if hidden {
                    counts.hidden += 1;
                }
            }
            return Ok(recursive);
        }
        Ok(false)
    })?;

    Ok(json!({
        "action": "count_inventory",
        "path": path,
        "resolved_path": real.display().to_string(),
        "recursive": recursive,
        "include_hidden": include_hidden,
        "kind_filter": kind_filter,
        "ext_filter": ext_filters,
        "counts": {
            "total": counts.total,
            "files": counts.files,
            "dirs": counts.dirs,
            "hidden": counts.hidden,
            "total_size_bytes": counts.total_size_bytes,
            "by_extension": counts.by_extension,
        }
    })
    .to_string())
}

fn workspace_glance(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let path = obj.get("path").and_then(Value::as_str).unwrap_or(".");
    let real = resolve_path(workspace_root, path)?;
    let include_hidden = bool_arg(obj, "include_hidden", false);
    let max_entries = u64_arg(obj, "max_entries", 20).clamp(1, 100) as usize;
    let mut entries = Vec::new();
    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    let mut hidden_count = 0usize;
    let mut ext_counts = std::collections::BTreeMap::<String, usize>::new();

    let iter = std::fs::read_dir(&real).map_err(|err| format!("read_dir failed: {err}"))?;
    for item in iter {
        let item = item.map_err(|err| format!("dir entry failed: {err}"))?;
        let entry_path = item.path();
        let file_name = item.file_name().to_string_lossy().to_string();
        let is_hidden = file_name.starts_with('.');
        if !include_hidden && is_hidden {
            continue;
        }
        let meta = item
            .metadata()
            .map_err(|err| format!("metadata failed for {}: {err}", entry_path.display()))?;
        let kind = path_kind(&meta);
        match kind {
            "file" => {
                file_count += 1;
                let ext = entry_path
                    .extension()
                    .and_then(OsStr::to_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !ext.is_empty() {
                    *ext_counts.entry(ext).or_insert(0) += 1;
                }
            }
            "dir" => dir_count += 1,
            _ => {}
        }
        if is_hidden {
            hidden_count += 1;
        }
        let modified_ts = meta.modified().ok().and_then(system_time_to_ts).unwrap_or(0);
        entries.push(json!({
            "name": file_name,
            "path": to_rel(workspace_root, &entry_path),
            "kind": kind,
            "size_bytes": if meta.is_file() { meta.len() } else { 0 },
            "modified_ts": modified_ts,
            "hidden": is_hidden,
        }));
    }

    sort_inventory_entries(&mut entries, "name");
    let omitted_entries = entries.len().saturating_sub(max_entries);
    if entries.len() > max_entries {
        entries.truncate(max_entries);
    }

    Ok(json!({
        "action": "workspace_glance",
        "path": path,
        "resolved_path": real.display().to_string(),
        "include_hidden": include_hidden,
        "counts": {
            "files": file_count,
            "dirs": dir_count,
            "total": file_count + dir_count,
            "hidden": hidden_count,
        },
        "top_file_extensions": top_extension_pairs(&ext_counts, 10),
        "omitted_entries": omitted_entries,
        "entries": entries,
    })
    .to_string())
}

fn tree_summary(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let path = obj.get("path").and_then(Value::as_str).unwrap_or(".");
    let real = resolve_path(workspace_root, path)?;
    let include_hidden = bool_arg(obj, "include_hidden", false);
    let max_depth = u64_arg(obj, "max_depth", 2).clamp(1, 6) as usize;
    let max_children_per_dir = u64_arg(obj, "max_children_per_dir", 12).clamp(1, 50) as usize;
    let max_nodes = u64_arg(obj, "max_nodes", 200).clamp(20, 1000) as usize;

    let mut state = TreeSummaryState {
        remaining_nodes: max_nodes,
        truncated_nodes: 0,
    };
    let tree = build_tree_summary_node(
        workspace_root,
        &real,
        include_hidden,
        max_depth,
        max_children_per_dir,
        0,
        &mut state,
    )?;

    Ok(json!({
        "action": "tree_summary",
        "path": path,
        "resolved_path": real.display().to_string(),
        "include_hidden": include_hidden,
        "max_depth": max_depth,
        "max_children_per_dir": max_children_per_dir,
        "max_nodes": max_nodes,
        "truncated_nodes": state.truncated_nodes,
        "tree": tree,
    })
    .to_string())
}

fn dir_compare(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let left = required_str(obj, "left_path")?;
    let right = required_str(obj, "right_path")?;
    let left_real = resolve_path(workspace_root, left)?;
    let right_real = resolve_path(workspace_root, right)?;
    let include_hidden = bool_arg(obj, "include_hidden", false);
    let recursive = bool_arg(obj, "recursive", false);
    let max_diffs = u64_arg(obj, "max_diffs", 100).clamp(1, 500) as usize;

    let left_meta = std::fs::metadata(&left_real).map_err(|err| format!("left metadata failed: {err}"))?;
    let right_meta = std::fs::metadata(&right_real).map_err(|err| format!("right metadata failed: {err}"))?;
    if !left_meta.is_dir() || !right_meta.is_dir() {
        return Err("dir_compare requires both paths to be directories".to_string());
    }

    let left_entries = collect_dir_signatures(&left_real, include_hidden, recursive, max_diffs * 20)?;
    let right_entries = collect_dir_signatures(&right_real, include_hidden, recursive, max_diffs * 20)?;

    let left_keys = left_entries.keys().cloned().collect::<std::collections::BTreeSet<_>>();
    let right_keys = right_entries.keys().cloned().collect::<std::collections::BTreeSet<_>>();

    let common = left_keys.intersection(&right_keys).cloned().collect::<Vec<_>>();
    let left_only = left_keys
        .difference(&right_keys)
        .take(max_diffs)
        .cloned()
        .collect::<Vec<_>>();
    let right_only = right_keys
        .difference(&left_keys)
        .take(max_diffs)
        .cloned()
        .collect::<Vec<_>>();

    let mut kind_mismatches = Vec::new();
    for key in common.iter().take(max_diffs) {
        let left_kind = left_entries.get(key).map(String::as_str).unwrap_or("other");
        let right_kind = right_entries.get(key).map(String::as_str).unwrap_or("other");
        if left_kind != right_kind {
            kind_mismatches.push(json!({
                "path": key,
                "left_kind": left_kind,
                "right_kind": right_kind,
            }));
        }
    }

    Ok(json!({
        "action": "dir_compare",
        "left_path": left,
        "right_path": right,
        "left_resolved_path": left_real.display().to_string(),
        "right_resolved_path": right_real.display().to_string(),
        "recursive": recursive,
        "include_hidden": include_hidden,
        "counts": {
            "left_entries": left_entries.len(),
            "right_entries": right_entries.len(),
            "common": common.len(),
            "left_only": left_keys.len().saturating_sub(common.len()),
            "right_only": right_keys.len().saturating_sub(common.len()),
            "kind_mismatches": kind_mismatches.len(),
        },
        "left_only": left_only,
        "right_only": right_only,
        "kind_mismatches": kind_mismatches,
    })
    .to_string())
}

fn extract_field(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let path = required_str(obj, "path")?;
    let field_path = required_str(obj, "field_path")?;
    let real = resolve_path(workspace_root, path)?;
    let (format, root_value) = parse_structured_root(&real, obj.get("format").and_then(Value::as_str))?;

    let found = lookup_field_value(&root_value, field_path);
    let (exists, value, value_type, value_text) = match found {
        Some(v) => (
            true,
            v.clone(),
            json_value_type(v).to_string(),
            json_value_to_text(v),
        ),
        None => (false, Value::Null, "null".to_string(), String::new()),
    };

    Ok(json!({
        "action": "extract_field",
        "path": path,
        "resolved_path": real.display().to_string(),
        "format": format,
        "field_path": field_path,
        "exists": exists,
        "value_type": value_type,
        "value_text": value_text,
        "value": value,
    })
    .to_string())
}

fn extract_fields(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let path = required_str(obj, "path")?;
    let real = resolve_path(workspace_root, path)?;
    let field_paths = string_list_arg(obj, "field_paths");
    if field_paths.is_empty() {
        return Err("field_paths is required".to_string());
    }
    let (format, root_value) = parse_structured_root(&real, obj.get("format").and_then(Value::as_str))?;

    let mut results = Vec::new();
    for field_path in field_paths {
        let found = lookup_field_value(&root_value, &field_path);
        let (exists, value, value_type, value_text) = match found {
            Some(v) => (
                true,
                v.clone(),
                json_value_type(v).to_string(),
                json_value_to_text(v),
            ),
            None => (false, Value::Null, "null".to_string(), String::new()),
        };
        results.push(json!({
            "field_path": field_path,
            "exists": exists,
            "value_type": value_type,
            "value_text": value_text,
            "value": value,
        }));
    }

    Ok(json!({
        "action": "extract_fields",
        "path": path,
        "resolved_path": real.display().to_string(),
        "format": format,
        "count": results.len(),
        "results": results,
    })
    .to_string())
}

fn structured_keys(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let path = required_str(obj, "path")?;
    let real = resolve_path(workspace_root, path)?;
    let field_path = obj.get("field_path").and_then(Value::as_str).unwrap_or("");
    let max_keys = u64_arg(obj, "max_keys", 200).clamp(1, 1000) as usize;
    let (format, root_value) = parse_structured_root(&real, obj.get("format").and_then(Value::as_str))?;

    let target = if field_path.is_empty() {
        Some(&root_value)
    } else {
        lookup_field_value(&root_value, field_path)
    }
    .ok_or_else(|| "field_path not found".to_string())?;

    match target {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let omitted = keys.len().saturating_sub(max_keys);
            if keys.len() > max_keys {
                keys.truncate(max_keys);
            }
            Ok(json!({
                "action": "structured_keys",
                "path": path,
                "resolved_path": real.display().to_string(),
                "format": format,
                "field_path": field_path,
                "container_type": "object",
                "count": map.len(),
                "omitted": omitted,
                "keys": keys,
            })
            .to_string())
        }
        Value::Array(arr) => {
            let preview = arr
                .iter()
                .take(max_keys.min(20))
                .enumerate()
                .map(|(idx, item)| {
                    json!({
                        "index": idx,
                        "value_type": json_value_type(item),
                    })
                })
                .collect::<Vec<_>>();
            Ok(json!({
                "action": "structured_keys",
                "path": path,
                "resolved_path": real.display().to_string(),
                "format": format,
                "field_path": field_path,
                "container_type": "array",
                "count": arr.len(),
                "indices_preview": preview,
            })
            .to_string())
        }
        other => Ok(json!({
            "action": "structured_keys",
            "path": path,
            "resolved_path": real.display().to_string(),
            "format": format,
            "field_path": field_path,
            "container_type": json_value_type(other),
            "count": 0,
            "keys": [],
        })
        .to_string()),
    }
}

fn find_path(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let root = obj.get("root").and_then(Value::as_str).unwrap_or(".");
    let real_root = resolve_path(workspace_root, root)?;
    let needle = obj
        .get("name")
        .or_else(|| obj.get("pattern"))
        .and_then(Value::as_str)
        .ok_or_else(|| "name or pattern is required".to_string())?;
    let needle_norm = needle.to_ascii_lowercase();
    let match_mode = obj
        .get("match_mode")
        .and_then(Value::as_str)
        .unwrap_or("contains")
        .to_ascii_lowercase();
    let target_kind = obj
        .get("target_kind")
        .and_then(Value::as_str)
        .unwrap_or("any")
        .to_ascii_lowercase();
    let max_results = u64_arg(obj, "max_results", 20).clamp(1, 200) as usize;

    let mut matches = Vec::new();
    walk_collect(&real_root, &mut |p| {
        let Some(name) = p.file_name().and_then(OsStr::to_str) else {
            return false;
        };
        let meta = match std::fs::metadata(p) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let kind = if meta.is_dir() {
            "dir"
        } else if meta.is_file() {
            "file"
        } else {
            "other"
        };
        if target_kind != "any" && target_kind != kind {
            return false;
        }
        let name_norm = name.to_ascii_lowercase();
        let hit = match match_mode.as_str() {
            "exact" => name_norm == needle_norm,
            "starts_with" => name_norm.starts_with(&needle_norm),
            "ends_with" => name_norm.ends_with(&needle_norm),
            _ => name_norm.contains(&needle_norm),
        };
        if hit {
            matches.push(json!({
                "name": name,
                "path": to_rel(workspace_root, p),
                "kind": kind,
            }));
        }
        matches.len() >= max_results
    })?;

    Ok(json!({
        "action": "find_path",
        "root": root,
        "resolved_root": real_root.display().to_string(),
        "query": needle,
        "match_mode": match_mode,
        "target_kind": target_kind,
        "count": matches.len(),
        "matches": matches,
    })
    .to_string())
}

fn read_range(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let path = required_str(obj, "path")?;
    let real = resolve_path(workspace_root, path)?;
    let text = std::fs::read_to_string(&real).map_err(|err| format!("read file failed: {err}"))?;
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();
    let mode = obj
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("head")
        .to_ascii_lowercase();
    let n = u64_arg(obj, "n", 20).clamp(1, 500) as usize;
    let start = obj.get("start_line").and_then(Value::as_u64).map(|v| v as usize);
    let end = obj.get("end_line").and_then(Value::as_u64).map(|v| v as usize);

    let (from, to) = if total_lines == 0 {
        (0, 0)
    } else {
        match mode.as_str() {
            "tail" => {
                let from = total_lines.saturating_sub(n) + 1;
                (from, total_lines)
            }
            "range" => {
                let from = start.unwrap_or(1).max(1);
                let to = end.unwrap_or(from.saturating_add(n).saturating_sub(1)).max(from);
                (from, to.min(total_lines))
            }
            _ => (1, n.min(total_lines)),
        }
    };

    let mut excerpt_lines = Vec::new();
    if total_lines > 0 {
        for idx in from..=to {
            if let Some(line) = lines.get(idx - 1) {
                excerpt_lines.push(format!("{idx}|{line}"));
            }
        }
    }

    Ok(json!({
        "action": "read_range",
        "path": path,
        "resolved_path": real.display().to_string(),
        "mode": mode,
        "requested_n": n,
        "start_line": from,
        "end_line": to,
        "total_lines": total_lines,
        "excerpt": excerpt_lines.join("\n"),
    })
    .to_string())
}

fn compare_paths(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let left = required_str(obj, "left_path")?;
    let right = required_str(obj, "right_path")?;
    let left_real = resolve_path(workspace_root, left)?;
    let right_real = resolve_path(workspace_root, right)?;
    let left_meta = std::fs::metadata(&left_real).map_err(|err| format!("left metadata failed: {err}"))?;
    let right_meta =
        std::fs::metadata(&right_real).map_err(|err| format!("right metadata failed: {err}"))?;

    let left_kind = path_kind(&left_meta);
    let right_kind = path_kind(&right_meta);
    let left_mtime = left_meta.modified().ok().and_then(system_time_to_ts);
    let right_mtime = right_meta.modified().ok().and_then(system_time_to_ts);
    let left_name = left_real.file_name().and_then(OsStr::to_str).unwrap_or("");
    let right_name = right_real.file_name().and_then(OsStr::to_str).unwrap_or("");
    let same_content = if left_meta.is_file() && right_meta.is_file() {
        same_file_content(&left_real, &right_real).ok()
    } else {
        None
    };

    Ok(json!({
        "action": "compare_paths",
        "left": build_path_fact(workspace_root, &left_real, &left_meta),
        "right": build_path_fact(workspace_root, &right_real, &right_meta),
        "comparison": {
            "same_kind": left_kind == right_kind,
            "same_name": left_name == right_name,
            "same_size": left_meta.len() == right_meta.len(),
            "size_delta_bytes": (left_meta.len() as i128 - right_meta.len() as i128),
            "left_newer": match (left_mtime, right_mtime) {
                (Some(l), Some(r)) => Some(l > r),
                _ => None,
            },
            "same_content": same_content,
        }
    })
    .to_string())
}

fn path_batch_facts(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let paths = string_list_arg(obj, "paths");
    if paths.is_empty() {
        return Err("paths is required".to_string());
    }
    let include_missing = bool_arg(obj, "include_missing", true);
    let mut facts = Vec::new();

    for path in paths {
        let real = resolve_path(workspace_root, &path)?;
        match std::fs::metadata(&real) {
            Ok(meta) => facts.push(json!({
                "path": path,
                "exists": true,
                "fact": build_path_fact(workspace_root, &real, &meta),
            })),
            Err(err) if include_missing && err.kind() == io::ErrorKind::NotFound => facts.push(json!({
                "path": path,
                "exists": false,
                "error": "not found",
            })),
            Err(err) => return Err(format!("metadata failed for {}: {err}", real.display())),
        }
    }

    Ok(json!({
        "action": "path_batch_facts",
        "count": facts.len(),
        "include_missing": include_missing,
        "facts": facts,
    })
    .to_string())
}

fn diagnose_runtime(workspace_root: &Path, obj: &Map<String, Value>) -> Result<String, String> {
    let info = serde_json::from_str::<Value>(&system_info(workspace_root)?)
        .map_err(|err| format!("system info encode failed: {err}"))?;
    let include_process = bool_arg(obj, "include_process", false);
    let include_ports = bool_arg(obj, "include_ports", false);
    let include_env_summary = bool_arg(obj, "include_env_summary", false);

    let loadavg = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.lines().next().map(str::trim).map(str::to_string));
    let meminfo = summarize_meminfo();
    let disk = summarize_df(workspace_root);
    let process_snapshot = if include_process {
        run_command_lines("ps", &["-eo", "pid,comm,%cpu,rss", "--sort=-rss"], 8)
    } else {
        None
    };
    let ports_snapshot = if include_ports {
        run_command_lines("ss", &["-ltn"], 10).or_else(|| run_command_lines("netstat", &["-ltn"], 10))
    } else {
        None
    };
    let env_summary = if include_env_summary {
        Some(json!({
            "rust_log": std::env::var("RUST_LOG").ok(),
            "workspace_root_env": std::env::var("WORKSPACE_ROOT").ok(),
            "path_entries": std::env::var("PATH").ok().map(|v| v.split(':').take(8).map(str::to_string).collect::<Vec<_>>()),
        }))
    } else {
        None
    };

    Ok(json!({
        "action": "diagnose_runtime",
        "info": info,
        "loadavg": loadavg,
        "meminfo": meminfo,
        "disk": disk,
        "process_snapshot": process_snapshot,
        "ports_snapshot": ports_snapshot,
        "env_summary": env_summary,
    })
    .to_string())
}

#[derive(Default)]
struct CountSummary {
    total: usize,
    files: usize,
    dirs: usize,
    hidden: usize,
    total_size_bytes: u64,
    by_extension: std::collections::BTreeMap<String, usize>,
}

struct TreeSummaryState {
    remaining_nodes: usize,
    truncated_nodes: usize,
}

fn summarize_meminfo() -> Value {
    let text = match std::fs::read_to_string("/proc/meminfo") {
        Ok(v) => v,
        Err(_) => return Value::Null,
    };
    let mut picked = Map::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if matches!(key, "MemTotal" | "MemFree" | "MemAvailable" | "SwapTotal" | "SwapFree") {
            picked.insert(key.to_string(), Value::String(value.trim().to_string()));
        }
    }
    Value::Object(picked)
}

fn summarize_df(workspace_root: &Path) -> Value {
    let path = workspace_root.display().to_string();
    let Some(lines) = run_command_lines("df", &["-kP", &path], 2) else {
        return Value::Null;
    };
    let Some(last) = lines.last() else {
        return Value::Null;
    };
    let cols: Vec<&str> = last.split_whitespace().collect();
    if cols.len() < 6 {
        return Value::Null;
    }
    json!({
        "filesystem": cols[0],
        "size_kb": cols[1],
        "used_kb": cols[2],
        "avail_kb": cols[3],
        "use_percent": cols[4],
        "mountpoint": cols[5],
    })
}

fn run_command_lines(cmd: &str, args: &[&str], max_lines: usize) -> Option<Vec<String>> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let lines = text
        .lines()
        .take(max_lines)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

fn walk_inventory(
    path: &Path,
    recursive: bool,
    f: &mut dyn FnMut(&Path, &std::fs::Metadata, usize) -> Result<bool, String>,
) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|err| format!("metadata failed: {err}"))?;
    if meta.is_file() {
        let _ = f(path, &meta, 0)?;
        return Ok(());
    }
    walk_inventory_inner(path, recursive, 0, f)
}

fn walk_inventory_inner(
    dir: &Path,
    recursive: bool,
    depth: usize,
    f: &mut dyn FnMut(&Path, &std::fs::Metadata, usize) -> Result<bool, String>,
) -> Result<(), String> {
    let iter = std::fs::read_dir(dir).map_err(|err| format!("read_dir failed: {err}"))?;
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let p = entry.path();
        let meta = entry
            .metadata()
            .map_err(|err| format!("metadata failed for {}: {err}", p.display()))?;
        let descend = f(&p, &meta, depth + 1)?;
        if recursive && meta.is_dir() && descend {
            walk_inventory_inner(&p, recursive, depth + 1, f)?;
        }
    }
    Ok(())
}

fn sort_inventory_entries(entries: &mut [Value], sort_by: &str) {
    entries.sort_by(|a, b| match sort_by {
        "mtime_desc" => cmp_u64_field(b, a, "modified_ts").then_with(|| cmp_name(a, b)),
        "mtime_asc" => cmp_u64_field(a, b, "modified_ts").then_with(|| cmp_name(a, b)),
        "size_desc" => cmp_u64_field(b, a, "size_bytes").then_with(|| cmp_name(a, b)),
        "size_asc" => cmp_u64_field(a, b, "size_bytes").then_with(|| cmp_name(a, b)),
        _ => cmp_name(a, b),
    });
}

fn cmp_u64_field(a: &Value, b: &Value, key: &str) -> Ordering {
    a.get(key)
        .and_then(Value::as_u64)
        .cmp(&b.get(key).and_then(Value::as_u64))
}

fn cmp_name(a: &Value, b: &Value) -> Ordering {
    a.get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .cmp(b.get("name").and_then(Value::as_str).unwrap_or(""))
}

fn ext_filters(obj: &Map<String, Value>) -> Vec<String> {
    if let Some(s) = obj.get("ext_filter").and_then(Value::as_str) {
        return vec![s.trim_start_matches('.').to_ascii_lowercase()];
    }
    obj.get("ext_filter")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|v| v.trim_start_matches('.').to_ascii_lowercase())
                .filter(|v| !v.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn bool_arg(obj: &Map<String, Value>, key: &str, default: bool) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn u64_arg(obj: &Map<String, Value>, key: &str, default: u64) -> u64 {
    obj.get(key).and_then(Value::as_u64).unwrap_or(default)
}

fn required_str<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    obj.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} is required"))
}

fn string_list_arg(obj: &Map<String, Value>, key: &str) -> Vec<String> {
    if let Some(s) = obj.get(key).and_then(Value::as_str) {
        let trimmed = s.trim();
        return if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![trimmed.to_string()]
        };
    }
    obj.get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn detect_format_from_path(path: &Path) -> String {
    match path.extension().and_then(OsStr::to_str).unwrap_or("").to_ascii_lowercase().as_str() {
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        _ => "json",
    }
    .to_string()
}

fn parse_structured_root(path: &Path, format_hint: Option<&str>) -> Result<(String, Value), String> {
    let format = format_hint
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| detect_format_from_path(path));
    let raw = std::fs::read_to_string(path).map_err(|err| format!("read file failed: {err}"))?;
    let root_value = match format.as_str() {
        "json" => serde_json::from_str::<Value>(&raw).map_err(|err| format!("json parse failed: {err}"))?,
        "toml" => {
            let value = raw.parse::<toml::Value>().map_err(|err| format!("toml parse failed: {err}"))?;
            serde_json::to_value(value).map_err(|err| format!("toml convert failed: {err}"))?
        }
        "yaml" | "yml" => {
            serde_yaml::from_str::<Value>(&raw).map_err(|err| format!("yaml parse failed: {err}"))?
        }
        other => return Err(format!("unsupported format: {other}; use json|toml|yaml")),
    };
    Ok((format, root_value))
}

fn collect_dir_signatures(
    root: &Path,
    include_hidden: bool,
    recursive: bool,
    max_entries: usize,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut out = std::collections::BTreeMap::new();
    walk_inventory(root, recursive, &mut |entry_path, meta, depth| {
        if depth == 0 {
            return Ok(false);
        }
        let rel = entry_path
            .strip_prefix(root)
            .unwrap_or(entry_path)
            .to_string_lossy()
            .to_string();
        let name = entry_path.file_name().and_then(OsStr::to_str).unwrap_or("");
        if !include_hidden && name.starts_with('.') {
            return Ok(meta.is_dir() && recursive);
        }
        if out.len() < max_entries {
            out.insert(rel, path_kind(meta).to_string());
        }
        Ok(meta.is_dir() && recursive)
    })?;
    Ok(out)
}

fn lookup_field_value<'a>(value: &'a Value, field_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for seg in field_path.split('.') {
        if seg.is_empty() {
            return None;
        }
        if let Ok(idx) = seg.parse::<usize>() {
            current = current.as_array()?.get(idx)?;
        } else {
            current = current.get(seg)?;
        }
    }
    Some(current)
}

fn json_value_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn json_value_to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(v).unwrap_or_default(),
    }
}

fn path_kind(meta: &std::fs::Metadata) -> &'static str {
    if meta.is_dir() {
        "dir"
    } else if meta.is_file() {
        "file"
    } else {
        "other"
    }
}

fn build_path_fact(workspace_root: &Path, path: &Path, meta: &std::fs::Metadata) -> Value {
    json!({
        "path": to_rel(workspace_root, path),
        "resolved_path": path.display().to_string(),
        "kind": path_kind(meta),
        "size_bytes": meta.len(),
        "modified_ts": meta.modified().ok().and_then(system_time_to_ts),
    })
}

fn top_extension_pairs(
    counts: &std::collections::BTreeMap<String, usize>,
    limit: usize,
) -> Vec<Value> {
    let mut pairs = counts
        .iter()
        .map(|(ext, count)| (ext.clone(), *count))
        .collect::<Vec<_>>();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pairs
        .into_iter()
        .take(limit)
        .map(|(ext, count)| json!({ "ext": ext, "count": count }))
        .collect()
}

fn build_tree_summary_node(
    workspace_root: &Path,
    path: &Path,
    include_hidden: bool,
    max_depth: usize,
    max_children_per_dir: usize,
    depth: usize,
    state: &mut TreeSummaryState,
) -> Result<Value, String> {
    if state.remaining_nodes == 0 {
        state.truncated_nodes += 1;
        return Ok(json!({
            "path": to_rel(workspace_root, path),
            "truncated": true,
        }));
    }
    state.remaining_nodes -= 1;

    let meta = std::fs::metadata(path).map_err(|err| format!("metadata failed for {}: {err}", path.display()))?;
    let mut node = build_path_fact(workspace_root, path, &meta);
    if !meta.is_dir() {
        return Ok(node);
    }

    let mut visible_entries: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|err| format!("read_dir failed: {err}"))?
        .filter_map(|entry| entry.ok().map(|v| v.path()))
        .filter(|p| {
            include_hidden
                || p.file_name()
                    .and_then(OsStr::to_str)
                    .map(|v| !v.starts_with('.'))
                    .unwrap_or(true)
        })
        .collect();
    visible_entries.sort_by(|a, b| {
        a.file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .cmp(b.file_name().and_then(OsStr::to_str).unwrap_or(""))
    });

    let child_count = visible_entries.len();
    let omitted_children = if depth >= max_depth {
        child_count
    } else {
        child_count.saturating_sub(max_children_per_dir)
    };
    let mut children = Vec::new();
    if depth < max_depth {
        for child_path in visible_entries.into_iter().take(max_children_per_dir) {
            children.push(build_tree_summary_node(
                workspace_root,
                &child_path,
                include_hidden,
                max_depth,
                max_children_per_dir,
                depth + 1,
                state,
            )?);
        }
    }

    if let Some(obj) = node.as_object_mut() {
        obj.insert("depth".to_string(), json!(depth));
        obj.insert("child_count".to_string(), json!(child_count));
        obj.insert("omitted_children".to_string(), json!(omitted_children));
        obj.insert("children".to_string(), Value::Array(children));
    }
    Ok(node)
}

fn same_file_content(left: &Path, right: &Path) -> Result<bool, String> {
    const MAX_COMPARE_BYTES: u64 = 4 * 1024 * 1024;
    let left_meta = std::fs::metadata(left).map_err(|err| format!("left metadata failed: {err}"))?;
    let right_meta = std::fs::metadata(right).map_err(|err| format!("right metadata failed: {err}"))?;
    if left_meta.len() != right_meta.len() {
        return Ok(false);
    }
    if left_meta.len() > MAX_COMPARE_BYTES {
        return Err(format!(
            "file too large to compare content directly: {} bytes exceeds {}",
            left_meta.len(),
            MAX_COMPARE_BYTES
        ));
    }
    let left_bytes = std::fs::read(left).map_err(|err| format!("left read failed: {err}"))?;
    let right_bytes = std::fs::read(right).map_err(|err| format!("right read failed: {err}"))?;
    Ok(left_bytes == right_bytes)
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
    let normalized = base;
    if !normalized.starts_with(workspace_root) {
        return Err("path is outside workspace".to_string());
    }
    Ok(normalized)
}

fn walk_collect(path: &Path, f: &mut dyn FnMut(&Path) -> bool) -> Result<(), String> {
    if path.is_file() {
        let _ = f(path);
        return Ok(());
    }
    if path.is_dir() && f(path) {
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

fn system_time_to_ts(st: SystemTime) -> Option<u64> {
    st.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

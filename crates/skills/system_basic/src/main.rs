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
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
}

#[derive(Debug, Clone)]
struct SkillError {
    kind: &'static str,
    message: String,
    extra: Option<Value>,
}

type SkillResult<T> = Result<T, SkillError>;

impl SkillError {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            extra: None,
        }
    }

    fn with_extra(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }

    fn invalid_input(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message)
    }

    fn invalid_data(message: impl Into<String>) -> Self {
        Self::new("invalid_data", message)
    }

    fn unsupported_action(message: impl Into<String>) -> Self {
        Self::new("unsupported_action", message)
    }

    fn path_denied(message: impl Into<String>) -> Self {
        Self::new("path_denied", message)
    }

    fn not_a_directory(message: impl Into<String>) -> Self {
        Self::new("not_a_directory", message)
    }

    fn is_directory(message: impl Into<String>) -> Self {
        Self::new("is_directory", message)
    }

    fn io(operation: &'static str, path: &Path, err: io::Error) -> Self {
        let kind = io_error_kind(&err);
        let path_text = path.display().to_string();
        Self::new(kind, format!("{operation} failed for {path_text}: {err}")).with_extra(json!({
            "operation": operation,
            "path": path_text,
        }))
    }
}

fn io_error_kind(err: &io::Error) -> &'static str {
    match err.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::InvalidInput => "invalid_input",
        io::ErrorKind::InvalidData => "invalid_data",
        _ => "io_error",
    }
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
                extra: None,
                error_text: Some(format!("invalid input: {err}")),
                error_kind: Some("invalid_input".to_string()),
                platform: Some(std::env::consts::OS.to_string()),
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

    let allow_path_outside_workspace = context_allows_path_outside_workspace(req.context.as_ref());
    let result = execute_action(&workspace_root, req.args, allow_path_outside_workspace);
    match result {
        Ok(text) => Resp {
            request_id: req.request_id,
            status: "ok".to_string(),
            extra: serde_json::from_str(&text).ok(),
            text,
            error_text: None,
            error_kind: None,
            platform: None,
        },
        Err(err) => Resp {
            request_id: req.request_id,
            status: "error".to_string(),
            text: String::new(),
            extra: err.extra,
            error_text: Some(err.message),
            error_kind: Some(err.kind.to_string()),
            platform: Some(std::env::consts::OS.to_string()),
        },
    }
}

fn execute_action(
    workspace_root: &Path,
    args: Value,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let obj = args
        .as_object()
        .ok_or_else(|| SkillError::invalid_input("args must be object"))?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("info")
        .to_ascii_lowercase();

    match action.as_str() {
        "info" => system_info(workspace_root),
        "inventory_dir" => inventory_dir(workspace_root, obj, allow_path_outside_workspace),
        "count_inventory" => count_inventory(workspace_root, obj, allow_path_outside_workspace),
        "workspace_glance" => workspace_glance(workspace_root, obj, allow_path_outside_workspace),
        "tree_summary" => tree_summary(workspace_root, obj, allow_path_outside_workspace),
        "dir_compare" => dir_compare(workspace_root, obj, allow_path_outside_workspace),
        "extract_field" => extract_field(workspace_root, obj, allow_path_outside_workspace),
        "extract_fields" => extract_fields(workspace_root, obj, allow_path_outside_workspace),
        "structured_keys" => structured_keys(workspace_root, obj, allow_path_outside_workspace),
        "validate_structured" => {
            validate_structured(workspace_root, obj, allow_path_outside_workspace)
        }
        "find_path" => find_path(workspace_root, obj, allow_path_outside_workspace),
        "read_range" => read_range(workspace_root, obj, allow_path_outside_workspace),
        "compare_paths" => compare_paths(workspace_root, obj, allow_path_outside_workspace),
        "path_batch_facts" => path_batch_facts(workspace_root, obj, allow_path_outside_workspace),
        "diagnose_runtime" => diagnose_runtime(workspace_root, obj),
        other => Err(SkillError::unsupported_action(format!(
            "unknown action: {other}; allowed: info|inventory_dir|count_inventory|workspace_glance|tree_summary|dir_compare|extract_field|extract_fields|structured_keys|validate_structured|find_path|read_range|compare_paths|path_batch_facts|diagnose_runtime"
        ))),
    }
}

fn context_allows_path_outside_workspace(context: Option<&Value>) -> bool {
    context
        .and_then(|ctx| {
            ctx.get("permissions")
                .and_then(|permissions| permissions.get("allow_path_outside_workspace"))
                .or_else(|| ctx.get("allow_path_outside_workspace"))
        })
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn system_info(workspace_root: &Path) -> SkillResult<String> {
    let hostname = detect_hostname();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let uptime = uptime_seconds_platform().unwrap_or_else(|| "-".to_string());
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
    memory_rss_bytes_platform()
}

#[cfg(target_os = "linux")]
fn uptime_seconds_platform() -> Option<String> {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(|v| v.to_string()))
}

#[cfg(target_os = "macos")]
fn uptime_seconds_platform() -> Option<String> {
    let boot_ts = run_command_lines("sysctl", &["-n", "kern.boottime"], 1)
        .and_then(|lines| lines.into_iter().next())
        .as_deref()
        .and_then(parse_macos_boot_time_seconds)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    now.checked_sub(boot_ts).map(|seconds| seconds.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn uptime_seconds_platform() -> Option<String> {
    None
}

fn inventory_dir(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = obj.get("path").and_then(Value::as_str).unwrap_or(".");
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
    let include_hidden = bool_arg(obj, "include_hidden", false);
    let files_only = bool_arg(obj, "files_only", false);
    let dirs_only = bool_arg(obj, "dirs_only", false);
    let names_only = bool_arg(obj, "names_only", false);
    let max_entries = u64_arg_any(obj, &["max_entries", "limit"], 200).clamp(1, 1000) as usize;
    let sort_by = obj
        .get("sort_by")
        .and_then(Value::as_str)
        .unwrap_or("name")
        .to_ascii_lowercase();
    let ext_filters = ext_filters(obj);

    let meta = std::fs::metadata(&real).map_err(|err| SkillError::io("metadata", &real, err))?;
    if !meta.is_dir() {
        return Err(SkillError::not_a_directory(format!(
            "inventory_dir requires a directory: {}",
            real.display()
        )));
    }

    let mut entries = Vec::new();
    let iter = std::fs::read_dir(&real).map_err(|err| SkillError::io("read_dir", &real, err))?;
    for item in iter {
        let item = item.map_err(|err| SkillError::io("dir_entry", &real, err))?;
        let entry_path = item.path();
        let file_name = item.file_name().to_string_lossy().to_string();
        let is_hidden = file_name.starts_with('.');
        if !include_hidden && is_hidden {
            continue;
        }
        let meta = item
            .metadata()
            .map_err(|err| SkillError::io("metadata", &entry_path, err))?;
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
    let mut file_names = Vec::new();
    let mut dir_names = Vec::new();
    let mut other_names = Vec::new();
    for entry in &entries {
        let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("other");
        if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
            hidden_count += 1;
        }
        if let Some(name) = entry.get("name").and_then(Value::as_str) {
            names.push(name.to_string());
            match kind {
                "file" => {
                    file_count += 1;
                    file_names.push(name.to_string());
                }
                "dir" => {
                    dir_count += 1;
                    dir_names.push(name.to_string());
                }
                _ => other_names.push(name.to_string()),
            }
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
        "names_by_kind": {
            "files": file_names,
            "dirs": dir_names,
            "other": other_names,
        },
        "entries": if names_only { Value::Array(Vec::new()) } else { Value::Array(entries) },
    })
    .to_string())
}

fn count_inventory(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = obj.get("path").and_then(Value::as_str).unwrap_or(".");
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
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

fn workspace_glance(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = obj.get("path").and_then(Value::as_str).unwrap_or(".");
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
    let include_hidden = bool_arg(obj, "include_hidden", false);
    let max_entries = u64_arg(obj, "max_entries", 20).clamp(1, 100) as usize;
    let mut entries = Vec::new();
    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    let mut hidden_count = 0usize;
    let mut ext_counts = std::collections::BTreeMap::<String, usize>::new();

    let meta = std::fs::metadata(&real).map_err(|err| SkillError::io("metadata", &real, err))?;
    if !meta.is_dir() {
        return Err(SkillError::not_a_directory(format!(
            "workspace_glance requires a directory: {}",
            real.display()
        )));
    }

    let iter = std::fs::read_dir(&real).map_err(|err| SkillError::io("read_dir", &real, err))?;
    for item in iter {
        let item = item.map_err(|err| SkillError::io("dir_entry", &real, err))?;
        let entry_path = item.path();
        let file_name = item.file_name().to_string_lossy().to_string();
        let is_hidden = file_name.starts_with('.');
        if !include_hidden && is_hidden {
            continue;
        }
        let meta = item
            .metadata()
            .map_err(|err| SkillError::io("metadata", &entry_path, err))?;
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
        let modified_ts = meta
            .modified()
            .ok()
            .and_then(system_time_to_ts)
            .unwrap_or(0);
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

fn tree_summary(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = obj.get("path").and_then(Value::as_str).unwrap_or(".");
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
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

fn dir_compare(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let left = required_str(obj, "left_path")?;
    let right = required_str(obj, "right_path")?;
    let left_real = resolve_path(workspace_root, left, allow_path_outside_workspace)?;
    let right_real = resolve_path(workspace_root, right, allow_path_outside_workspace)?;
    let include_hidden = bool_arg(obj, "include_hidden", false);
    let recursive = bool_arg(obj, "recursive", false);
    let max_diffs = u64_arg(obj, "max_diffs", 100).clamp(1, 500) as usize;

    let left_meta =
        std::fs::metadata(&left_real).map_err(|err| SkillError::io("metadata", &left_real, err))?;
    let right_meta = std::fs::metadata(&right_real)
        .map_err(|err| SkillError::io("metadata", &right_real, err))?;
    if !left_meta.is_dir() || !right_meta.is_dir() {
        return Err(SkillError::not_a_directory(
            "dir_compare requires both paths to be directories",
        ));
    }

    let left_entries =
        collect_dir_signatures(&left_real, include_hidden, recursive, max_diffs * 20)?;
    let right_entries =
        collect_dir_signatures(&right_real, include_hidden, recursive, max_diffs * 20)?;

    let left_keys = left_entries
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let right_keys = right_entries
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();

    let common = left_keys
        .intersection(&right_keys)
        .cloned()
        .collect::<Vec<_>>();
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
        let right_kind = right_entries
            .get(key)
            .map(String::as_str)
            .unwrap_or("other");
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

fn extract_field(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = required_str(obj, "path")?;
    let field_path = required_str(obj, "field_path")?;
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
    let (format, root_value) =
        parse_structured_root(&real, obj.get("format").and_then(Value::as_str))?;

    let found = lookup_field_value_with_resolution(&root_value, field_path);
    let (exists, value, value_type, value_text) = match found.value {
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
        "resolved_field_path": found.resolved_field_path.unwrap_or_default(),
        "match_strategy": found.match_strategy,
        "match_count": found.match_count,
        "exists": exists,
        "value_type": value_type,
        "value_text": value_text,
        "value": value,
    })
    .to_string())
}

fn extract_fields(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = required_str(obj, "path")?;
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
    let field_paths = string_list_arg(obj, "field_paths");
    if field_paths.is_empty() {
        return Err(SkillError::invalid_input("field_paths is required"));
    }
    let (format, root_value) =
        parse_structured_root(&real, obj.get("format").and_then(Value::as_str))?;

    let mut results = Vec::new();
    for field_path in field_paths {
        let found = lookup_field_value_with_resolution(&root_value, &field_path);
        let (exists, value, value_type, value_text) = match found.value {
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
            "resolved_field_path": found.resolved_field_path.unwrap_or_default(),
            "match_strategy": found.match_strategy,
            "match_count": found.match_count,
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

fn structured_keys(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = required_str(obj, "path")?;
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
    let field_path = obj.get("field_path").and_then(Value::as_str).unwrap_or("");
    let max_keys = u64_arg(obj, "max_keys", 200).clamp(1, 1000) as usize;
    let (format, root_value) =
        parse_structured_root(&real, obj.get("format").and_then(Value::as_str))?;

    let target = if field_path.is_empty() {
        Some(&root_value)
    } else {
        lookup_field_value(&root_value, field_path)
    };

    let Some(target) = target else {
        return Ok(json!({
            "action": "structured_keys",
            "path": path,
            "resolved_path": real.display().to_string(),
            "format": format,
            "field_path": field_path,
            "exists": false,
            "container_type": "missing",
            "count": 0,
            "keys": [],
        })
        .to_string());
    };

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
                "exists": true,
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
                "exists": true,
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
            "exists": true,
            "container_type": json_value_type(other),
            "count": 0,
            "keys": [],
        })
        .to_string()),
    }
}

fn validate_structured(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = required_str(obj, "path")?;
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
    match parse_structured_root(&real, obj.get("format").and_then(Value::as_str)) {
        Ok((format, root_value)) => Ok(json!({
            "action": "validate_structured",
            "path": path,
            "resolved_path": real.display().to_string(),
            "format": format,
            "valid": true,
            "root_type": json_value_type(&root_value),
        })
        .to_string()),
        Err(err) if matches!(err.kind, "invalid_data" | "invalid_input") => Ok(json!({
            "action": "validate_structured",
            "path": path,
            "resolved_path": real.display().to_string(),
            "format": obj
                .get("format")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| detect_format_from_path(&real)),
            "valid": false,
            "error_kind": err.kind,
            "error_text": err.message,
        })
        .to_string()),
        Err(err) => Err(err),
    }
}

fn find_path(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let root = obj.get("root").and_then(Value::as_str).unwrap_or(".");
    let real_root = resolve_path(workspace_root, root, allow_path_outside_workspace)?;
    let needle = obj
        .get("name")
        .or_else(|| obj.get("pattern"))
        .and_then(Value::as_str)
        .ok_or_else(|| SkillError::invalid_input("name or pattern is required"))?;
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
            let resolved_path = p.display().to_string();
            matches.push(json!({
                "name": name,
                "path": to_rel(workspace_root, p),
                "resolved_path": resolved_path,
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

fn read_range(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = required_str(obj, "path")?;
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
    let meta = std::fs::metadata(&real).map_err(|err| SkillError::io("metadata", &real, err))?;
    if meta.is_dir() {
        return Err(SkillError::is_directory(format!(
            "read_range requires a file, but target is a directory: {}",
            real.display()
        )));
    }
    let text =
        std::fs::read_to_string(&real).map_err(|err| SkillError::io("read_file", &real, err))?;
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();
    let start = obj
        .get("start_line")
        .and_then(Value::as_u64)
        .map(|v| v as usize);
    let end = obj
        .get("end_line")
        .and_then(Value::as_u64)
        .map(|v| v as usize);
    let mode = obj
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if start.is_some() || end.is_some() {
                "range"
            } else {
                "head"
            }
        })
        .to_ascii_lowercase();
    let n = u64_arg(obj, "n", 20).clamp(1, 500) as usize;
    let raw = bool_arg(obj, "raw", false) || bool_arg(obj, "verbatim", false);
    let max_line_chars = u64_arg(obj, "max_line_chars", 800).clamp(80, 4000) as usize;

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
                let to = end
                    .unwrap_or(from.saturating_add(n).saturating_sub(1))
                    .max(from);
                (from, to.min(total_lines))
            }
            _ => (1, n.min(total_lines)),
        }
    };

    let mut excerpt_lines = Vec::new();
    let mut compacted_lines = 0usize;
    let mut truncated_lines = 0usize;
    if total_lines > 0 {
        for idx in from..=to {
            if let Some(line) = lines.get(idx - 1) {
                let rendered = render_read_range_line(line, raw, max_line_chars);
                if rendered.compacted {
                    compacted_lines += 1;
                }
                if rendered.truncated {
                    truncated_lines += 1;
                }
                excerpt_lines.push(format!("{idx}|{}", rendered.text));
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
        "line_safety": {
            "raw": raw,
            "max_line_chars": max_line_chars,
            "compacted_lines": compacted_lines,
            "truncated_lines": truncated_lines,
        },
    })
    .to_string())
}

#[derive(Debug, Clone)]
struct RenderedReadRangeLine {
    text: String,
    compacted: bool,
    truncated: bool,
}

fn render_read_range_line(line: &str, raw: bool, max_line_chars: usize) -> RenderedReadRangeLine {
    if !raw {
        if let Some(text) = compact_internal_json_log_line(line, max_line_chars) {
            return RenderedReadRangeLine {
                text,
                compacted: true,
                truncated: false,
            };
        }
    }
    let text = truncate_chars(line, max_line_chars);
    let truncated = text != line;
    RenderedReadRangeLine {
        text,
        compacted: false,
        truncated,
    }
}

fn compact_internal_json_log_line(line: &str, max_value_chars: usize) -> Option<String> {
    let value = serde_json::from_str::<Value>(line.trim()).ok()?;
    let Value::Object(obj) = value else {
        return None;
    };
    const INTERNAL_BULKY_FIELDS: &[&str] = &[
        "prompt",
        "raw_prompt",
        "system_prompt",
        "raw_response",
        "request_payload",
        "messages",
    ];
    let omitted_fields = INTERNAL_BULKY_FIELDS
        .iter()
        .copied()
        .filter(|key| obj.contains_key(*key))
        .collect::<Vec<_>>();
    if omitted_fields.is_empty() {
        return None;
    }

    let mut compact = Map::new();
    for key in [
        "ts",
        "status",
        "vendor",
        "provider",
        "provider_type",
        "model",
        "model_kind",
        "mode",
        "task_id",
        "call_id",
        "prompt_hash",
        "prompt_source",
        "sanitized",
    ] {
        if let Some(value) = obj.get(key) {
            compact.insert(key.to_string(), value.clone());
        }
    }
    if let Some(error) = obj.get("error").filter(|value| !value.is_null()) {
        compact.insert("error".to_string(), error.clone());
    }
    if let Some(usage) = obj.get("usage").filter(|value| value.is_object()) {
        compact.insert("usage".to_string(), usage.clone());
    }
    for (source_key, target_key) in [
        ("clean_response", "clean_response_preview"),
        ("response", "response_preview"),
    ] {
        if let Some(value) = obj
            .get(source_key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            compact.insert(
                target_key.to_string(),
                Value::String(truncate_chars(value, max_value_chars)),
            );
        }
    }
    compact.insert(
        "omitted_fields".to_string(),
        Value::Array(
            omitted_fields
                .iter()
                .map(|field| Value::String((*field).to_string()))
                .collect(),
        ),
    );
    serde_json::to_string(&Value::Object(compact)).ok()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let mut out = text.chars().take(max_chars).collect::<String>();
    out.push_str("...[truncated]");
    out
}

fn compare_paths(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let left = required_str(obj, "left_path")?;
    let right = required_str(obj, "right_path")?;
    let left_real = resolve_path(workspace_root, left, allow_path_outside_workspace)?;
    let right_real = resolve_path(workspace_root, right, allow_path_outside_workspace)?;
    let left_meta =
        std::fs::metadata(&left_real).map_err(|err| SkillError::io("metadata", &left_real, err))?;
    let right_meta = std::fs::metadata(&right_real)
        .map_err(|err| SkillError::io("metadata", &right_real, err))?;

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

fn path_batch_facts(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let paths = string_list_arg(obj, "paths");
    if paths.is_empty() {
        return Err(SkillError::invalid_input("paths is required"));
    }
    let fields = string_list_arg(obj, "fields");
    let include_missing = bool_arg(obj, "include_missing", true);
    let mut facts = Vec::new();

    for path in paths {
        let real = resolve_path(workspace_root, &path, allow_path_outside_workspace)?;
        match std::fs::metadata(&real) {
            Ok(meta) => facts.push(json!({
                "path": path,
                "exists": true,
                "fact": build_path_fact(workspace_root, &real, &meta),
            })),
            Err(err) if include_missing && err.kind() == io::ErrorKind::NotFound => {
                if let Some(resolved) =
                    resolve_case_insensitive_leaf(&real).or_else(|| resolve_unique_stem_leaf(&real))
                {
                    let meta = std::fs::metadata(&resolved)
                        .map_err(|err| SkillError::io("metadata", &resolved, err))?;
                    facts.push(json!({
                        "path": path,
                        "exists": true,
                        "resolved_from_case_insensitive": case_equivalent_path_leaf(&resolved, &real),
                        "resolved_from_stem": path_leaf_matches_file_stem(&resolved, &real),
                        "fact": build_path_fact(workspace_root, &resolved, &meta),
                    }));
                } else {
                    facts.push(json!({
                        "path": path,
                        "exists": false,
                        "error": "not found",
                    }))
                }
            }
            Err(err) => return Err(SkillError::io("metadata", &real, err)),
        }
    }

    let mut response = json!({
        "action": "path_batch_facts",
        "count": facts.len(),
        "include_missing": include_missing,
        "facts": facts,
    });
    if !fields.is_empty() {
        response["fields"] = json!(fields);
    }
    Ok(response.to_string())
}

fn resolve_case_insensitive_leaf(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let target_name = path.file_name()?.to_str()?;
    let entries = std::fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        let candidate_name = entry.file_name();
        let Some(candidate_name) = candidate_name.to_str() else {
            continue;
        };
        if candidate_name.eq_ignore_ascii_case(target_name) {
            return Some(entry.path());
        }
    }
    None
}

fn case_equivalent(a: &str, b: &str) -> bool {
    a == b || a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

fn case_equivalent_path_leaf(resolved: &Path, requested: &Path) -> bool {
    match (
        resolved.file_name().and_then(|name| name.to_str()),
        requested.file_name().and_then(|name| name.to_str()),
    ) {
        (Some(resolved), Some(requested)) => case_equivalent(resolved, requested),
        _ => false,
    }
}

fn path_leaf_matches_file_stem(resolved: &Path, requested: &Path) -> bool {
    match (
        resolved.file_stem().and_then(|name| name.to_str()),
        requested.file_name().and_then(|name| name.to_str()),
    ) {
        (Some(resolved_stem), Some(requested_leaf)) => {
            !requested_leaf.contains('.') && case_equivalent(resolved_stem, requested_leaf)
        }
        _ => false,
    }
}

fn resolve_unique_stem_leaf(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let target_name = path.file_name()?.to_str()?;
    if target_name.contains('.') {
        return None;
    }
    let mut matched: Option<PathBuf> = None;
    for entry in std::fs::read_dir(parent).ok()?.flatten() {
        let candidate_path = entry.path();
        if !entry.metadata().ok().is_some_and(|meta| meta.is_file()) {
            continue;
        }
        let Some(candidate_stem) = candidate_path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !case_equivalent(candidate_stem, target_name) {
            continue;
        }
        if matched.is_some() {
            return None;
        }
        matched = Some(candidate_path);
    }
    matched
}

fn diagnose_runtime(workspace_root: &Path, obj: &Map<String, Value>) -> SkillResult<String> {
    let info = serde_json::from_str::<Value>(&system_info(workspace_root)?)
        .map_err(|err| SkillError::invalid_data(format!("system info encode failed: {err}")))?;
    let include_process = bool_arg(obj, "include_process", false);
    let include_ports = bool_arg(obj, "include_ports", false);
    let include_env_summary = bool_arg(obj, "include_env_summary", false);

    let loadavg = load_average_platform();
    let meminfo = summarize_meminfo_platform();
    let disk = summarize_df(workspace_root);
    let process_snapshot = if include_process {
        top_process_snapshot(8)
    } else {
        None
    };
    let ports_snapshot = if include_ports {
        run_command_lines("ss", &["-ltn"], 10)
            .or_else(|| run_command_lines("lsof", &["-nP", "-iTCP", "-sTCP:LISTEN"], 10))
            .or_else(|| run_command_lines("netstat", &["-anv", "-p", "tcp"], 10))
            .or_else(|| run_command_lines("netstat", &["-ltn"], 10))
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

#[cfg(target_os = "linux")]
fn memory_rss_bytes_platform() -> Option<u64> {
    memory_rss_bytes_from_proc().or_else(memory_rss_bytes_from_ps)
}

#[cfg(not(target_os = "linux"))]
fn memory_rss_bytes_platform() -> Option<u64> {
    memory_rss_bytes_from_ps()
}

#[cfg(target_os = "linux")]
fn load_average_platform() -> Option<String> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.lines().next().map(str::trim).map(str::to_string))
        .or_else(|| {
            run_command_lines("sysctl", &["-n", "vm.loadavg"], 1)
                .and_then(|lines| lines.into_iter().next())
        })
}

#[cfg(not(target_os = "linux"))]
fn load_average_platform() -> Option<String> {
    run_command_lines("sysctl", &["-n", "vm.loadavg"], 1).and_then(|lines| lines.into_iter().next())
}

#[cfg(target_os = "linux")]
fn summarize_meminfo_platform() -> Value {
    let text = match std::fs::read_to_string("/proc/meminfo") {
        Ok(v) => v,
        Err(_) => return summarize_meminfo_from_sysctl(),
    };
    let mut picked = Map::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if matches!(
            key,
            "MemTotal" | "MemFree" | "MemAvailable" | "SwapTotal" | "SwapFree"
        ) {
            picked.insert(key.to_string(), Value::String(value.trim().to_string()));
        }
    }
    Value::Object(picked)
}

#[cfg(not(target_os = "linux"))]
fn summarize_meminfo_platform() -> Value {
    summarize_meminfo_from_sysctl()
}

fn detect_hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            Command::new("hostname")
                .output()
                .ok()
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(target_os = "linux")]
fn memory_rss_bytes_from_proc() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

fn memory_rss_bytes_from_ps() -> Option<u64> {
    let pid = std::process::id().to_string();
    let lines = run_command_lines("ps", &["-o", "rss=", "-p", &pid], 1)?;
    let kb = lines.first()?.trim().parse::<u64>().ok()?;
    Some(kb * 1024)
}

fn summarize_meminfo_from_sysctl() -> Value {
    let total_bytes = run_command_lines("sysctl", &["-n", "hw.memsize"], 1)
        .and_then(|lines| lines.into_iter().next())
        .map(|v| v.trim().to_string());
    match total_bytes {
        Some(total_bytes) => json!({
            "MemTotalBytes": total_bytes,
        }),
        None => Value::Null,
    }
}

#[cfg(target_os = "macos")]
fn parse_macos_boot_time_seconds(raw: &str) -> Option<u64> {
    let (_, after_sec) = raw.split_once("sec =")?;
    after_sec
        .split([',', '}'])
        .next()?
        .trim()
        .parse::<u64>()
        .ok()
}

fn top_process_snapshot(limit: usize) -> Option<Vec<String>> {
    let lines = run_command_lines("ps", &["-Ao", "pid=,comm=,pcpu=,rss="], 4096)?;
    let mut rows = lines
        .into_iter()
        .filter_map(|line| parse_process_snapshot_row(&line))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| b.rss_kb.cmp(&a.rss_kb).then_with(|| a.pid.cmp(&b.pid)));
    let mut out = vec!["PID COMM %CPU RSS_KB".to_string()];
    for row in rows.into_iter().take(limit) {
        out.push(format!(
            "{} {} {:.1} {}",
            row.pid, row.comm, row.cpu, row.rss_kb
        ));
    }
    Some(out)
}

struct ProcessSnapshotRow {
    pid: u32,
    comm: String,
    cpu: f64,
    rss_kb: u64,
}

fn parse_process_snapshot_row(line: &str) -> Option<ProcessSnapshotRow> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse::<u32>().ok()?;
    let comm = parts.next()?.to_string();
    let cpu = parts.next()?.parse::<f64>().ok()?;
    let rss_kb = parts.next()?.parse::<u64>().ok()?;
    Some(ProcessSnapshotRow {
        pid,
        comm,
        cpu,
        rss_kb,
    })
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
    f: &mut dyn FnMut(&Path, &std::fs::Metadata, usize) -> SkillResult<bool>,
) -> SkillResult<()> {
    let meta = std::fs::metadata(path).map_err(|err| SkillError::io("metadata", path, err))?;
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
    f: &mut dyn FnMut(&Path, &std::fs::Metadata, usize) -> SkillResult<bool>,
) -> SkillResult<()> {
    let meta = std::fs::metadata(dir).map_err(|err| SkillError::io("metadata", dir, err))?;
    if !meta.is_dir() {
        return Err(SkillError::not_a_directory(format!(
            "directory traversal requires a directory: {}",
            dir.display()
        )));
    }
    let iter = std::fs::read_dir(dir).map_err(|err| SkillError::io("read_dir", dir, err))?;
    for entry in iter {
        let entry = entry.map_err(|err| SkillError::io("dir_entry", dir, err))?;
        let p = entry.path();
        let meta = entry
            .metadata()
            .map_err(|err| SkillError::io("metadata", &p, err))?;
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
        return normalize_ext_filter(s).into_iter().collect();
    }
    obj.get("ext_filter")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .filter_map(normalize_ext_filter)
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_ext_filter(value: &str) -> Option<String> {
    let normalized = value.trim().trim_start_matches('.').to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn bool_arg(obj: &Map<String, Value>, key: &str, default: bool) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn u64_arg(obj: &Map<String, Value>, key: &str, default: u64) -> u64 {
    obj.get(key).and_then(Value::as_u64).unwrap_or(default)
}

fn u64_arg_any(obj: &Map<String, Value>, keys: &[&str], default: u64) -> u64 {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_u64))
        .unwrap_or(default)
}

fn required_str<'a>(obj: &'a Map<String, Value>, key: &str) -> SkillResult<&'a str> {
    obj.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| SkillError::invalid_input(format!("{key} is required")))
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
    match path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        _ => "json",
    }
    .to_string()
}

fn parse_structured_root(path: &Path, format_hint: Option<&str>) -> SkillResult<(String, Value)> {
    let format = format_hint
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| detect_format_from_path(path));
    let meta = std::fs::metadata(path).map_err(|err| SkillError::io("metadata", path, err))?;
    if meta.is_dir() {
        return Err(SkillError::is_directory(format!(
            "structured document parsing requires a file, but target is a directory: {}",
            path.display()
        )));
    }
    let raw =
        std::fs::read_to_string(path).map_err(|err| SkillError::io("read_file", path, err))?;
    let root_value = match format.as_str() {
        "json" => serde_json::from_str::<Value>(&raw)
            .map_err(|err| SkillError::invalid_data(format!("json parse failed: {err}")))?,
        "toml" => {
            let value = raw
                .parse::<toml::Value>()
                .map_err(|err| SkillError::invalid_data(format!("toml parse failed: {err}")))?;
            serde_json::to_value(value)
                .map_err(|err| SkillError::invalid_data(format!("toml convert failed: {err}")))?
        }
        "yaml" | "yml" => serde_yaml::from_str::<Value>(&raw)
            .map_err(|err| SkillError::invalid_data(format!("yaml parse failed: {err}")))?,
        other => {
            return Err(SkillError::invalid_input(format!(
                "unsupported format: {other}; use json|toml|yaml"
            )));
        }
    };
    Ok((format, root_value))
}

fn collect_dir_signatures(
    root: &Path,
    include_hidden: bool,
    recursive: bool,
    max_entries: usize,
) -> SkillResult<std::collections::BTreeMap<String, String>> {
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
    for seg in split_field_path(field_path)? {
        if seg.is_empty() {
            return None;
        }
        current = lookup_field_segment(current, seg)?;
    }
    Some(current)
}

struct FieldLookup<'a> {
    value: Option<&'a Value>,
    resolved_field_path: Option<String>,
    match_strategy: &'static str,
    match_count: usize,
}

fn lookup_field_value_with_resolution<'a>(value: &'a Value, field_path: &str) -> FieldLookup<'a> {
    if let Some(found) = lookup_field_value(value, field_path) {
        return FieldLookup {
            value: Some(found),
            resolved_field_path: Some(field_path.to_string()),
            match_strategy: "exact_path",
            match_count: 1,
        };
    }

    if let Some(found) = lookup_array_item_key_path(value, field_path) {
        return found;
    }

    if let Some(found) = lookup_parent_scoped_suffix_field(value, field_path) {
        return found;
    }

    if let Some(found) = lookup_missing_parent_leaf_suffix_field(value, field_path) {
        return found;
    }

    let Some(key) = bare_field_key_selector(field_path) else {
        return FieldLookup {
            value: None,
            resolved_field_path: None,
            match_strategy: "exact_path",
            match_count: 0,
        };
    };

    let mut matches = Vec::new();
    collect_bare_key_matches(value, key, "", &mut matches);
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "unique_bare_key",
            match_count: 1,
        };
    }

    if matches.is_empty() {
        collect_bare_key_suffix_matches(value, key, "", &mut matches);
        if matches.len() == 1 {
            let (resolved_field_path, found) = matches.remove(0);
            return FieldLookup {
                value: Some(found),
                resolved_field_path: Some(resolved_field_path),
                match_strategy: "unique_bare_key_suffix",
                match_count: 1,
            };
        }
    }

    FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "unique_bare_key",
        match_count: matches.len(),
    }
}

fn bare_field_key_selector(field_path: &str) -> Option<&str> {
    let key = field_path.trim();
    if key.is_empty()
        || key
            .chars()
            .any(|ch| ch == '.' || ch == '[' || ch == ']' || ch.is_whitespace())
    {
        return None;
    }
    Some(key)
}

fn collect_bare_key_matches<'a>(
    value: &'a Value,
    target_key: &str,
    current_path: &str,
    out: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{current_path}.{key}")
                };
                if key == target_key {
                    out.push((child_path.clone(), child));
                }
                collect_bare_key_matches(child, target_key, &child_path, out);
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let child_path = if current_path.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{current_path}[{idx}]")
                };
                collect_bare_key_matches(child, target_key, &child_path, out);
            }
        }
        _ => {}
    }
}

fn collect_bare_key_suffix_matches<'a>(
    value: &'a Value,
    target_key: &str,
    current_path: &str,
    out: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{current_path}.{key}")
                };
                if bare_key_suffix_matches(key, target_key) && is_safe_suffix_field_value(child) {
                    out.push((child_path.clone(), child));
                }
                collect_bare_key_suffix_matches(child, target_key, &child_path, out);
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let child_path = if current_path.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{current_path}[{idx}]")
                };
                collect_bare_key_suffix_matches(child, target_key, &child_path, out);
            }
        }
        _ => {}
    }
}

fn is_safe_suffix_field_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn bare_key_suffix_matches(key: &str, target_key: &str) -> bool {
    let key = key.trim();
    let target_key = target_key.trim();
    if target_key.len() < 3 || key.eq_ignore_ascii_case(target_key) {
        return false;
    }
    let key_lower = key.to_ascii_lowercase();
    let target_lower = target_key.to_ascii_lowercase();
    let Some(prefix) = key_lower.strip_suffix(&target_lower) else {
        return false;
    };
    prefix.ends_with(['_', '-'])
}

fn lookup_parent_scoped_suffix_field<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<FieldLookup<'a>> {
    let segments = split_field_path(field_path)?;
    if segments.len() < 2 {
        return None;
    }
    let leaf = segments.last()?.trim();
    let target_key = bare_field_key_selector(leaf)?;
    let parent_path = segments[..segments.len() - 1].join(".");
    if parent_path.trim().is_empty() {
        return None;
    }
    let parent = lookup_field_value(value, &parent_path)?;
    let mut matches = Vec::new();
    collect_bare_key_suffix_matches(parent, target_key, &parent_path, &mut matches);
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return Some(FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "parent_scoped_key_suffix",
            match_count: 1,
        });
    }
    (!matches.is_empty()).then_some(FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "parent_scoped_key_suffix",
        match_count: matches.len(),
    })
}

fn lookup_missing_parent_leaf_suffix_field<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<FieldLookup<'a>> {
    let segments = split_field_path(field_path)?;
    if segments.len() < 2 {
        return None;
    }
    let leaf = segments.last()?.trim();
    let target_key = bare_field_key_selector(leaf)?;
    let parent_path = segments[..segments.len() - 1].join(".");
    if parent_path.trim().is_empty() || lookup_field_value(value, &parent_path).is_some() {
        return None;
    }

    let mut matches = Vec::new();
    collect_bare_key_suffix_matches(value, target_key, "", &mut matches);
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return Some(FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "missing_parent_leaf_key_suffix",
            match_count: 1,
        });
    }
    (!matches.is_empty()).then_some(FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "missing_parent_leaf_key_suffix",
        match_count: matches.len(),
    })
}

fn lookup_array_item_key_path<'a>(value: &'a Value, field_path: &str) -> Option<FieldLookup<'a>> {
    let segments = split_field_path(field_path)?;
    if segments.len() < 2 {
        return None;
    }
    let selector_value = segments[0].trim();
    if selector_value.is_empty() || selector_value.contains('[') || selector_value.contains(']') {
        return None;
    }
    let nested_field_path = segments[1..].join(".");
    if nested_field_path.trim().is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    collect_array_item_key_path_matches(
        value,
        selector_value,
        &nested_field_path,
        "",
        &mut matches,
    );
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return Some(FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "array_item_key_path",
            match_count: 1,
        });
    }
    (!matches.is_empty()).then_some(FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "array_item_key_path",
        match_count: matches.len(),
    })
}

fn collect_array_item_key_path_matches<'a>(
    value: &'a Value,
    selector_value: &str,
    nested_field_path: &str,
    current_path: &str,
    out: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{current_path}.{key}")
                };
                collect_array_item_key_path_matches(
                    child,
                    selector_value,
                    nested_field_path,
                    &child_path,
                    out,
                );
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let item_path = if current_path.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{current_path}[{idx}]")
                };
                if let Some((selector_key, nested_value)) =
                    array_item_key_path_match(child, selector_value, nested_field_path)
                {
                    let resolved_path = format!(
                        "{current_path}[{selector_key}={selector_value}].{nested_field_path}"
                    );
                    out.push((resolved_path, nested_value));
                }
                collect_array_item_key_path_matches(
                    child,
                    selector_value,
                    nested_field_path,
                    &item_path,
                    out,
                );
            }
        }
        _ => {}
    }
}

fn array_item_key_path_match<'a>(
    item: &'a Value,
    selector_value: &str,
    nested_field_path: &str,
) -> Option<(&'static str, &'a Value)> {
    let map = item.as_object()?;
    for selector_key in ["name", "id", "key"] {
        if map
            .get(selector_key)
            .and_then(Value::as_str)
            .is_some_and(|value| value == selector_value)
        {
            let nested_value = lookup_field_value(item, nested_field_path)?;
            return Some((selector_key, nested_value));
        }
    }
    None
}

fn split_field_path(field_path: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut bracket_depth = 0usize;
    let mut quote: Option<char> = None;
    for (idx, ch) in field_path.char_indices() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.checked_sub(1)?,
            '.' if bracket_depth == 0 => {
                out.push(&field_path[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    if quote.is_some() || bracket_depth != 0 {
        return None;
    }
    out.push(&field_path[start..]);
    Some(out)
}

fn lookup_field_segment<'a>(mut current: &'a Value, segment: &str) -> Option<&'a Value> {
    if let Ok(idx) = segment.parse::<usize>() {
        return current.as_array()?.get(idx);
    }

    let Some(first_bracket) = segment.find('[') else {
        return current.get(segment);
    };
    let key = &segment[..first_bracket];
    if !key.is_empty() {
        current = current.get(key)?;
    }

    let mut rest = &segment[first_bracket..];
    while !rest.is_empty() {
        let inner_start = rest.strip_prefix('[')?;
        let end = find_selector_end(inner_start)?;
        let selector = &inner_start[..end];
        current = lookup_field_selector(current, selector)?;
        rest = &inner_start[end + 1..];
    }
    Some(current)
}

fn find_selector_end(selector_and_tail: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (idx, ch) in selector_and_tail.char_indices() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ']' => return Some(idx),
            _ => {}
        }
    }
    None
}

fn lookup_field_selector<'a>(value: &'a Value, selector: &str) -> Option<&'a Value> {
    let selector = selector.trim();
    if let Ok(idx) = selector.parse::<usize>() {
        return value.as_array()?.get(idx);
    }
    let condition = selector
        .strip_prefix("?(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(selector);
    let (field_path, expected) = parse_field_filter_condition(condition)?;
    value.as_array()?.iter().find(|item| {
        lookup_field_value(item, field_path)
            .is_some_and(|found| json_value_matches_text(found, expected))
    })
}

fn parse_field_filter_condition(condition: &str) -> Option<(&str, &str)> {
    let (left, right) = condition
        .split_once("==")
        .or_else(|| condition.split_once('='))?;
    let left = left.trim();
    let field_path = left
        .strip_prefix("@.")
        .or_else(|| left.strip_prefix('@'))
        .unwrap_or(left)
        .trim();
    if field_path.is_empty() {
        return None;
    }
    let expected = strip_matching_quotes(right.trim())?;
    Some((field_path, expected))
}

fn strip_matching_quotes(value: &str) -> Option<&str> {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[value.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return Some(&value[1..value.len() - 1]);
        }
    }
    Some(value)
}

fn json_value_matches_text(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(text) => text == expected,
        Value::Bool(flag) => flag.to_string() == expected,
        Value::Number(number) => number.to_string() == expected,
        Value::Null => expected.eq_ignore_ascii_case("null"),
        Value::Array(_) | Value::Object(_) => json_value_to_text(value) == expected,
    }
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
) -> SkillResult<Value> {
    if state.remaining_nodes == 0 {
        state.truncated_nodes += 1;
        return Ok(json!({
            "path": to_rel(workspace_root, path),
            "truncated": true,
        }));
    }
    state.remaining_nodes -= 1;

    let meta = std::fs::metadata(path).map_err(|err| SkillError::io("metadata", path, err))?;
    let mut node = build_path_fact(workspace_root, path, &meta);
    if !meta.is_dir() {
        return Ok(node);
    }

    let mut visible_entries: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|err| SkillError::io("read_dir", path, err))?
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

fn same_file_content(left: &Path, right: &Path) -> SkillResult<bool> {
    const MAX_COMPARE_BYTES: u64 = 4 * 1024 * 1024;
    let left_meta = std::fs::metadata(left).map_err(|err| SkillError::io("metadata", left, err))?;
    let right_meta =
        std::fs::metadata(right).map_err(|err| SkillError::io("metadata", right, err))?;
    if left_meta.len() != right_meta.len() {
        return Ok(false);
    }
    if left_meta.len() > MAX_COMPARE_BYTES {
        return Err(SkillError::invalid_input(format!(
            "file too large to compare content directly: {} bytes exceeds {}",
            left_meta.len(),
            MAX_COMPARE_BYTES
        )));
    }
    let left_bytes = std::fs::read(left).map_err(|err| SkillError::io("read_file", left, err))?;
    let right_bytes =
        std::fs::read(right).map_err(|err| SkillError::io("read_file", right, err))?;
    Ok(left_bytes == right_bytes)
}

fn resolve_path(
    workspace_root: &Path,
    input: &str,
    allow_path_outside_workspace: bool,
) -> SkillResult<PathBuf> {
    let raw = Path::new(input);
    if allow_path_outside_workspace {
        return if raw.is_absolute() {
            Ok(raw.to_path_buf())
        } else {
            Ok(workspace_root.join(raw))
        };
    }

    let mut normalized = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::ParentDir => {
                return Err(SkillError::path_denied("path with '..' is not allowed"));
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }

    let candidate = if raw.is_absolute() {
        normalized
    } else {
        workspace_root.join(normalized)
    };
    let normalized_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let normalized_candidate = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.clone());
    if !normalized_candidate.starts_with(normalized_root) {
        return Err(SkillError::path_denied("path is outside workspace"));
    }
    Ok(candidate)
}

fn walk_collect(path: &Path, f: &mut dyn FnMut(&Path) -> bool) -> SkillResult<()> {
    if path.is_file() {
        let _ = f(path);
        return Ok(());
    }
    if path.is_dir() && f(path) {
        return Ok(());
    }
    let meta = std::fs::metadata(path).map_err(|err| SkillError::io("metadata", path, err))?;
    if !meta.is_dir() {
        return Err(SkillError::not_a_directory(format!(
            "path search requires a directory: {}",
            path.display()
        )));
    }
    let iter = std::fs::read_dir(path).map_err(|err| SkillError::io("read_dir", path, err))?;
    for entry in iter {
        let entry = entry.map_err(|err| SkillError::io("dir_entry", path, err))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "rustclaw_system_basic_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp root");
        root
    }

    #[test]
    fn resolve_path_blocks_absolute_outside_workspace_without_permission() {
        let root = temp_root("deny_abs");
        let denied = resolve_path(&root, "/etc/passwd", false).expect_err("should deny");
        assert_eq!(denied.kind, "path_denied");
        assert_eq!(denied.message, "path is outside workspace");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resolve_path_allows_absolute_outside_workspace_with_permission() {
        let root = temp_root("allow_abs");
        let resolved = resolve_path(&root, "/etc/passwd", true).expect("should allow");
        assert_eq!(resolved, PathBuf::from("/etc/passwd"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn path_batch_facts_resolves_case_insensitive_leaf() {
        let root = temp_root("path_facts_case_leaf");
        let dir = root.join("reports");
        std::fs::create_dir_all(&dir).expect("create reports");
        std::fs::write(dir.join("Report.MD"), "ok").expect("write report");
        let mut obj = Map::new();
        obj.insert(
            "paths".to_string(),
            json!([root.join("reports/report.md").display().to_string()]),
        );
        obj.insert("fields".to_string(), json!(["exists", "size"]));

        let out = path_batch_facts(&root, &obj, true).expect("path facts");
        let value: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(
            value.get("fields").and_then(Value::as_array).map(Vec::len),
            Some(2)
        );
        let fact = value
            .get("facts")
            .and_then(Value::as_array)
            .and_then(|facts| facts.first())
            .expect("first fact");
        assert_eq!(fact.get("exists").and_then(Value::as_bool), Some(true));
        assert_eq!(
            fact.get("resolved_from_case_insensitive")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(fact
            .get("fact")
            .and_then(|inner| inner.get("resolved_path"))
            .and_then(Value::as_str)
            .is_some_and(|path| path.ends_with("reports/Report.MD")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn path_batch_facts_resolves_unique_stem_leaf() {
        let root = temp_root("path_facts_stem_leaf");
        let dir = root.join("stem_unique");
        std::fs::create_dir_all(&dir).expect("create stem dir");
        std::fs::write(dir.join("ABCD.txt"), "ok").expect("write target");
        let mut obj = Map::new();
        obj.insert(
            "paths".to_string(),
            json!([root.join("stem_unique/abcd").display().to_string()]),
        );

        let out = path_batch_facts(&root, &obj, true).expect("path facts");
        let value: Value = serde_json::from_str(&out).expect("json");
        let fact = value
            .get("facts")
            .and_then(Value::as_array)
            .and_then(|facts| facts.first())
            .expect("first fact");
        assert_eq!(fact.get("exists").and_then(Value::as_bool), Some(true));
        assert_eq!(
            fact.get("resolved_from_stem").and_then(Value::as_bool),
            Some(true)
        );
        assert!(fact
            .get("fact")
            .and_then(|inner| inner.get("resolved_path"))
            .and_then(Value::as_str)
            .is_some_and(|path| path.ends_with("stem_unique/ABCD.txt")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn path_batch_facts_keeps_ambiguous_stem_missing() {
        let root = temp_root("path_facts_ambiguous_stem");
        let dir = root.join("stem_ambiguous");
        std::fs::create_dir_all(&dir).expect("create stem dir");
        std::fs::write(dir.join("ABCD.txt"), "one").expect("write first");
        std::fs::write(dir.join("abcd.md"), "two").expect("write second");
        let mut obj = Map::new();
        obj.insert(
            "paths".to_string(),
            json!([root.join("stem_ambiguous/abcd").display().to_string()]),
        );

        let out = path_batch_facts(&root, &obj, true).expect("path facts");
        let value: Value = serde_json::from_str(&out).expect("json");
        let fact = value
            .get("facts")
            .and_then(Value::as_array)
            .and_then(|facts| facts.first())
            .expect("first fact");
        assert_eq!(fact.get("exists").and_then(Value::as_bool), Some(false));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn extract_field_supports_array_filter_segments_for_toml() {
        let root = temp_root("extract_field_toml_filter");
        let target = root.join("skills_registry.toml");
        std::fs::write(
            &target,
            r#"
[[skills]]
name = "read_file"
planner_kind = "tool"

[[skills]]
name = "stock"
planner_kind = "skill"

[[skills]]
name = "run_cmd"
planner_kind = "tool"
"#,
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("format".to_string(), json!("toml"));
        obj.insert(
            "field_path".to_string(),
            json!("skills[?(@.name=='run_cmd')].planner_kind"),
        );

        let out = extract_field(&root, &obj, true).expect("extract field");
        let value: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value.get("value_text").and_then(Value::as_str),
            Some("tool")
        );
        assert_eq!(
            value.get("value_type").and_then(Value::as_str),
            Some("string")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn extract_field_resolves_array_item_key_path_for_toml() {
        let root = temp_root("extract_field_array_item_key");
        let target = root.join("skills_registry.toml");
        std::fs::write(
            &target,
            r#"
[[skills]]
name = "read_file"
planner_kind = "tool"

[[skills]]
name = "stock"
planner_kind = "skill"

[[skills]]
name = "run_cmd"
planner_kind = "tool"
"#,
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("format".to_string(), json!("toml"));
        obj.insert("field_path".to_string(), json!("run_cmd.planner_kind"));

        let out = extract_field(&root, &obj, true).expect("extract field");
        let value: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value.get("value_text").and_then(Value::as_str),
            Some("tool")
        );
        assert_eq!(
            value.get("resolved_field_path").and_then(Value::as_str),
            Some("skills[name=run_cmd].planner_kind")
        );
        assert_eq!(
            value.get("match_strategy").and_then(Value::as_str),
            Some("array_item_key_path")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn extract_field_resolves_unique_bare_key_in_nested_toml() {
        let root = temp_root("extract_field_unique_bare_key");
        let target = root.join("config.toml");
        std::fs::write(
            &target,
            r#"
[llm]
selected_vendor = "mimo"
selected_model = "mimo-v2.5-pro"
"#,
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("format".to_string(), json!("toml"));
        obj.insert("field_path".to_string(), json!("selected_vendor"));

        let out = extract_field(&root, &obj, true).expect("extract field");
        let value: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value.get("value_text").and_then(Value::as_str),
            Some("mimo")
        );
        assert_eq!(
            value.get("resolved_field_path").and_then(Value::as_str),
            Some("llm.selected_vendor")
        );
        assert_eq!(
            value.get("match_strategy").and_then(Value::as_str),
            Some("unique_bare_key")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn extract_field_resolves_unique_suffix_bare_key_in_nested_toml() {
        let root = temp_root("extract_field_unique_suffix_bare_key");
        let target = root.join("config.toml");
        std::fs::write(
            &target,
            r#"
[llm]
selected_vendor = "mimo"
selected_model = "mimo-v2.5-pro"
"#,
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("format".to_string(), json!("toml"));
        obj.insert("field_path".to_string(), json!("vendor"));

        let out = extract_field(&root, &obj, true).expect("extract field");
        let value: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value.get("value_text").and_then(Value::as_str),
            Some("mimo")
        );
        assert_eq!(
            value.get("resolved_field_path").and_then(Value::as_str),
            Some("llm.selected_vendor")
        );
        assert_eq!(
            value.get("match_strategy").and_then(Value::as_str),
            Some("unique_bare_key_suffix")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn extract_field_resolves_parent_scoped_suffix_key_in_nested_toml() {
        let root = temp_root("extract_field_parent_scoped_suffix_key");
        let target = root.join("config.toml");
        std::fs::write(
            &target,
            r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M2.7"
"#,
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("format".to_string(), json!("toml"));
        obj.insert("field_path".to_string(), json!("llm.vendor"));

        let out = extract_field(&root, &obj, true).expect("extract field");
        let value: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value.get("value_text").and_then(Value::as_str),
            Some("minimax")
        );
        assert_eq!(
            value.get("resolved_field_path").and_then(Value::as_str),
            Some("llm.selected_vendor")
        );
        assert_eq!(
            value.get("match_strategy").and_then(Value::as_str),
            Some("parent_scoped_key_suffix")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn extract_field_resolves_missing_parent_leaf_suffix_key() {
        let root = temp_root("extract_field_missing_parent_leaf_suffix_key");
        let target = root.join("config.toml");
        std::fs::write(
            &target,
            r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M2.7"
"#,
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("format".to_string(), json!("toml"));
        obj.insert("field_path".to_string(), json!("model.vendor"));

        let out = extract_field(&root, &obj, true).expect("extract field");
        let value: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value.get("value_text").and_then(Value::as_str),
            Some("minimax")
        );
        assert_eq!(
            value.get("resolved_field_path").and_then(Value::as_str),
            Some("llm.selected_vendor")
        );
        assert_eq!(
            value.get("match_strategy").and_then(Value::as_str),
            Some("missing_parent_leaf_key_suffix")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn extract_field_does_not_suffix_match_object_values() {
        let root = temp_root("extract_field_suffix_object_value");
        let target = root.join("config.toml");
        std::fs::write(
            &target,
            r#"
[tools.by_provider.openai]
allow = []
deny = []
"#,
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("format".to_string(), json!("toml"));
        obj.insert("field_path".to_string(), json!("provider"));

        let out = extract_field(&root, &obj, true).expect("extract field");
        let value: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(value.get("exists").and_then(Value::as_bool), Some(false));
        assert_eq!(value.get("match_count").and_then(Value::as_u64), Some(0));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn extract_fields_resolves_suffix_scalars_without_object_matches() {
        let root = temp_root("extract_fields_suffix_scalars");
        let target = root.join("config.toml");
        std::fs::write(
            &target,
            r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M2.7"

[tools.by_provider.openai]
allow = []
deny = []
"#,
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("format".to_string(), json!("toml"));
        obj.insert(
            "field_paths".to_string(),
            json!(["llm.vendor", "provider", "selected_model"]),
        );

        let out = extract_fields(&root, &obj, true).expect("extract fields");
        let value: Value = serde_json::from_str(&out).expect("json");
        let results = value
            .get("results")
            .and_then(Value::as_array)
            .expect("results");

        assert_eq!(
            results[0]
                .get("resolved_field_path")
                .and_then(Value::as_str),
            Some("llm.selected_vendor")
        );
        assert_eq!(
            results[0].get("value_text").and_then(Value::as_str),
            Some("minimax")
        );
        assert_eq!(
            results[1].get("exists").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            results[2]
                .get("resolved_field_path")
                .and_then(Value::as_str),
            Some("llm.selected_model")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn extract_field_keeps_ambiguous_bare_key_missing() {
        let root = temp_root("extract_field_ambiguous_bare_key");
        let target = root.join("config.toml");
        std::fs::write(
            &target,
            r#"
[primary]
name = "alpha"

[secondary]
name = "beta"
"#,
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("format".to_string(), json!("toml"));
        obj.insert("field_path".to_string(), json!("name"));

        let out = extract_field(&root, &obj, true).expect("extract field");
        let value: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(value.get("exists").and_then(Value::as_bool), Some(false));
        assert_eq!(value.get("match_count").and_then(Value::as_u64), Some(2));
        assert_eq!(
            value.get("match_strategy").and_then(Value::as_str),
            Some("unique_bare_key")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lookup_field_value_supports_bracket_index_and_filter() {
        let value = json!({
            "items": [
                {"name": "alpha", "versions": [{"kind": "old", "value": 1}]},
                {"name": "beta", "versions": [{"kind": "new", "value": 2}]}
            ]
        });

        assert_eq!(
            lookup_field_value(&value, "items[1].versions[0].value").and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            lookup_field_value(
                &value,
                "items[?(@.name==\"beta\")].versions[?(@.kind=='new')].value"
            )
            .and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            lookup_field_value(&value, "items.[name=beta].versions.[kind=new].value")
                .and_then(Value::as_i64),
            Some(2)
        );
    }

    #[test]
    fn read_range_uses_range_mode_when_line_bounds_are_present() {
        let root = temp_root("read_range_bounds");
        let target = root.join("README.md");
        std::fs::write(&target, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n").expect("write readme");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("start_line".to_string(), json!(1));
        obj.insert("end_line".to_string(), json!(8));

        let out = read_range(&root, &obj, true).expect("read range");
        let value: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(value.get("mode").and_then(Value::as_str), Some("range"));
        assert_eq!(value.get("requested_n").and_then(Value::as_u64), Some(20));
        assert_eq!(value.get("start_line").and_then(Value::as_u64), Some(1));
        assert_eq!(value.get("end_line").and_then(Value::as_u64), Some(8));
        assert!(value
            .get("excerpt")
            .and_then(Value::as_str)
            .is_some_and(|excerpt| excerpt.contains("8|8") && !excerpt.contains("9|9")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn read_range_compacts_internal_model_io_json_lines_by_default() {
        let root = temp_root("read_range_model_io_compact");
        let target = root.join("model_io.log");
        let line = json!({
            "task_id": "task-1",
            "vendor": "minimax",
            "model": "MiniMax-M2.7",
            "status": "ok",
            "prompt": "SECRET_PROMPT_SHOULD_NOT_BE_VISIBLE",
            "raw_response": "RAW_RESPONSE_SHOULD_NOT_BE_VISIBLE",
            "request_payload": {"messages": [{"role": "user", "content": "payload body"}]},
            "response": "{\"steps\":[]}",
            "usage": {"total_tokens": 12}
        })
        .to_string();
        std::fs::write(&target, format!("plain\n{line}\n")).expect("write model io log");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!(target.display().to_string()));
        obj.insert("mode".to_string(), json!("tail"));
        obj.insert("n".to_string(), json!(1));

        let out = read_range(&root, &obj, true).expect("read range");
        let value: Value = serde_json::from_str(&out).expect("json");
        let excerpt = value
            .get("excerpt")
            .and_then(Value::as_str)
            .expect("excerpt");

        assert!(excerpt.contains("task-1"));
        assert!(excerpt.contains("omitted_fields"));
        assert!(excerpt.contains("response_preview"));
        assert!(!excerpt.contains("SECRET_PROMPT_SHOULD_NOT_BE_VISIBLE"));
        assert!(!excerpt.contains("RAW_RESPONSE_SHOULD_NOT_BE_VISIBLE"));
        assert!(!excerpt.contains("payload body"));
        assert_eq!(
            value
                .get("line_safety")
                .and_then(|safety| safety.get("compacted_lines"))
                .and_then(Value::as_u64),
            Some(1)
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_path_match_includes_resolved_path() {
        let root = temp_root("find_path_resolved_path");
        let dir = root.join("case_only");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let target = dir.join("Report.MD");
        std::fs::write(&target, "hello").expect("write target");
        let mut obj = Map::new();
        obj.insert("root".to_string(), json!("case_only"));
        obj.insert("name".to_string(), json!("report.md"));
        obj.insert("match_mode".to_string(), json!("exact"));
        obj.insert("target_kind".to_string(), json!("file"));

        let out = find_path(&root, &obj, false).expect("find path");
        let value: Value = serde_json::from_str(&out).expect("json");
        let first = value
            .get("matches")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("first match");

        assert_eq!(
            first.get("path").and_then(Value::as_str),
            Some("case_only/Report.MD")
        );
        assert_eq!(
            first.get("resolved_path").and_then(Value::as_str),
            Some(target.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn read_range_directory_error_is_structured() {
        let root = temp_root("read_range_directory_error");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!("."));

        let err = read_range(&root, &obj, true).expect_err("directory read should fail");
        assert_eq!(err.kind, "is_directory");
        assert!(err.message.contains("target is a directory"));

        let resp = handle(Req {
            request_id: "structured-dir".to_string(),
            args: json!({"action": "read_range", "path": "."}),
            context: Some(json!({"allow_path_outside_workspace": true})),
        });
        assert_eq!(resp.status, "error");
        assert_eq!(resp.error_kind.as_deref(), Some("is_directory"));
        assert_eq!(resp.platform.as_deref(), Some(std::env::consts::OS));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inventory_missing_path_error_is_structured() {
        let root = temp_root("inventory_missing_error");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!("missing-directory"));

        let err = inventory_dir(&root, &obj, true).expect_err("missing directory should fail");
        assert_eq!(err.kind, "not_found");
        assert!(err
            .extra
            .as_ref()
            .and_then(|extra| extra.get("operation"))
            .and_then(Value::as_str)
            .is_some_and(|operation| operation == "metadata"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inventory_dir_accepts_limit_alias_for_max_entries() {
        let root = temp_root("inventory_limit_alias");
        for name in ["a.log", "b.log", "c.log"] {
            std::fs::write(root.join(name), name).expect("write file");
        }
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!("."));
        obj.insert("names_only".to_string(), json!(true));
        obj.insert("files_only".to_string(), json!(true));
        obj.insert("limit".to_string(), json!(2));

        let out = inventory_dir(&root, &obj, true).expect("inventory");
        let value: Value = serde_json::from_str(&out).expect("json");
        let names = value.get("names").and_then(Value::as_array).expect("names");
        assert_eq!(names.len(), 2);
        assert_eq!(
            value.pointer("/counts/total").and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            value
                .pointer("/names_by_kind/files")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            value
                .pointer("/names_by_kind/dirs")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ext_filter_blank_string_means_no_filter() {
        let mut obj = Map::new();
        obj.insert("ext_filter".to_string(), json!(""));

        assert!(ext_filters(&obj).is_empty());
    }

    #[test]
    fn ext_filter_normalizes_arrays_and_ignores_blank_items() {
        let mut obj = Map::new();
        obj.insert("ext_filter".to_string(), json!([" .MD ", "", ".toml"]));

        assert_eq!(ext_filters(&obj), vec!["md", "toml"]);
    }

    #[test]
    fn context_permission_reads_nested_or_legacy_flag() {
        assert!(context_allows_path_outside_workspace(Some(&json!({
            "permissions": {"allow_path_outside_workspace": true}
        }))));
        assert!(context_allows_path_outside_workspace(Some(&json!({
            "allow_path_outside_workspace": true
        }))));
        assert!(!context_allows_path_outside_workspace(Some(&json!({
            "permissions": {"allow_path_outside_workspace": false}
        }))));
        assert!(!context_allows_path_outside_workspace(None));
    }

    #[test]
    fn validate_structured_reports_parse_success_without_listing_keys() {
        let root = temp_root("validate_structured_ok");
        std::fs::write(
            root.join("config.toml"),
            "[llm]\nselected_vendor = \"mimo\"\n",
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!("config.toml"));
        obj.insert("format".to_string(), json!("toml"));

        let out = validate_structured(&root, &obj, true).expect("validate");
        let value: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(
            value.get("action").and_then(Value::as_str),
            Some("validate_structured")
        );
        assert_eq!(value.get("valid").and_then(Value::as_bool), Some(true));
        assert!(value.get("keys").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn validate_structured_reports_parse_failure_as_structured_output() {
        let root = temp_root("validate_structured_fail");
        std::fs::write(
            root.join("config.toml"),
            "[llm\nselected_vendor = \"mimo\"\n",
        )
        .expect("write toml");
        let mut obj = Map::new();
        obj.insert("path".to_string(), json!("config.toml"));
        obj.insert("format".to_string(), json!("toml"));

        let out = validate_structured(&root, &obj, true).expect("validate");
        let value: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(value.get("valid").and_then(Value::as_bool), Some(false));
        assert_eq!(
            value.get("error_kind").and_then(Value::as_str),
            Some("invalid_data")
        );
        assert!(value
            .get("error_text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("toml parse failed")));
        let _ = std::fs::remove_dir_all(root);
    }
}

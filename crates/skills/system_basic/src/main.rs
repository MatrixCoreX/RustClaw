use std::cmp::Ordering;
use std::ffi::OsStr;
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

mod path_helpers;
mod platform_helpers;
mod structured_helpers;

use path_helpers::*;
use platform_helpers::*;
use structured_helpers::*;

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
        "runtime_status" => runtime_status(obj),
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
            "unknown action: {other}; allowed: info|runtime_status|inventory_dir|count_inventory|workspace_glance|tree_summary|dir_compare|extract_field|extract_fields|structured_keys|validate_structured|find_path|read_range|compare_paths|path_batch_facts|diagnose_runtime"
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
    let current_user = detect_current_user();
    let kernel_release = detect_kernel_release();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let now_rfc3339 = current_time_rfc3339();
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
        "current_user": current_user,
        "kernel_release": kernel_release,
        "now_ts": now,
        "now_rfc3339": now_rfc3339,
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

fn runtime_status(obj: &Map<String, Value>) -> SkillResult<String> {
    let raw_kind = obj
        .get("kind")
        .or_else(|| obj.get("query"))
        .or_else(|| obj.get("field"))
        .and_then(Value::as_str)
        .unwrap_or("info");
    let kind = normalize_runtime_status_kind(raw_kind);
    let value = match kind.as_str() {
        "current_user" => detect_current_user().unwrap_or_else(|| "-".to_string()),
        "host_name" => detect_hostname(),
        "kernel_release" => detect_kernel_release().unwrap_or_else(|| "-".to_string()),
        "current_time" => current_time_rfc3339(),
        "current_working_directory" => std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "-".to_string()),
        _ => {
            return Err(SkillError::invalid_input(
                "unsupported runtime_status kind; use current_user|host_name|kernel_release|current_time|current_working_directory",
            ));
        }
    };
    Ok(json!({
        "action": "runtime_status",
        "kind": kind,
        "value": value.clone(),
        "field_value": value.clone(),
        "command_output": value,
    })
    .to_string())
}

fn normalize_runtime_status_kind(raw: &str) -> String {
    match raw
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .trim_matches('_')
    {
        "whoami" | "current_user" | "current_username" | "os_user" | "system_user"
        | "runtime_user" | "user" | "username" => "current_user".to_string(),
        "hostname" | "host_name" | "current_hostname" | "current_host" | "machine_name" => {
            "host_name".to_string()
        }
        "kernel" | "kernel_name" | "kernel_release" | "os_kernel" | "system_kernel" | "uname"
        | "uname_r" => "kernel_release".to_string(),
        "now"
        | "time"
        | "date"
        | "datetime"
        | "timestamp"
        | "current_time"
        | "system_time"
        | "current_system_time" => "current_time".to_string(),
        "pwd"
        | "cwd"
        | "current_working_directory"
        | "current_directory"
        | "process_cwd"
        | "current_process_cwd" => "current_working_directory".to_string(),
        other => other.to_string(),
    }
}

fn current_time_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn detect_current_user() -> Option<String> {
    for key in ["USER", "LOGNAME", "USERNAME"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    std::env::var("HOME").ok().and_then(|home| {
        Path::new(&home)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    })
}

fn detect_kernel_release() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Some(value) = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            return Some(value);
        }
    }
    Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

    let size_summary = inventory_size_summary(&entries);
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
        "size_summary": size_summary,
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

fn inventory_size_summary(entries: &[Value]) -> Value {
    let mut matched_file_count = 0usize;
    let mut total_file_size_bytes = 0u64;
    let mut largest_file: Option<&Value> = None;
    let mut smallest_file: Option<&Value> = None;

    for entry in entries {
        if entry.get("kind").and_then(Value::as_str) != Some("file") {
            continue;
        }
        matched_file_count += 1;
        total_file_size_bytes =
            total_file_size_bytes.saturating_add(entry_size_bytes(entry).unwrap_or(0));
        largest_file = better_inventory_extreme(largest_file, entry, true);
        smallest_file = better_inventory_extreme(smallest_file, entry, false);
    }

    json!({
        "matched_file_count": matched_file_count,
        "total_file_size_bytes": total_file_size_bytes,
        "largest_file": largest_file.cloned().unwrap_or(Value::Null),
        "smallest_file": smallest_file.cloned().unwrap_or(Value::Null),
    })
}

fn better_inventory_extreme<'a>(
    current: Option<&'a Value>,
    candidate: &'a Value,
    largest: bool,
) -> Option<&'a Value> {
    let Some(current) = current else {
        return Some(candidate);
    };
    let current_size = entry_size_bytes(current).unwrap_or(0);
    let candidate_size = entry_size_bytes(candidate).unwrap_or(0);
    let better_size = if largest {
        candidate_size > current_size
    } else {
        candidate_size < current_size
    };
    if better_size {
        return Some(candidate);
    }
    if candidate_size == current_size && entry_name(candidate) < entry_name(current) {
        return Some(candidate);
    }
    Some(current)
}

fn entry_size_bytes(entry: &Value) -> Option<u64> {
    entry.get("size_bytes").and_then(Value::as_u64)
}

fn entry_name(entry: &Value) -> &str {
    entry.get("name").and_then(Value::as_str).unwrap_or("")
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
            let preview_limit = max_keys.min(20);
            let preview = arr
                .iter()
                .take(preview_limit)
                .enumerate()
                .map(|(idx, item)| structured_array_item_preview(idx, item))
                .collect::<Vec<_>>();
            let (identity_values, identity_omitted) =
                structured_array_identity_values(arr, max_keys);
            Ok(json!({
                "action": "structured_keys",
                "path": path,
                "resolved_path": real.display().to_string(),
                "format": format,
                "field_path": field_path,
                "exists": true,
                "container_type": "array",
                "count": arr.len(),
                "identity_values": identity_values,
                "identity_omitted": identity_omitted,
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

fn structured_array_item_preview(idx: usize, item: &Value) -> Value {
    let mut out = Map::new();
    out.insert("index".to_string(), json!(idx));
    out.insert("value_type".to_string(), json!(json_value_type(item)));
    if let Some(map) = item.as_object() {
        let mut keys = map.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        let omitted = keys.len().saturating_sub(8);
        if keys.len() > 8 {
            keys.truncate(8);
        }
        out.insert("keys".to_string(), json!(keys));
        out.insert("keys_omitted".to_string(), json!(omitted));
        if let Some((key, value)) = structured_array_item_identity(map) {
            out.insert("identity_key".to_string(), json!(key));
            out.insert("identity_value".to_string(), json!(value));
        }
    }
    Value::Object(out)
}

fn structured_array_identity_values(arr: &[Value], max_values: usize) -> (Vec<String>, usize) {
    let all_values = arr
        .iter()
        .filter_map(|item| item.as_object())
        .filter_map(|map| structured_array_item_identity(map).map(|(_, value)| value.to_string()))
        .collect::<Vec<_>>();
    let omitted = all_values.len().saturating_sub(max_values);
    let values = all_values.into_iter().take(max_values).collect();
    (values, omitted)
}

fn structured_array_item_identity(map: &Map<String, Value>) -> Option<(&'static str, &str)> {
    for selector_key in ["name", "id", "key"] {
        if let Some(value) = map.get(selector_key).and_then(Value::as_str) {
            return Some((selector_key, value));
        }
    }
    None
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

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

use super::*;

#[derive(Default)]
pub(super) struct CountSummary {
    pub(super) total: usize,
    pub(super) files: usize,
    pub(super) dirs: usize,
    pub(super) hidden: usize,
    pub(super) total_size_bytes: u64,
    pub(super) by_extension: std::collections::BTreeMap<String, usize>,
}

pub(super) struct TreeSummaryState {
    pub(super) remaining_nodes: usize,
    pub(super) truncated_nodes: usize,
}

#[cfg(target_os = "linux")]
pub(super) fn memory_rss_bytes_platform() -> Option<u64> {
    memory_rss_bytes_from_proc().or_else(memory_rss_bytes_from_ps)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn memory_rss_bytes_platform() -> Option<u64> {
    memory_rss_bytes_from_ps()
}

#[cfg(target_os = "linux")]
pub(super) fn load_average_platform() -> Option<String> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.lines().next().map(str::trim).map(str::to_string))
        .or_else(|| {
            run_command_lines("sysctl", &["-n", "vm.loadavg"], 1)
                .and_then(|lines| lines.into_iter().next())
        })
}

#[cfg(not(target_os = "linux"))]
pub(super) fn load_average_platform() -> Option<String> {
    run_command_lines("sysctl", &["-n", "vm.loadavg"], 1).and_then(|lines| lines.into_iter().next())
}

#[cfg(target_os = "linux")]
pub(super) fn summarize_meminfo_platform() -> Value {
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
pub(super) fn summarize_meminfo_platform() -> Value {
    summarize_meminfo_from_sysctl()
}

pub(super) fn detect_hostname() -> String {
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
pub(super) fn memory_rss_bytes_from_proc() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

pub(super) fn memory_rss_bytes_from_ps() -> Option<u64> {
    let pid = std::process::id().to_string();
    let lines = run_command_lines("ps", &["-o", "rss=", "-p", &pid], 1)?;
    let kb = lines.first()?.trim().parse::<u64>().ok()?;
    Some(kb * 1024)
}

pub(super) fn summarize_meminfo_from_sysctl() -> Value {
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
pub(super) fn parse_macos_boot_time_seconds(raw: &str) -> Option<u64> {
    let (_, after_sec) = raw.split_once("sec =")?;
    after_sec
        .split([',', '}'])
        .next()?
        .trim()
        .parse::<u64>()
        .ok()
}

pub(super) fn top_process_snapshot(limit: usize) -> Option<Vec<String>> {
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

pub(super) struct ProcessSnapshotRow {
    pub(super) pid: u32,
    pub(super) comm: String,
    pub(super) cpu: f64,
    pub(super) rss_kb: u64,
}

pub(super) fn parse_process_snapshot_row(line: &str) -> Option<ProcessSnapshotRow> {
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

pub(super) fn summarize_df(workspace_root: &Path) -> Value {
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

pub(super) fn run_command_lines(cmd: &str, args: &[&str], max_lines: usize) -> Option<Vec<String>> {
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

pub(super) fn walk_inventory(
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

pub(super) fn walk_inventory_inner(
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

pub(super) fn sort_inventory_entries(entries: &mut [Value], sort_by: &str) {
    entries.sort_by(|a, b| match sort_by {
        "name_desc" => cmp_name(b, a),
        "mtime_desc" => cmp_u64_field(b, a, "modified_ts").then_with(|| cmp_name(a, b)),
        "mtime_asc" => cmp_u64_field(a, b, "modified_ts").then_with(|| cmp_name(a, b)),
        "size_desc" => cmp_u64_field(b, a, "size_bytes").then_with(|| cmp_name(a, b)),
        "size_asc" => cmp_u64_field(a, b, "size_bytes").then_with(|| cmp_name(a, b)),
        _ => cmp_name(a, b),
    });
}

pub(super) fn cmp_u64_field(a: &Value, b: &Value, key: &str) -> Ordering {
    a.get(key)
        .and_then(Value::as_u64)
        .cmp(&b.get(key).and_then(Value::as_u64))
}

pub(super) fn cmp_name(a: &Value, b: &Value) -> Ordering {
    a.get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .cmp(b.get("name").and_then(Value::as_str).unwrap_or(""))
}

pub(super) fn ext_filters(obj: &Map<String, Value>) -> Vec<String> {
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

pub(super) fn normalize_ext_filter(value: &str) -> Option<String> {
    let normalized = value.trim().trim_start_matches('.').to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

pub(super) fn bool_arg(obj: &Map<String, Value>, key: &str, default: bool) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub(super) fn u64_arg(obj: &Map<String, Value>, key: &str, default: u64) -> u64 {
    obj.get(key).and_then(Value::as_u64).unwrap_or(default)
}

pub(super) fn u64_arg_any(obj: &Map<String, Value>, keys: &[&str], default: u64) -> u64 {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_u64))
        .unwrap_or(default)
}

pub(super) fn required_str<'a>(obj: &'a Map<String, Value>, key: &str) -> SkillResult<&'a str> {
    obj.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| SkillError::invalid_input(format!("{key} is required")))
}

pub(super) fn string_list_arg(obj: &Map<String, Value>, key: &str) -> Vec<String> {
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

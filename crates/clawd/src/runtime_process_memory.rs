use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RuntimeProcessMemorySample {
    pub(crate) measurement: String,
    pub(crate) process_count: usize,
    pub(crate) resident_and_swap_bytes: u64,
    pub(crate) roles: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub(crate) struct RuntimeProcessMemoryStatus {
    pub(crate) measurement: String,
    pub(crate) process_count: usize,
    pub(crate) current_bytes: u64,
    pub(crate) peak_bytes: u64,
    pub(crate) warning: bool,
    pub(crate) roles_current_bytes: BTreeMap<String, u64>,
    pub(crate) roles_peak_bytes: BTreeMap<String, u64>,
    pub(crate) collected_at_epoch: u64,
}

#[derive(Debug, Clone)]
struct ProcessRow {
    pid: u32,
    parent_pid: u32,
    executable: PathBuf,
}

pub(crate) fn collect_runtime_process_memory(
    workspace_root: &Path,
) -> Result<RuntimeProcessMemorySample, &'static str> {
    #[cfg(target_os = "linux")]
    {
        collect_linux_runtime_process_memory(workspace_root)
    }
    #[cfg(target_os = "macos")]
    {
        collect_macos_runtime_process_memory(workspace_root)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = workspace_root;
        Err("runtime_process_memory_platform_unsupported")
    }
}

fn runtime_binary_directories(workspace_root: &Path) -> BTreeSet<PathBuf> {
    let mut directories = BTreeSet::new();
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            directories.insert(normalize_path(parent));
        }
    }
    for relative in ["target/release", "release-bin"] {
        directories.insert(normalize_path(&workspace_root.join(relative)));
    }
    directories
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn is_runtime_root_name(name: &str) -> bool {
    matches!(
        name,
        "clawd"
            | "webd"
            | "skill-runner"
            | "wechatd"
            | "telegramd"
            | "whatsappd"
            | "wa-webd"
            | "feishud"
            | "larkd"
    )
}

fn runtime_root_pids(
    rows: &BTreeMap<u32, ProcessRow>,
    binary_directories: &BTreeSet<PathBuf>,
) -> BTreeSet<u32> {
    let current_pid = std::process::id();
    rows.values()
        .filter(|row| {
            row.pid == current_pid
                || (row
                    .executable
                    .parent()
                    .map(normalize_path)
                    .is_some_and(|parent| binary_directories.contains(&parent))
                    && row
                        .executable
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(is_runtime_root_name))
        })
        .map(|row| row.pid)
        .collect()
}

fn runtime_descendants(rows: &BTreeMap<u32, ProcessRow>, roots: &BTreeSet<u32>) -> BTreeSet<u32> {
    let mut selected = roots.clone();
    loop {
        let mut changed = false;
        for row in rows.values() {
            if !selected.contains(&row.pid) && selected.contains(&row.parent_pid) {
                selected.insert(row.pid);
                changed = true;
            }
        }
        if !changed {
            return selected;
        }
    }
}

fn process_role(executable: &Path) -> &'static str {
    let name = executable
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match name.as_str() {
        "clawd" => "core",
        "webd" => "web_gateway",
        "skill-runner" => "skill_runner",
        "wechatd" | "telegramd" | "whatsappd" | "wa-webd" | "feishud" | "larkd" => "channel",
        "chrome" | "google-chrome" | "chromium" | "chromium-browser" => "browser",
        "python" | "python3" | "node" | "go" => "skill_process",
        _ => "child_process",
    }
}

fn aggregate_selected<F>(
    rows: &BTreeMap<u32, ProcessRow>,
    selected: &BTreeSet<u32>,
    measurement: &str,
    mut memory_bytes: F,
) -> RuntimeProcessMemorySample
where
    F: FnMut(u32) -> Option<u64>,
{
    let mut sample = RuntimeProcessMemorySample {
        measurement: measurement.to_string(),
        ..RuntimeProcessMemorySample::default()
    };
    for pid in selected {
        let Some(row) = rows.get(pid) else {
            continue;
        };
        let Some(bytes) = memory_bytes(*pid) else {
            continue;
        };
        sample.process_count = sample.process_count.saturating_add(1);
        sample.resident_and_swap_bytes = sample.resident_and_swap_bytes.saturating_add(bytes);
        let role = process_role(&row.executable).to_string();
        let entry = sample.roles.entry(role).or_default();
        *entry = entry.saturating_add(bytes);
    }
    sample
}

#[cfg(target_os = "linux")]
fn collect_linux_runtime_process_memory(
    workspace_root: &Path,
) -> Result<RuntimeProcessMemorySample, &'static str> {
    let rows = linux_process_rows();
    if rows.is_empty() {
        return Err("runtime_process_table_unavailable");
    }
    let roots = runtime_root_pids(&rows, &runtime_binary_directories(workspace_root));
    if roots.is_empty() {
        return Err("runtime_process_roots_unavailable");
    }
    let selected = runtime_descendants(&rows, &roots);
    let sample = aggregate_selected(
        &rows,
        &selected,
        "pss_plus_swap_pss",
        linux_process_resident_and_swap_bytes,
    );
    (sample.process_count > 0)
        .then_some(sample)
        .ok_or("runtime_process_memory_unavailable")
}

#[cfg(target_os = "linux")]
fn linux_process_rows() -> BTreeMap<u32, ProcessRow> {
    let mut rows = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return rows;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let process_root = entry.path();
        let Ok(executable) = std::fs::read_link(process_root.join("exe")) else {
            continue;
        };
        let Ok(status) = std::fs::read_to_string(process_root.join("status")) else {
            continue;
        };
        let Some(parent_pid) = linux_status_kib(&status, "PPid") else {
            continue;
        };
        rows.insert(
            pid,
            ProcessRow {
                pid,
                parent_pid: parent_pid.min(u32::MAX as u64) as u32,
                executable,
            },
        );
    }
    rows
}

#[cfg(target_os = "linux")]
fn linux_process_resident_and_swap_bytes(pid: u32) -> Option<u64> {
    let root = PathBuf::from(format!("/proc/{pid}"));
    if let Ok(rollup) = std::fs::read_to_string(root.join("smaps_rollup")) {
        if let Some(pss_kib) = linux_status_kib(&rollup, "Pss") {
            let swap_pss_kib = linux_status_kib(&rollup, "SwapPss").unwrap_or_default();
            return Some(pss_kib.saturating_add(swap_pss_kib).saturating_mul(1024));
        }
    }
    let status = std::fs::read_to_string(root.join("status")).ok()?;
    let rss_kib = linux_status_kib(&status, "VmRSS")?;
    let swap_kib = linux_status_kib(&status, "VmSwap").unwrap_or_default();
    Some(rss_kib.saturating_add(swap_kib).saturating_mul(1024))
}

#[cfg(target_os = "linux")]
fn linux_status_kib(raw: &str, key: &str) -> Option<u64> {
    raw.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name == key)
            .then(|| value.split_whitespace().next()?.parse::<u64>().ok())
            .flatten()
    })
}

#[cfg(target_os = "macos")]
fn collect_macos_runtime_process_memory(
    workspace_root: &Path,
) -> Result<RuntimeProcessMemorySample, &'static str> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,rss=,comm="])
        .output()
        .map_err(|_| "runtime_process_table_unavailable")?;
    if !output.status.success() {
        return Err("runtime_process_table_unavailable");
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let mut rows = BTreeMap::new();
    let mut rss_bytes = BTreeMap::new();
    for line in raw.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 {
            continue;
        }
        let (Ok(pid), Ok(parent_pid), Ok(rss_kib)) = (
            fields[0].parse::<u32>(),
            fields[1].parse::<u32>(),
            fields[2].parse::<u64>(),
        ) else {
            continue;
        };
        rows.insert(
            pid,
            ProcessRow {
                pid,
                parent_pid,
                executable: PathBuf::from(fields[3..].join(" ")),
            },
        );
        rss_bytes.insert(pid, rss_kib.saturating_mul(1024));
    }
    let roots = runtime_root_pids(&rows, &runtime_binary_directories(workspace_root));
    if roots.is_empty() {
        return Err("runtime_process_roots_unavailable");
    }
    let selected = runtime_descendants(&rows, &roots);
    let sample = aggregate_selected(&rows, &selected, "rss", |pid| rss_bytes.get(&pid).copied());
    (sample.process_count > 0)
        .then_some(sample)
        .ok_or("runtime_process_memory_unavailable")
}

#[cfg(test)]
#[path = "runtime_process_memory_tests.rs"]
mod tests;

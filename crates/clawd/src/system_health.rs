use crate::AppState;
use rusqlite::OptionalExtension;
#[cfg(not(target_os = "linux"))]
use std::process::Command as StdCommand;

pub(crate) fn current_rss_bytes() -> Option<u64> {
    process_snapshots()
        .ok()?
        .into_iter()
        .find(|proc| proc.pid == std::process::id())
        .and_then(|proc| proc.rss_bytes)
}

pub(crate) fn telegramd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("telegramd")
}

pub(crate) fn channel_gateway_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("channel-gateway")
}

pub(crate) fn whatsappd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("whatsappd")
}

pub(crate) fn wa_webd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("whatsapp_webd")
}

pub(crate) fn webd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("webd")
}

pub(crate) fn wechatd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("wechatd")
}

pub(crate) fn feishud_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("feishud")
}

pub(crate) fn larkd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("larkd")
}

pub(crate) fn daemon_process_pids_by_name(process_name: &str) -> Option<Vec<u32>> {
    Some(collect_matching_pids(
        &process_snapshots().ok()?,
        process_name,
        std::process::id(),
    ))
}

fn daemon_process_stats(process_name: &str) -> Option<(usize, u64)> {
    let mut count = 0usize;
    let mut total_rss_bytes = 0u64;

    for proc in process_snapshots().ok()? {
        if !process_name_matches(&proc, process_name) {
            continue;
        }
        count += 1;
        if let Some(rss_bytes) = proc.rss_bytes {
            total_rss_bytes = total_rss_bytes.saturating_add(rss_bytes);
        }
    }

    Some((count, total_rss_bytes))
}

#[derive(Debug, Clone)]
struct ProcessSnapshot {
    pid: u32,
    rss_bytes: Option<u64>,
    comm: String,
    args: String,
}

fn process_snapshots() -> anyhow::Result<Vec<ProcessSnapshot>> {
    process_snapshots_impl()
}

#[cfg(target_os = "linux")]
fn process_snapshots_impl() -> anyhow::Result<Vec<ProcessSnapshot>> {
    process_snapshots_from_linux_proc()
}

#[cfg(not(target_os = "linux"))]
fn process_snapshots_impl() -> anyhow::Result<Vec<ProcessSnapshot>> {
    process_snapshots_from_ps()
}

#[cfg(target_os = "linux")]
fn process_snapshots_from_linux_proc() -> anyhow::Result<Vec<ProcessSnapshot>> {
    let entries = std::fs::read_dir("/proc")?;
    let mut processes = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name.to_string_lossy();
        if !pid.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let pid_num = match pid.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };

        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let args = read_linux_cmdline(&pid);
        let rss_bytes = current_rss_bytes_from_status_path(&format!("/proc/{pid}/status"));
        processes.push(ProcessSnapshot {
            pid: pid_num,
            rss_bytes,
            comm,
            args,
        });
    }

    Ok(processes)
}

#[cfg(not(target_os = "linux"))]
fn process_snapshots_from_ps() -> anyhow::Result<Vec<ProcessSnapshot>> {
    let output = StdCommand::new("ps")
        .args(["-axo", "pid=,rss=,comm=,args="])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("ps command failed with status {}", output.status);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(pid_raw) = parts.next() else {
            continue;
        };
        let Some(rss_kb_raw) = parts.next() else {
            continue;
        };
        let Some(comm_raw) = parts.next() else {
            continue;
        };
        let pid = match pid_raw.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let rss_bytes = rss_kb_raw
            .parse::<u64>()
            .ok()
            .map(|kb| kb.saturating_mul(1024));
        let args = parts.collect::<Vec<_>>().join(" ");
        processes.push(ProcessSnapshot {
            pid,
            rss_bytes,
            comm: process_basename(comm_raw),
            args,
        });
    }
    Ok(processes)
}

#[cfg(target_os = "linux")]
fn read_linux_cmdline(pid: &str) -> String {
    let cmdline_path = format!("/proc/{pid}/cmdline");
    let Ok(bytes) = std::fs::read(&cmdline_path) else {
        return String::new();
    };
    bytes
        .split(|&b| b == 0)
        .filter_map(|part| {
            if part.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(part).into_owned())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "linux")]
fn current_rss_bytes_from_status_path(status_path: &str) -> Option<u64> {
    let status = std::fs::read_to_string(status_path).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())?;
            return Some(kb.saturating_mul(1024));
        }
    }
    None
}

fn process_basename(raw: &str) -> String {
    raw.trim()
        .trim_matches('"')
        .rsplit('/')
        .next()
        .unwrap_or(raw.trim())
        .trim()
        .to_string()
}

fn process_name_matches(proc: &ProcessSnapshot, process_name: &str) -> bool {
    if proc.comm == process_name {
        return true;
    }

    let args = proc.args.trim();
    if args.is_empty() {
        return false;
    }

    if args
        .split_whitespace()
        .any(|part| process_basename(part) == process_name)
    {
        return true;
    }

    let cargo_pattern = format!("-p {process_name}");
    args.contains(&cargo_pattern)
        || args.contains(&format!("cargo run -p {process_name}"))
        || args.contains(&format!("cargo run --package {process_name}"))
}

fn collect_matching_pids(
    processes: &[ProcessSnapshot],
    process_name: &str,
    self_pid: u32,
) -> Vec<u32> {
    processes
        .iter()
        .filter(|proc| proc.pid != self_pid && process_name_matches(proc, process_name))
        .map(|proc| proc.pid)
        .collect()
}

#[cfg(test)]
#[path = "system_health_tests.rs"]
mod tests;
pub(crate) fn oldest_running_task_age_seconds(state: &AppState) -> anyhow::Result<u64> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;

    let min_created_at: Option<i64> = db
        .query_row(
            "SELECT MIN(CAST(created_at AS INTEGER)) FROM tasks WHERE status = 'running'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(created_ts) = min_created_at {
        let now = crate::now_ts_u64() as i64;
        let age = now.saturating_sub(created_ts).max(0) as u64;
        Ok(age)
    } else {
        Ok(0)
    }
}

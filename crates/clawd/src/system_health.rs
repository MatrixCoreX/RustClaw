use rusqlite::OptionalExtension;

use crate::AppState;

pub(crate) fn current_rss_bytes() -> Option<u64> {
    current_rss_bytes_from_status("/proc/self/status")
}

fn current_rss_bytes_from_status(status_path: &str) -> Option<u64> {
    let status = std::fs::read_to_string(status_path).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())?;
            return Some(kb * 1024);
        }
    }
    None
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

pub(crate) fn feishud_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("feishud")
}

pub(crate) fn larkd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("larkd")
}

fn daemon_process_stats(process_name: &str) -> Option<(usize, u64)> {
    let entries = std::fs::read_dir("/proc").ok()?;
    let mut count = 0usize;
    let mut total_rss_bytes = 0u64;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name.to_string_lossy();
        if !pid.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if !process_name_matches(&pid, process_name) {
            continue;
        }
        count += 1;
        let status_path = format!("/proc/{pid}/status");
        if let Some(rss_bytes) = current_rss_bytes_from_status(&status_path) {
            total_rss_bytes = total_rss_bytes.saturating_add(rss_bytes);
        }
    }

    Some((count, total_rss_bytes))
}

fn process_name_matches(pid: &str, process_name: &str) -> bool {
    let exe_path = format!("/proc/{pid}/exe");
    if let Ok(target) = std::fs::read_link(&exe_path) {
        let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let name = name.strip_suffix(" (deleted)").unwrap_or(name);
        if name == process_name {
            return true;
        }
    }

    let comm_path = format!("/proc/{pid}/comm");
    if let Ok(s) = std::fs::read_to_string(&comm_path) {
        let comm = s.trim();
        if comm == process_name {
            return true;
        }
    }

    let cmdline_path = format!("/proc/{pid}/cmdline");
    if let Ok(bytes) = std::fs::read(&cmdline_path) {
        if let Some(first_arg) = bytes.split(|&b| b == 0).next() {
            let argv0 = std::str::from_utf8(first_arg).unwrap_or("");
            let base = argv0.rsplit('/').next().unwrap_or(argv0);
            if base == process_name {
                return true;
            }
        }
    }

    false
}

pub(crate) fn oldest_running_task_age_seconds(state: &AppState) -> anyhow::Result<u64> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

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

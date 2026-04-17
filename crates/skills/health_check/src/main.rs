use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
struct SystemHealthSnapshot {
    os_family: String,
    arch: String,
    kernel_release: Option<String>,
    hostname: Option<String>,
    service_manager: String,
    cpu_count: Option<u64>,
    uptime_seconds: Option<u64>,
    load_avg_1m: Option<f64>,
    load_avg_5m: Option<f64>,
    load_avg_15m: Option<f64>,
    memory_total_bytes: Option<u64>,
    memory_available_bytes: Option<u64>,
    disk_root_total_bytes: Option<u64>,
    disk_root_available_bytes: Option<u64>,
    warnings: Vec<String>,
}

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
    let root = workspace_root();
    let log_dir = obj
        .get("log_dir")
        .and_then(|v| v.as_str())
        .map(|v| resolve_path(&root, v))
        .transpose()?
        .unwrap_or_else(|| root.join("logs"));

    let clawd_count = process_count("clawd");
    let telegramd_count = process_count("telegramd");
    let health_port_open = is_port_open("127.0.0.1", 8787);
    let system_health = collect_system_health();

    let clawd_log = summarize_log_file(&log_dir.join("clawd.log"));
    let telegramd_log = summarize_log_file(&log_dir.join("telegramd.log"));

    Ok(json!({
        "ts": now_ts(),
        "workspace_root": root.display().to_string(),
        "log_dir": log_dir.display().to_string(),
        "clawd_process_count": clawd_count,
        "telegramd_process_count": telegramd_count,
        "clawd_health_port_open": health_port_open,
        "clawd_log": clawd_log,
        "telegramd_log": telegramd_log,
        "system_health": system_health
    }))
}

fn summarize_log_file(path: &PathBuf) -> Value {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return json!({"exists": false}),
    };
    let modified_ts = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut err_count = 0usize;
    for line in text.lines() {
        let l = line.to_ascii_lowercase();
        if l.contains("error")
            || l.contains("failed")
            || l.contains("panic")
            || l.contains("timeout")
            || l.contains("unauthorized")
        {
            err_count += 1;
        }
    }
    json!({
        "exists": true,
        "size_bytes": meta.len(),
        "modified_ts": modified_ts,
        "keyword_error_count": err_count
    })
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
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

fn process_count(keyword: &str) -> usize {
    let pgrep_out = Command::new("pgrep").args(["-fc", keyword]).output().ok();
    if let Some(count) = pgrep_out
        .and_then(|v| String::from_utf8(v.stdout).ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
    {
        return count;
    }

    Command::new("ps")
        .args(["-ax", "-o", "command="])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| text.lines().filter(|line| line.contains(keyword)).count())
        .unwrap_or(0)
}

fn is_port_open(host: &str, port: u16) -> bool {
    std::net::TcpStream::connect((host, port)).is_ok()
}

fn collect_system_health() -> SystemHealthSnapshot {
    let os_family = runtime_os_family().to_string();
    let cpu_count = std::thread::available_parallelism()
        .ok()
        .and_then(|count| u64::try_from(count.get()).ok());
    let (load_avg_1m, load_avg_5m, load_avg_15m) = current_load_average(&os_family);
    let (memory_total_bytes, memory_available_bytes) = current_memory_bytes(&os_family);
    let (disk_root_total_bytes, disk_root_available_bytes) = disk_root_bytes();

    let mut snapshot = SystemHealthSnapshot {
        os_family,
        arch: std::env::consts::ARCH.to_string(),
        kernel_release: read_command_output("uname", &["-r"]),
        hostname: current_hostname(),
        service_manager: default_service_manager(runtime_os_family()).to_string(),
        cpu_count,
        uptime_seconds: current_uptime_seconds(runtime_os_family()),
        load_avg_1m,
        load_avg_5m,
        load_avg_15m,
        memory_total_bytes,
        memory_available_bytes,
        disk_root_total_bytes,
        disk_root_available_bytes,
        warnings: Vec::new(),
    };
    snapshot.warnings = build_system_warnings(&snapshot);
    snapshot
}

fn runtime_os_family() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        other => other,
    }
}

fn default_service_manager(os_family: &str) -> &'static str {
    match os_family {
        "macos" => "launchd",
        "linux" => "systemd",
        _ => "unknown",
    }
}

fn read_command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn current_hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| read_command_output("hostname", &[]))
}

fn current_uptime_seconds(os_family: &str) -> Option<u64> {
    match os_family {
        "linux" => parse_linux_uptime(
            &std::fs::read_to_string("/proc/uptime")
                .ok()
                .unwrap_or_default(),
        ),
        "macos" => read_command_output("sysctl", &["-n", "kern.boottime"])
            .as_deref()
            .and_then(parse_macos_boot_time_seconds)
            .and_then(|boot_ts| now_ts().checked_sub(boot_ts)),
        _ => None,
    }
}

fn current_load_average(os_family: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
    match os_family {
        "linux" => parse_load_avg_triplet(
            &std::fs::read_to_string("/proc/loadavg")
                .ok()
                .unwrap_or_default(),
        ),
        "macos" => read_command_output("sysctl", &["-n", "vm.loadavg"])
            .as_deref()
            .map(parse_macos_load_avg)
            .unwrap_or((None, None, None)),
        _ => (None, None, None),
    }
}

fn current_memory_bytes(os_family: &str) -> (Option<u64>, Option<u64>) {
    match os_family {
        "linux" => parse_linux_meminfo(
            &std::fs::read_to_string("/proc/meminfo")
                .ok()
                .unwrap_or_default(),
        ),
        "macos" => current_macos_memory_bytes(),
        _ => (None, None),
    }
}

fn current_macos_memory_bytes() -> (Option<u64>, Option<u64>) {
    let total = read_command_output("sysctl", &["-n", "hw.memsize"])
        .as_deref()
        .and_then(|raw| raw.parse::<u64>().ok());
    let page_size = read_command_output("sysctl", &["-n", "hw.pagesize"])
        .as_deref()
        .and_then(|raw| raw.parse::<u64>().ok());
    let available = match (
        read_command_output("vm_stat", &[]),
        page_size.or_else(|| {
            read_command_output("pagesize", &[])
                .as_deref()
                .and_then(|raw| raw.parse::<u64>().ok())
        }),
    ) {
        (Some(vm_stat), Some(page_size)) => parse_macos_available_memory_bytes(&vm_stat, page_size),
        _ => None,
    };
    (total, available)
}

fn disk_root_bytes() -> (Option<u64>, Option<u64>) {
    let Some(text) = read_command_output("df", &["-k", "/"]) else {
        return (None, None);
    };
    match parse_df_root_kilobytes(&text) {
        Some((total_kb, available_kb)) => (
            Some(total_kb.saturating_mul(1024)),
            Some(available_kb.saturating_mul(1024)),
        ),
        None => (None, None),
    }
}

fn build_system_warnings(snapshot: &SystemHealthSnapshot) -> Vec<String> {
    let mut warnings = Vec::new();
    if resource_is_low(
        snapshot.memory_total_bytes,
        snapshot.memory_available_bytes,
        512 * MIB,
        0.10,
    ) {
        warnings.push("memory_available_low".to_string());
    }
    if resource_is_low(
        snapshot.disk_root_total_bytes,
        snapshot.disk_root_available_bytes,
        5 * GIB,
        0.10,
    ) {
        warnings.push("disk_root_low".to_string());
    }
    if load_is_high(snapshot.load_avg_1m, snapshot.cpu_count) {
        warnings.push("load_high".to_string());
    }
    warnings
}

fn resource_is_low(
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
    absolute_threshold: u64,
    pct_threshold: f64,
) -> bool {
    let (Some(total), Some(available)) = (total_bytes, available_bytes) else {
        return false;
    };
    if total == 0 {
        return false;
    }
    let pct = available as f64 / total as f64;
    available < absolute_threshold || pct < pct_threshold
}

fn load_is_high(load_1m: Option<f64>, cpu_count: Option<u64>) -> bool {
    let Some(load_1m) = load_1m else {
        return false;
    };
    let cpu_count = cpu_count.unwrap_or(2).max(1) as f64;
    load_1m >= cpu_count * 2.0 && load_1m >= 2.0
}

fn parse_linux_uptime(text: &str) -> Option<u64> {
    text.split_whitespace()
        .next()
        .and_then(|raw| raw.parse::<f64>().ok())
        .map(|seconds| seconds.max(0.0) as u64)
}

fn parse_macos_boot_time_seconds(text: &str) -> Option<u64> {
    let marker = "sec = ";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find(',').unwrap_or(rest.len());
    rest[..end].trim().parse::<u64>().ok()
}

fn parse_load_avg_triplet(text: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
    let mut nums = text
        .split_whitespace()
        .take(3)
        .map(|raw| raw.parse::<f64>().ok());
    (
        nums.next().flatten(),
        nums.next().flatten(),
        nums.next().flatten(),
    )
}

fn parse_macos_load_avg(text: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
    let normalized = text.replace(['{', '}'], " ");
    parse_load_avg_triplet(&normalized)
}

fn parse_linux_meminfo(text: &str) -> (Option<u64>, Option<u64>) {
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        if total.is_none() && line.starts_with("MemTotal:") {
            total = parse_meminfo_kib_line(line).map(|value| value.saturating_mul(1024));
        } else if available.is_none() && line.starts_with("MemAvailable:") {
            available = parse_meminfo_kib_line(line).map(|value| value.saturating_mul(1024));
        }
    }
    (total, available)
}

fn parse_meminfo_kib_line(line: &str) -> Option<u64> {
    line.split_whitespace().nth(1)?.parse::<u64>().ok()
}

fn parse_macos_available_memory_bytes(vm_stat: &str, page_size: u64) -> Option<u64> {
    let free = parse_vm_stat_pages(vm_stat, "Pages free")?;
    let inactive = parse_vm_stat_pages(vm_stat, "Pages inactive").unwrap_or(0);
    let speculative = parse_vm_stat_pages(vm_stat, "Pages speculative").unwrap_or(0);
    Some(
        free.saturating_add(inactive)
            .saturating_add(speculative)
            .saturating_mul(page_size),
    )
}

fn parse_vm_stat_pages(text: &str, key: &str) -> Option<u64> {
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(key) {
            continue;
        }
        let raw = trimmed
            .split(':')
            .nth(1)?
            .trim()
            .trim_end_matches('.')
            .replace('.', "");
        return raw.parse::<u64>().ok();
    }
    None
}

fn parse_df_root_kilobytes(text: &str) -> Option<(u64, u64)> {
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("Filesystem"))?;
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }
    let total_kb = parts.get(1)?.parse::<u64>().ok()?;
    let available_kb = parts.get(3)?.parse::<u64>().ok()?;
    Some((total_kb, available_kb))
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{
        build_system_warnings, load_is_high, parse_df_root_kilobytes, parse_linux_meminfo,
        parse_linux_uptime, parse_macos_available_memory_bytes, parse_macos_boot_time_seconds,
        parse_macos_load_avg, resource_is_low, SystemHealthSnapshot,
    };

    fn snapshot() -> SystemHealthSnapshot {
        SystemHealthSnapshot {
            os_family: "linux".to_string(),
            arch: "x86_64".to_string(),
            kernel_release: Some("6.8.0".to_string()),
            hostname: Some("demo".to_string()),
            service_manager: "systemd".to_string(),
            cpu_count: Some(4),
            uptime_seconds: Some(10),
            load_avg_1m: Some(0.5),
            load_avg_5m: Some(0.4),
            load_avg_15m: Some(0.3),
            memory_total_bytes: Some(8 * 1024 * 1024 * 1024),
            memory_available_bytes: Some(4 * 1024 * 1024 * 1024),
            disk_root_total_bytes: Some(100 * 1024 * 1024 * 1024),
            disk_root_available_bytes: Some(40 * 1024 * 1024 * 1024),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn linux_meminfo_parser_reads_total_and_available() {
        let text = "MemTotal:       16384256 kB\nMemFree:         2048000 kB\nMemAvailable:    8192000 kB\n";
        let (total, available) = parse_linux_meminfo(text);
        assert_eq!(total, Some(16_384_256 * 1024));
        assert_eq!(available, Some(8_192_000 * 1024));
    }

    #[test]
    fn macos_vm_stat_parser_estimates_available_bytes() {
        let vm_stat = "\
Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
Pages free:                               100.\n\
Pages active:                            5000.\n\
Pages inactive:                           250.\n\
Pages speculative:                         50.\n";
        assert_eq!(
            parse_macos_available_memory_bytes(vm_stat, 16_384),
            Some((100 + 250 + 50) * 16_384)
        );
    }

    #[test]
    fn df_parser_reads_root_capacity() {
        let text = "\
Filesystem 1024-blocks      Used Available Capacity Mounted on\n\
/dev/disk3s1   976490576 12345678 98765432    12% /\n";
        assert_eq!(
            parse_df_root_kilobytes(text),
            Some((976_490_576, 98_765_432))
        );
    }

    #[test]
    fn system_warnings_include_low_disk_memory_and_high_load() {
        let mut data = snapshot();
        data.memory_available_bytes = Some(128 * 1024 * 1024);
        data.disk_root_available_bytes = Some(2 * 1024 * 1024 * 1024);
        data.load_avg_1m = Some(12.0);
        let warnings = build_system_warnings(&data);
        assert!(warnings.contains(&"memory_available_low".to_string()));
        assert!(warnings.contains(&"disk_root_low".to_string()));
        assert!(warnings.contains(&"load_high".to_string()));
    }

    #[test]
    fn resource_thresholds_use_absolute_or_percent_floor() {
        assert!(resource_is_low(
            Some(10 * 1024 * 1024 * 1024),
            Some(400 * 1024 * 1024),
            512 * 1024 * 1024,
            0.10,
        ));
        assert!(resource_is_low(
            Some(100 * 1024 * 1024 * 1024),
            Some(8 * 1024 * 1024 * 1024),
            5 * 1024 * 1024 * 1024,
            0.10,
        ));
    }

    #[test]
    fn load_warning_threshold_scales_with_cpu_count() {
        assert!(load_is_high(Some(8.5), Some(4)));
        assert!(!load_is_high(Some(3.5), Some(4)));
    }

    #[test]
    fn parse_os_specific_runtime_values() {
        assert_eq!(parse_linux_uptime("12345.67 67890.12"), Some(12_345));
        assert_eq!(
            parse_macos_boot_time_seconds("{ sec = 1718649985, usec = 0 }"),
            Some(1_718_649_985)
        );
        assert_eq!(
            parse_macos_load_avg("{ 2.31 1.82 1.40 }"),
            (Some(2.31), Some(1.82), Some(1.40))
        );
    }
}

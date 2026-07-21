use std::path::PathBuf;
use std::process::Command;

const DISCOVER_CANDIDATES_MAX: usize = 20;

fn strip_service_suffix(s: &str) -> &str {
    let s = s.trim();
    let s_lower = s.to_lowercase();
    if s_lower.ends_with(" service") {
        s_lower
            .rfind(" service")
            .map(|i| s[..i].trim())
            .unwrap_or(s)
    } else if s_lower.ends_with(".service") {
        s[..s.len().saturating_sub(".service".len())].trim()
    } else {
        s
    }
}

pub(crate) fn normalize_target_alias(input: &str) -> String {
    let s = strip_service_suffix(input).trim().to_lowercase();
    if s.is_empty() {
        return input.trim().to_string();
    }
    let canonical = match s.as_str() {
        "nginx" => "nginx",
        "mysql" | "mysqld" => "mysql",
        "redis" | "redis-server" => "redis",
        "postgres" | "postgresql" => "postgresql",
        "docker" | "dockerd" => "docker",
        "caddy" => "caddy",
        "ssh" | "sshd" => "sshd",
        "cron" | "crond" => "cron",
        _ => return s,
    };
    canonical.to_string()
}

#[cfg(target_os = "linux")]
pub(crate) fn discover_systemd_candidates(target: &str) -> Vec<String> {
    let target = target.trim().to_lowercase();
    if target.is_empty() {
        return Vec::new();
    }
    let out = match Command::new("systemctl")
        .args(["list-unit-files", "--type=service", "--no-legend"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    let mut contains = Vec::new();
    let line = String::from_utf8_lossy(&out.stdout);
    for raw in line.lines() {
        let unit = raw.split_whitespace().next().unwrap_or("").trim();
        let name = unit.strip_suffix(".service").unwrap_or(unit);
        if name.is_empty() {
            continue;
        }
        let name_lower = name.to_lowercase();
        if name_lower == target || name_lower == format!("{}.service", target) {
            exact.push(name.to_string());
        } else if name_lower.starts_with(&target) || target.starts_with(&name_lower) {
            prefix.push(name.to_string());
        } else if name_lower.contains(&target) {
            contains.push(name.to_string());
        }
    }
    exact.sort();
    prefix.sort();
    contains.sort();
    let mut out_vec = Vec::new();
    out_vec.extend(exact);
    out_vec.extend(prefix);
    out_vec.extend(contains);
    out_vec.truncate(DISCOVER_CANDIDATES_MAX);
    out_vec
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn discover_systemd_candidates(_target: &str) -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "linux")]
pub(crate) fn discover_service_candidates(target: &str) -> Vec<String> {
    let target = target.trim().to_lowercase();
    if target.is_empty() {
        return Vec::new();
    }
    let out = match Command::new("service").args(["--status-all"]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let line = String::from_utf8_lossy(&out.stdout);
    let mut names = Vec::new();
    for raw in line.lines() {
        let rest = raw.trim();
        if let Some(idx) = rest.find(']') {
            let name = rest[idx + 1..].trim();
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                names.push(name.to_string());
            }
        }
    }
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    let mut contains = Vec::new();
    for name in names {
        let name_lower = name.to_lowercase();
        if name_lower == target {
            exact.push(name);
        } else if name_lower.starts_with(&target) || target.starts_with(&name_lower) {
            prefix.push(name);
        } else if name_lower.contains(&target) {
            contains.push(name);
        }
    }
    exact.sort();
    prefix.sort();
    contains.sort();
    let mut out_vec = Vec::new();
    out_vec.extend(exact);
    out_vec.extend(prefix);
    out_vec.extend(contains);
    out_vec.truncate(DISCOVER_CANDIDATES_MAX);
    out_vec
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn discover_service_candidates(_target: &str) -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "linux")]
pub(super) fn detect_linux_manager_for_target(target: &str) -> Option<&'static str> {
    if let Ok(output) = Command::new("systemctl")
        .args(["is-active", target])
        .output()
    {
        let state = String::from_utf8_lossy(&output.stdout);
        let state = state.trim();
        if output.status.code().is_some()
            && !state.is_empty()
            && state.len() < 50
            && state
                .chars()
                .all(|value| value.is_ascii_alphabetic() || " ()".contains(value))
        {
            return Some("systemd");
        }
    }
    Command::new("service")
        .args([target, "status"])
        .output()
        .ok()
        .and_then(|output| output.status.code())
        .map(|_| "service")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn detect_linux_manager_for_target(_target: &str) -> Option<&'static str> {
    None
}

pub(super) fn manager_supported_on_current_platform(manager: &str) -> bool {
    manager_supported_on_platform(manager, std::env::consts::OS)
}

pub(crate) fn manager_supported_on_platform(manager: &str, platform: &str) -> bool {
    match manager {
        "systemd" | "service" => platform == "linux",
        "launchd" => platform == "macos",
        "brew_services" | "process_only" | "rustclaw" | "unknown" => true,
        _ => false,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn systemctl_output(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("systemctl").args(args).output()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn systemctl_output(_args: &[&str]) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "service_manager_unsupported_platform",
    ))
}

#[cfg(target_os = "linux")]
pub(super) fn service_output(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("service").args(args).output()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn service_output(_args: &[&str]) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "service_manager_unsupported_platform",
    ))
}

#[cfg(target_os = "linux")]
pub(super) fn sudo_systemctl_output(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("sudo")
        .arg("-n")
        .arg("systemctl")
        .args(args)
        .output()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn sudo_systemctl_output(_args: &[&str]) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "service_manager_unsupported_platform",
    ))
}

#[cfg(target_os = "linux")]
pub(super) fn sudo_service_output(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("sudo")
        .arg("-n")
        .arg("service")
        .args(args)
        .output()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn sudo_service_output(_args: &[&str]) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "service_manager_unsupported_platform",
    ))
}

fn discover_brew_service_candidates(target: &str) -> Vec<String> {
    let target = target.trim().to_lowercase();
    if target.is_empty() {
        return Vec::new();
    }
    let Some(entries) = brew_services_list() else {
        return Vec::new();
    };
    rank_candidate_names(
        entries.into_iter().map(|entry| entry.name).collect(),
        &target,
    )
}

fn discover_launchd_candidates(target: &str) -> Vec<String> {
    let target = target.trim().to_lowercase();
    if target.is_empty() {
        return Vec::new();
    }
    let Some(entries) = launchctl_list() else {
        return Vec::new();
    };
    rank_candidate_names(
        entries.into_iter().map(|entry| entry.label).collect(),
        &target,
    )
}

pub(super) fn discover_all_candidates(normalized_target: &str) -> Vec<String> {
    let brew = discover_brew_service_candidates(normalized_target);
    let launchd = discover_launchd_candidates(normalized_target);
    let sys = discover_systemd_candidates(normalized_target);
    let svc = discover_service_candidates(normalized_target);
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();
    for name in brew.into_iter().chain(launchd).chain(sys).chain(svc) {
        if seen.insert(name.clone()) {
            merged.push(name);
        }
    }
    merged.truncate(DISCOVER_CANDIDATES_MAX);
    merged
}

fn rank_candidate_names(names: Vec<String>, target: &str) -> Vec<String> {
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    let mut contains = Vec::new();
    for name in names {
        let name_lower = name.to_lowercase();
        if name_lower == target {
            exact.push(name);
        } else if name_lower.starts_with(target) || target.starts_with(&name_lower) {
            prefix.push(name);
        } else if name_lower.contains(target) {
            contains.push(name);
        }
    }
    exact.sort();
    prefix.sort();
    contains.sort();
    let mut out = Vec::new();
    out.extend(exact);
    out.extend(prefix);
    out.extend(contains);
    out.truncate(DISCOVER_CANDIDATES_MAX);
    out
}

pub(super) fn command_output_text(outp: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&outp.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&outp.stdout).trim().to_string();
    if !stderr.is_empty() && !stdout.is_empty() {
        format!("stderr: {}; stdout: {}", stderr, stdout)
    } else if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "no output".to_string()
    }
}

pub(super) fn looks_like_permission_error(message: &str) -> bool {
    let m = message.to_lowercase();
    [
        "permission denied",
        "operation not permitted",
        "access denied",
        "must be root",
        "authentication is required",
        "interactive authentication required",
        "not in the sudoers",
        "a password is required",
        "password is required",
        "no tty present",
        "pkttyagent",
        "authorization failed",
        "polkit",
        "permission",
        "denied",
    ]
    .iter()
    .any(|k| m.contains(k))
}

fn command_exists(bin: &str) -> bool {
    Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {bin} >/dev/null 2>&1"))
        .status()
        .ok()
        .is_some_and(|status| status.success())
}

pub(crate) fn is_safe_target(s: &str) -> bool {
    if s.is_empty() || s.len() > 256 {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' || c == '@')
}

#[derive(Debug, Clone)]
pub(super) struct BrewServiceEntry {
    pub(super) name: String,
    pub(super) status: String,
    pub(super) user: String,
    pub(super) file: String,
}

fn brew_services_list() -> Option<Vec<BrewServiceEntry>> {
    if !command_exists("brew") {
        return None;
    }
    let output = Command::new("brew")
        .args(["services", "list"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if idx == 0
            && line.to_lowercase().contains("name")
            && line.to_lowercase().contains("status")
        {
            continue;
        }
        let cols = line.split_whitespace().collect::<Vec<_>>();
        if cols.len() < 2 {
            continue;
        }
        entries.push(BrewServiceEntry {
            name: cols[0].to_string(),
            status: cols[1].to_string(),
            user: cols.get(2).copied().unwrap_or("").to_string(),
            file: cols.get(3).copied().unwrap_or("").to_string(),
        });
    }
    Some(entries)
}

pub(super) fn brew_service_entry(target: &str) -> Option<BrewServiceEntry> {
    let normalized = normalize_target_alias(target);
    brew_services_list()?.into_iter().find(|entry| {
        let name = entry.name.to_lowercase();
        name == normalized || normalize_target_alias(&entry.name) == normalized
    })
}

#[derive(Debug, Clone)]
pub(super) struct LaunchdEntry {
    pub(super) pid: Option<i64>,
    pub(super) status_code: Option<i64>,
    pub(super) label: String,
}

fn launchctl_list() -> Option<Vec<LaunchdEntry>> {
    if !command_exists("launchctl") {
        return None;
    }
    let output = Command::new("launchctl").arg("list").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("PID") {
            continue;
        }
        let cols = trimmed.split_whitespace().collect::<Vec<_>>();
        if cols.len() < 3 {
            continue;
        }
        let label = cols[cols.len() - 1].to_string();
        let status_code = cols.get(cols.len() - 2).and_then(|v| v.parse::<i64>().ok());
        let pid = cols.first().and_then(|v| {
            if *v == "-" {
                None
            } else {
                v.parse::<i64>().ok()
            }
        });
        entries.push(LaunchdEntry {
            pid,
            status_code,
            label,
        });
    }
    Some(entries)
}

pub(super) fn launchctl_entry(target: &str) -> Option<LaunchdEntry> {
    let normalized = normalize_target_alias(target);
    launchctl_list()?.into_iter().find(|entry| {
        let label = entry.label.to_lowercase();
        label == normalized
            || normalize_target_alias(&entry.label) == normalized
            || label.ends_with(&format!(".{}", normalized))
            || label.contains(&normalized)
    })
}

pub(super) fn process_count_for_target(target: &str) -> usize {
    let output = Command::new("ps").args(["-ax", "-o", "command="]).output();
    let Ok(output) = output else {
        return 0;
    };
    let Ok(text) = String::from_utf8(output.stdout) else {
        return 0;
    };
    let normalized = normalize_target_alias(target);
    text.lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            lower.contains(&normalized)
                || lower.contains(target)
                || lower.contains(&normalized.replace('_', "-"))
        })
        .count()
}

#[cfg(target_os = "macos")]
fn macos_log_excerpt(target: &str, tail_lines: usize) -> Option<String> {
    if !command_exists("log") {
        return None;
    }
    let predicate = format!(
        "process == \"{target}\" OR eventMessage CONTAINS[c] \"{target}\" OR senderImagePath CONTAINS[c] \"{target}\""
    );
    let output = Command::new("log")
        .args([
            "show",
            "--style",
            "compact",
            "--last",
            "15m",
            "--predicate",
            &predicate,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let recent = text
        .lines()
        .rev()
        .take(tail_lines.min(20))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" ");
    if recent.trim().is_empty() {
        None
    } else {
        Some(recent)
    }
}

#[cfg(not(target_os = "macos"))]
fn macos_log_excerpt(_target: &str, _tail_lines: usize) -> Option<String> {
    None
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub(super) fn fetch_logs_inner(target: &str, manager: &str, tail_lines: usize) -> Vec<String> {
    let mut evidence = Vec::new();
    match manager {
        "rustclaw" => {
            if !super::RUSTCLAW_SERVICES.contains(&target) {
                evidence.push(format!("service {} not in whitelist, no log path", target));
                return evidence;
            }
            let root = workspace_root();
            let path = root.join("logs").join(format!("{}.log", target));
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let lines: Vec<&str> = content.lines().collect();
                    let from = lines.len().saturating_sub(tail_lines);
                    let slice = &lines[from..];
                    let summary: String = slice
                        .iter()
                        .rev()
                        .take(20)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ");
                    evidence.push(format!(
                        "last {} lines (total {}); recent: {}",
                        slice.len(),
                        lines.len(),
                        if summary.len() > 400 {
                            format!("{}...", &summary[..400])
                        } else {
                            summary
                        }
                    ));
                }
                Err(e) => {
                    evidence.push(format!("read log failed: {} ({})", path.display(), e));
                }
            }
        }
        "systemd" => {
            if !is_safe_target(target) {
                return evidence;
            }
            #[cfg(target_os = "linux")]
            let o = Command::new("journalctl")
                .args(["-u", target, "-n", &tail_lines.to_string(), "--no-pager"])
                .output();
            #[cfg(not(target_os = "linux"))]
            let o: std::io::Result<std::process::Output> = Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "service_manager_unsupported_platform",
            ));
            if let Ok(outp) = o {
                let s = String::from_utf8_lossy(&outp.stdout);
                let last: String = s.lines().rev().take(10).collect::<Vec<_>>().join(" ");
                evidence.push(format!(
                    "journalctl last {} lines; recent: {}",
                    s.lines().count(),
                    if last.len() > 300 {
                        format!("{}...", &last[..300])
                    } else {
                        last
                    }
                ));
            }
        }
        "brew_services" => {
            if let Some(summary) = macos_log_excerpt(target, tail_lines) {
                evidence.push(format!("macOS log show recent: {}", summary));
            } else {
                evidence.push(format!(
                    "brew service {} logs not directly available; try 'brew services list' or 'log show' manually",
                    target
                ));
            }
        }
        "launchd" | "process_only" => {
            if let Some(summary) = macos_log_excerpt(target, tail_lines) {
                evidence.push(format!("macOS log show recent: {}", summary));
            } else {
                evidence.push(format!(
                    "no recent macOS unified log entries found for {}",
                    target
                ));
            }
        }
        _ => {
            evidence.push(format!("manager {} logs not implemented", manager));
        }
    }
    evidence
}

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
use std::process::Command;

use serde::{Deserialize, Serialize};

const TEXT_LIMIT_BYTES: u64 = 64 * 1024;
const UNLIMITED_CGROUP_THRESHOLD_BYTES: u64 = 1_u64 << 60;
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CgroupMemoryEvents {
    pub low: u64,
    pub high: u64,
    pub max: u64,
    pub oom: u64,
    pub oom_kill: u64,
    pub oom_group_kill: u64,
}

impl CgroupMemoryEvents {
    fn has_new_critical_event_since(&self, previous: &Self) -> bool {
        self.high > previous.high
            || self.max > previous.max
            || self.oom > previous.oom
            || self.oom_kill > previous.oom_kill
            || self.oom_group_kill > previous.oom_group_kill
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryPressureSample {
    pub some_avg10: Option<f64>,
    pub some_avg60: Option<f64>,
    pub full_avg10: Option<f64>,
    pub full_avg60: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HostResourceSnapshot {
    pub schema_version: u32,
    pub sampled_at_epoch_ms: u64,
    pub physical_memory_bytes: Option<u64>,
    pub effective_memory_limit_bytes: Option<u64>,
    pub memory_available_bytes: Option<u64>,
    pub memory_current_bytes: Option<u64>,
    pub swap_used_bytes: Option<u64>,
    pub cpu_parallelism: usize,
    pub cgroup_version: Option<u8>,
    pub cgroup_events: CgroupMemoryEvents,
    pub memory_pressure: MemoryPressureSample,
    pub memory_source: &'static str,
    pub pressure_source: &'static str,
}

impl HostResourceSnapshot {
    pub fn collect() -> Self {
        collect_platform_snapshot()
    }

    pub fn effective_memory_mib(&self) -> Option<u64> {
        self.effective_memory_limit_bytes.map(|value| value / MIB)
    }

    pub fn available_memory_mib(&self) -> Option<u64> {
        self.memory_available_bytes.map(|value| value / MIB)
    }

    pub fn available_ratio(&self) -> Option<f64> {
        let limit = self.effective_memory_limit_bytes?;
        let available = self.memory_available_bytes?;
        (limit > 0).then(|| (available as f64 / limit as f64).clamp(0.0, 1.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourcePressureState {
    Normal,
    Compact,
    Constrained,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourcePressurePolicy {
    pub critical_available_floor_bytes: u64,
    pub critical_available_ratio: f64,
    pub constrained_available_ratio: f64,
    pub compact_available_ratio: f64,
    pub critical_psi_avg10: f64,
    pub constrained_psi_avg10: f64,
    pub compact_psi_avg10: f64,
    pub escalation_samples: u8,
    pub recovery_samples: u8,
}

impl Default for ResourcePressurePolicy {
    fn default() -> Self {
        Self {
            critical_available_floor_bytes: 128 * MIB,
            critical_available_ratio: 0.08,
            constrained_available_ratio: 0.18,
            compact_available_ratio: 0.30,
            critical_psi_avg10: 20.0,
            constrained_psi_avg10: 10.0,
            compact_psi_avg10: 2.0,
            escalation_samples: 2,
            recovery_samples: 6,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResourcePressureTracker {
    policy: ResourcePressurePolicy,
    current: ResourcePressureState,
    pending: Option<(ResourcePressureState, u8)>,
    previous_events: CgroupMemoryEvents,
    events_initialized: bool,
}

impl ResourcePressureTracker {
    pub fn new(policy: ResourcePressurePolicy) -> Self {
        Self {
            policy,
            current: ResourcePressureState::Normal,
            pending: None,
            previous_events: CgroupMemoryEvents::default(),
            events_initialized: false,
        }
    }

    pub fn state(&self) -> ResourcePressureState {
        self.current
    }

    pub fn observe(&mut self, snapshot: &HostResourceSnapshot) -> ResourcePressureState {
        let event_escalation = self.events_initialized
            && snapshot
                .cgroup_events
                .has_new_critical_event_since(&self.previous_events);
        self.previous_events = snapshot.cgroup_events.clone();
        self.events_initialized = true;
        let target = if event_escalation {
            ResourcePressureState::Critical
        } else {
            classify_pressure(snapshot, self.policy)
        };
        if target == self.current {
            self.pending = None;
            return self.current;
        }
        if event_escalation {
            self.current = ResourcePressureState::Critical;
            self.pending = None;
            return self.current;
        }
        let required = if target > self.current {
            self.policy.escalation_samples.max(1)
        } else {
            self.policy.recovery_samples.max(1)
        };
        let count = match self.pending {
            Some((pending, count)) if pending == target => count.saturating_add(1),
            _ => 1,
        };
        if count >= required {
            self.current = target;
            self.pending = None;
        } else {
            self.pending = Some((target, count));
        }
        self.current
    }
}

impl Default for ResourcePressureTracker {
    fn default() -> Self {
        Self::new(ResourcePressurePolicy::default())
    }
}

pub fn classify_pressure(
    snapshot: &HostResourceSnapshot,
    policy: ResourcePressurePolicy,
) -> ResourcePressureState {
    let available_ratio = snapshot.available_ratio();
    let available = snapshot.memory_available_bytes;
    let psi = snapshot
        .memory_pressure
        .full_avg10
        .or(snapshot.memory_pressure.some_avg10);

    if available.is_some_and(|value| value <= policy.critical_available_floor_bytes)
        || available_ratio.is_some_and(|value| value <= policy.critical_available_ratio)
        || psi.is_some_and(|value| value >= policy.critical_psi_avg10)
    {
        return ResourcePressureState::Critical;
    }
    if snapshot
        .effective_memory_limit_bytes
        .is_some_and(|value| value <= 2 * 1024 * MIB)
        || available_ratio.is_some_and(|value| value <= policy.constrained_available_ratio)
        || psi.is_some_and(|value| value >= policy.constrained_psi_avg10)
    {
        return ResourcePressureState::Constrained;
    }
    if snapshot
        .effective_memory_limit_bytes
        .is_some_and(|value| value <= 4 * 1024 * MIB)
        || available_ratio.is_some_and(|value| value <= policy.compact_available_ratio)
        || psi.is_some_and(|value| value >= policy.compact_psi_avg10)
    {
        ResourcePressureState::Compact
    } else {
        ResourcePressureState::Normal
    }
}

#[cfg(target_os = "linux")]
fn collect_platform_snapshot() -> HostResourceSnapshot {
    let meminfo = read_bounded_text(Path::new("/proc/meminfo")).unwrap_or_default();
    let memory = parse_linux_meminfo(&meminfo);
    let cgroup = collect_linux_cgroup();
    let effective_memory_limit_bytes =
        minimum_known(memory.physical_memory_bytes, cgroup.memory_limit_bytes);
    let cgroup_available = match (cgroup.memory_limit_bytes, cgroup.memory_current_bytes) {
        (Some(limit), Some(current)) => Some(limit.saturating_sub(current)),
        _ => None,
    };
    let memory_available_bytes = minimum_known(memory.memory_available_bytes, cgroup_available)
        .map(|available| {
            effective_memory_limit_bytes.map_or(available, |limit| available.min(limit))
        });
    let pressure = cgroup
        .pressure
        .clone()
        .or_else(|| {
            read_bounded_text(Path::new("/proc/pressure/memory"))
                .map(|raw| parse_linux_pressure(&raw))
        })
        .unwrap_or_default();
    let pressure_source = if cgroup.pressure.is_some() {
        "cgroup"
    } else if pressure != MemoryPressureSample::default() {
        "host"
    } else {
        "unavailable"
    };
    let memory_source = if cgroup.memory_limit_bytes.is_some() {
        "host_and_cgroup"
    } else if memory.physical_memory_bytes.is_some() {
        "host"
    } else {
        "unavailable"
    };
    HostResourceSnapshot {
        schema_version: 1,
        sampled_at_epoch_ms: now_epoch_ms(),
        physical_memory_bytes: memory.physical_memory_bytes,
        effective_memory_limit_bytes,
        memory_available_bytes,
        memory_current_bytes: cgroup.memory_current_bytes,
        swap_used_bytes: cgroup.swap_used_bytes.or(memory.swap_used_bytes),
        cpu_parallelism: cpu_parallelism(),
        cgroup_version: cgroup.version,
        cgroup_events: cgroup.events,
        memory_pressure: pressure,
        memory_source,
        pressure_source,
    }
}

#[cfg(target_os = "macos")]
fn collect_platform_snapshot() -> HostResourceSnapshot {
    let physical_memory_bytes = command_output("/usr/sbin/sysctl", &["-n", "hw.memsize"])
        .and_then(|value| value.parse::<u64>().ok());
    let page_size = command_output("/usr/sbin/sysctl", &["-n", "hw.pagesize"])
        .and_then(|value| value.parse::<u64>().ok());
    let memory_available_bytes = match (command_output("/usr/bin/vm_stat", &[]), page_size) {
        (Some(raw), Some(page_size)) => parse_macos_available_memory(&raw, page_size),
        _ => None,
    };
    HostResourceSnapshot {
        schema_version: 1,
        sampled_at_epoch_ms: now_epoch_ms(),
        physical_memory_bytes,
        effective_memory_limit_bytes: physical_memory_bytes,
        memory_available_bytes,
        memory_current_bytes: match (physical_memory_bytes, memory_available_bytes) {
            (Some(total), Some(available)) => Some(total.saturating_sub(available)),
            _ => None,
        },
        swap_used_bytes: parse_macos_swap_used(
            &command_output("/usr/sbin/sysctl", &["-n", "vm.swapusage"]).unwrap_or_default(),
        ),
        cpu_parallelism: cpu_parallelism(),
        cgroup_version: None,
        cgroup_events: CgroupMemoryEvents::default(),
        memory_pressure: MemoryPressureSample::default(),
        memory_source: if physical_memory_bytes.is_some() {
            "host"
        } else {
            "unavailable"
        },
        pressure_source: "unavailable",
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn collect_platform_snapshot() -> HostResourceSnapshot {
    HostResourceSnapshot {
        schema_version: 1,
        sampled_at_epoch_ms: now_epoch_ms(),
        physical_memory_bytes: None,
        effective_memory_limit_bytes: None,
        memory_available_bytes: None,
        memory_current_bytes: None,
        swap_used_bytes: None,
        cpu_parallelism: cpu_parallelism(),
        cgroup_version: None,
        cgroup_events: CgroupMemoryEvents::default(),
        memory_pressure: MemoryPressureSample::default(),
        memory_source: "unavailable",
        pressure_source: "unavailable",
    }
}

fn cpu_parallelism() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn minimum_known(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn read_bounded_text(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > TEXT_LIMIT_BYTES {
        return None;
    }
    let value = fs::read_to_string(path).ok()?;
    (value.len() as u64 <= TEXT_LIMIT_BYTES).then_some(value)
}

#[cfg(target_os = "macos")]
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() || output.stdout.len() as u64 > TEXT_LIMIT_BYTES {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
fn command_output(_program: &str, _args: &[&str]) -> Option<String> {
    None
}

#[derive(Default)]
struct LinuxMemoryInfo {
    physical_memory_bytes: Option<u64>,
    memory_available_bytes: Option<u64>,
    swap_used_bytes: Option<u64>,
}

fn parse_linux_meminfo(text: &str) -> LinuxMemoryInfo {
    let mut total = None;
    let mut available = None;
    let mut swap_total = None;
    let mut swap_free = None;
    for line in text.lines().take(256) {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            total = parse_kib(value);
        } else if let Some(value) = line.strip_prefix("MemAvailable:") {
            available = parse_kib(value);
        } else if let Some(value) = line.strip_prefix("SwapTotal:") {
            swap_total = parse_kib(value);
        } else if let Some(value) = line.strip_prefix("SwapFree:") {
            swap_free = parse_kib(value);
        }
    }
    LinuxMemoryInfo {
        physical_memory_bytes: total,
        memory_available_bytes: available,
        swap_used_bytes: match (swap_total, swap_free) {
            (Some(total), Some(free)) => Some(total.saturating_sub(free)),
            _ => None,
        },
    }
}

fn parse_kib(value: &str) -> Option<u64> {
    value
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|value| value.saturating_mul(1024))
}

#[derive(Default)]
struct LinuxCgroupSnapshot {
    version: Option<u8>,
    memory_limit_bytes: Option<u64>,
    memory_current_bytes: Option<u64>,
    swap_used_bytes: Option<u64>,
    events: CgroupMemoryEvents,
    pressure: Option<MemoryPressureSample>,
}

#[cfg(target_os = "linux")]
fn collect_linux_cgroup() -> LinuxCgroupSnapshot {
    let membership = read_bounded_text(Path::new("/proc/self/cgroup")).unwrap_or_default();
    if let Some(relative) = parse_cgroup_v2_path(&membership) {
        for directory in cgroup_directories(Path::new("/sys/fs/cgroup"), &relative) {
            if directory.join("cgroup.controllers").exists()
                || directory.join("memory.max").exists()
            {
                return collect_cgroup_v2_dir(&directory);
            }
        }
    }
    if let Some(relative) = parse_cgroup_v1_memory_path(&membership) {
        for root in [
            Path::new("/sys/fs/cgroup/memory"),
            Path::new("/sys/fs/cgroup"),
        ] {
            for directory in cgroup_directories(root, &relative) {
                if directory.join("memory.limit_in_bytes").exists() {
                    return collect_cgroup_v1_dir(&directory);
                }
            }
        }
    }
    LinuxCgroupSnapshot::default()
}

#[cfg(not(target_os = "linux"))]
fn collect_linux_cgroup() -> LinuxCgroupSnapshot {
    LinuxCgroupSnapshot::default()
}

fn cgroup_directories(root: &Path, relative: &Path) -> Vec<PathBuf> {
    let joined = root.join(relative.strip_prefix("/").unwrap_or(relative));
    if joined == root {
        vec![root.to_path_buf()]
    } else {
        vec![joined, root.to_path_buf()]
    }
}

fn parse_cgroup_v2_path(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let mut fields = line.splitn(3, ':');
        (fields.next()? == "0" && fields.next()?.is_empty())
            .then(|| PathBuf::from(fields.next().unwrap_or("/")))
    })
}

fn parse_cgroup_v1_memory_path(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let mut fields = line.splitn(3, ':');
        let _hierarchy = fields.next()?;
        let controllers = fields.next()?;
        let path = fields.next()?;
        controllers
            .split(',')
            .any(|controller| controller == "memory")
            .then(|| PathBuf::from(path))
    })
}

fn collect_cgroup_v2_dir(directory: &Path) -> LinuxCgroupSnapshot {
    LinuxCgroupSnapshot {
        version: Some(2),
        memory_limit_bytes: read_limit_file(&directory.join("memory.max")),
        memory_current_bytes: read_u64_file(&directory.join("memory.current")),
        swap_used_bytes: read_u64_file(&directory.join("memory.swap.current")),
        events: read_bounded_text(&directory.join("memory.events"))
            .map(|value| parse_cgroup_events(&value))
            .unwrap_or_default(),
        pressure: read_bounded_text(&directory.join("memory.pressure"))
            .map(|value| parse_linux_pressure(&value)),
    }
}

fn collect_cgroup_v1_dir(directory: &Path) -> LinuxCgroupSnapshot {
    let memory_current_bytes = read_u64_file(&directory.join("memory.usage_in_bytes"));
    let memsw_current = read_u64_file(&directory.join("memory.memsw.usage_in_bytes"));
    LinuxCgroupSnapshot {
        version: Some(1),
        memory_limit_bytes: read_limit_file(&directory.join("memory.limit_in_bytes")),
        memory_current_bytes,
        swap_used_bytes: match (memsw_current, memory_current_bytes) {
            (Some(memsw), Some(memory)) => Some(memsw.saturating_sub(memory)),
            _ => None,
        },
        ..LinuxCgroupSnapshot::default()
    }
}

fn read_u64_file(path: &Path) -> Option<u64> {
    read_bounded_text(path)?.trim().parse::<u64>().ok()
}

fn read_limit_file(path: &Path) -> Option<u64> {
    let value = read_bounded_text(path)?;
    let value = value.trim();
    if value == "max" {
        return None;
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0 && *value < UNLIMITED_CGROUP_THRESHOLD_BYTES)
}

fn parse_cgroup_events(text: &str) -> CgroupMemoryEvents {
    let mut events = CgroupMemoryEvents::default();
    for line in text.lines().take(32) {
        let mut fields = line.split_whitespace();
        let Some(key) = fields.next() else {
            continue;
        };
        let Some(value) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        match key {
            "low" => events.low = value,
            "high" => events.high = value,
            "max" => events.max = value,
            "oom" => events.oom = value,
            "oom_kill" => events.oom_kill = value,
            "oom_group_kill" => events.oom_group_kill = value,
            _ => {}
        }
    }
    events
}

fn parse_linux_pressure(text: &str) -> MemoryPressureSample {
    let mut sample = MemoryPressureSample::default();
    for line in text.lines().take(4) {
        let mut fields = line.split_whitespace();
        let Some(scope) = fields.next() else {
            continue;
        };
        let mut avg10 = None;
        let mut avg60 = None;
        for field in fields {
            if let Some(value) = field.strip_prefix("avg10=") {
                avg10 = value.parse::<f64>().ok().filter(|value| value.is_finite());
            } else if let Some(value) = field.strip_prefix("avg60=") {
                avg60 = value.parse::<f64>().ok().filter(|value| value.is_finite());
            }
        }
        match scope {
            "some" => {
                sample.some_avg10 = avg10;
                sample.some_avg60 = avg60;
            }
            "full" => {
                sample.full_avg10 = avg10;
                sample.full_avg60 = avg60;
            }
            _ => {}
        }
    }
    sample
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_available_memory(text: &str, page_size: u64) -> Option<u64> {
    let free = parse_macos_pages(text, "Pages free")?;
    let inactive = parse_macos_pages(text, "Pages inactive").unwrap_or(0);
    let speculative = parse_macos_pages(text, "Pages speculative").unwrap_or(0);
    let purgeable = parse_macos_pages(text, "Pages purgeable").unwrap_or(0);
    Some(
        free.saturating_add(inactive)
            .saturating_add(speculative)
            .saturating_add(purgeable)
            .saturating_mul(page_size),
    )
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_pages(text: &str, key: &str) -> Option<u64> {
    text.lines().take(256).find_map(|line| {
        line.trim()
            .strip_prefix(key)?
            .strip_prefix(':')?
            .trim()
            .trim_end_matches('.')
            .parse::<u64>()
            .ok()
    })
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_swap_used(text: &str) -> Option<u64> {
    let (_, after_used) = text.split_once("used")?;
    let value = after_used
        .trim_start()
        .strip_prefix('=')?
        .split_whitespace()
        .next()?;
    parse_human_bytes(value)
}

#[cfg(any(target_os = "macos", test))]
fn parse_human_bytes(value: &str) -> Option<u64> {
    let split = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(value.len());
    let number = value[..split].parse::<f64>().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    let unit = value[split..].trim().to_ascii_uppercase();
    let multiplier = match unit.as_str() {
        "B" | "" => 1.0,
        "K" | "KB" => 1024.0,
        "M" | "MB" => (1024 * 1024) as f64,
        "G" | "GB" => (1024_u64 * 1024 * 1024) as f64,
        "T" | "TB" => (1024_u64 * 1024 * 1024 * 1024) as f64,
        _ => return None,
    };
    Some((number * multiplier) as u64)
}

#[cfg(test)]
#[path = "host_resources_tests.rs"]
mod tests;

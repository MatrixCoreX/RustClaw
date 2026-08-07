use claw_core::skill_registry::{SkillResourceClass, SkillResourceRequest};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceGrant {
    pub(crate) admitted: bool,
    pub(crate) max_concurrency: usize,
    pub(crate) wait_reason: Option<&'static str>,
    pub(crate) projection: Value,
}

pub(crate) fn host_grant(
    request: Option<&SkillResourceRequest>,
    safety_ceiling: usize,
) -> ResourceGrant {
    let cpu_total = std::thread::available_parallelism().map_or(1, usize::from);
    let memory_available_mb = available_memory_mb().unwrap_or(1024);
    let gpu_available = gpu_device_available();
    let request = request.cloned().unwrap_or_default();
    let cpu_cores = request.cpu_cores.max(1);
    let memory_mb = request.memory_mb.max(128);
    let gpu_required = request.gpu_slots > 0 || request.class == SkillResourceClass::Gpu;
    let fallback = gpu_required && !gpu_available && request.allow_cpu_fallback;
    let memory_sufficient = memory_available_mb >= memory_mb;
    let admitted = (!gpu_required || gpu_available || fallback) && memory_sufficient;
    let granted_cpu_cores = cpu_cores.min(cpu_total);
    let cpu_slots = (cpu_total / granted_cpu_cores).max(1);
    let memory_slots = (memory_available_mb / memory_mb).max(1) as usize;
    let max_concurrency = safety_ceiling
        .max(1)
        .min(cpu_slots)
        .min(memory_slots)
        .max(1);
    let wait_reason = if !memory_sufficient {
        Some("memory_unavailable")
    } else if !admitted {
        Some("gpu_unavailable_no_fallback")
    } else {
        None
    };
    ResourceGrant {
        admitted,
        max_concurrency,
        wait_reason,
        projection: json!({
            "schema_version": 1,
            "resource_class": request.class.as_token(),
            "request": {
                "cpu_cores": cpu_cores,
                "memory_mb": memory_mb,
                "gpu_slots": request.gpu_slots,
                "disk_io_weight": request.disk_io_weight,
                "network_slots": request.network_slots,
                "provider_slots": request.provider_slots,
            },
            "grant": {
                "cpu_cores": granted_cpu_cores,
                "memory_mb": memory_mb,
                "gpu_slots": usize::from(gpu_required && gpu_available),
            },
            "host": {
                "cpu_cores": cpu_total,
                "memory_available_mb": memory_available_mb,
                "gpu_available": gpu_available,
            },
            "admitted": admitted,
            "fallback": fallback.then_some("cpu"),
            "max_concurrency": max_concurrency,
            "wait_reason": wait_reason,
        }),
    }
}

fn available_memory_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read_to_string("/proc/meminfo").ok()?;
        return raw.lines().find_map(|line| {
            let value = line.strip_prefix("MemAvailable:")?;
            value
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
                .map(|kb| kb / 1024)
        });
    }
    #[cfg(not(target_os = "linux"))]
    None
}

fn gpu_device_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/dev/nvidia0").exists()
            || std::path::Path::new("/dev/dri/renderD128").exists()
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    false
}

#[cfg(test)]
#[path = "resource_scheduler_tests.rs"]
mod tests;

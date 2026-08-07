use serde::Deserialize;

/// Host-owned queue scope for resource-heavy skill invocations. Missing queue
/// policy keeps the existing dispatch behavior unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillDispatchQueueScope {
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDispatchQueuePolicy {
    pub scope: SkillDispatchQueueScope,
    #[serde(default)]
    pub actions: Vec<String>,
}

impl SkillDispatchQueuePolicy {
    pub fn applies_to(&self, action: Option<&str>) -> bool {
        if self.actions.is_empty() {
            return true;
        }
        let Some(action) = action
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(super::normalize_schema_token)
        else {
            return false;
        };
        self.actions.iter().any(|candidate| candidate == &action)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillResourceClass {
    #[default]
    General,
    Cpu,
    Memory,
    Gpu,
    DiskIo,
    Network,
    ProviderQuota,
}

impl SkillResourceClass {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Gpu => "gpu",
            Self::DiskIo => "disk_io",
            Self::Network => "network",
            Self::ProviderQuota => "provider_quota",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SkillResourceRequest {
    #[serde(default)]
    pub class: SkillResourceClass,
    #[serde(default)]
    pub cpu_cores: usize,
    #[serde(default)]
    pub memory_mb: u64,
    #[serde(default)]
    pub gpu_slots: usize,
    #[serde(default)]
    pub disk_io_weight: u8,
    #[serde(default)]
    pub network_slots: usize,
    #[serde(default)]
    pub provider_slots: usize,
    #[serde(default)]
    pub allow_cpu_fallback: bool,
}

pub(super) fn validate_resource_request(
    request: &SkillResourceRequest,
) -> Result<(), &'static str> {
    if request.cpu_cores > 4_096 {
        return Err("cpu_cores_out_of_range");
    }
    if request.memory_mb > 1_048_576 {
        return Err("memory_mb_out_of_range");
    }
    if request.gpu_slots > 64 {
        return Err("gpu_slots_out_of_range");
    }
    if request.disk_io_weight > 100 {
        return Err("disk_io_weight_out_of_range");
    }
    if request.network_slots > 4_096 || request.provider_slots > 4_096 {
        return Err("resource_slots_out_of_range");
    }
    Ok(())
}

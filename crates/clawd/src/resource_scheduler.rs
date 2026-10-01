use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use claw_core::host_resources::{
    HostResourceSnapshot, ResourcePressurePolicy, ResourcePressureState, ResourcePressureTracker,
};
use claw_core::skill_registry::{SkillResourceClass, SkillResourceRequest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const LOW_MEMORY_HOST_MAX_MIB: u64 = 2 * 1024;
const CONSTRAINED_MEMORY_HOST_MAX_MIB: u64 = 4 * 1024;
const DEFAULT_SKILL_MEMORY_MIB: u64 = 128;
const BROWSER_SLOT_MEMORY_FLOOR_MIB: u64 = 384;
const RESOURCE_ESTIMATE_CACHE_SCHEMA_VERSION: u32 = 1;
const RESOURCE_ESTIMATE_MIN_SAMPLES: u64 = 3;
const RESOURCE_ESTIMATE_MAX_ENTRIES: usize = 512;
const RESOURCE_ESTIMATE_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const RESOURCE_PRESSURE_LOG_MAX_BYTES: u64 = 1024 * 1024;
const RUNTIME_PROCESS_MEMORY_WARNING_PERCENT: u64 = 75;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResourceEstimateKey {
    pub(crate) scope: String,
    pub(crate) action: String,
    pub(crate) registry_generation: u64,
    pub(crate) registry_generation_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedResourceEstimate {
    key: ResourceEstimateKey,
    peak_memory_mib: u64,
    sample_count: u64,
    updated_at_epoch: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedResourceEstimates {
    schema_version: u32,
    entries: Vec<ObservedResourceEstimate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeConcurrencyPlan {
    pub(crate) worker_concurrency: usize,
    pub(crate) skill_concurrency: usize,
    pub(crate) memory_background_concurrency: usize,
    pub(crate) runner_warm_pool_enabled: bool,
    pub(crate) cpu_total: usize,
    pub(crate) memory_total_mib: Option<u64>,
}

pub(crate) fn runtime_concurrency_plan(
    configured_workers: usize,
    configured_skills: usize,
    configured_memory_background: usize,
    configured_runner_warm_pool: bool,
) -> RuntimeConcurrencyPlan {
    let snapshot = HostResourceSnapshot::collect();
    runtime_concurrency_plan_for_host(
        configured_workers,
        configured_skills,
        configured_memory_background,
        configured_runner_warm_pool,
        snapshot.cpu_parallelism,
        snapshot.effective_memory_mib(),
    )
}

fn runtime_concurrency_plan_for_host(
    configured_workers: usize,
    configured_skills: usize,
    configured_memory_background: usize,
    configured_runner_warm_pool: bool,
    cpu_total: usize,
    memory_total_mib: Option<u64>,
) -> RuntimeConcurrencyPlan {
    let cpu_total = cpu_total.max(1);
    let configured_workers = configured_workers.max(1);
    let configured_skills = configured_skills.max(1);
    let configured_memory_background = configured_memory_background.max(1);
    let (foreground_ceiling, background_ceiling, warm_pool_allowed) = match memory_total_mib {
        Some(total) if total <= LOW_MEMORY_HOST_MAX_MIB => (1, 1, false),
        Some(total) if total <= CONSTRAINED_MEMORY_HOST_MAX_MIB => (2, 1, false),
        _ => (cpu_total, cpu_total, true),
    };
    RuntimeConcurrencyPlan {
        worker_concurrency: configured_workers.min(cpu_total).min(foreground_ceiling),
        skill_concurrency: configured_skills.min(cpu_total).min(foreground_ceiling),
        memory_background_concurrency: configured_memory_background
            .min(cpu_total)
            .min(background_ceiling),
        runner_warm_pool_enabled: configured_runner_warm_pool && warm_pool_allowed,
        cpu_total,
        memory_total_mib,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceGrant {
    pub(crate) admitted: bool,
    pub(crate) max_concurrency: usize,
    pub(crate) wait_reason: Option<&'static str>,
    pub(crate) projection: Value,
}

#[derive(Debug, Clone, Copy)]
struct ReservedResources {
    memory_mib: u64,
    cpu_cores: usize,
    network_slots: usize,
    provider_slots: usize,
    browser_slots: usize,
    heavy: bool,
    background: bool,
}

#[derive(Debug)]
struct ResourceBrokerState {
    next_lease_id: u64,
    leases: BTreeMap<u64, ReservedResources>,
    pressure: ResourcePressureTracker,
    recent_admission_refusal_reason: Option<&'static str>,
    recent_admission_refusal_at_epoch: Option<u64>,
    observed_estimates: BTreeMap<ResourceEstimateKey, ObservedResourceEstimate>,
    runtime_process_memory: Option<crate::runtime_process_memory::RuntimeProcessMemoryStatus>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResourceBroker {
    state: Arc<Mutex<ResourceBrokerState>>,
    safety_reserve_mib: Option<u64>,
    estimate_store_path: Option<Arc<PathBuf>>,
    estimate_store_write: Arc<Mutex<()>>,
}

#[derive(Debug)]
pub(crate) struct ResourceLease {
    broker: ResourceBroker,
    lease_id: u64,
    grant: ResourceGrant,
    reserved: ReservedResources,
    estimate_key: Option<ResourceEstimateKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableResourceLeaseMetadata {
    schema_version: u32,
    lease_token: String,
    memory_mib: u64,
    cpu_cores: usize,
    #[serde(default)]
    network_slots: usize,
    #[serde(default)]
    provider_slots: usize,
    #[serde(default)]
    browser_slots: usize,
    heavy: bool,
    #[serde(default)]
    background: bool,
    #[serde(default)]
    estimate_key: Option<ResourceEstimateKey>,
    pid: u32,
    created_at_epoch: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResourceBrokerStatus {
    pub(crate) pressure_state: ResourcePressureState,
    pub(crate) reserved_memory_mib: u64,
    pub(crate) active_leases: usize,
    pub(crate) reserved_cpu_cores: usize,
    pub(crate) reserved_network_slots: usize,
    pub(crate) reserved_provider_slots: usize,
    pub(crate) reserved_browser_slots: usize,
    pub(crate) active_heavy_leases: usize,
    pub(crate) recent_admission_refusal_reason: Option<&'static str>,
    pub(crate) recent_admission_refusal_at_epoch: Option<u64>,
    pub(crate) snapshot: HostResourceSnapshot,
    pub(crate) runtime_process_memory:
        Option<crate::runtime_process_memory::RuntimeProcessMemoryStatus>,
}

impl ResourceLease {
    pub(crate) fn grant(&self) -> &ResourceGrant {
        &self.grant
    }

    pub(crate) async fn hold_until_durable_job_terminal(
        mut self,
        job_dir: PathBuf,
    ) -> Result<(), String> {
        let pid = crate::local_process_job::read_pid(&job_dir)
            .ok_or_else(|| "durable_resource_lease_pid_missing".to_string())?;
        if !await_durable_process_identity_or_terminal(
            &job_dir,
            pid,
            durable_resource_identity_handoff_timeout(),
        )
        .await?
        {
            return Ok(());
        }
        if let Some(memory_mib) = crate::skills::runner_pool::process_tree_resident_memory_mib(pid)
        {
            self.refresh_materialized_memory(memory_mib);
        }
        self.mark_background();
        let metadata = DurableResourceLeaseMetadata {
            schema_version: 1,
            lease_token: uuid::Uuid::new_v4().to_string(),
            memory_mib: self.reserved.memory_mib,
            cpu_cores: self.reserved.cpu_cores,
            network_slots: self.reserved.network_slots,
            provider_slots: self.reserved.provider_slots,
            browser_slots: self.reserved.browser_slots,
            heavy: self.reserved.heavy,
            background: self.reserved.background,
            estimate_key: self.estimate_key.clone(),
            pid,
            created_at_epoch: crate::now_ts_u64(),
        };
        let payload = serde_json::to_string(&metadata)
            .map_err(|error| format!("durable_resource_lease_encode_failed: {error}"))?;
        if let Err(error) =
            crate::local_process_job::write_atomic(&job_dir.join("resource_lease.json"), &payload)
        {
            let _ =
                crate::local_process_job::terminate_verified_process_group(&job_dir, pid, "TERM");
            return Err(format!("durable_resource_lease_write_failed: {error}"));
        }
        spawn_durable_resource_lease_monitor(self, job_dir);
        Ok(())
    }

    /// Keep only the unmaterialized part of the grant reserved. The host
    /// snapshot already includes resident process memory, so retaining the
    /// full grant after a child has started would count the same memory twice.
    pub(crate) fn refresh_materialized_memory(&self, observed_memory_mib: u64) {
        let outstanding_memory_mib = self.reserved.memory_mib.saturating_sub(observed_memory_mib);
        if let Some(reserved) = self
            .broker
            .state
            .lock()
            .unwrap()
            .leases
            .get_mut(&self.lease_id)
        {
            reserved.memory_mib = outstanding_memory_mib;
        }
    }

    fn mark_background(&mut self) {
        self.reserved.background = true;
        if let Some(reserved) = self
            .broker
            .state
            .lock()
            .unwrap()
            .leases
            .get_mut(&self.lease_id)
        {
            reserved.background = true;
        }
    }
}

async fn await_durable_process_identity_or_terminal(
    job_dir: &Path,
    pid: u32,
    timeout: Duration,
) -> Result<bool, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if job_dir.join("exit_code").is_file() {
            return Ok(false);
        }
        if crate::local_process_job::process_identity_state(job_dir, pid)
            == crate::local_process_job::ProcessIdentityState::AliveVerified
        {
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("durable_resource_lease_process_identity_unverified".to_string());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn durable_resource_identity_handoff_timeout() -> Duration {
    Duration::from_secs(5)
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        let mut state = self.broker.state.lock().unwrap();
        state.leases.remove(&self.lease_id);
    }
}

impl Default for ResourceBroker {
    fn default() -> Self {
        Self::new(ResourcePressurePolicy::default(), None)
    }
}

impl ResourceBroker {
    pub(crate) fn new(
        pressure_policy: ResourcePressurePolicy,
        safety_reserve_mib: Option<u64>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(ResourceBrokerState {
                next_lease_id: 0,
                leases: BTreeMap::new(),
                pressure: ResourcePressureTracker::new(pressure_policy),
                recent_admission_refusal_reason: None,
                recent_admission_refusal_at_epoch: None,
                observed_estimates: BTreeMap::new(),
                runtime_process_memory: None,
            })),
            safety_reserve_mib,
            estimate_store_path: None,
            estimate_store_write: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn with_estimate_store_path(mut self, path: PathBuf) -> Self {
        let estimates = load_resource_estimates(&path);
        self.state.lock().unwrap().observed_estimates = estimates;
        self.estimate_store_path = Some(Arc::new(path));
        self
    }

    pub(crate) fn try_acquire(
        &self,
        request: Option<&SkillResourceRequest>,
        safety_ceiling: usize,
    ) -> Result<ResourceLease, ResourceGrant> {
        self.try_acquire_for_snapshot(
            request,
            safety_ceiling,
            HostResourceSnapshot::collect(),
            gpu_device_available(),
        )
    }

    pub(crate) fn try_acquire_for_estimate_key(
        &self,
        request: Option<&SkillResourceRequest>,
        safety_ceiling: usize,
        estimate_key: Option<&ResourceEstimateKey>,
    ) -> Result<ResourceLease, ResourceGrant> {
        self.try_acquire_for_snapshot_and_key(
            request,
            safety_ceiling,
            HostResourceSnapshot::collect(),
            gpu_device_available(),
            estimate_key,
            false,
        )
    }

    pub(crate) fn try_acquire_background_for_estimate_key(
        &self,
        request: Option<&SkillResourceRequest>,
        safety_ceiling: usize,
        estimate_key: Option<&ResourceEstimateKey>,
    ) -> Result<ResourceLease, ResourceGrant> {
        self.try_acquire_for_snapshot_and_key(
            request,
            safety_ceiling,
            HostResourceSnapshot::collect(),
            gpu_device_available(),
            estimate_key,
            true,
        )
    }

    fn try_acquire_for_snapshot(
        &self,
        request: Option<&SkillResourceRequest>,
        safety_ceiling: usize,
        snapshot: HostResourceSnapshot,
        gpu_available: bool,
    ) -> Result<ResourceLease, ResourceGrant> {
        self.try_acquire_for_snapshot_and_key(
            request,
            safety_ceiling,
            snapshot,
            gpu_available,
            None,
            false,
        )
    }

    fn try_acquire_for_snapshot_and_key(
        &self,
        request: Option<&SkillResourceRequest>,
        safety_ceiling: usize,
        snapshot: HostResourceSnapshot,
        gpu_available: bool,
        estimate_key: Option<&ResourceEstimateKey>,
        background: bool,
    ) -> Result<ResourceLease, ResourceGrant> {
        let mut state = self.state.lock().unwrap();
        let pressure = state.pressure.observe(&snapshot);
        let reserved_before = state
            .leases
            .values()
            .map(|lease| lease.memory_mib)
            .sum::<u64>();
        let reserved_cpu_before = state
            .leases
            .values()
            .map(|lease| lease.cpu_cores)
            .sum::<usize>();
        let reserved_network_before = state
            .leases
            .values()
            .map(|lease| lease.network_slots)
            .sum::<usize>();
        let reserved_provider_before = state
            .leases
            .values()
            .map(|lease| lease.provider_slots)
            .sum::<usize>();
        let reserved_browser_before = state
            .leases
            .values()
            .map(|lease| lease.browser_slots)
            .sum::<usize>();
        let active_heavy_leases = state.leases.values().filter(|lease| lease.heavy).count();
        let active_background_heavy_leases = state
            .leases
            .values()
            .filter(|lease| lease.heavy && lease.background)
            .count();
        let safety_reserve_mib = self
            .safety_reserve_mib
            .unwrap_or_else(|| host_safety_reserve_mib(snapshot.effective_memory_mib()));
        let reservable_memory_mib = snapshot
            .available_memory_mib()
            .unwrap_or(1024)
            .saturating_sub(safety_reserve_mib)
            .saturating_sub(reserved_before);
        let mut budget_snapshot = snapshot.clone();
        budget_snapshot.memory_available_bytes = Some(reservable_memory_mib * 1024 * 1024);
        budget_snapshot.cpu_parallelism =
            snapshot.cpu_parallelism.saturating_sub(reserved_cpu_before);
        let mut request = request.cloned().unwrap_or_default();
        let declared_memory_mib = request.memory_mb;
        let observed_memory_mib =
            estimate_key.and_then(|key| observed_memory_estimate(&state.observed_estimates, key));
        request.memory_mb = request
            .memory_mb
            .max(observed_memory_mib.unwrap_or_default());
        let network_capacity = network_slot_capacity(&snapshot);
        let provider_capacity = provider_slot_capacity(&snapshot);
        let browser_capacity = browser_slot_capacity(&snapshot);
        let available_network_slots = network_capacity.saturating_sub(reserved_network_before);
        let available_provider_slots = provider_capacity.saturating_sub(reserved_provider_before);
        let available_browser_slots = browser_capacity.saturating_sub(reserved_browser_before);
        let granted_browser_slots = request.browser_slots.min(available_browser_slots);
        let mut budget_request = request.clone();
        budget_request.browser_slots = granted_browser_slots;
        let mut grant = host_grant_for_snapshot(
            Some(&budget_request),
            safety_ceiling,
            &budget_snapshot,
            gpu_available,
        );
        let heavy = resource_request_is_heavy(&request);
        let pressure_blocks_request = heavy
            && (pressure == ResourcePressureState::Critical
                || (pressure == ResourcePressureState::Constrained && active_heavy_leases > 0)
                || (pressure == ResourcePressureState::Compact
                    && background
                    && active_background_heavy_leases > 0));
        if pressure_blocks_request {
            grant.admitted = false;
            grant.wait_reason = Some("resource_pressure");
        } else if request.network_slots > available_network_slots {
            grant.admitted = false;
            grant.wait_reason = Some("network_slot_unavailable");
        } else if request.provider_slots > available_provider_slots {
            grant.admitted = false;
            grant.wait_reason = Some("provider_slot_unavailable");
        } else if request.browser_slots > 0 && granted_browser_slots == 0 {
            grant.admitted = false;
            grant.wait_reason = Some("browser_slot_unavailable");
        }
        if let Some(object) = grant.projection.as_object_mut() {
            object.insert("resource_pressure_state".to_string(), json!(pressure));
            object.insert(
                "host_memory_available_mb".to_string(),
                json!(snapshot.available_memory_mib()),
            );
            object.insert(
                "host_safety_reserve_mb".to_string(),
                json!(safety_reserve_mib),
            );
            object.insert(
                "reserved_memory_mb_before".to_string(),
                json!(reserved_before),
            );
            object.insert(
                "reserved_cpu_cores_before".to_string(),
                json!(reserved_cpu_before),
            );
            object.insert(
                "reserved_network_slots_before".to_string(),
                json!(reserved_network_before),
            );
            object.insert(
                "reserved_provider_slots_before".to_string(),
                json!(reserved_provider_before),
            );
            object.insert(
                "reserved_browser_slots_before".to_string(),
                json!(reserved_browser_before),
            );
            object.insert(
                "slot_capacity".to_string(),
                json!({
                    "network": network_capacity,
                    "provider": provider_capacity,
                    "browser": browser_capacity,
                }),
            );
            object.insert("resource_heavy".to_string(), json!(heavy));
            object.insert("resource_background".to_string(), json!(background));
            if let Some(request_projection) = object
                .get_mut("request")
                .and_then(serde_json::Value::as_object_mut)
            {
                request_projection
                    .insert("declared_memory_mb".to_string(), json!(declared_memory_mib));
                request_projection.insert(
                    "observed_estimate_memory_mb".to_string(),
                    json!(observed_memory_mib),
                );
                request_projection
                    .insert("browser_slots".to_string(), json!(request.browser_slots));
            }
            if let Some(grant_projection) = object
                .get_mut("grant")
                .and_then(serde_json::Value::as_object_mut)
            {
                grant_projection.insert("network_slots".to_string(), json!(request.network_slots));
                grant_projection
                    .insert("provider_slots".to_string(), json!(request.provider_slots));
                grant_projection.insert("browser_slots".to_string(), json!(granted_browser_slots));
            }
            object.insert("admitted".to_string(), json!(grant.admitted));
            object.insert("wait_reason".to_string(), json!(grant.wait_reason));
        }
        if !grant.admitted {
            state.recent_admission_refusal_reason = grant.wait_reason;
            state.recent_admission_refusal_at_epoch = Some(crate::now_ts_u64());
            return Err(grant);
        }

        let memory_mib = grant.projection["grant"]["memory_mb"]
            .as_u64()
            .unwrap_or(DEFAULT_SKILL_MEMORY_MIB);
        let cpu_cores = grant.projection["grant"]["cpu_cores"].as_u64().unwrap_or(1) as usize;
        state.next_lease_id = state.next_lease_id.wrapping_add(1).max(1);
        let lease_id = state.next_lease_id;
        let reserved = ReservedResources {
            memory_mib,
            cpu_cores,
            network_slots: request.network_slots,
            provider_slots: request.provider_slots,
            browser_slots: granted_browser_slots,
            heavy,
            background,
        };
        state.leases.insert(lease_id, reserved);
        Ok(ResourceLease {
            broker: self.clone(),
            lease_id,
            grant,
            reserved,
            estimate_key: estimate_key.cloned(),
        })
    }

    pub(crate) fn record_observed_peak_memory(
        &self,
        key: ResourceEstimateKey,
        peak_memory_mib: u64,
    ) {
        if !valid_resource_estimate_key(&key) || peak_memory_mib == 0 || peak_memory_mib > 1_048_576
        {
            return;
        }
        let _write_guard = self.estimate_store_write.lock().unwrap();
        let now = crate::now_ts_u64();
        let payload = {
            let mut state = self.state.lock().unwrap();
            let should_persist = {
                let entry = state
                    .observed_estimates
                    .entry(key.clone())
                    .or_insert_with(|| ObservedResourceEstimate {
                        key,
                        peak_memory_mib,
                        sample_count: 0,
                        updated_at_epoch: now,
                    });
                let previous_peak = entry.peak_memory_mib;
                let previous_samples = entry.sample_count;
                entry.peak_memory_mib = entry.peak_memory_mib.max(peak_memory_mib);
                entry.sample_count = entry.sample_count.saturating_add(1);
                entry.updated_at_epoch = now;
                entry.peak_memory_mib != previous_peak
                    || previous_samples < RESOURCE_ESTIMATE_MIN_SAMPLES
            };
            prune_resource_estimates(&mut state.observed_estimates);
            if should_persist {
                Some(encode_resource_estimates(&state.observed_estimates))
            } else {
                None
            }
        };
        let (Some(path), Some(payload)) = (self.estimate_store_path.as_deref(), payload) else {
            return;
        };
        if let Err(error) = persist_resource_estimates(path, &payload) {
            tracing::warn!(error = %error, "resource_estimate_cache_write_failed");
        }
    }

    pub(crate) fn observe_host(&self) -> ResourceBrokerStatus {
        self.observe_snapshot(HostResourceSnapshot::collect())
    }

    pub(crate) fn status(&self) -> ResourceBrokerStatus {
        let snapshot = HostResourceSnapshot::collect();
        let state = self.state.lock().unwrap();
        ResourceBrokerStatus {
            pressure_state: state.pressure.state(),
            reserved_memory_mib: state.leases.values().map(|lease| lease.memory_mib).sum(),
            active_leases: state.leases.len(),
            reserved_cpu_cores: state.leases.values().map(|lease| lease.cpu_cores).sum(),
            reserved_network_slots: state.leases.values().map(|lease| lease.network_slots).sum(),
            reserved_provider_slots: state
                .leases
                .values()
                .map(|lease| lease.provider_slots)
                .sum(),
            reserved_browser_slots: state.leases.values().map(|lease| lease.browser_slots).sum(),
            active_heavy_leases: state.leases.values().filter(|lease| lease.heavy).count(),
            recent_admission_refusal_reason: state.recent_admission_refusal_reason,
            recent_admission_refusal_at_epoch: state.recent_admission_refusal_at_epoch,
            runtime_process_memory: state.runtime_process_memory.clone(),
            snapshot,
        }
    }

    pub(crate) fn record_runtime_process_memory(
        &self,
        sample: crate::runtime_process_memory::RuntimeProcessMemorySample,
        effective_memory_limit_bytes: Option<u64>,
    ) -> crate::runtime_process_memory::RuntimeProcessMemoryStatus {
        let mut state = self.state.lock().unwrap();
        let previous = state.runtime_process_memory.as_ref();
        let mut role_peaks = previous
            .map(|status| status.roles_peak_bytes.clone())
            .unwrap_or_default();
        for (role, bytes) in &sample.roles {
            let peak = role_peaks.entry(role.clone()).or_default();
            *peak = (*peak).max(*bytes);
        }
        let warning = effective_memory_limit_bytes
            .filter(|limit| *limit > 0)
            .is_some_and(|limit| {
                sample.resident_and_swap_bytes.saturating_mul(100)
                    >= limit.saturating_mul(RUNTIME_PROCESS_MEMORY_WARNING_PERCENT)
            });
        let status = crate::runtime_process_memory::RuntimeProcessMemoryStatus {
            measurement: sample.measurement,
            process_count: sample.process_count,
            current_bytes: sample.resident_and_swap_bytes,
            peak_bytes: previous
                .map(|status| status.peak_bytes)
                .unwrap_or_default()
                .max(sample.resident_and_swap_bytes),
            warning,
            roles_current_bytes: sample.roles,
            roles_peak_bytes: role_peaks,
            collected_at_epoch: crate::now_ts_u64(),
        };
        state.runtime_process_memory = Some(status.clone());
        status
    }

    pub(crate) fn pressure_state(&self) -> ResourcePressureState {
        self.state.lock().unwrap().pressure.state()
    }

    fn observe_snapshot(&self, snapshot: HostResourceSnapshot) -> ResourceBrokerStatus {
        let mut state = self.state.lock().unwrap();
        let pressure_state = state.pressure.observe(&snapshot);
        ResourceBrokerStatus {
            pressure_state,
            reserved_memory_mib: state.leases.values().map(|lease| lease.memory_mib).sum(),
            active_leases: state.leases.len(),
            reserved_cpu_cores: state.leases.values().map(|lease| lease.cpu_cores).sum(),
            reserved_network_slots: state.leases.values().map(|lease| lease.network_slots).sum(),
            reserved_provider_slots: state
                .leases
                .values()
                .map(|lease| lease.provider_slots)
                .sum(),
            reserved_browser_slots: state.leases.values().map(|lease| lease.browser_slots).sum(),
            active_heavy_leases: state.leases.values().filter(|lease| lease.heavy).count(),
            recent_admission_refusal_reason: state.recent_admission_refusal_reason,
            recent_admission_refusal_at_epoch: state.recent_admission_refusal_at_epoch,
            runtime_process_memory: state.runtime_process_memory.clone(),
            snapshot,
        }
    }

    fn restore_durable_lease(
        &self,
        metadata: &DurableResourceLeaseMetadata,
    ) -> Option<ResourceLease> {
        if metadata.schema_version != 1
            || uuid::Uuid::parse_str(&metadata.lease_token).is_err()
            || metadata.memory_mib == 0
            || metadata.memory_mib > 1_048_576
            || metadata.cpu_cores == 0
            || metadata.cpu_cores > 4_096
            || metadata.network_slots > 4_096
            || metadata.provider_slots > 4_096
            || metadata.browser_slots > 64
            || metadata
                .estimate_key
                .as_ref()
                .is_some_and(|key| !valid_resource_estimate_key(key))
        {
            return None;
        }
        let reserved = ReservedResources {
            memory_mib: metadata.memory_mib,
            cpu_cores: metadata.cpu_cores,
            network_slots: metadata.network_slots,
            provider_slots: metadata.provider_slots,
            browser_slots: metadata.browser_slots,
            heavy: metadata.heavy,
            background: metadata.background,
        };
        let mut outstanding = reserved;
        if let Some(memory_mib) =
            crate::skills::runner_pool::process_tree_resident_memory_mib(metadata.pid)
        {
            outstanding.memory_mib = reserved.memory_mib.saturating_sub(memory_mib);
        }
        let mut state = self.state.lock().unwrap();
        state.next_lease_id = state.next_lease_id.wrapping_add(1).max(1);
        let lease_id = state.next_lease_id;
        state.leases.insert(lease_id, outstanding);
        Some(ResourceLease {
            broker: self.clone(),
            lease_id,
            grant: ResourceGrant {
                admitted: true,
                max_concurrency: 1,
                wait_reason: None,
                projection: json!({
                    "schema_version": 1,
                    "restored": true,
                    "resource_heavy": reserved.heavy,
                    "resource_background": reserved.background,
                    "grant": {
                        "cpu_cores": reserved.cpu_cores,
                        "memory_mb": reserved.memory_mib,
                        "network_slots": reserved.network_slots,
                        "provider_slots": reserved.provider_slots,
                        "browser_slots": reserved.browser_slots,
                    },
                }),
            },
            reserved,
            estimate_key: metadata.estimate_key.clone(),
        })
    }

    #[cfg(test)]
    fn reserved_memory_mib(&self) -> u64 {
        self.state
            .lock()
            .unwrap()
            .leases
            .values()
            .map(|lease| lease.memory_mib)
            .sum()
    }
}

fn observed_memory_estimate(
    estimates: &BTreeMap<ResourceEstimateKey, ObservedResourceEstimate>,
    key: &ResourceEstimateKey,
) -> Option<u64> {
    let exact = estimates.get(key);
    if exact.is_some_and(|entry| entry.sample_count >= RESOURCE_ESTIMATE_MIN_SAMPLES) {
        return exact.map(|entry| entry.peak_memory_mib);
    }
    estimates
        .values()
        .filter(|entry| entry.key.scope == key.scope && entry.key.action == key.action)
        .map(|entry| entry.peak_memory_mib)
        .chain(exact.map(|entry| entry.peak_memory_mib))
        .max()
}

fn load_resource_estimates(path: &Path) -> BTreeMap<ResourceEstimateKey, ObservedResourceEstimate> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return BTreeMap::new();
    };
    if metadata.len() > RESOURCE_ESTIMATE_MAX_FILE_BYTES {
        return BTreeMap::new();
    }
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(cache) = serde_json::from_str::<PersistedResourceEstimates>(&raw) else {
        return BTreeMap::new();
    };
    if cache.schema_version != RESOURCE_ESTIMATE_CACHE_SCHEMA_VERSION {
        return BTreeMap::new();
    }
    cache
        .entries
        .into_iter()
        .filter(|entry| {
            valid_resource_estimate_key(&entry.key)
                && entry.peak_memory_mib > 0
                && entry.peak_memory_mib <= 1_048_576
                && entry.sample_count > 0
        })
        .take(RESOURCE_ESTIMATE_MAX_ENTRIES)
        .map(|entry| (entry.key.clone(), entry))
        .collect()
}

fn valid_resource_estimate_key(key: &ResourceEstimateKey) -> bool {
    !key.scope.trim().is_empty()
        && key.scope.len() <= 256
        && !key.action.trim().is_empty()
        && key.action.len() <= 128
        && key
            .registry_generation_digest
            .as_ref()
            .is_none_or(|digest| digest.len() <= 256)
}

fn encode_resource_estimates(
    estimates: &BTreeMap<ResourceEstimateKey, ObservedResourceEstimate>,
) -> String {
    serde_json::to_string(&PersistedResourceEstimates {
        schema_version: RESOURCE_ESTIMATE_CACHE_SCHEMA_VERSION,
        entries: estimates.values().cloned().collect(),
    })
    .expect("resource estimate cache contains only serializable fields")
}

fn persist_resource_estimates(path: &Path, payload: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temp_path, payload)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(temp_path, path)
}

fn prune_resource_estimates(
    estimates: &mut BTreeMap<ResourceEstimateKey, ObservedResourceEstimate>,
) {
    if estimates.len() <= RESOURCE_ESTIMATE_MAX_ENTRIES {
        return;
    }
    let mut oldest = estimates
        .values()
        .map(|entry| (entry.updated_at_epoch, entry.key.clone()))
        .collect::<Vec<_>>();
    oldest.sort();
    for (_, key) in oldest
        .into_iter()
        .take(estimates.len() - RESOURCE_ESTIMATE_MAX_ENTRIES)
    {
        estimates.remove(&key);
    }
}

pub(crate) fn restore_durable_resource_leases(state: &crate::AppState) -> usize {
    let jobs_root =
        claw_core::workspace_state::workspace_state_root(&state.skill_rt.workspace_root)
            .join("async_jobs");
    let Ok(entries) = std::fs::read_dir(jobs_root) else {
        return 0;
    };
    let broker = state
        .skill_rt
        .skill_concurrency_gates
        .resource_broker()
        .clone();
    let mut restored = 0;
    for entry in entries.flatten() {
        let job_dir = entry.path();
        if !job_dir.is_dir()
            || job_dir.join("exit_code").is_file()
            || !job_dir.join("lease_ready").is_file()
        {
            continue;
        }
        let Some(metadata) = read_durable_resource_lease_metadata(&job_dir) else {
            continue;
        };
        let Some(pid) = crate::local_process_job::read_pid(&job_dir) else {
            continue;
        };
        if pid != metadata.pid
            || crate::local_process_job::process_identity_state(&job_dir, pid)
                != crate::local_process_job::ProcessIdentityState::AliveVerified
        {
            continue;
        }
        let Some(lease) = broker.restore_durable_lease(&metadata) else {
            continue;
        };
        spawn_durable_resource_lease_monitor(lease, job_dir);
        restored += 1;
    }
    restored
}

fn read_durable_resource_lease_metadata(job_dir: &Path) -> Option<DurableResourceLeaseMetadata> {
    const MAX_METADATA_BYTES: u64 = 16 * 1024;
    let path = job_dir.join("resource_lease.json");
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.len() > MAX_METADATA_BYTES {
        return None;
    }
    let payload = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&payload).ok()
}

fn spawn_durable_resource_lease_monitor(lease: ResourceLease, job_dir: PathBuf) {
    tokio::spawn(async move {
        let pid = crate::local_process_job::read_pid(&job_dir);
        let mut observed_peak_memory_mib = 0_u64;
        loop {
            if job_dir.join("exit_code").is_file() {
                let _ = crate::local_process_job::write_atomic(
                    &job_dir.join("resource_lease_release_reason"),
                    "terminal_recorded",
                );
                break;
            }
            let identity = pid.map_or(
                crate::local_process_job::ProcessIdentityState::Missing,
                |pid| crate::local_process_job::process_identity_state(&job_dir, pid),
            );
            if let Some(memory_mib) =
                pid.and_then(crate::skills::runner_pool::process_tree_resident_memory_mib)
            {
                observed_peak_memory_mib = observed_peak_memory_mib.max(memory_mib);
                lease.refresh_materialized_memory(memory_mib);
            }
            match identity {
                crate::local_process_job::ProcessIdentityState::AliveVerified
                | crate::local_process_job::ProcessIdentityState::Unknown => {
                    let _ = crate::local_process_job::write_atomic(
                        &job_dir.join("resource_lease_heartbeat_at"),
                        &crate::now_ts_u64().to_string(),
                    );
                }
                crate::local_process_job::ProcessIdentityState::Missing
                | crate::local_process_job::ProcessIdentityState::IdentityMismatch => {
                    if crate::local_process_job::process_loss_is_stable(
                        &job_dir,
                        identity,
                        crate::now_ts_u64().min(i64::MAX as u64) as i64,
                        5,
                    ) {
                        let _ = crate::local_process_job::write_atomic(
                            &job_dir.join("resource_lease_release_reason"),
                            identity.as_token(),
                        );
                        break;
                    }
                }
            }
            tokio::time::sleep(durable_resource_monitor_interval()).await;
        }
        if let Some(estimate_key) = lease.estimate_key.clone() {
            lease
                .broker
                .record_observed_peak_memory(estimate_key, observed_peak_memory_mib);
        }
        drop(lease);
    });
}

fn durable_resource_monitor_interval() -> Duration {
    #[cfg(test)]
    {
        Duration::from_millis(20)
    }
    #[cfg(not(test))]
    {
        Duration::from_secs(5)
    }
}

pub(crate) fn spawn_resource_pressure_monitor(state: crate::AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut previous = None;
        let mut previous_process_warning = None;
        loop {
            interval.tick().await;
            let mut status = state
                .skill_rt
                .skill_concurrency_gates
                .resource_broker()
                .observe_host();
            let workspace_root = state.skill_rt.workspace_root.clone();
            let process_sample = tokio::task::spawn_blocking(move || {
                crate::runtime_process_memory::collect_runtime_process_memory(&workspace_root)
            })
            .await;
            match process_sample {
                Ok(Ok(sample)) => {
                    let process_status = state
                        .skill_rt
                        .skill_concurrency_gates
                        .resource_broker()
                        .record_runtime_process_memory(
                            sample,
                            status.snapshot.effective_memory_limit_bytes,
                        );
                    status.runtime_process_memory = Some(process_status);
                }
                Ok(Err(error_code)) => {
                    tracing::debug!(error_code, "runtime_process_memory_sample_unavailable");
                }
                Err(error) => {
                    tracing::warn!(error = %error, "runtime_process_memory_sample_join_failed");
                }
            }
            if matches!(
                status.pressure_state,
                ResourcePressureState::Constrained | ResourcePressureState::Critical
            ) {
                state.skill_rt.runner_pool.invalidate_all();
            }
            let reclaimed_browsers = if status.pressure_state == ResourcePressureState::Critical {
                state
                    .core
                    .browser_sessions
                    .reclaim_idle(std::time::Duration::from_secs(30))
                    .await
            } else {
                0
            };
            let process_warning = status
                .runtime_process_memory
                .as_ref()
                .map(|sample| sample.warning);
            let process_warning_changed = process_warning != previous_process_warning;
            if previous != Some(status.pressure_state) || process_warning_changed {
                tracing::info!(
                    pressure_state = ?status.pressure_state,
                    effective_memory_limit_bytes = status.snapshot.effective_memory_limit_bytes,
                    memory_available_bytes = status.snapshot.memory_available_bytes,
                    reserved_memory_mib = status.reserved_memory_mib,
                    active_leases = status.active_leases,
                    reclaimed_browsers,
                    runtime_process_memory_bytes = status
                        .runtime_process_memory
                        .as_ref()
                        .map(|sample| sample.current_bytes),
                    runtime_process_memory_peak_bytes = status
                        .runtime_process_memory
                        .as_ref()
                        .map(|sample| sample.peak_bytes),
                    runtime_process_memory_warning = process_warning,
                    "host_resource_pressure_changed"
                );
                if process_warning == Some(true) && previous_process_warning != Some(true) {
                    tracing::warn!(
                        current_bytes = status
                            .runtime_process_memory
                            .as_ref()
                            .map(|sample| sample.current_bytes),
                        peak_bytes = status
                            .runtime_process_memory
                            .as_ref()
                            .map(|sample| sample.peak_bytes),
                        process_count = status
                            .runtime_process_memory
                            .as_ref()
                            .map(|sample| sample.process_count),
                        "runtime_process_memory_high"
                    );
                }
                if let Err(error) = append_resource_pressure_event(
                    &state.skill_rt.workspace_root,
                    &status,
                    reclaimed_browsers,
                ) {
                    tracing::warn!(error = %error, "resource_pressure_log_write_failed");
                }
                previous = Some(status.pressure_state);
                previous_process_warning = process_warning;
            }
        }
    });
}

fn append_resource_pressure_event(
    workspace_root: &Path,
    status: &ResourceBrokerStatus,
    reclaimed_browsers: usize,
) -> std::io::Result<()> {
    use std::io::Write;

    let logs_root = claw_core::workspace_state::workspace_state_root(workspace_root).join("logs");
    std::fs::create_dir_all(&logs_root)?;
    let path = logs_root.join("resource-pressure.jsonl");
    if std::fs::metadata(&path)
        .map(|metadata| metadata.len() >= RESOURCE_PRESSURE_LOG_MAX_BYTES)
        .unwrap_or(false)
    {
        let previous = logs_root.join("resource-pressure.previous.jsonl");
        let _ = std::fs::remove_file(&previous);
        std::fs::rename(&path, previous)?;
    }
    let payload = json!({
        "schema_version": 1,
        "record_type": "resource_pressure_transition",
        "observed_at_epoch": crate::now_ts_u64(),
        "pressure_state": status.pressure_state,
        "effective_memory_limit_bytes": status.snapshot.effective_memory_limit_bytes,
        "memory_available_bytes": status.snapshot.memory_available_bytes,
        "swap_used_bytes": status.snapshot.swap_used_bytes,
        "reserved_memory_mib": status.reserved_memory_mib,
        "active_leases": status.active_leases,
        "active_heavy_leases": status.active_heavy_leases,
        "reserved_browser_slots": status.reserved_browser_slots,
        "reclaimed_browsers": reclaimed_browsers,
        "runtime_process_memory": status.runtime_process_memory,
        "reason_code": resource_pressure_reason_code(status.pressure_state),
    });
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer(&mut file, &payload).map_err(std::io::Error::other)?;
    file.write_all(b"\n")
}

fn resource_pressure_reason_code(state: ResourcePressureState) -> &'static str {
    match state {
        ResourcePressureState::Normal => "resource_pressure_normal",
        ResourcePressureState::Compact => "resource_pressure_compact",
        ResourcePressureState::Constrained => "resource_pressure_constrained",
        ResourcePressureState::Critical => "resource_pressure_critical",
    }
}

fn host_safety_reserve_mib(effective_memory_mib: Option<u64>) -> u64 {
    match effective_memory_mib {
        Some(total) if total <= LOW_MEMORY_HOST_MAX_MIB => (total / 8).clamp(128, 256),
        Some(total) => (total / 10).clamp(256, 1024),
        None => 256,
    }
}

#[cfg(test)]
pub(crate) fn host_grant(
    request: Option<&SkillResourceRequest>,
    safety_ceiling: usize,
) -> ResourceGrant {
    host_grant_for_snapshot(
        request,
        safety_ceiling,
        &HostResourceSnapshot::collect(),
        gpu_device_available(),
    )
}

fn host_grant_for_snapshot(
    request: Option<&SkillResourceRequest>,
    safety_ceiling: usize,
    snapshot: &HostResourceSnapshot,
    gpu_available: bool,
) -> ResourceGrant {
    let cpu_total = snapshot.cpu_parallelism;
    let memory_available_mb = snapshot.available_memory_mib().unwrap_or(1024);
    let request = request.cloned().unwrap_or_default();
    let cpu_cores = request.cpu_cores.max(1);
    let declared_memory_mb = request.memory_mb;
    let host_policy_floor_memory_mb = host_policy_memory_floor_mb(request.class);
    let memory_mb = requested_memory_mb(&request);
    let gpu_required = request.gpu_slots > 0 || request.class == SkillResourceClass::Gpu;
    let fallback = gpu_required && !gpu_available && request.allow_cpu_fallback;
    let memory_sufficient = memory_available_mb >= memory_mb;
    let cpu_sufficient = cpu_total >= cpu_cores;
    let admitted =
        (!gpu_required || gpu_available || fallback) && memory_sufficient && cpu_sufficient;
    let granted_cpu_cores = cpu_cores.min(cpu_total);
    let cpu_slots = if cpu_sufficient {
        (cpu_total / cpu_cores).max(1)
    } else {
        1
    };
    let memory_slots = (memory_available_mb / memory_mb).max(1) as usize;
    let max_concurrency = safety_ceiling
        .max(1)
        .min(cpu_slots)
        .min(memory_slots)
        .max(1);
    let wait_reason = if !memory_sufficient {
        Some("memory_unavailable")
    } else if !cpu_sufficient {
        Some("cpu_unavailable")
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
                "declared_memory_mb": declared_memory_mb,
                "host_policy_floor_memory_mb": host_policy_floor_memory_mb,
                "memory_mb": memory_mb,
                "gpu_slots": request.gpu_slots,
                "disk_io_weight": request.disk_io_weight,
                "network_slots": request.network_slots,
                "provider_slots": request.provider_slots,
                "browser_slots": request.browser_slots,
            },
            "grant": {
                "cpu_cores": granted_cpu_cores,
                "memory_mb": memory_mb,
                "gpu_slots": usize::from(gpu_required && gpu_available),
                "network_slots": request.network_slots,
                "provider_slots": request.provider_slots,
                "browser_slots": request.browser_slots,
            },
            "host": {
                "cpu_cores": cpu_total,
                "memory_available_mb": memory_available_mb,
                "effective_memory_limit_mb": snapshot.effective_memory_mib(),
                "memory_source": snapshot.memory_source,
                "gpu_available": gpu_available,
            },
            "admitted": admitted,
            "fallback": fallback.then_some("cpu"),
            "max_concurrency": max_concurrency,
            "wait_reason": wait_reason,
        }),
    }
}

pub(crate) fn resource_request_is_heavy(request: &SkillResourceRequest) -> bool {
    matches!(
        request.class,
        SkillResourceClass::Cpu
            | SkillResourceClass::Memory
            | SkillResourceClass::Gpu
            | SkillResourceClass::LocalModel
    ) || requested_memory_mb(request) >= 256
}

pub(crate) fn requested_memory_mb(request: &SkillResourceRequest) -> u64 {
    request
        .memory_mb
        .max(host_policy_memory_floor_mb(request.class))
        .max((request.browser_slots as u64).saturating_mul(BROWSER_SLOT_MEMORY_FLOOR_MIB))
}

fn network_slot_capacity(snapshot: &HostResourceSnapshot) -> usize {
    match snapshot.effective_memory_mib() {
        Some(total) if total <= LOW_MEMORY_HOST_MAX_MIB => 2,
        Some(total) if total <= CONSTRAINED_MEMORY_HOST_MAX_MIB => 4,
        _ => snapshot.cpu_parallelism.saturating_mul(4).clamp(4, 64),
    }
}

fn provider_slot_capacity(snapshot: &HostResourceSnapshot) -> usize {
    match snapshot.effective_memory_mib() {
        Some(total) if total <= LOW_MEMORY_HOST_MAX_MIB => 1,
        Some(total) if total <= CONSTRAINED_MEMORY_HOST_MAX_MIB => 2,
        _ => snapshot.cpu_parallelism.clamp(2, 16),
    }
}

fn browser_slot_capacity(snapshot: &HostResourceSnapshot) -> usize {
    match snapshot.effective_memory_mib() {
        Some(total) if total <= CONSTRAINED_MEMORY_HOST_MAX_MIB => 1,
        Some(total) if total <= 8 * 1024 => 2,
        _ => snapshot.cpu_parallelism.div_ceil(2).clamp(1, 4),
    }
}

fn host_policy_memory_floor_mb(class: SkillResourceClass) -> u64 {
    match class {
        SkillResourceClass::Cpu => 256,
        SkillResourceClass::Memory => 512,
        SkillResourceClass::Gpu => 1024,
        SkillResourceClass::LocalModel => 1024,
        SkillResourceClass::General
        | SkillResourceClass::DiskIo
        | SkillResourceClass::Network
        | SkillResourceClass::ProviderQuota => DEFAULT_SKILL_MEMORY_MIB,
    }
}

pub(crate) fn total_memory_mib() -> Option<u64> {
    HostResourceSnapshot::collect().effective_memory_mib()
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

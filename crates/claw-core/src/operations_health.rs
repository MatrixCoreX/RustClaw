//! Additive host operations telemetry. Paths and credentials are never exposed.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageVolumeHealth {
    pub role: String,
    pub state: String,
    pub available_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub warning_reserve_bytes: u64,
    pub critical_reserve_bytes: u64,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoragePressureHealth {
    pub state: String,
    pub write_admission: bool,
    pub sampled_at_ts: u64,
    pub volumes: Vec<StorageVolumeHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundWorkerHealth {
    pub name: String,
    pub alive: bool,
    pub stalled: bool,
    pub restart_count: u64,
    pub heartbeat_age_seconds: u64,
    pub in_flight_blocking_operations: usize,
    pub last_error_code: Option<String>,
}

//! Host-wide free-space admission, separate from skill storage quotas.
use claw_core::operations_health::{StoragePressureHealth, StorageVolumeHealth};
use std::path::{Path, PathBuf};

const GIB: u64 = 1024 * 1024 * 1024;

pub(super) fn volume(
    role: &str,
    available: Option<u64>,
    total: Option<u64>,
) -> StorageVolumeHealth {
    let warning = (total.unwrap_or(0) / 100 * 3).max(2 * GIB);
    let critical = (total.unwrap_or(0) / 100).max(GIB / 2);
    let state = match (available, total) {
        (Some(free), Some(total)) if total > 0 => {
            if free <= critical {
                "critical"
            } else if free <= warning {
                "warning"
            } else {
                "normal"
            }
        }
        _ => "unavailable",
    };
    StorageVolumeHealth {
        role: role.into(),
        state: state.into(),
        available_bytes: available,
        total_bytes: total,
        warning_reserve_bytes: warning,
        critical_reserve_bytes: critical,
        error_code: (state == "unavailable").then(|| "storage_probe_failed".into()),
    }
}

fn sample(role: &str, path: &Path) -> StorageVolumeHealth {
    volume(
        role,
        fs2::available_space(path).ok(),
        fs2::total_space(path).ok(),
    )
}

pub(crate) fn collect(state: &crate::AppState) -> StoragePressureHealth {
    let mut volumes = vec![
        // Protect the system filesystem even when TMPDIR and data live on another volume.
        sample("system", Path::new(std::path::MAIN_SEPARATOR_STR)),
        sample("workspace", &state.skill_rt.workspace_root),
        sample("temporary", &std::env::temp_dir()),
    ];
    for (role, pool) in [
        ("database", &state.core.db),
        ("audit_database", &state.core.audit_db),
    ] {
        match pool.get() {
            Ok(db) => {
                // SQLite memory databases have no filesystem path to reserve.
                if let Some(path) = db.path().filter(|path| !path.is_empty()) {
                    let path = PathBuf::from(path);
                    let parent = path
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new("."));
                    volumes.push(sample(role, parent));
                }
            }
            Err(_) => volumes.push(volume(role, None, None)),
        }
    }
    let status = if volumes.iter().any(|v| v.state == "critical") {
        "critical"
    } else if volumes.iter().any(|v| v.state == "unavailable") {
        "unavailable"
    } else if volumes.iter().any(|v| v.state == "warning") {
        "warning"
    } else {
        "normal"
    };
    StoragePressureHealth {
        state: status.into(),
        write_admission: matches!(status, "normal" | "warning"),
        sampled_at_ts: crate::now_ts_u64(),
        volumes,
    }
}

pub(crate) fn refresh(state: &crate::AppState) -> StoragePressureHealth {
    let snapshot = collect(state);
    state.worker.supervision.observe_storage(snapshot.clone());
    snapshot
}

pub(crate) async fn admit_write(state: &crate::AppState) -> anyhow::Result<StoragePressureHealth> {
    let state = state.clone();
    Ok(tokio::task::spawn_blocking(move || refresh(&state)).await?)
}

pub(crate) async fn background_write_admitted(state: &crate::AppState) -> bool {
    let snapshot = match state.worker.supervision.storage_snapshot() {
        Some(snapshot) if crate::now_ts_u64().saturating_sub(snapshot.sampled_at_ts) <= 30 => {
            Some(snapshot)
        }
        _ => admit_write(state).await.ok(),
    };
    snapshot.is_some_and(|snapshot| snapshot.write_admission)
}

#[cfg(test)]
#[path = "storage_pressure_tests.rs"]
mod tests;

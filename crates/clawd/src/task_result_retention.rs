//! Host-owned retention for explicitly enrolled distributed deliveries.
//! Untrusted task payload flags never authorize retention changes or deletion.
use std::path::{Path, PathBuf};

use anyhow::ensure;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[path = "task_owned_cleanup.rs"]
pub(crate) mod files;
#[path = "durable_job_monitor.rs"]
pub(crate) mod monitors;
#[path = "task_retention_temp.rs"]
pub(crate) mod temporary;
#[path = "task_retention_storage.rs"]
pub(crate) mod storage;
#[cfg(test)]
#[path = "task_retention_test_support.rs"]
pub(crate) mod test_support;
#[path = "task_retention_write_gate.rs"]
pub(crate) mod write_gate;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeliveryCleanupEnrollment {
    pub(crate) schema_version: u32,
    pub(crate) work_id: String,
    pub(crate) attempt: u32,
    pub(crate) capability: String,
    pub(crate) origin: String,
    pub(crate) job_id: String,
    pub(crate) payload_sha256: String,
}

impl DeliveryCleanupEnrollment {
    pub(crate) fn idempotency_key(&self) -> String {
        format!("nni-capability:{}:{}", self.work_id, self.attempt)
    }

    pub(crate) fn matches_payload(&self, payload: &Value) -> bool {
        payload.get("_task_source").and_then(Value::as_str) == Some("distributed_work")
            && payload.get("_requested_capability").and_then(Value::as_str)
                == Some(self.capability.as_str())
            && payload.get("_delivery_cleanup") == serde_json::to_value(self).ok().as_ref()
    }
}

fn enrollment_path(workspace: &Path, task_id: &str) -> anyhow::Result<PathBuf> {
    let id = uuid::Uuid::parse_str(task_id)?;
    ensure!(id.to_string() == task_id, "work_cleanup_task_id_invalid");
    Ok(workspace
        .join("data/nni/work/enrollments")
        .join(format!("{id}.json")))
}

pub(crate) fn read_enrollment(
    workspace: &Path,
    task_id: &str,
) -> anyhow::Result<Option<DeliveryCleanupEnrollment>> {
    let path = enrollment_path(workspace, task_id)?;
    if !crate::task_result_retention::files::validate_owned_path(workspace, &path)? {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    ensure!(bytes.len() <= 4096, "work_cleanup_enrollment_invalid");
    let value: DeliveryCleanupEnrollment = serde_json::from_slice(&bytes)?;
    ensure!(value.schema_version == 1, "work_cleanup_enrollment_invalid");
    Ok(Some(value))
}

pub(crate) fn register_enrollment(
    workspace: &Path,
    task_id: &str,
    enrollment: &DeliveryCleanupEnrollment,
) -> anyhow::Result<()> {
    use std::io::Write;
    let path = enrollment_path(workspace, task_id)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("work_cleanup_path_invalid"))?;
    crate::task_result_retention::files::validate_owned_path(workspace, parent)?;
    std::fs::create_dir_all(parent)?;
    crate::task_result_retention::files::validate_owned_path(workspace, parent)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(&serde_json::to_vec(enrollment)?)?;
    file.sync_all()?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(crate) fn discard_candidate(workspace: &Path, task_id: &str) -> anyhow::Result<()> {
    let path = enrollment_path(workspace, task_id)?;
    if crate::task_result_retention::files::validate_owned_path(workspace, &path)? {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub(crate) fn retains_shared_outcome(
    state: &crate::AppState,
    task_id: &str,
    payload: &Value,
) -> bool {
    let Some(enrollment) = read_enrollment(&state.skill_rt.workspace_root, task_id)
        .ok()
        .flatten()
    else {
        return true;
    };
    if !enrollment.matches_payload(payload) {
        return true;
    }
    let Ok(db) = state.core.db.get() else {
        // A valid private enrollment is already known. Temporary DB failure
        // must not turn a delivery result into permanent shared prose.
        return false;
    };
    match db.query_row(
        "SELECT kind, idempotency_key, payload_json FROM tasks WHERE task_id = ?1",
        [task_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    ) {
        Ok((kind, key, raw)) => {
            if kind != "run_skill" || key.as_deref() != Some(enrollment.idempotency_key().as_str())
            {
                return true;
            }
            match serde_json::from_str::<Value>(&raw) {
                Ok(payload) => !enrollment.matches_payload(&payload),
                Err(_) => false,
            }
        }
        Err(_) => false,
    }
}

/// Register a runner job before it starts; this private index avoids discovering
/// deletion candidates by walking shared jobs or trusting output paths.
pub(crate) fn record_durable_job(
    state: &crate::AppState,
    task_id: &str,
    job_id: &str,
) -> anyhow::Result<()> {
    if read_enrollment(&state.skill_rt.workspace_root, task_id)
        .ok()
        .flatten()
        .is_none()
    {
        return Ok(());
    }
    let payload = state.core.db.get()?.query_row(
        "SELECT payload_json FROM tasks WHERE task_id=?1",
        [task_id],
        |r| r.get::<_, String>(0),
    )?;
    let payload: Value = serde_json::from_str(&payload)?;
    if retains_shared_outcome(state, task_id, &payload) {
        return Ok(());
    }
    let id = job_id
        .strip_prefix("local_process:")
        .ok_or_else(|| anyhow::anyhow!("work_cleanup_job_id_invalid"))?;
    let parsed = uuid::Uuid::parse_str(id)?;
    ensure!(parsed.to_string() == id, "work_cleanup_job_id_invalid");
    let path = enrollment_path(&state.skill_rt.workspace_root, task_id)?.with_extension("jobs");
    crate::task_result_retention::files::validate_owned_path(
        &state.skill_rt.workspace_root,
        &path,
    )?;
    // Atomic replacement prevents a torn appended UUID from permanently
    // stranding cleanup after a crash. Serialize registrations for one host.
    static INDEX_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = INDEX_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("work_cleanup_index_lock_failed"))?;
    let mut ids = durable_job_ids(&state.skill_rt.workspace_root, task_id)?;
    if !ids.iter().any(|candidate| candidate == id) {
        ids.push(id.to_string());
    }
    ids.sort();
    let temporary = path.with_extension(format!("jobs-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&temporary)?;
    file.write_all(format!("{}\n", ids.join("\n")).as_bytes())?;
    file.sync_all()?;
    std::fs::rename(temporary, &path)?;
    std::fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}

pub(crate) fn durable_job_ids(workspace: &Path, task_id: &str) -> anyhow::Result<Vec<String>> {
    let path = enrollment_path(workspace, task_id)?.with_extension("jobs");
    if !crate::task_result_retention::files::validate_owned_path(workspace, &path)? {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(path)?;
    ensure!(raw.len() <= 16 * 1024, "work_cleanup_job_index_invalid");
    let mut ids = std::collections::BTreeSet::new();
    for id in raw.lines() {
        let parsed = uuid::Uuid::parse_str(id)?;
        ensure!(parsed.to_string() == id, "work_cleanup_job_id_invalid");
        ids.insert(id.to_string());
    }
    Ok(ids.into_iter().collect())
}

pub(crate) fn is_retired(
    state: &crate::AppState,
    db: &rusqlite::Connection,
    task_id: &str,
) -> anyhow::Result<bool> {
    let Some(enrollment) = read_enrollment(&state.skill_rt.workspace_root, task_id)
        .ok()
        .flatten()
    else {
        return Ok(false);
    };
    let (kind, key, payload, result) = db.query_row(
        "SELECT kind, idempotency_key, payload_json, result_json FROM tasks WHERE task_id=?1",
        [task_id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        },
    )?;
    if kind != "run_skill"
        || key.as_deref() != Some(enrollment.idempotency_key().as_str())
        || !enrollment.matches_payload(&serde_json::from_str::<Value>(&payload)?)
    {
        return Ok(false);
    }
    let result: Option<Value> = result.as_deref().map(serde_json::from_str).transpose()?;
    Ok(result.is_some_and(|r| {
        r.get("result_retention").and_then(Value::as_str) == Some("removed_after_delivery_ack")
    }))
}

#[cfg(test)]
#[path = "task_result_retention_tests.rs"]
mod tests;

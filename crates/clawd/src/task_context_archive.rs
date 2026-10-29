//! Immutable, redacted execution evidence owned by a task, never by diagnostic logs.
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use claw_core::capability_result::CapabilityResultEnvelope;

const MAX_ITEMS: usize = 4096;
const MAX_ITEM_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    task_id: String,
    items: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    sha256: String,
    size_bytes: usize,
    capability: Value,
    status: Value,
}

struct LimitedBytes(Vec<u8>);
impl Write for LimitedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_ITEM_BYTES {
            return Err(io::Error::other("context_archive_budget_exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialize(value: &impl Serialize) -> Result<Vec<u8>, &'static str> {
    let mut output = LimitedBytes(Vec::new());
    serde_json::to_writer(&mut output, value).map_err(|_| "context_archive_budget_exceeded")?;
    Ok(output.0)
}

pub(crate) fn capture(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    items: &[CapabilityResultEnvelope],
) -> Value {
    let result = (|| {
        let _gate =
            crate::task_result_retention::write_gate::try_begin_writing(state, &task.task_id)
                .map_err(|_| "context_archive_write_unavailable")?;
        if !crate::repo::is_task_claim_active_or_pending_ask_success_projection(
            state,
            &task.task_id,
            task.claim_attempt,
        )
        .map_err(|_| "context_archive_write_unavailable")?
        {
            return Err("context_archive_stale_claim");
        }
        publish(&state.skill_rt.workspace_root, &task.task_id, items)
    })();
    result
        .unwrap_or_else(|code| json!({"schema_version":1,"status":"unavailable","error_code":code}))
}

fn publish(root: &Path, task_id: &str, items: &[impl Serialize]) -> Result<Value, &'static str> {
    if uuid::Uuid::parse_str(task_id)
        .ok()
        .is_none_or(|id| id.to_string() != task_id)
        || items.len() > MAX_ITEMS
    {
        return Err("context_archive_budget_exceeded");
    }
    let mut manifest = Manifest {
        schema_version: 1,
        task_id: task_id.to_string(),
        items: Vec::new(),
    };
    let mut total = 0usize;
    for item in items {
        let bytes = serialize(item)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "context_archive_invalid")?;
        let (safe, _) = crate::skill_output_artifact::sensitivity_aware_json_model_view(&value);
        let bytes = serialize(&safe)?;
        total = total.saturating_add(bytes.len());
        if total > MAX_TOTAL_BYTES {
            return Err("context_archive_budget_exceeded");
        }
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        crate::skill_output_artifact::publish_canonical_evidence_artifact(
            root, task_id, &sha256, &bytes,
        )
        .map_err(|_| "context_archive_write_unavailable")?;
        manifest.items.push(Entry {
            sha256,
            size_bytes: bytes.len(),
            capability: safe.get("capability").cloned().unwrap_or(Value::Null),
            status: safe.get("status").cloned().unwrap_or(Value::Null),
        });
    }
    let bytes = serialize(&manifest)?;
    total = total.saturating_add(bytes.len());
    if total > MAX_TOTAL_BYTES {
        return Err("context_archive_budget_exceeded");
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    crate::skill_output_artifact::publish_canonical_evidence_artifact(
        root, task_id, &sha256, &bytes,
    )
    .map_err(|_| "context_archive_write_unavailable")?;
    Ok(
        json!({"schema_version":1,"status":"available","task_id":task_id,
        "manifest_sha256":sha256,"total":items.len(),"total_bytes":total,
        "redacted":true,"instruction_authority":"none"}),
    )
}

#[cfg(test)]
pub(crate) fn publish_fixture(root: &Path, task_id: &str, items: &[Value]) -> Value {
    publish(root, task_id, items).expect("publish isolated context fixture")
}

fn object_path(root: &Path, task_id: &str, digest: &str) -> Result<PathBuf, &'static str> {
    if uuid::Uuid::parse_str(task_id)
        .ok()
        .is_none_or(|id| id.to_string() != task_id)
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("context_archive_corrupt");
    }
    Ok(claw_core::workspace_state::workspace_artifacts_root(root)
        .join("canonical-evidence")
        .join(task_id)
        .join(format!("{digest}.json")))
}

fn read_object(root: &Path, task_id: &str, digest: &str) -> Result<Vec<u8>, &'static str> {
    let path = object_path(root, task_id, digest)?;
    let file = crate::secure_workspace_fs::open_workspace_file(root, &path)
        .map_err(|_| "context_archive_unavailable")?;
    let mut bytes = Vec::new();
    file.take(MAX_ITEM_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "context_archive_unavailable")?;
    if bytes.len() > MAX_ITEM_BYTES || format!("{:x}", Sha256::digest(&bytes)) != digest {
        return Err("context_archive_corrupt");
    }
    Ok(bytes)
}

pub(crate) fn read_selected(
    root: &Path,
    task_id: &str,
    reference: &Value,
    paths: &[String],
) -> Result<Vec<Value>, &'static str> {
    if reference.get("schema_version").and_then(Value::as_u64) != Some(1)
        || reference.get("status").and_then(Value::as_str) != Some("available")
        || reference.get("task_id").and_then(Value::as_str) != Some(task_id)
    {
        return Err("context_archive_unavailable");
    }
    let digest = reference
        .get("manifest_sha256")
        .and_then(Value::as_str)
        .ok_or("context_archive_corrupt")?;
    let manifest_bytes = read_object(root, task_id, digest)?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| "context_archive_corrupt")?;
    if manifest.schema_version != 1
        || manifest.task_id != task_id
        || manifest.items.len() > MAX_ITEMS
        || reference.get("total").and_then(Value::as_u64) != Some(manifest.items.len() as u64)
    {
        return Err("context_archive_corrupt");
    }
    let total = manifest
        .items
        .iter()
        .try_fold(manifest_bytes.len(), |total, entry| {
            total.checked_add(entry.size_bytes)
        });
    if total.is_none_or(|bytes| {
        bytes > MAX_TOTAL_BYTES
            || reference.get("total_bytes").and_then(Value::as_u64) != Some(bytes as u64)
    }) {
        return Err("context_archive_corrupt");
    }
    let mut output = Vec::with_capacity(manifest.items.len());
    for (index, entry) in manifest.items.iter().enumerate() {
        if entry.size_bytes > MAX_ITEM_BYTES {
            return Err("context_archive_corrupt");
        }
        let selected = paths.iter().any(|path| {
            path.strip_prefix("/capabilities/")
                .and_then(|suffix| suffix.split('/').next())
                .and_then(|index| index.parse::<usize>().ok())
                == Some(index)
        });
        if selected {
            let bytes = read_object(root, task_id, &entry.sha256)?;
            if bytes.len() != entry.size_bytes {
                return Err("context_archive_corrupt");
            }
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "context_archive_corrupt")?;
            if value.get("capability") != Some(&entry.capability)
                || value.get("status") != Some(&entry.status)
            {
                return Err("context_archive_corrupt");
            }
            output.push(value);
        } else {
            output.push(json!({"capability":entry.capability,"status":entry.status}));
        }
    }
    Ok(output)
}

#[cfg(test)]
#[path = "task_context_archive_tests.rs"]
mod tests;

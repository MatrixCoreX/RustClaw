//! Host-owned comparison of submitted UTF-8 bytes with a committed write snapshot.
use claw_core::capability_result::{CapabilityResultEnvelope, CapabilityResultStatus};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) fn bind_workspace_write_receipt(
    result: &mut CapabilityResultEnvelope,
    submitted_args: &Value,
) {
    // A receipt is host provenance, never a producer's similarly named field.
    if let Some(provenance) = result.provenance.as_object_mut() {
        provenance.remove("write_content_receipt");
    }
    if let Some(receipt) = workspace_write_receipt(result, submitted_args) {
        result.provenance["write_content_receipt"] = receipt;
    }
}

fn workspace_write_receipt(result: &CapabilityResultEnvelope, args: &Value) -> Option<Value> {
    if result.status != CapabilityResultStatus::Ok
        || result.effect.as_deref() != Some("mutate")
        || result.provenance["source"].as_str() != Some("runtime_step")
        || result.provenance["host_workspace_primitive"].as_bool() != Some(true)
    {
        return None;
    }
    let output = result.data.get("output")?;
    if output["source"].as_str() != Some("workspace_mutation")
        || output["status"].as_str() != Some("ok")
        || output["action"].as_str() != Some("write_text")
        || output["isolation_root"].as_str() != Some("workspace://current")
        || !matches!(output["state"].as_str(), Some("applied" | "no_op"))
    {
        return None;
    }
    let content = args.get("content")?.as_str()?;
    let target = output.get("target_path")?.as_str()?;
    let checkpoint_id = output
        .get("checkpoint_id")?
        .as_str()
        .filter(|id| !id.is_empty())?;
    let mutation_id = output
        .get("mutation_id")?
        .as_str()
        .filter(|id| !id.is_empty())?;
    let [after] = output.get("after")?.as_array()?.as_slice() else {
        return None;
    };
    if after["kind"].as_str() != Some("file") || after["path"].as_str() != Some(target) {
        return None;
    }
    let observed_sha256 = after.get("sha256")?.as_str()?;
    let observed_bytes = after.get("size_bytes")?.as_u64()?;
    let submitted_sha256 = format!("sha256:{:x}", Sha256::digest(content.as_bytes()));
    let matches = observed_bytes == content.len() as u64 && observed_sha256 == submitted_sha256;
    Some(json!({
        "schema_version": 1,
        "source": "host_workspace_write",
        "status": if matches { "verified" } else { "mismatch" },
        "target_path": target,
        "checkpoint_id": checkpoint_id,
        "mutation_id": mutation_id,
        "matches_submitted_content": matches,
        "submitted_encoding": "utf-8",
        "submitted_utf8_bytes": content.len(),
        "submitted_trailing_lf": content.ends_with('\n'),
        "submitted_trailing_crlf": content.ends_with("\r\n"),
        "submitted_sha256": submitted_sha256,
        "observed_size_bytes": observed_bytes,
        "observed_sha256": observed_sha256,
    }))
}

#[cfg(test)]
#[path = "capability_result_write_receipt_tests.rs"]
mod tests;

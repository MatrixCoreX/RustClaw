use std::collections::BTreeSet;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::LoopState;

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        Value::Object(map) => {
            let mut sorted = serde_json::Map::new();
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(item) = map.get(key) {
                    sorted.insert(key.clone(), canonical_value(item));
                }
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

fn insert_json_fingerprint(fingerprints: &mut BTreeSet<String>, kind: &str, value: Value) {
    let canonical = canonical_value(&value).to_string();
    fingerprints.insert(format!(
        "{kind}:sha256:{:x}",
        Sha256::digest(canonical.as_bytes())
    ));
}

fn lifecycle_progress_projection(value: &Value) -> Value {
    json!({
        "state": value.get("state"),
        "source": value.get("source"),
        "resume_reason": value.get("resume_reason"),
        "message_key": value.get("message_key"),
        "poll_ref": value.get("poll_ref"),
        "resume_entrypoint": value.get("resume_entrypoint"),
        "pending_async_job": value.get("pending_async_job"),
    })
}

fn checkpoint_progress_projection(value: &Value) -> Value {
    json!({
        "resume_entrypoint": value.get("resume_entrypoint"),
        "pending_async_job": value.get("pending_async_job"),
        "completed_side_effect_refs": value.get("completed_side_effect_refs"),
        "observations": value.get("observations"),
        "capability_results": value.get("capability_results"),
        "async_poll_adapter": value.pointer("/boundary_context/async_poll_adapter"),
        "stage": value.pointer("/boundary_context/stage"),
    })
}

pub(super) fn machine_progress_fingerprints(loop_state: &LoopState) -> BTreeSet<String> {
    let mut fingerprints = BTreeSet::new();

    for skill in &loop_state.loaded_capability_skills {
        fingerprints.insert(format!("registry_capability:{skill}"));
    }
    for capability in &loop_state.loaded_mcp_capabilities {
        fingerprints.insert(format!("mcp_capability:{capability}"));
    }
    for fingerprint in loop_state.successful_action_fingerprints.keys() {
        fingerprints.insert(format!("successful_action:{fingerprint}"));
    }
    for result in &loop_state.capability_results {
        insert_json_fingerprint(
            &mut fingerprints,
            "capability_result",
            json!({
                "schema_version": result.schema_version,
                "status": result.status,
                "capability": result.capability,
                "action": result.action,
                "data": result.data,
                "artifacts": result.artifacts,
                "page": result.page,
                "truncated": result.truncated,
                "retry": result.retry,
                "effect": result.effect,
                "verification": result.verification,
                "evidence": result.evidence,
                "error": result.error,
                "continuation": result.continuation,
                "delivery": result.delivery,
            }),
        );
    }
    for step in &loop_state.executed_step_results {
        insert_json_fingerprint(
            &mut fingerprints,
            "step_result",
            json!({
                "skill": step.skill,
                "status": step.status.as_str(),
                "output": step.output,
                "error": step.error,
            }),
        );
    }
    for (alias, path) in &loop_state.written_file_aliases {
        fingerprints.insert(format!("artifact_alias:{alias}:{path}"));
    }
    if let Some(path) = loop_state.last_written_file_path.as_deref() {
        fingerprints.insert(format!("artifact_path:{path}"));
    }
    if let Some(validation) = loop_state.latest_validation_result.as_ref() {
        insert_json_fingerprint(&mut fingerprints, "validation", validation.clone());
    }
    if let Some(lifecycle) = loop_state.task_lifecycle.as_ref() {
        insert_json_fingerprint(
            &mut fingerprints,
            "task_lifecycle",
            lifecycle_progress_projection(lifecycle),
        );
    }
    if let Some(checkpoint) = loop_state.task_checkpoint.as_ref() {
        insert_json_fingerprint(
            &mut fingerprints,
            "task_checkpoint",
            checkpoint_progress_projection(checkpoint),
        );
    }

    fingerprints
}

pub(super) fn machine_progress_fingerprint_count(loop_state: &LoopState) -> usize {
    machine_progress_fingerprints(loop_state).len()
}

pub(super) fn machine_progress_digest(loop_state: &LoopState) -> Option<String> {
    let fingerprints = machine_progress_fingerprints(loop_state);
    if fingerprints.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    for fingerprint in fingerprints {
        hasher.update(fingerprint.as_bytes());
        hasher.update(b"\n");
    }
    Some(format!("sha256:{:x}", hasher.finalize()))
}

pub(super) fn unique_artifact_count(loop_state: &LoopState) -> usize {
    loop_state
        .written_file_aliases
        .values()
        .map(String::as_str)
        .chain(loop_state.last_written_file_path.as_deref())
        .collect::<BTreeSet<_>>()
        .len()
}

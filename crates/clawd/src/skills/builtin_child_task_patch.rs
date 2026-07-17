use serde_json::{json, Map, Value};

use crate::execution_isolation::{
    child_patch_base_is_parent_ancestor, cleanup_child_worktree_artifacts,
    load_validated_child_worktree_patch_artifact, ValidatedChildPatchArtifact,
};
use crate::repo::child_patch::{
    load_child_patch_record, record_child_patch_disposition, ChildPatchRecord,
};
use crate::{AppState, ClaimedTask};

use super::builtin_workspace_patch::execute_workspace_patch_for_root;

pub(super) fn execute_child_task_patch(
    state: &AppState,
    task: Option<&ClaimedTask>,
    args: &Map<String, Value>,
) -> Result<String, String> {
    ensure_only_keys(args, &["action", "child_task_id", "patch_ref"])?;
    let action = required_string(args, "action")?;
    let task = task.ok_or_else(|| child_patch_error("parent_task_context_missing", Value::Null))?;
    let child_task_id = required_string(args, "child_task_id")?;
    let record = load_child_patch_record(state, &task.task_id, child_task_id)
        .map_err(|err| child_patch_repo_error(&err))?;
    validate_optional_patch_ref(args, &record)?;

    match action {
        "review_child_patch" => review_child_patch(state, &record),
        "apply_child_patch" => apply_child_patch(state, task, &record),
        "reject_child_patch" => reject_child_patch(state, task, &record),
        _ => Err(child_patch_error(
            "unsupported_child_patch_action",
            json!({"action": action}),
        )),
    }
}

fn review_child_patch(state: &AppState, record: &ChildPatchRecord) -> Result<String, String> {
    let artifact = validated_artifact(state, record)?;
    let base_commit = required_artifact_string(&artifact.metadata, "base_commit")?;
    let base_is_parent_ancestor =
        child_patch_base_is_parent_ancestor(&state.skill_rt.workspace_root, base_commit)
            .map_err(|err| child_patch_runtime_error(&err))?;
    let patch = if artifact.patch.is_empty() {
        Value::Null
    } else {
        Value::String(
            String::from_utf8(artifact.patch)
                .map_err(|_| child_patch_error("patch_not_utf8", Value::Null))?,
        )
    };
    encode_result(json!({
        "schema_version": 1,
        "source": "workspace_patch",
        "status": "ok",
        "action": "review_child_patch",
        "parent_task_id": record.parent_task_id,
        "child_task_id": record.child_task_id,
        "child_terminal_status": record.terminal_status,
        "permission_profile": record.permission_profile,
        "allowed_capabilities": record.allowed_capabilities,
        "patch_ref": artifact.metadata.get("patch_ref").cloned().unwrap_or(Value::Null),
        "base_commit": base_commit,
        "base_is_parent_ancestor": base_is_parent_ancestor,
        "changed_files": artifact.metadata.get("changed_files").cloned().unwrap_or_else(|| json!([])),
        "changed_file_count": artifact.metadata.get("changed_file_count").cloned().unwrap_or_else(|| json!(0)),
        "patch_bytes": artifact.metadata.get("patch_bytes").cloned().unwrap_or_else(|| json!(0)),
        "precondition_hashes": artifact.metadata.get("precondition_hashes").cloned().unwrap_or_else(|| json!({})),
        "verification_artifact": record.verification_artifact,
        "patch": patch,
        "patch_disposition": record.patch_disposition,
    }))
}

fn apply_child_patch(
    state: &AppState,
    task: &ClaimedTask,
    record: &ChildPatchRecord,
) -> Result<String, String> {
    if let Some(existing) = record.patch_disposition.as_ref() {
        return resume_or_return_existing_disposition(state, task, record, existing, "applied");
    }
    if record.terminal_status != "succeeded" {
        return Err(child_patch_error(
            "child_patch_apply_requires_success",
            json!({"child_terminal_status": record.terminal_status}),
        ));
    }
    let artifact = validated_artifact(state, record)?;
    let base_commit = required_artifact_string(&artifact.metadata, "base_commit")?;
    if !child_patch_base_is_parent_ancestor(&state.skill_rt.workspace_root, base_commit)
        .map_err(|err| child_patch_runtime_error(&err))?
    {
        return Err(child_patch_error(
            "child_patch_base_not_parent_ancestor",
            json!({"base_commit": base_commit}),
        ));
    }
    let workspace_patch = apply_validated_patch(state, task, &artifact)?;
    let pending = disposition_projection(record, "applied", "pending", Some(workspace_patch));
    record_child_patch_disposition(
        state,
        &record.parent_task_id,
        &record.child_task_id,
        &pending,
    )
    .map_err(|err| child_patch_repo_error(&err))?;
    finish_cleanup(state, record, pending)
}

fn reject_child_patch(
    state: &AppState,
    task: &ClaimedTask,
    record: &ChildPatchRecord,
) -> Result<String, String> {
    if let Some(existing) = record.patch_disposition.as_ref() {
        return resume_or_return_existing_disposition(state, task, record, existing, "rejected");
    }
    let pending = disposition_projection(record, "rejected", "pending", None);
    record_child_patch_disposition(
        state,
        &record.parent_task_id,
        &record.child_task_id,
        &pending,
    )
    .map_err(|err| child_patch_repo_error(&err))?;
    finish_cleanup(state, record, pending)
}

fn resume_or_return_existing_disposition(
    state: &AppState,
    _task: &ClaimedTask,
    record: &ChildPatchRecord,
    existing: &Value,
    requested: &str,
) -> Result<String, String> {
    let observed = existing
        .get("disposition")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if observed != requested {
        return Err(child_patch_error(
            "child_patch_already_decided",
            json!({"requested_disposition": requested, "observed_disposition": observed}),
        ));
    }
    if existing.get("cleanup_status").and_then(Value::as_str) == Some("complete") {
        return encode_result(existing.clone());
    }
    finish_cleanup(state, record, existing.clone())
}

fn apply_validated_patch(
    state: &AppState,
    task: &ClaimedTask,
    artifact: &ValidatedChildPatchArtifact,
) -> Result<Value, String> {
    if artifact.patch.is_empty() {
        return Ok(json!({
            "status": "ok",
            "action": "apply_patch",
            "outcome_code": "child_patch_empty",
            "changed_files": [],
            "reversible": false,
        }));
    }
    let patch = std::str::from_utf8(&artifact.patch)
        .map_err(|_| child_patch_error("patch_not_utf8", Value::Null))?;
    let mut args = Map::new();
    args.insert("action".to_string(), json!("apply_patch"));
    args.insert("patch".to_string(), json!(patch));
    args.insert(
        "precondition_hashes".to_string(),
        artifact
            .metadata
            .get("precondition_hashes")
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    let output =
        execute_workspace_patch_for_root(&state.skill_rt.workspace_root, &task.task_id, &args)?;
    serde_json::from_str(&output)
        .map_err(|_| child_patch_error("workspace_patch_result_invalid", Value::Null))
}

fn finish_cleanup(
    state: &AppState,
    record: &ChildPatchRecord,
    mut disposition: Value,
) -> Result<String, String> {
    let cleanup =
        cleanup_child_worktree_artifacts(&state.skill_rt.workspace_root, &record.child_task_id);
    let object = disposition
        .as_object_mut()
        .ok_or_else(|| child_patch_error("patch_disposition_invalid", Value::Null))?;
    match cleanup {
        Ok(cleanup) => {
            object.insert("status".to_string(), json!("ok"));
            object.insert("cleanup_status".to_string(), json!("complete"));
            object.insert("cleanup".to_string(), cleanup);
        }
        Err(_) => {
            object.insert("status".to_string(), json!("partial"));
            object.insert("cleanup_status".to_string(), json!("pending"));
            object.insert(
                "cleanup_error_code".to_string(),
                json!("child_patch_cleanup_failed"),
            );
        }
    }
    record_child_patch_disposition(
        state,
        &record.parent_task_id,
        &record.child_task_id,
        &disposition,
    )
    .map_err(|err| child_patch_repo_error(&err))?;
    encode_result(disposition)
}

fn disposition_projection(
    record: &ChildPatchRecord,
    disposition: &str,
    cleanup_status: &str,
    workspace_patch: Option<Value>,
) -> Value {
    json!({
        "schema_version": 1,
        "source": "workspace_patch",
        "status": "pending",
        "action": if disposition == "applied" {
            "apply_child_patch"
        } else {
            "reject_child_patch"
        },
        "parent_task_id": record.parent_task_id,
        "child_task_id": record.child_task_id,
        "permission_profile": record.permission_profile,
        "allowed_capabilities": record.allowed_capabilities,
        "patch_ref": record.patch_artifact.get("patch_ref").cloned().unwrap_or(Value::Null),
        "disposition": disposition,
        "cleanup_status": cleanup_status,
        "verification_artifact": record.verification_artifact,
        "workspace_patch": workspace_patch,
    })
}

fn validated_artifact(
    state: &AppState,
    record: &ChildPatchRecord,
) -> Result<ValidatedChildPatchArtifact, String> {
    load_validated_child_worktree_patch_artifact(
        &state.skill_rt.workspace_root,
        &record.child_task_id,
        &record.patch_artifact,
    )
    .map_err(|err| child_patch_runtime_error(&err))
}

fn validate_optional_patch_ref(
    args: &Map<String, Value>,
    record: &ChildPatchRecord,
) -> Result<(), String> {
    let Some(requested) = args
        .get("patch_ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if record
        .patch_artifact
        .get("patch_ref")
        .and_then(Value::as_str)
        != Some(requested)
    {
        return Err(child_patch_error(
            "child_patch_ref_mismatch",
            json!({"requested_patch_ref": requested}),
        ));
    }
    Ok(())
}

fn required_artifact_string<'a>(artifact: &'a Value, key: &str) -> Result<&'a str, String> {
    artifact
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            child_patch_error("child_patch_artifact_field_missing", json!({"field": key}))
        })
}

fn ensure_only_keys(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(key) = args.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(child_patch_error("unexpected_arg", json!({"arg": key})));
    }
    Ok(())
}

fn required_string<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| child_patch_error("missing_arg", json!({"arg": key})))
}

fn encode_result(value: Value) -> Result<String, String> {
    serde_json::to_string(&value)
        .map_err(|_| child_patch_error("result_encode_failed", Value::Null))
}

fn child_patch_repo_error(error: &anyhow::Error) -> String {
    child_patch_error(&machine_error_code(error), Value::Null)
}

fn child_patch_runtime_error(error: &anyhow::Error) -> String {
    child_patch_error(&machine_error_code(error), Value::Null)
}

fn machine_error_code(error: &anyhow::Error) -> String {
    error
        .to_string()
        .split(':')
        .next()
        .filter(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .unwrap_or("child_patch_operation_failed")
        .to_string()
}

fn child_patch_error(error_code: &str, details: Value) -> String {
    super::builtin_error(
        "workspace_patch",
        error_code,
        format!("workspace.child_patch.{error_code}"),
        None,
        None,
        Some(json!({
            "error_code": error_code,
            "message_key": format!("workspace.child_patch.{error_code}"),
            "details": details,
        })),
    )
}

#[cfg(test)]
#[path = "builtin_child_task_patch_tests.rs"]
mod tests;

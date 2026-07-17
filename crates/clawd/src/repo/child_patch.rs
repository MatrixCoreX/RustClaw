use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::{now_ts, AppState};

#[derive(Debug, Clone)]
pub(crate) struct ChildPatchRecord {
    pub(crate) child_task_id: String,
    pub(crate) parent_task_id: String,
    pub(crate) terminal_status: String,
    pub(crate) permission_profile: String,
    pub(crate) allowed_capabilities: Vec<String>,
    pub(crate) patch_artifact: Value,
    pub(crate) verification_artifact: Option<Value>,
    pub(crate) patch_disposition: Option<Value>,
}

pub(crate) fn load_child_patch_record(
    state: &AppState,
    parent_task_id: &str,
    child_task_id: &str,
) -> anyhow::Result<ChildPatchRecord> {
    validate_task_ref(parent_task_id)?;
    validate_task_ref(child_task_id)?;
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("db_pool_failed:{err}"))?;
    let row = db
        .query_row(
            "SELECT status, payload_json, result_json
             FROM tasks
             WHERE task_id = ?1
             LIMIT 1",
            params![child_task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("child_patch_task_not_found"))?;
    child_patch_record_from_row(parent_task_id, child_task_id, row)
}

pub(crate) fn record_child_patch_disposition(
    state: &AppState,
    parent_task_id: &str,
    child_task_id: &str,
    disposition: &Value,
) -> anyhow::Result<()> {
    if !disposition.is_object() {
        anyhow::bail!("child_patch_disposition_invalid");
    }
    validate_task_ref(parent_task_id)?;
    validate_task_ref(child_task_id)?;
    let mut db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("db_pool_failed:{err}"))?;
    let transaction = db.transaction()?;
    let row = transaction
        .query_row(
            "SELECT status, payload_json, result_json
             FROM tasks
             WHERE task_id = ?1
             LIMIT 1",
            params![child_task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("child_patch_task_not_found"))?;
    let _record = child_patch_record_from_row(parent_task_id, child_task_id, row.clone())?;
    let mut child_result = parse_object(row.2.as_deref());
    let scope = child_result
        .as_object_mut()
        .expect("normalized object")
        .entry("child_task_execution_scope")
        .or_insert_with(|| json!({}));
    if !scope.is_object() {
        *scope = json!({});
    }
    scope
        .as_object_mut()
        .expect("normalized scope")
        .insert("patch_disposition".to_string(), disposition.clone());
    if let Some(child_projection) = child_result
        .get_mut("child_task_result")
        .and_then(Value::as_object_mut)
    {
        child_projection.insert("patch_disposition".to_string(), disposition.clone());
    }
    transaction.execute(
        "UPDATE tasks
         SET result_json = ?2, updated_at = ?3
         WHERE task_id = ?1",
        params![child_task_id, child_result.to_string(), now_ts()],
    )?;

    let raw_parent_result = transaction
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![parent_task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("child_patch_parent_task_not_found"))?;
    let mut parent_result = parse_object(raw_parent_result.as_deref());
    let parent_object = parent_result.as_object_mut().expect("normalized object");
    let dispositions = parent_object
        .entry("child_patch_dispositions")
        .or_insert_with(|| json!([]));
    if !dispositions.is_array() {
        *dispositions = json!([]);
    }
    let list = dispositions.as_array_mut().expect("normalized array");
    list.retain(|item| item.get("child_task_id").and_then(Value::as_str) != Some(child_task_id));
    list.push(disposition.clone());
    if list.len() > crate::child_task_contract::DEFAULT_MAX_CHILDREN_PER_PARENT {
        let remove_count = list.len() - crate::child_task_contract::DEFAULT_MAX_CHILDREN_PER_PARENT;
        list.drain(0..remove_count);
    }
    transaction.execute(
        "UPDATE tasks
         SET result_json = ?2, updated_at = ?3
         WHERE task_id = ?1",
        params![parent_task_id, parent_result.to_string(), now_ts()],
    )?;
    transaction.commit()?;
    Ok(())
}

fn child_patch_record_from_row(
    expected_parent_task_id: &str,
    expected_child_task_id: &str,
    row: (String, String, Option<String>),
) -> anyhow::Result<ChildPatchRecord> {
    if !matches!(
        row.0.as_str(),
        "succeeded" | "failed" | "timeout" | "canceled"
    ) {
        anyhow::bail!("child_patch_task_not_terminal");
    }
    let payload: Value =
        serde_json::from_str(&row.1).map_err(|_| anyhow::anyhow!("child_patch_payload_invalid"))?;
    if !crate::repo::child_tasks::is_child_subagent_payload(&payload) {
        anyhow::bail!("child_patch_payload_not_child_task");
    }
    let contract = payload
        .get("child_task_contract")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("child_patch_contract_missing"))?;
    if contract.get("parent_task_id").and_then(Value::as_str) != Some(expected_parent_task_id) {
        anyhow::bail!("child_patch_parent_mismatch");
    }
    if contract.get("child_task_id").and_then(Value::as_str) != Some(expected_child_task_id) {
        anyhow::bail!("child_patch_child_id_mismatch");
    }
    let permission_profile = contract
        .get("permission_profile")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if permission_profile != "local_worktree" {
        anyhow::bail!("child_patch_profile_mismatch");
    }
    let allowed_capabilities = contract
        .get("scope")
        .and_then(Value::as_object)
        .and_then(|scope| scope.get("allowed_capabilities"))
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= 32)
        .ok_or_else(|| anyhow::anyhow!("child_patch_capability_scope_invalid"))?
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|value| machine_capability_token(value))
                .map(|value| value.trim().to_string())
                .ok_or_else(|| anyhow::anyhow!("child_patch_capability_scope_invalid"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let result = parse_object(row.2.as_deref());
    let patch_artifact = result
        .pointer("/child_task_execution_scope/patch_artifact")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("child_patch_artifact_missing"))?;
    let patch_disposition = result
        .pointer("/child_task_execution_scope/patch_disposition")
        .filter(|value| value.is_object())
        .cloned();
    let verification_artifact = result
        .pointer("/child_task_result/verification_artifact")
        .filter(|value| value.is_object())
        .cloned();
    Ok(ChildPatchRecord {
        child_task_id: expected_child_task_id.to_string(),
        parent_task_id: expected_parent_task_id.to_string(),
        terminal_status: row.0,
        permission_profile: permission_profile.to_string(),
        allowed_capabilities,
        patch_artifact,
        verification_artifact,
        patch_disposition,
    })
}

fn machine_capability_token(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 160
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase()
                || ch.is_ascii_digit()
                || matches!(ch, '_' | '-' | '.' | ':' | '/')
        })
}

fn parse_object(raw: Option<&str>) -> Value {
    raw.and_then(|value| serde_json::from_str::<Value>(value).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn validate_task_ref(value: &str) -> anyhow::Result<()> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/'))
    {
        anyhow::bail!("child_patch_task_ref_invalid");
    }
    Ok(())
}

use super::*;

pub(super) async fn admit(state: &AppState, skill_name: &str, args: &Value) -> anyhow::Result<()> {
    if action_scoped_planner_mapping(state, skill_name, args)
        .is_none_or(|mapping| mapping.filesystem_write != Some(false))
    {
        let storage = crate::runtime::storage_pressure::admit_write(state).await?;
        if !storage.write_admission {
            return Err(structured_skill_error_from_parts(
                skill_name,
                "resource_admission_unavailable",
                "resource_admission_unavailable",
                Some(std::env::consts::OS),
                Some(json!({
                    "message_key": "clawd.execution.storage_pressure",
                    "retryable": true, "failure_phase": "pre_dispatch",
                    "side_effect_applied": false, "wait_reason": "storage_pressure",
                    "storage_pressure": storage,
                })),
            ));
        }
    }
    Ok(())
}

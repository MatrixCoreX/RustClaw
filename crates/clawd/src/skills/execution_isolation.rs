use claw_core::skill_registry::CapabilityIsolationProfile;
use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

pub(super) struct SkillExecutionIsolation {
    pub(super) state: AppState,
    pub(super) artifact_refs: Vec<Value>,
}

fn setup_error(skill_name: &str, detail: String) -> String {
    super::structured_skill_error_from_parts(
        skill_name,
        "execution_isolation_setup_failed",
        "execution_isolation_setup_failed",
        Some(std::env::consts::OS),
        Some(json!({
            "reason_code": "execution_isolation_setup_failed",
            "message_key": "clawd.execution.isolation_setup_failed",
            "detail": detail,
        })),
    )
}

pub(super) fn prepare_skill_execution_isolation(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: &Value,
) -> Result<Option<SkillExecutionIsolation>, String> {
    let Some(profile) = super::action_scoped_isolation_profile(state, skill_name, args) else {
        return Ok(None);
    };
    if profile == CapabilityIsolationProfile::RemoteExecutor {
        let remote = &state.worker.remote_executor;
        let configured = crate::remote_executor_admission::validate_feature_config(remote).is_ok();
        let reason_code = if configured {
            "remote_executor_transport_unavailable"
        } else {
            "remote_executor_unavailable"
        };
        return Err(super::structured_skill_error_from_parts(
            skill_name,
            reason_code,
            reason_code,
            Some(std::env::consts::OS),
            Some(json!({
                "schema_version": 1,
                "reason_code": reason_code,
                "message_key": format!("clawd.execution.{reason_code}"),
                "retryable": configured,
                "local_fallback": false,
            })),
        ));
    }
    if crate::task_execution_policy::task_has_unrestricted_admin_authority(state, task) {
        return Ok(None);
    }
    if let Some(current_profile) =
        crate::execution_isolation::execution_isolation_root_profile(&state.skill_rt.workspace_root)
    {
        let compatible = matches!(
            (current_profile.as_str(), profile),
            (
                "local_worktree",
                CapabilityIsolationProfile::LocalWorktree | CapabilityIsolationProfile::ReadOnly
            ) | (
                "local_temp_workspace",
                CapabilityIsolationProfile::LocalTempWorkspace
                    | CapabilityIsolationProfile::ReadOnly
            )
        );
        if compatible {
            return Ok(None);
        }
    }
    let plan = crate::execution_isolation::plan_execution_isolation(
        &state.skill_rt.workspace_root,
        &task.task_id,
        profile,
    )
    .map_err(|error| setup_error(skill_name, error.to_string()))?;
    if !plan.requires_cleanup {
        return Ok(None);
    }
    let runtime =
        crate::execution_isolation::create_execution_isolation(&plan, crate::now_ts_u64())
            .map_err(|error| setup_error(skill_name, error.to_string()))?;
    let mut isolated_state = state.clone();
    isolated_state.skill_rt.workspace_root = runtime.plan.execution_root.clone();
    isolated_state.skill_rt.default_locator_search_dir = runtime.plan.execution_root.clone();
    Ok(Some(SkillExecutionIsolation {
        state: isolated_state,
        artifact_refs: runtime.artifact_refs,
    }))
}

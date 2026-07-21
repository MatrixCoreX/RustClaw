use tracing::info;

use super::planning_prompt::{runtime_os_label, runtime_shell_label};
use super::PLAN_REPAIR_PROMPT_LOGICAL_PATH;
use crate::{llm_gateway, AppState, ClaimedTask};

#[allow(clippy::too_many_arguments)]
pub(super) async fn repair_plan_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    turn_analysis: &str,
    user_text: &str,
    repair_reason: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    attempt_ledger: &str,
    raw_plan: &str,
    round_no: usize,
    provider_timeout_seconds: Option<u64>,
) -> Result<String, String> {
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let resolved_prompt = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        PLAN_REPAIR_PROMPT_LOGICAL_PATH,
    )
    .map_err(|error| error.to_string())?;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, user_text);
    let prompt = crate::render_prompt_template(
        &resolved_prompt.template,
        &[
            ("__GOAL__", goal),
            ("__TURN_ANALYSIS__", turn_analysis),
            ("__USER_REQUEST__", &user_request_for_prompt),
            ("__REPAIR_REASON__", repair_reason),
            ("__TOOL_SPEC__", tool_spec),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
            ("__ATTEMPT_LEDGER__", attempt_ledger),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            ("__RUNTIME_OS__", &runtime_os),
            ("__RUNTIME_SHELL__", &runtime_shell),
            ("__WORKSPACE_ROOT__", &workspace_root),
            ("__RAW_PLAN__", raw_plan),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "plan_repair_prompt",
        &resolved_prompt.source,
        resolved_prompt.version.as_deref(),
        Some(round_no),
    );
    let repaired = llm_gateway::run_with_fallback_with_hints(
        state,
        task,
        &prompt,
        &resolved_prompt.source,
        crate::ChatRequestHints {
            timeout_seconds: provider_timeout_seconds,
            ..Default::default()
        },
    )
    .await?;
    info!(
        "plan_llm_repair_response task_id={} round={} raw={}",
        task.task_id,
        round_no,
        crate::truncate_for_log(&repaired)
    );
    Ok(repaired)
}

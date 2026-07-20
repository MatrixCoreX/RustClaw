use serde_json::Value;
use tracing::{info, warn};

use super::super::planning_parse::parse_single_plan_actions;
use super::super::planning_prompt::{runtime_os_label, runtime_shell_label};
use super::super::PLANNER_ABORT_COMPACT_RETRY_PROMPT_LOGICAL_PATH;
use super::LoopState;
use crate::{llm_gateway, AgentAction, AppState, ClaimedTask};

const MAX_BLOCK_CHARS: usize = 7_000;
const MAX_INVALID_OUTPUT_CHARS: usize = 1_400;

pub(super) struct PlannerAbortRecoveryInput<'a> {
    pub(super) goal: &'a str,
    pub(super) turn_analysis: &'a str,
    pub(super) user_text: &'a str,
    pub(super) tool_spec: &'a str,
    pub(super) skill_playbooks: &'a str,
    pub(super) attempt_ledger: &'a str,
    pub(super) first_raw_plan: &'a str,
    pub(super) latest_raw_plan: Option<&'a str>,
    pub(super) round_no: usize,
    pub(super) loop_state: &'a LoopState,
}

pub(super) async fn compact_retry_plan_actions(
    state: &AppState,
    task: &ClaimedTask,
    input: PlannerAbortRecoveryInput<'_>,
) -> Result<Option<(Vec<AgentAction>, String)>, String> {
    if !should_try_compact_planner_abort_recovery(input.loop_state) {
        return Ok(None);
    }

    let raw = run_compact_retry_prompt(state, task, &input).await?;
    let Some(actions) = parse_single_plan_actions(&raw, state, task).await else {
        warn!(
            "planner_abort_compact_retry_parse_failed task_id={} round={}",
            task.task_id, input.round_no
        );
        return Ok(None);
    };
    Ok(Some((actions, raw)))
}

pub(super) fn should_try_compact_planner_abort_recovery(loop_state: &LoopState) -> bool {
    loop_state.execution_recipe.is_active() || loop_state.output_contract.is_some()
}

async fn run_compact_retry_prompt(
    state: &AppState,
    task: &ClaimedTask,
    input: &PlannerAbortRecoveryInput<'_>,
) -> Result<String, String> {
    let resolved_prompt = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        PLANNER_ABORT_COMPACT_RETRY_PROMPT_LOGICAL_PATH,
    )
    .map_err(|err| err.to_string())?;
    let prompt_source = resolved_prompt.source;
    let prompt_version = resolved_prompt.version;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, input.user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, input.user_text);
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let planner_contract_summary =
        planner_contract_summary(input.loop_state.output_contract.as_ref());
    let invalid_plan_summary =
        invalid_plan_summary(input.first_raw_plan, input.latest_raw_plan.unwrap_or(""));
    let tool_spec = truncate_chars(input.tool_spec, MAX_BLOCK_CHARS);
    let skill_playbooks = truncate_chars(input.skill_playbooks, MAX_BLOCK_CHARS);
    let attempt_ledger = truncate_chars(input.attempt_ledger, MAX_BLOCK_CHARS);
    let prompt = crate::render_prompt_template(
        &resolved_prompt.template,
        &[
            ("__GOAL__", input.goal),
            ("__TURN_ANALYSIS__", input.turn_analysis),
            ("__USER_REQUEST__", &user_request_for_prompt),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            ("__PLANNER_CONTRACT_SUMMARY__", &planner_contract_summary),
            ("__TOOL_SPEC__", &tool_spec),
            ("__SKILL_PLAYBOOKS__", &skill_playbooks),
            ("__ATTEMPT_LEDGER__", &attempt_ledger),
            ("__INVALID_PLAN_SUMMARY__", &invalid_plan_summary),
            ("__RUNTIME_OS__", &runtime_os),
            ("__RUNTIME_SHELL__", &runtime_shell),
            ("__WORKSPACE_ROOT__", &workspace_root),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "planner_abort_compact_retry_prompt",
        &prompt_source,
        prompt_version.as_deref(),
        Some(input.round_no),
    );
    let raw =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await?;
    info!(
        "planner_abort_compact_retry_response task_id={} round={} raw={}",
        task.task_id,
        input.round_no,
        crate::truncate_for_log(&raw)
    );
    Ok(raw)
}

fn planner_contract_summary(output_contract: Option<&crate::IntentOutputContract>) -> String {
    let Some(contract) = output_contract else {
        return "{}".to_string();
    };
    let value = serde_json::json!({
        "response_shape": contract.response_shape.as_str(),
        "exact_sentence_count": contract.exact_sentence_count,
        "requires_content_evidence": contract.requires_content_evidence,
        "delivery_required": contract.delivery_required,
        "locator_kind": contract.locator_kind.as_str(),
        "delivery_intent": contract.delivery_intent.as_str(),
        "structured_field_selector": contract.selection.structured_field_selector,
        "locator_hint": contract.locator_hint,
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn invalid_plan_summary(first_raw_plan: &str, latest_raw_plan: &str) -> String {
    let value = serde_json::json!({
        "first_raw_plan": invalid_output_entry(first_raw_plan),
        "latest_raw_plan": invalid_output_entry(latest_raw_plan),
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn invalid_output_entry(raw: &str) -> Value {
    let trimmed = raw.trim();
    serde_json::json!({
        "empty": trimmed.is_empty(),
        "chars": raw.chars().count(),
        "preview": truncate_chars(trimmed, MAX_INVALID_OUTPUT_CHARS),
    })
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

#[cfg(test)]
#[path = "planner_abort_recovery_tests.rs"]
mod tests;

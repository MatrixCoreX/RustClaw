use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    looks_like_raw_command_snapshot, looks_like_structured_machine_output,
    message_is_non_answer_separator,
};

pub(super) fn prefer_english_for_user_text(state: &AppState, user_text: &str) -> bool {
    match crate::language_policy::request_language_hint(user_text) {
        "zh-CN" => false,
        "mixed" => !crate::language_policy::mixed_language_prefers_cjk_response(user_text),
        "config_default" => state
            .policy
            .command_intent
            .default_locale
            .to_ascii_lowercase()
            .starts_with("en"),
        _ => true,
    }
}

pub(super) fn prefer_english_for_agent_contextual_user_text(
    state: &AppState,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    for candidate in [
        agent_run_context.and_then(|ctx| ctx.original_user_request.as_deref()),
        agent_run_context.and_then(|ctx| ctx.user_request.as_deref()),
        Some(user_text),
    ]
    .into_iter()
    .flatten()
    {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        let hint = crate::language_policy::request_language_hint(candidate);
        if hint != "config_default" {
            return match hint {
                "zh-CN" => false,
                "mixed" => !crate::language_policy::mixed_language_prefers_cjk_response(candidate),
                _ => true,
            };
        }
    }
    prefer_english_for_user_text(state, user_text)
}

pub(super) fn final_reply_language_hint(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    let mut candidates = Vec::new();
    if let Some(ctx) = agent_run_context {
        if let Some(original) = ctx.original_user_request.as_deref() {
            candidates.push(original);
        }
        if let Some(request) = ctx.user_request.as_deref() {
            candidates.push(request);
        }
        if let Some(route) = ctx.route_result.as_ref() {
            candidates.push(route.resolved_intent.as_str());
        }
    }
    candidates.push(user_text);
    if let Some(hint) = crate::language_policy::first_clear_request_language_hint(candidates) {
        return hint;
    }
    crate::language_policy::task_response_language_hint(state, task, user_text)
}

pub(super) fn route_resolved_intent(agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.resolved_intent.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

pub(super) async fn execution_recipe_budget_exhausted_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    let repair_count = loop_state.execution_recipe.repair_count.to_string();
    let max_repairs = loop_state.execution_recipe.max_repairs.to_string();
    let language_hint = final_reply_language_hint(state, task, user_text, agent_run_context);
    let contract = crate::fallback::UserResponseContract::tool_failure(
        "execution_recipe_repair_budget_exhausted",
        user_text,
        &route_resolved_intent(agent_run_context),
        vec![
            "closed_loop_stage: inspect/apply/validate".to_string(),
            format!("repair_count: {repair_count}"),
            format!("max_repairs: {max_repairs}"),
            "result_validated: false".to_string(),
        ],
        vec![
            "Do not mark the run as successful.".to_string(),
            "Do not claim validation passed.".to_string(),
            "Explain the blocker and ask for permission to continue with a different approach or more context."
                .to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
    )
    .await
}

pub(super) async fn execution_recipe_missing_success_marker_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    marker: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let contract = crate::fallback::UserResponseContract::tool_failure(
        "execution_recipe_missing_success_marker",
        user_text,
        &route_resolved_intent(agent_run_context),
        vec![
            format!("required_success_marker: {marker}"),
            "marker_observed: false".to_string(),
            "result_marked_success: false".to_string(),
        ],
        vec![
            "Do not mark the run as successful.".to_string(),
            "Do not invent the required verification marker.".to_string(),
            "Explain that the required verification signal is missing and offer to continue verification."
                .to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
    )
    .await
}

fn execution_recipe_profile_closeout_label(
    state: Option<&AppState>,
    profile: crate::execution_recipe::ExecutionRecipeProfile,
    prefer_english: bool,
) -> String {
    let token = profile.as_str();
    let key = format!("clawd.msg.execution_recipe_profile.{token}");
    match state {
        Some(state) => {
            crate::bilingual_t_with_default_vars(state, &key, token, token, prefer_english, &[])
        }
        None => token.to_string(),
    }
}

fn prefer_english_for_user_text_without_state(user_text: &str) -> bool {
    !matches!(
        crate::language_policy::request_language_hint(user_text),
        "zh-CN" | "config_default"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExecutionSummaryLanguage {
    Zh,
    En,
    Ja,
    Ko,
}

fn execution_summary_language_from_hint(hint: &str) -> ExecutionSummaryLanguage {
    let normalized = hint.trim().to_ascii_lowercase();
    if normalized.starts_with("ja") {
        ExecutionSummaryLanguage::Ja
    } else if normalized.starts_with("ko") {
        ExecutionSummaryLanguage::Ko
    } else if normalized.starts_with("zh") || normalized == "mixed" {
        ExecutionSummaryLanguage::Zh
    } else if normalized == "config_default" || normalized.is_empty() {
        ExecutionSummaryLanguage::Zh
    } else {
        ExecutionSummaryLanguage::En
    }
}

pub(super) fn execution_summary_language(
    agent_run_context: Option<&AgentRunContext>,
    user_text: Option<&str>,
) -> ExecutionSummaryLanguage {
    if let Some(original) = agent_run_context
        .and_then(|ctx| ctx.original_user_request.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let hint = crate::language_policy::request_language_hint(original);
        if hint != "config_default" {
            return execution_summary_language_from_hint(hint);
        }
    }
    user_text
        .map(crate::language_policy::request_language_hint)
        .map(execution_summary_language_from_hint)
        .unwrap_or(ExecutionSummaryLanguage::Zh)
}

pub(super) fn execution_summary_prefix(language: ExecutionSummaryLanguage) -> &'static str {
    match language {
        ExecutionSummaryLanguage::Zh => crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX,
        ExecutionSummaryLanguage::En => crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX_EN,
        ExecutionSummaryLanguage::Ja => crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX_JA,
        ExecutionSummaryLanguage::Ko => crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX_KO,
    }
}

pub(super) fn execution_summary_status_label(
    language: ExecutionSummaryLanguage,
    ok: bool,
) -> &'static str {
    match (language, ok) {
        (ExecutionSummaryLanguage::Zh, true) => "输出",
        (ExecutionSummaryLanguage::Zh, false) => "错误",
        (ExecutionSummaryLanguage::En, true) => "Output",
        (ExecutionSummaryLanguage::En, false) => "Error",
        (ExecutionSummaryLanguage::Ja, true) => "出力",
        (ExecutionSummaryLanguage::Ja, false) => "エラー",
        (ExecutionSummaryLanguage::Ko, true) => "출력",
        (ExecutionSummaryLanguage::Ko, false) => "오류",
    }
}

pub(super) fn execution_recipe_closeout_note(
    state: Option<&AppState>,
    user_text: &str,
    loop_state: &LoopState,
) -> Option<String> {
    let recipe = loop_state.execution_recipe;
    if !recipe.is_active() || !recipe.saw_validation {
        return None;
    }

    let prefer_english = state
        .map(|state| prefer_english_for_user_text(state, user_text))
        .unwrap_or_else(|| prefer_english_for_user_text_without_state(user_text));
    let profile = execution_recipe_profile_closeout_label(state, recipe.profile, prefer_english);
    let note = match recipe.target_scope {
        crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace
            if recipe.saw_external_target =>
        {
            Some(render_execution_recipe_closeout(
                state,
                "clawd.msg.execution_recipe_closeout_external_workspace",
                "external_workspace",
                recipe.profile,
                profile.as_str(),
                prefer_english,
            ))
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo => {
            Some(render_execution_recipe_closeout(
                state,
                "clawd.msg.execution_recipe_closeout_current_repo",
                "current_repo",
                recipe.profile,
                profile.as_str(),
                prefer_english,
            ))
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::System => {
            Some(render_execution_recipe_closeout(
                state,
                "clawd.msg.execution_recipe_closeout_system",
                "system",
                recipe.profile,
                profile.as_str(),
                prefer_english,
            ))
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield
            if recipe.saw_greenfield_creation =>
        {
            Some(render_execution_recipe_closeout(
                state,
                "clawd.msg.execution_recipe_closeout_greenfield",
                "greenfield",
                recipe.profile,
                profile.as_str(),
                prefer_english,
            ))
        }
        _ => None,
    };
    note.map(|mut note| {
        if let Some(validation_result) = validation_result_token_line(loop_state) {
            note.push('\n');
            note.push_str(&validation_result);
        }
        note
    })
}

fn render_execution_recipe_closeout(
    state: Option<&AppState>,
    message_key: &str,
    target_scope: &str,
    profile: crate::execution_recipe::ExecutionRecipeProfile,
    profile_label: &str,
    prefer_english: bool,
) -> String {
    let fallback = execution_recipe_closeout_machine_payload(message_key, target_scope, profile);
    match state {
        Some(state) => crate::bilingual_t_with_default_vars(
            state,
            message_key,
            &fallback,
            &fallback,
            prefer_english,
            &[("profile", profile_label)],
        ),
        None => fallback,
    }
}

fn execution_recipe_closeout_machine_payload(
    message_key: &str,
    target_scope: &str,
    profile: crate::execution_recipe::ExecutionRecipeProfile,
) -> String {
    format!(
        "message_key={message_key} target_scope={target_scope} profile={} validation_status=validated",
        profile.as_str()
    )
}

fn validation_result_token_line(loop_state: &LoopState) -> Option<String> {
    let result = loop_state.latest_validation_result.as_ref()?;
    let status = result
        .get("status_code")
        .or_else(|| result.get("status"))
        .and_then(serde_json::Value::as_str)
        .map(machine_token)?;
    let skill = result
        .get("skill")
        .and_then(serde_json::Value::as_str)
        .map(machine_token)?;
    let step = result
        .get("global_step")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            result
                .get("step_in_round")
                .and_then(serde_json::Value::as_u64)
        })
        .map(|value| value.to_string())?;
    Some(format!(
        "validation_status={status} validation_skill={skill} validation_step={step}"
    ))
}

fn machine_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '/'))
        .take(80)
        .collect()
}

fn can_attach_execution_recipe_closeout(
    final_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let trimmed = final_text.trim();
    if trimmed.is_empty()
        || crate::finalize::parse_delivery_token(trimmed).is_some()
        || looks_like_structured_machine_output(trimmed)
        || looks_like_raw_command_snapshot(trimmed)
    {
        return false;
    }
    let is_scalar = matches!(
        agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .map(|route| route.output_contract.response_shape),
        Some(crate::OutputResponseShape::Scalar)
    );
    !is_scalar
        || crate::agent_engine::loop_control::requested_success_marker(agent_run_context).is_some()
}

pub(super) fn attach_execution_recipe_closeout_to_delivery(
    state: Option<&AppState>,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut [String],
) {
    let Some(last) = delivery_messages.last_mut() else {
        return;
    };
    if !can_attach_execution_recipe_closeout(last, agent_run_context) {
        return;
    }
    let Some(mut note) = execution_recipe_closeout_note(state, user_text, loop_state) else {
        return;
    };
    if let Some(marker) =
        crate::agent_engine::loop_control::requested_success_marker(agent_run_context)
    {
        if !note.contains(marker) {
            note = format!("{note}\n\n{marker}");
        }
    }
    *last = format!("{note}\n\n{}", last.trim());
}

pub(super) fn ensure_requested_success_marker_visible(
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
) {
    let Some(marker) =
        crate::agent_engine::loop_control::requested_success_marker(agent_run_context)
    else {
        return;
    };
    if delivery_messages.iter().any(|item| item.contains(marker)) {
        return;
    }

    if let Some(last) = delivery_messages.last_mut() {
        let trimmed = last.trim();
        if !trimmed.is_empty() && crate::finalize::parse_delivery_token(trimmed).is_none() {
            *last = format!("{trimmed}\n\n{marker}");
            return;
        }
    }
    delivery_messages.push(marker.to_string());
}

pub(super) fn missing_requested_success_marker<'a>(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
    delivery_messages: &'a [String],
) -> Option<&'static str> {
    let marker = crate::agent_engine::loop_control::requested_success_marker(agent_run_context)?;
    let has_marker = delivery_messages.iter().any(|item| item.contains(marker));
    if loop_state.execution_recipe.is_active() && !has_marker {
        Some(marker)
    } else {
        None
    }
}

pub(super) fn auto_requested_success_marker<'a>(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
    delivery_messages: &'a [String],
) -> Option<&'static str> {
    let marker = crate::agent_engine::loop_control::requested_success_marker(agent_run_context)?;
    let has_marker = delivery_messages.iter().any(|item| item.contains(marker));
    if loop_state.execution_recipe.is_active()
        && matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Done
        )
        && loop_state.execution_recipe.saw_validation
        && !has_marker
    {
        Some(marker)
    } else {
        None
    }
}

pub(super) fn route_allows_model_language_final_answer(route: &crate::RouteResult) -> bool {
    crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
        .is_some_and(|shape| shape.allows_model_language())
}

pub(super) fn route_prefers_language_rendered_execution_failed_step(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::ExecutionFailedStep
                && route_allows_model_language_final_answer(route)
        })
}

pub(super) fn planned_delivery_is_publishable_model_language_answer(delivery: &str) -> bool {
    let delivery = delivery.trim();
    !delivery.is_empty()
        && crate::finalize::parse_delivery_token(delivery).is_none()
        && !crate::finalize::looks_like_planner_artifact(delivery)
        && !crate::finalize::looks_like_internal_trace_artifact(delivery)
        && !looks_like_structured_machine_output(delivery)
        && !looks_like_raw_command_snapshot(delivery)
        && !message_is_non_answer_separator(delivery)
}

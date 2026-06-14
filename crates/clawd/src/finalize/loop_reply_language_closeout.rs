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

pub(super) fn prefer_english_for_final_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let normalized = final_reply_language_hint(state, task, user_text, agent_run_context)
        .trim()
        .to_ascii_lowercase()
        .to_string();
    !(normalized.starts_with("zh") || normalized == "mixed")
}

pub(super) fn deterministic_template_language_preference(
    state: &AppState,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<bool> {
    let hint = agent_run_context
        .and_then(|ctx| ctx.original_user_request.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(crate::language_policy::request_language_hint)
        .filter(|hint| *hint != "config_default")
        .unwrap_or_else(|| crate::language_policy::request_language_hint(user_text));
    let normalized = hint.trim().to_ascii_lowercase();
    if normalized.starts_with("zh") {
        Some(false)
    } else if normalized.starts_with("en") {
        Some(true)
    } else if normalized == "mixed" {
        Some(!crate::language_policy::mixed_language_prefers_cjk_response(user_text))
    } else if normalized == "config_default" || normalized.is_empty() {
        Some(
            state
                .policy
                .command_intent
                .default_locale
                .to_ascii_lowercase()
                .starts_with("en"),
        )
    } else {
        None
    }
}

pub(super) fn route_resolved_intent(agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.resolved_intent.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

fn execution_recipe_budget_exhausted_default_message(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> String {
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let repair_count = loop_state.execution_recipe.repair_count.to_string();
    let max_repairs = loop_state.execution_recipe.max_repairs.to_string();
    crate::bilingual_t_with_default_vars(
        state,
        "clawd.msg.execution_recipe_repair_budget_exhausted",
        "我已经按闭环流程继续检查、应用和验证，但修复次数已达到上限（{repair_count}/{max_repairs}），当前还没有验证通过。",
        "I kept iterating through inspect, apply, and validation, but the repair budget is exhausted ({repair_count}/{max_repairs}) and the result is still not validated.",
        prefer_english,
        &[("repair_count", &repair_count), ("max_repairs", &max_repairs)],
    )
}

pub(super) async fn execution_recipe_budget_exhausted_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    let default_text =
        execution_recipe_budget_exhausted_default_message(state, user_text, loop_state);
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
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &default_text,
    )
    .await
}

fn execution_recipe_missing_success_marker_default_message(
    state: &AppState,
    user_text: &str,
    marker: &str,
) -> String {
    let prefer_english = prefer_english_for_user_text(state, user_text);
    crate::bilingual_t_with_default_vars(
        state,
        "clawd.msg.execution_recipe_missing_success_marker",
        "这次闭环执行还没有拿到你要求的验证标记 {marker}，所以我先不把结果标记为成功。",
        "This closed-loop run did not produce the required verification marker {marker}, so I am not marking it as successful yet.",
        prefer_english,
        &[("marker", marker)],
    )
}

pub(super) async fn execution_recipe_missing_success_marker_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    marker: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    let default_text =
        execution_recipe_missing_success_marker_default_message(state, user_text, marker);
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
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &default_text,
    )
    .await
}

fn execution_recipe_profile_closeout_label(
    profile: crate::execution_recipe::ExecutionRecipeProfile,
    prefer_english: bool,
) -> &'static str {
    match (profile, prefer_english) {
        (crate::execution_recipe::ExecutionRecipeProfile::ConfigChange, false) => "配置变更",
        (crate::execution_recipe::ExecutionRecipeProfile::ConfigChange, true) => {
            "configuration change"
        }
        (crate::execution_recipe::ExecutionRecipeProfile::CodeChange, false) => "代码修改",
        (crate::execution_recipe::ExecutionRecipeProfile::CodeChange, true) => "code changes",
        (crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring, false) => "技能开发",
        (crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring, true) => {
            "skill authoring"
        }
        (crate::execution_recipe::ExecutionRecipeProfile::OpsService, false) => "运维处理",
        (crate::execution_recipe::ExecutionRecipeProfile::OpsService, true) => "ops work",
        (crate::execution_recipe::ExecutionRecipeProfile::None, false) => "处理",
        (crate::execution_recipe::ExecutionRecipeProfile::None, true) => "work",
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
    let profile = execution_recipe_profile_closeout_label(recipe.profile, prefer_english);
    match recipe.target_scope {
        crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace
            if recipe.saw_external_target =>
        {
            Some(match state {
                Some(state) => crate::bilingual_t_with_default_vars(
                    state,
                    "clawd.msg.execution_recipe_closeout_external_workspace",
                    "已在外部工作区完成{profile}，并已通过验证。",
                    "Completed {profile} in the external workspace and validated it.",
                    prefer_english,
                    &[("profile", profile)],
                ),
                None if prefer_english => {
                    format!("Completed {profile} in the external workspace and validated it.")
                }
                None => format!("已在外部工作区完成{profile}，并已通过验证。"),
            })
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo => Some(match state {
            Some(state) => crate::bilingual_t_with_default_vars(
                state,
                "clawd.msg.execution_recipe_closeout_current_repo",
                "已在当前仓库完成{profile}，并已通过验证。",
                "Completed {profile} in the current repository and validated it.",
                prefer_english,
                &[("profile", profile)],
            ),
            None if prefer_english => {
                format!("Completed {profile} in the current repository and validated it.")
            }
            None => format!("已在当前仓库完成{profile}，并已通过验证。"),
        }),
        crate::execution_recipe::ExecutionRecipeTargetScope::System => Some(match state {
            Some(state) => crate::bilingual_t_with_default_vars(
                state,
                "clawd.msg.execution_recipe_closeout_system",
                "已在系统范围完成{profile}，并已通过验证。",
                "Completed {profile} at the system scope and validated it.",
                prefer_english,
                &[("profile", profile)],
            ),
            None if prefer_english => {
                format!("Completed {profile} at the system scope and validated it.")
            }
            None => format!("已在系统范围完成{profile}，并已通过验证。"),
        }),
        crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield
            if recipe.saw_greenfield_creation =>
        {
            Some(match state {
                Some(state) => crate::bilingual_t_with_default_vars(
                    state,
                    "clawd.msg.execution_recipe_closeout_greenfield",
                    "已完成新产物创建，并已完成{profile}验证。",
                    "Created the new artifact and completed {profile} validation.",
                    prefer_english,
                    &[("profile", profile)],
                ),
                None if prefer_english => {
                    format!("Created the new artifact and completed {profile} validation.")
                }
                None => format!("已完成新产物创建，并已完成{profile}验证。"),
            })
        }
        _ => None,
    }
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

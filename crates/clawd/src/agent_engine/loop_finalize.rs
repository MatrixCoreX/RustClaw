use std::path::Path;

use tracing::info;

use super::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

fn backfill_delivery_from_last_outputs(task: &ClaimedTask, loop_state: &mut LoopState) {
    if loop_state.delivery_messages.is_empty() {
        if let Some(ref last_respond) = loop_state.last_user_visible_respond {
            if !last_respond.trim().is_empty() {
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    last_respond.clone(),
                );
                info!(
                    "final_result_use_last_respond task_id={} (delivery was empty)",
                    task.task_id
                );
            }
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(ref last_chat_output) = loop_state.last_publishable_chat_output {
            if !last_chat_output.trim().is_empty() {
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    last_chat_output.clone(),
                );
                info!(
                    "final_result_use_chat_output task_id={} (delivery was empty)",
                    task.task_id
                );
            }
        }
    }
}

fn route_requires_content_evidence(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.requires_content_evidence)
        .unwrap_or(false)
}

fn preferred_route_clarify_question(agent_run_context: Option<&AgentRunContext>) -> Option<&str> {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .filter(|route| route.needs_clarify)
        .map(|route| route.clarify_question.trim())
        .filter(|question| !question.is_empty())
}

fn route_requires_file_token(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| {
            route.output_contract.delivery_required
                || matches!(
                    route.output_contract.response_shape,
                    crate::OutputResponseShape::FileToken
                )
        })
        .unwrap_or(false)
}

fn has_missing_file_search_evidence(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().rev().any(|step| {
        if !step.is_ok() || step.skill != "fs_search" {
            return false;
        }
        let Some(output) = step.output.as_deref() else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            return false;
        };
        value.get("action").and_then(|v| v.as_str()) == Some("find_name")
            && value.get("count").and_then(|v| v.as_i64()) == Some(0)
            && value
                .get("results")
                .and_then(|v| v.as_array())
                .is_some_and(|results| results.is_empty())
    })
}

fn missing_file_delivery_answer_text(state: &AppState) -> String {
    crate::i18n_t_with_default(
        state,
        "clawd.msg.delivery.rule3_file_not_found",
        "File not found.",
    )
}

fn resolve_file_token_from_auto_locator_answer(
    answer: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let trimmed = answer.trim();
    if trimmed.is_empty()
        || trimmed.contains('\n')
        || crate::finalizer::parse_delivery_file_token(trimmed).is_some()
    {
        return None;
    }
    let auto_locator_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let auto_path = Path::new(auto_locator_path);

    let resolved = if auto_path.is_file() {
        let file_name = auto_path.file_name().and_then(|v| v.to_str())?;
        if trimmed != file_name {
            return None;
        }
        auto_path
            .canonicalize()
            .unwrap_or_else(|_| auto_path.to_path_buf())
    } else if auto_path.is_dir() {
        let candidate = auto_path.join(trimmed);
        if !candidate.is_file() {
            return None;
        }
        candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf())
    } else {
        return None;
    };

    Some(format!("FILE:{}", resolved.display()))
}

fn normalize_file_token_delivery_from_auto_locator(
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if !route_requires_file_token(agent_run_context) {
        return;
    }
    let auto_locator_path = agent_run_context.and_then(|ctx| ctx.auto_locator_path.as_deref());

    if let Some(token) = loop_state
        .last_user_visible_respond
        .as_deref()
        .and_then(|answer| resolve_file_token_from_auto_locator_answer(answer, auto_locator_path))
    {
        loop_state.last_user_visible_respond = Some(token);
    }

    for message in &mut loop_state.delivery_messages {
        if let Some(token) = resolve_file_token_from_auto_locator_answer(message, auto_locator_path)
        {
            *message = token;
        }
    }
}

fn enforce_delivery_output_contract(
    state: &AppState,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return;
    };
    if loop_state.delivery_messages.is_empty()
        && loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        return;
    }
    let seed_text = loop_state
        .last_user_visible_respond
        .clone()
        .or_else(|| loop_state.delivery_messages.last().cloned())
        .unwrap_or_default();
    let (normalized_text, normalized_messages) = crate::intercept_response_payload_for_delivery(
        state,
        user_text,
        route.wants_file_delivery,
        &route.output_contract,
        seed_text,
        loop_state.delivery_messages.clone(),
    );
    loop_state.last_user_visible_respond =
        (!normalized_text.trim().is_empty()).then_some(normalized_text);
    loop_state.delivery_messages = normalized_messages;
}

async fn discard_meta_respond_placeholder_for_content_evidence(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    requires_content_evidence: bool,
    agent_run_context: Option<&AgentRunContext>,
) {
    let Some(last_respond) = loop_state.last_user_visible_respond.as_deref() else {
        return;
    };
    let respond = last_respond.trim();
    let Some(raw_passthrough) = should_drop_passthrough_delivery_for_content_evidence(
        loop_state,
        requires_content_evidence,
        agent_run_context,
        respond,
    ) else {
        return;
    };
    let meta_placeholder =
        crate::semantic_judge::is_meta_respond_instruction(state, task, respond).await;
    if !raw_passthrough && !meta_placeholder {
        return;
    }
    info!(
        "content_evidence_drop_passthrough_respond task_id={} raw_passthrough={} meta_placeholder={} text={}",
        task.task_id,
        raw_passthrough,
        meta_placeholder,
        crate::truncate_for_log(respond)
    );
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
}

fn should_drop_passthrough_delivery_for_content_evidence(
    loop_state: &LoopState,
    requires_content_evidence: bool,
    agent_run_context: Option<&AgentRunContext>,
    respond: &str,
) -> Option<bool> {
    if !requires_content_evidence {
        return None;
    }
    if !loop_state.has_tool_or_skill_output {
        return None;
    }
    if loop_state.delivery_messages.len() != 1 {
        return None;
    }
    let delivery = loop_state.delivery_messages[0].trim();
    let respond = respond.trim();
    if delivery.is_empty() || respond.is_empty() || delivery != respond {
        return None;
    }

    let direct_observed_answer_matches =
        direct_scalar_observed_answer(None, loop_state, agent_run_context)
            .map(|(answer, _)| answer)
            .into_iter()
            .chain(
                direct_structured_observed_answer(None, loop_state, agent_run_context)
                    .map(|(answer, _)| answer),
            )
            .any(|answer| answer.trim() == respond);
    if direct_observed_answer_matches {
        return Some(false);
    }

    let raw_passthrough = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| step.is_ok() && !matches!(step.skill.as_str(), "chat" | "respond"))
        .and_then(|step| {
            let body = step.output.as_deref()?.trim();
            if body.is_empty() {
                return None;
            }
            if respond == body {
                return Some(true);
            }
            (step.skill == "list_dir"
                && super::observed_output::normalized_observed_listing(body, None).is_some_and(
                    |listing| {
                        listing.trim() == respond
                            || listing
                                .lines()
                                .map(str::trim)
                                .any(|entry| !entry.is_empty() && entry == respond)
                    },
                ))
            .then_some(true)
        })
        .unwrap_or(false);
    Some(raw_passthrough)
}

fn discard_raw_passthrough_delivery_when_structured_answer_available(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if loop_state.delivery_messages.len() != 1 {
        return;
    }
    let Some(current_delivery) = loop_state.delivery_messages.last().map(|v| v.trim()) else {
        return;
    };
    if current_delivery.is_empty() {
        return;
    }
    let Some((structured_answer, _)) =
        direct_structured_observed_answer(None, loop_state, agent_run_context)
    else {
        return;
    };
    if structured_answer.trim().is_empty() || structured_answer.trim() == current_delivery {
        return;
    }

    let raw_passthrough = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| step.is_ok() && !matches!(step.skill.as_str(), "chat" | "respond"))
        .and_then(|step| {
            let body = step.output.as_deref()?.trim();
            if body.is_empty() {
                return None;
            }
            if current_delivery == body {
                return Some(true);
            }
            let first_line = body.lines().map(str::trim).find(|line| !line.is_empty())?;
            (current_delivery == first_line).then_some(true)
        })
        .unwrap_or(false);
    if !raw_passthrough {
        return;
    }

    info!(
        "drop_raw_passthrough_delivery_for_structured_answer task_id={} raw={} structured={}",
        task.task_id,
        crate::truncate_for_log(current_delivery),
        crate::truncate_for_log(structured_answer.trim())
    );
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
}

fn direct_scalar_observed_answer(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return None;
    }
    let answer = if super::observed_output::scalar_route_prefers_structured_observed_answer(
        route, loop_state,
    ) {
        state
            .and_then(|state| {
                super::observed_output::extract_direct_answer_from_generic_output_i18n(
                    loop_state,
                    state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                super::observed_output::extract_direct_answer_from_generic_output(
                    loop_state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                state.and_then(|state| {
                    super::observed_output::extract_direct_scalar_from_generic_output_i18n(
                        loop_state,
                        state,
                        agent_run_context,
                    )
                })
            })
            .or_else(|| {
                super::observed_output::extract_direct_scalar_from_generic_output(
                    loop_state,
                    agent_run_context,
                )
            })?
    } else {
        state
            .and_then(|state| {
                super::observed_output::extract_direct_scalar_from_generic_output_i18n(
                    loop_state,
                    state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                super::observed_output::extract_direct_scalar_from_generic_output(
                    loop_state,
                    agent_run_context,
                )
            })?
    };
    if crate::finalizer::looks_like_planner_artifact(&answer)
        || crate::finalizer::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            ..Default::default()
        },
    ))
}

fn text_contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        )
    })
}

fn text_contains_ascii_alpha(text: &str) -> bool {
    text.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn prefer_english_for_user_text(state: &AppState, user_text: &str) -> bool {
    let trimmed = user_text.trim();
    match (
        text_contains_cjk(trimmed),
        text_contains_ascii_alpha(trimmed),
    ) {
        (true, false) => false,
        (false, true) => true,
        _ => state
            .command_intent
            .default_locale
            .to_ascii_lowercase()
            .starts_with("en"),
    }
}

fn execution_recipe_budget_exhausted_message(
    state: &AppState,
    user_text: &str,
    loop_state: &super::LoopState,
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

fn execution_recipe_missing_success_marker_message(
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
    let trimmed = user_text.trim();
    match (
        text_contains_cjk(trimmed),
        text_contains_ascii_alpha(trimmed),
    ) {
        (true, false) => false,
        (false, true) => true,
        _ => false,
    }
}

fn execution_recipe_closeout_note(
    state: Option<&AppState>,
    user_text: &str,
    loop_state: &LoopState,
) -> Option<String> {
    let recipe = loop_state.execution_recipe;
    if !recipe.is_active()
        || (!recipe.saw_inspect && !recipe.saw_mutation && !recipe.saw_validation)
    {
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
        || crate::finalizer::parse_delivery_token(trimmed).is_some()
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
    !is_scalar || super::loop_control::requested_success_marker(agent_run_context).is_some()
}

fn attach_execution_recipe_closeout_to_delivery(
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
    if let Some(marker) = super::loop_control::requested_success_marker(agent_run_context) {
        if !note.contains(marker) {
            note = format!("{note}\n\n{marker}");
        }
    }
    *last = format!("{note}\n\n{}", last.trim());
}

fn ensure_requested_success_marker_visible(
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
) {
    let Some(marker) = super::loop_control::requested_success_marker(agent_run_context) else {
        return;
    };
    if delivery_messages.iter().any(|item| item.contains(marker)) {
        return;
    }

    if let Some(last) = delivery_messages.last_mut() {
        let trimmed = last.trim();
        if !trimmed.is_empty() && crate::finalizer::parse_delivery_token(trimmed).is_none() {
            *last = format!("{trimmed}\n\n{marker}");
            return;
        }
    }
    delivery_messages.push(marker.to_string());
}

fn missing_requested_success_marker<'a>(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &super::LoopState,
    delivery_messages: &'a [String],
) -> Option<&'static str> {
    let marker = super::loop_control::requested_success_marker(agent_run_context)?;
    let has_marker = delivery_messages.iter().any(|item| item.contains(marker));
    if loop_state.execution_recipe.is_active() && !has_marker {
        Some(marker)
    } else {
        None
    }
}

fn auto_requested_success_marker<'a>(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &super::LoopState,
    delivery_messages: &'a [String],
) -> Option<&'static str> {
    let marker = super::loop_control::requested_success_marker(agent_run_context)?;
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

fn direct_structured_observed_answer(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) {
        return None;
    }
    let answer = state
        .and_then(|state| {
            super::observed_output::extract_direct_answer_from_generic_output_i18n(
                loop_state,
                state,
                agent_run_context,
            )
        })
        .or_else(|| {
            super::observed_output::extract_direct_answer_from_generic_output(
                loop_state,
                agent_run_context,
            )
        })?;
    if answer.trim().is_empty() {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

fn direct_non_builtin_skill_raw_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let last_skill_name = loop_state
        .output_vars
        .get("last_skill_name")
        .map(String::as_str)?;
    if state.is_builtin_skill(last_skill_name) {
        return None;
    }
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let answer = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| step.is_ok() && step.skill == last_skill_name)
        .and_then(|step| step.output.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())?
        .to_string();
    if crate::finalizer::looks_like_planner_artifact(&answer)
        || crate::finalizer::looks_like_internal_trace_artifact(&answer)
        || (looks_like_structured_machine_output(&answer)
            && !matches!(
                route.map(|route| route.output_contract.semantic_kind),
                Some(crate::OutputSemanticKind::RawCommandOutput)
            ))
    {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

async fn direct_publishable_observed_answer(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return None;
    };
    if route.output_contract.requires_content_evidence
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let observed = super::observed_output::extract_latest_generic_successful_output(loop_state)?;
    let answer = observed.body.trim().to_string();
    if answer.is_empty()
        || crate::finalizer::looks_like_planner_artifact(&answer)
        || crate::finalizer::looks_like_internal_trace_artifact(&answer)
        || looks_like_structured_machine_output(&answer)
    {
        return None;
    }
    if looks_like_raw_command_snapshot(&answer)
        && !(observed.skill == "run_cmd" && route_explicitly_requests_command_result(route))
    {
        return None;
    }
    if !crate::semantic_judge::is_publishable_raw(state, task, &answer).await {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

fn looks_like_structured_machine_output(answer: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(answer)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
}

fn looks_like_raw_command_snapshot(answer: &str) -> bool {
    let trimmed = answer.trim();
    trimmed.starts_with("exit=")
        && trimmed.contains('\n')
        && (trimmed.contains("\nCOMMAND ")
            || trimmed.contains("(LISTEN)")
            || trimmed.contains("%CPU")
            || trimmed.contains("PID PPID"))
}

fn route_explicitly_requests_command_result(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
}

fn pending_confirmation_resume_payload(
    state: &AppState,
    user_text: &str,
    loop_state: &LoopState,
) -> Option<(String, serde_json::Value)> {
    let round = loop_state.round_traces.last()?;
    let verify = round.verify_result.as_ref()?;
    if !verify_summary_requires_resume_confirmation(verify) {
        return None;
    }
    let plan = round.plan_result.as_ref()?;
    let detail = verify
        .issues
        .iter()
        .find(|issue| issue.kind == crate::verifier::VerifyIssueKind::ConfirmationRequired)
        .map(|issue| issue.detail.as_str())
        .unwrap_or("current plan requires explicit confirmation");
    Some(super::build_confirmation_required_resume_context(
        state,
        &plan.steps,
        user_text,
        &round.goal,
        &loop_state.subtask_results,
        &loop_state.delivery_messages,
        detail,
    ))
}

fn verify_summary_requires_resume_confirmation(
    verify: &crate::task_journal::TaskJournalVerifySummary,
) -> bool {
    verify.mode == crate::verifier::VerifyMode::Enforce
        && verify.approved
        && verify.needs_confirmation
}

fn finalizer_requires_clarify(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
    requires_content_evidence: bool,
    has_authoritative_delivery: bool,
) -> bool {
    if requires_content_evidence {
        if has_authoritative_delivery {
            return false;
        }
        return !matches!(
            summary.and_then(|summary| summary.disposition),
            Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
        );
    }
    false
}

fn build_finalizer_clarify_reason(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> String {
    let Some(summary) = summary else {
        return "finalizer could not confirm a reliable final answer from the observed execution result"
            .to_string();
    };
    let mut parts = Vec::new();
    if let Some(stage) = summary
        .stage
        .map(crate::task_journal::TaskJournalFinalizerStage::as_str)
    {
        parts.push(format!("stage={stage}"));
    }
    if let Some(disposition) = summary
        .disposition
        .map(crate::finalizer::FinalizerDisposition::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        parts.push(format!("disposition={disposition}"));
    }
    if let Some(fallback) = summary
        .fallback
        .map(crate::task_journal::TaskJournalFinalizerFallback::as_str)
    {
        parts.push(format!("fallback={fallback}"));
    }
    if let Some(value) = summary.completion_ok {
        parts.push(format!("completion_ok={value}"));
    }
    if let Some(value) = summary.grounded_ok {
        parts.push(format!("grounded_ok={value}"));
    }
    if let Some(value) = summary.format_ok {
        parts.push(format!("format_ok={value}"));
    }
    if let Some(value) = summary.needs_clarify {
        parts.push(format!("needs_clarify={value}"));
    }
    if parts.is_empty() {
        "finalizer could not confirm a reliable final answer from the observed execution result"
            .to_string()
    } else {
        format!(
            "finalizer could not confirm a reliable final answer from the observed execution result; {}",
            parts.join(", ")
        )
    }
}

fn build_missing_delivery_clarify_reason(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> String {
    match summary {
        Some(summary) => format!(
            "no publishable final answer was produced; {}",
            build_finalizer_clarify_reason(Some(summary))
        ),
        None => "no publishable final answer was produced from the execution result".to_string(),
    }
}

fn build_loop_journal(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_consistent: bool,
    final_text: &str,
    final_status: crate::task_journal::TaskJournalFinalStatus,
) -> crate::task_journal::TaskJournal {
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", user_text);
    if let Some(ctx) = agent_run_context {
        if let Some(route_result) = ctx.route_result.as_ref() {
            journal.record_route_result(route_result);
        }
        if let Some(context_summary) = ctx.context_bundle_summary.as_deref() {
            journal.record_context_bundle_summary(context_summary.to_string());
        }
    }
    journal.rounds = loop_state.round_traces.clone();
    for step in &loop_state.executed_step_results {
        journal.push_step_result(step);
    }
    if let Some(summary) = finalizer_summary {
        journal.record_finalizer_summary(summary);
    } else {
        journal.record_used_evidence_ids_count(0);
    }
    journal.record_delivery_consistent(delivery_consistent);
    journal.record_final_answer(final_text.to_string());
    journal.record_final_status(final_status);
    journal
}

pub(super) async fn finalize_loop_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    mut loop_state: LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<AskReply, String> {
    backfill_delivery_from_last_outputs(task, &mut loop_state);

    if let Some((user_error, resume_context)) =
        pending_confirmation_resume_payload(state, user_text, &loop_state)
    {
        let delivery_messages = vec![user_error.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&user_error, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            None,
            delivery_consistent,
            &user_error,
            crate::task_journal::TaskJournalFinalStatus::ResumeFailure,
        );
        return Ok(AskReply::non_llm(user_error.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal)
            .with_failure(user_error)
            .with_resume_context(resume_context));
    }

    if loop_state.last_stop_signal.as_deref() == Some("recipe_repair_budget_exhausted") {
        let message = execution_recipe_budget_exhausted_message(state, user_text, &loop_state);
        let delivery_messages = vec![message.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            None,
            delivery_consistent,
            &message,
            crate::task_journal::TaskJournalFinalStatus::Failure,
        );
        return Ok(AskReply::non_llm(message.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal)
            .with_failure(message));
    }

    let requires_content_evidence = route_requires_content_evidence(agent_run_context);
    discard_meta_respond_placeholder_for_content_evidence(
        state,
        task,
        &mut loop_state,
        requires_content_evidence,
        agent_run_context,
    )
    .await;
    discard_raw_passthrough_delivery_when_structured_answer_available(
        task,
        &mut loop_state,
        agent_run_context,
    );
    let mut finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary> = None;
    let should_try_observed_scalar_fallback = crate::finalizer::should_attempt_observed_fallback(
        loop_state.has_tool_or_skill_output,
        loop_state.has_recoverable_failure_context,
    ) && loop_state.delivery_messages.is_empty();
    if should_try_observed_scalar_fallback {
        if let Some((answer, summary)) =
            direct_scalar_observed_answer(Some(state), &loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_scalar task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_non_builtin_skill_raw_answer(state, &loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_non_builtin_skill_raw task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_structured_observed_answer(Some(state), &loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_structured task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            super::observed_output::synthesize_answer_from_observed_output(
                state,
                task,
                user_text,
                &loop_state,
                agent_run_context,
            )
            .await
        {
            if matches!(
                summary.disposition,
                Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
            ) && !answer.trim().is_empty()
            {
                finalizer_summary = Some(summary);
                loop_state.last_user_visible_respond = Some(answer.clone());
                append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
                info!(
                    "delivery fallback_from_observed_answer task_id={}",
                    task.task_id
                );
            } else if finalizer_summary.is_none() {
                finalizer_summary = Some(summary);
            }
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_publishable_observed_answer(state, task, &loop_state, agent_run_context).await
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_raw task_id={}",
                task.task_id
            );
        }
    }

    if let Some(marker) = auto_requested_success_marker(
        agent_run_context,
        &loop_state,
        &loop_state.delivery_messages,
    ) {
        let marker_text = marker.to_string();
        loop_state.last_user_visible_respond = Some(marker_text.clone());
        append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            marker_text,
        );
        info!(
            "delivery auto_requested_success_marker task_id={} marker={}",
            task.task_id, marker
        );
    }

    normalize_file_token_delivery_from_auto_locator(&mut loop_state, agent_run_context);
    enforce_delivery_output_contract(state, user_text, &mut loop_state, agent_run_context);

    let has_authoritative_delivery = !loop_state.delivery_messages.is_empty();
    if finalizer_requires_clarify(
        finalizer_summary.as_ref(),
        requires_content_evidence,
        has_authoritative_delivery,
    ) {
        let clarify_reason = build_finalizer_clarify_reason(finalizer_summary.as_ref());
        let clarify = crate::intent_router::generate_or_reuse_clarify_question(
            state,
            task,
            user_text,
            &clarify_reason,
            None,
            preferred_route_clarify_question(agent_run_context),
            crate::intent_router::ClarifyQuestionPolicy::SafeFallback,
        )
        .await;
        let delivery_messages = vec![clarify.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&clarify, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &clarify,
            crate::task_journal::TaskJournalFinalStatus::Clarify,
        );
        return Ok(AskReply::non_llm(clarify.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }

    let (mut delivery_deduped, _, used_last_respond) =
        crate::finalizer::build_final_delivery_with_priority(
            &loop_state.delivery_messages,
            loop_state.last_user_visible_respond.as_ref(),
        );

    if delivery_deduped.is_empty()
        && route_requires_file_token(agent_run_context)
        && has_missing_file_search_evidence(&loop_state)
    {
        let message = missing_file_delivery_answer_text(state);
        let delivery_messages = vec![message.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &message,
            crate::task_journal::TaskJournalFinalStatus::Success,
        );
        return Ok(AskReply::non_llm(message.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }

    if delivery_deduped.is_empty() {
        let clarify_reason = build_missing_delivery_clarify_reason(finalizer_summary.as_ref());
        let clarify = crate::intent_router::generate_or_reuse_clarify_question(
            state,
            task,
            user_text,
            &clarify_reason,
            None,
            preferred_route_clarify_question(agent_run_context),
            crate::intent_router::ClarifyQuestionPolicy::SafeFallback,
        )
        .await;
        let delivery_messages = vec![clarify.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&clarify, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &clarify,
            crate::task_journal::TaskJournalFinalStatus::Clarify,
        );
        return Ok(AskReply::non_llm(clarify.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }

    if let Some(marker) =
        missing_requested_success_marker(agent_run_context, &loop_state, &delivery_deduped)
    {
        let message = execution_recipe_missing_success_marker_message(state, user_text, marker);
        let delivery_messages = vec![message.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &message,
            crate::task_journal::TaskJournalFinalStatus::Failure,
        );
        return Ok(AskReply::non_llm(message.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal)
            .with_failure(message));
    }

    attach_execution_recipe_closeout_to_delivery(
        Some(state),
        user_text,
        &loop_state,
        agent_run_context,
        &mut delivery_deduped,
    );
    ensure_requested_success_marker_visible(agent_run_context, &mut delivery_deduped);

    let final_text = delivery_deduped.last().cloned().unwrap_or_default();

    if used_last_respond {
        info!(
            "final_result_source=last_respond task_id={} len={}",
            task.task_id,
            delivery_deduped.len()
        );
    } else if !delivery_deduped.is_empty() {
        info!(
            "final_result_source=delivery_messages task_id={} len={}",
            task.task_id,
            delivery_deduped.len()
        );
    }
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&final_text, &delivery_deduped);

    crate::append_act_plan_log(
        state,
        task,
        "loop_done",
        loop_state.total_steps_executed,
        loop_state.subtask_results.len(),
        loop_state.tool_calls_total,
        &format!(
            "rounds={} messages={} no_progress_count={}",
            loop_state.round_no,
            loop_state.delivery_messages.len(),
            loop_state.consecutive_no_progress
        ),
    );
    let journal = build_loop_journal(
        task,
        user_text,
        &loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_consistent,
        &final_text,
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    Ok(AskReply::non_llm(final_text)
        .with_messages(delivery_deduped)
        .with_task_journal(journal))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::Instant;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        attach_execution_recipe_closeout_to_delivery, auto_requested_success_marker,
        direct_non_builtin_skill_raw_answer, direct_scalar_observed_answer,
        discard_raw_passthrough_delivery_when_structured_answer_available,
        ensure_requested_success_marker_visible, execution_recipe_closeout_note,
        finalizer_requires_clarify, has_missing_file_search_evidence,
        looks_like_raw_command_snapshot, looks_like_structured_machine_output,
        missing_requested_success_marker, normalize_file_token_delivery_from_auto_locator,
        resolve_file_token_from_auto_locator_answer,
        should_drop_passthrough_delivery_for_content_evidence,
        verify_summary_requires_resume_confirmation,
    };
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    use crate::{
        AgentRuntimeConfig, AppState, ClaimedTask, CommandIntentRuntime, IntentOutputContract,
        OutputLocatorKind, OutputResponseShape, RateLimiter, ResumeBehavior, RiskCeiling,
        RouteResult, RoutedMode, ScheduleKind, ScheduleRuntime, SkillViewsSnapshot, ToolsPolicy,
        DEFAULT_AGENT_ID,
    };
    use claw_core::config::{
        AgentConfig, MaintenanceConfig, MemoryConfig, RoutingConfig, ToolsConfig,
    };
    use rusqlite::Connection;
    use tokio::sync::Semaphore;

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_loop_finalize_{prefix}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    fn claimed_task(task_id: &str) -> ClaimedTask {
        ClaimedTask {
            task_id: task_id.to_string(),
            user_id: 1,
            chat_id: 1,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    fn test_state() -> AppState {
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            started_at: Instant::now(),
            queue_limit: 1,
            db: Arc::new(Mutex::new(Connection::open_in_memory().expect("open db"))),
            llm_providers: Vec::new(),
            agents_by_id: Arc::new(agents_by_id),
            skill_timeout_seconds: 30,
            skill_runner_path: PathBuf::new(),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(
                    ["crypto".to_string(), "stock".to_string()]
                        .into_iter()
                        .collect::<HashSet<_>>(),
                ),
            }))),
            skill_semaphore: Arc::new(Semaphore::new(1)),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(60, 30))),
            llm_calls_per_task: Arc::new(Mutex::new(HashMap::new())),
            llm_elapsed_per_task: Arc::new(Mutex::new(HashMap::new())),
            llm_by_prompt_per_task: Arc::new(Mutex::new(HashMap::new())),
            task_schedule_intent_cache: Arc::new(Mutex::new(HashMap::new())),
            maintenance: MaintenanceConfig::default(),
            memory: MemoryConfig::default(),
            workspace_root: std::env::temp_dir(),
            default_locator_search_dir: std::env::temp_dir(),
            locator_scan_max_depth: 2,
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            active_provider_type: None,
            cmd_timeout_seconds: 30,
            max_cmd_length: 4096,
            allow_path_outside_workspace: false,
            allow_sudo: false,
            worker_task_timeout_seconds: 300,
            worker_task_heartbeat_seconds: 10,
            worker_running_no_progress_timeout_seconds: 300,
            worker_running_recovery_check_interval_seconds: 30,
            last_running_recovery_check_ts: Arc::new(Mutex::new(0)),
            routing: RoutingConfig::default(),
            persona_prompt: String::new(),
            command_intent: CommandIntentRuntime {
                all_result_suffixes: Vec::new(),
                default_locale: "zh-CN".to_string(),
                verify_enforce_enabled: false,
            },
            schedule: ScheduleRuntime {
                timezone: "Asia/Shanghai".to_string(),
                intent_prompt_template: String::new(),
                intent_prompt_source: String::new(),
                intent_rules_template: String::new(),
                locale: "zh-CN".to_string(),
                i18n_dict: HashMap::new(),
            },
            channels: crate::ChannelConfig::default(),
            http_client: reqwest::Client::new(),
            database_sqlite_path: PathBuf::new(),
            database_busy_timeout_ms: 5_000,
            self_extension: claw_core::config::SelfExtensionConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn verify_summary(
        mode: crate::verifier::VerifyMode,
    ) -> crate::task_journal::TaskJournalVerifySummary {
        crate::task_journal::TaskJournalVerifySummary {
            mode,
            approved: true,
            needs_confirmation: true,
            ..Default::default()
        }
    }

    fn finalizer_summary(
        disposition: crate::finalizer::FinalizerDisposition,
    ) -> crate::task_journal::TaskJournalFinalizerSummary {
        crate::task_journal::TaskJournalFinalizerSummary {
            disposition: Some(disposition),
            ..Default::default()
        }
    }

    fn scalar_route_result() -> RouteResult {
        RouteResult {
            routed_mode: RoutedMode::Act,
            resolved_intent: "extract scalar".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: Default::default(),
                semantic_kind: Default::default(),
                locator_hint: "package.json".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    fn free_route_result() -> RouteResult {
        let mut route = scalar_route_result();
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.requires_content_evidence = false;
        route
    }

    #[test]
    fn preferred_route_clarify_question_only_uses_explicit_route_clarify() {
        let mut route = scalar_route_result();
        route.needs_clarify = true;
        route.clarify_question = "请确认要读取哪个文件？".to_string();
        let ctx = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(
            super::preferred_route_clarify_question(Some(&ctx)),
            Some("请确认要读取哪个文件？")
        );

        let mut route = scalar_route_result();
        route.clarify_question = "不会被复用".to_string();
        let ctx = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(super::preferred_route_clarify_question(Some(&ctx)), None);
    }

    #[test]
    fn confirmation_resume_requires_enforce_mode() {
        let mut verify = verify_summary(crate::verifier::VerifyMode::ObserveOnly);
        assert!(!verify_summary_requires_resume_confirmation(&verify));

        verify.mode = crate::verifier::VerifyMode::Enforce;
        assert!(verify_summary_requires_resume_confirmation(&verify));

        verify.approved = false;
        assert!(!verify_summary_requires_resume_confirmation(&verify));
    }

    #[test]
    fn content_evidence_routes_require_clarify_without_qualified_completion() {
        assert!(finalizer_requires_clarify(None, true, false));
        assert!(!finalizer_requires_clarify(None, true, true));

        let allow_fallback =
            finalizer_summary(crate::finalizer::FinalizerDisposition::AllowFallback);
        assert!(finalizer_requires_clarify(
            Some(&allow_fallback),
            true,
            false
        ));
        assert!(!finalizer_requires_clarify(
            Some(&allow_fallback),
            true,
            true
        ));

        let qualified =
            finalizer_summary(crate::finalizer::FinalizerDisposition::QualifiedCompletion);
        assert!(!finalizer_requires_clarify(Some(&qualified), true, false));
        assert!(!finalizer_requires_clarify(None, false, false));
    }

    #[test]
    fn execution_recipe_closeout_note_mentions_external_workspace_for_english_code_change() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            saw_external_target: true,
            ..Default::default()
        };

        let note = execution_recipe_closeout_note(
            None,
            "Fix the issue in /tmp/demo and verify it.",
            &loop_state,
        )
        .expect("closeout note");
        assert!(note.contains("external workspace"));
        assert!(note.contains("code changes"));
    }

    #[test]
    fn execution_recipe_closeout_prefixes_greenfield_plain_text_delivery() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            saw_greenfield_creation: true,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["Validation passed.".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "Create a new script and verify it works.",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery.len(), 1);
        assert!(delivery[0].starts_with("Created the new artifact"));
        assert!(delivery[0].ends_with("Validation passed."));
    }

    #[test]
    fn execution_recipe_closeout_prefix_includes_requested_success_marker() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            route_result: Some(free_route_result()),
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let mut delivery = vec!["修复已经完成。".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "修复系统服务并在通过时明确输出 VALIDATION_PASSED。",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery.len(), 1);
        assert!(delivery[0].contains("系统范围"));
        assert!(delivery[0].contains("VALIDATION_PASSED"));
        assert!(delivery[0].ends_with("修复已经完成。"));
    }

    #[test]
    fn execution_recipe_closeout_prefixes_current_repo_plain_text_delivery() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["修复已经验证通过。".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "把当前仓库里的问题修好并验证。",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery.len(), 1);
        assert!(delivery[0].starts_with("已在当前仓库完成代码修改"));
        assert!(delivery[0].ends_with("修复已经验证通过。"));
    }

    #[test]
    fn execution_recipe_closeout_note_mentions_system_scope_for_english_ops() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };

        let note = execution_recipe_closeout_note(
            None,
            "Repair the system service and validate it.",
            &loop_state,
        )
        .expect("closeout note");
        assert!(note.contains("system scope"));
        assert!(note.contains("ops work"));
    }

    #[test]
    fn execution_recipe_closeout_note_allows_apply_phase_when_recipe_already_progressed() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };

        let note = execution_recipe_closeout_note(
            None,
            "Repair the system service and validate it.",
            &loop_state,
        )
        .expect("closeout note");
        assert!(note.contains("system scope"));
    }

    #[test]
    fn execution_recipe_closeout_skips_file_token_delivery() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            validation_required: true,
            saw_validation: true,
            saw_external_target: true,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["FILE:/tmp/report.txt".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "Update the config in another workspace and verify it.",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery, vec!["FILE:/tmp/report.txt".to_string()]);
    }

    #[test]
    fn execution_recipe_closeout_skips_scalar_route_delivery() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            validation_required: true,
            saw_validation: true,
            saw_external_target: true,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["42".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "Fix the value in /tmp/demo and just answer with the number.",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery, vec!["42".to_string()]);
    }

    #[test]
    fn execution_recipe_closeout_allows_scalar_route_when_success_marker_requested() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            route_result: Some(scalar_route_result()),
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let mut delivery = vec!["VALIDATION_PASSED".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "修复当前仓库问题，通过时明确输出 VALIDATION_PASSED。",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery.len(), 1);
        assert!(delivery[0].contains("当前仓库"));
        assert!(delivery[0].contains("VALIDATION_PASSED"));
    }

    #[test]
    fn ensure_requested_success_marker_visible_appends_marker_to_closeout_text() {
        let ctx = super::super::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let mut delivery =
            vec!["Completed ops work at the system scope and validated it.".to_string()];

        ensure_requested_success_marker_visible(Some(&ctx), &mut delivery);

        assert_eq!(delivery.len(), 1);
        assert!(delivery[0].contains("VALIDATION_PASSED"));
        assert!(delivery[0].contains("system scope"));
    }

    #[test]
    fn missing_requested_success_marker_blocks_recipe_success() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let delivery_messages = vec!["ops-repair-bad".to_string()];
        assert_eq!(
            missing_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
            Some("VALIDATION_PASSED")
        );
    }

    #[test]
    fn requested_success_marker_allows_recipe_success_when_present() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let delivery_messages = vec!["VALIDATION_PASSED".to_string()];
        assert_eq!(
            missing_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
            None
        );
    }

    #[test]
    fn auto_requested_success_marker_fires_when_recipe_done() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let delivery_messages = vec!["status=200\nops-repair-ok".to_string()];
        assert_eq!(
            auto_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
            Some("VALIDATION_PASSED")
        );
    }

    #[test]
    fn auto_requested_success_marker_stays_off_before_recipe_done() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };
        let ctx = super::super::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let delivery_messages = vec!["status=200\nops-repair-ok".to_string()];
        assert_eq!(
            auto_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
            None
        );
    }

    #[test]
    fn direct_scalar_finalize_uses_structured_extract_field_missing_result() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("scalar fallback should succeed");
        assert_eq!(answer, "name 字段不存在");
        assert_eq!(
            summary.disposition,
            Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_scalar_finalize_uses_hidden_entries_direct_answer() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "list_dir".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(".git\nREADME.md\n.env\nsrc\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent =
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint = ".".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("hidden entries scalar fallback should succeed");
        assert_eq!(answer, "有。示例：.git, .env");
        assert_eq!(
            summary.disposition,
            Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_scalar_finalize_prefers_health_check_summary_over_raw_scalar_field() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "health_check".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent =
            "执行基础健康检查，仅提取并返回操作系统相关的关键字段，排除 RustClaw 自身的状态摘要"
                .to_string();
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let (answer, _) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("health_check scalar fallback should succeed");
        assert!(answer.contains("macOS 宿主机"));
        assert!(answer.contains("clawd_process_count=1"));
        assert!(answer.contains("clawd_health_port_open=true"));
    }

    #[test]
    fn direct_structured_finalize_uses_existence_with_path_answer_when_shape_drifted_free() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/tmp/rustclaw-workspace/rustclaw.service","size_bytes":1190},"path":"/tmp/rustclaw-workspace/rustclaw.service"}],"include_missing":true}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent =
            "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径".to_string();
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_hint = "rustclaw.service".to_string();
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let (answer, summary) =
            super::direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("existence-with-path fallback should succeed");
        assert_eq!(answer, "有，路径：/tmp/rustclaw-workspace/rustclaw.service");
        assert_eq!(
            summary.disposition,
            Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_non_builtin_finalize_preserves_raw_skill_text() {
        let state = test_state();
        let mut loop_state = super::super::LoopState::new(2);
        loop_state
            .output_vars
            .insert("last_skill_name".to_string(), "crypto".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "crypto".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                "trade_submit order_id=123 status=FILLED binance BTCUSDT buy qty_filled=0.001 avg_price=100000 quote_spent=100 USDT"
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        let (answer, summary) =
            direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
                .expect("non-builtin fallback should preserve raw text");
        assert_eq!(
            answer,
            "trade_submit order_id=123 status=FILLED binance BTCUSDT buy qty_filled=0.001 avg_price=100000 quote_spent=100 USDT"
        );
        assert_eq!(
            summary.disposition,
            Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_non_builtin_finalize_skips_structured_machine_output() {
        let state = test_state();
        let mut loop_state = super::super::LoopState::new(2);
        loop_state
            .output_vars
            .insert("last_skill_name".to_string(), "stock".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "stock".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(r#"{"symbol":"AAPL","price":201.32}"#.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        assert!(
            direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn raw_structured_passthrough_is_dropped_for_scalar_contract() {
        let raw = r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#;
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some(raw.to_string());
        loop_state.delivery_messages.push(raw.to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(raw.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                raw
            ),
            Some(true)
        );
    }

    #[test]
    fn qualified_scalar_passthrough_is_not_dropped() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some("rustclaw".to_string());
        loop_state.delivery_messages.push("rustclaw".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("rustclaw\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                "rustclaw"
            ),
            Some(false)
        );
    }

    #[test]
    fn raw_listing_passthrough_is_dropped_for_content_evidence_free_shape() {
        let listing = "base_skill_response_contract.md\nskill_integration_guide.md";
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some(listing.to_string());
        loop_state.delivery_messages.push(listing.to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "list_dir".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(format!("{listing}\n")),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                listing
            ),
            Some(true)
        );
    }

    #[test]
    fn single_listing_entry_passthrough_is_dropped_for_content_evidence() {
        let listing = "base_skill_response_contract.md\nskill_integration_guide.md";
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some("base_skill_response_contract.md".to_string());
        loop_state
            .delivery_messages
            .push("base_skill_response_contract.md".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "list_dir".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(format!("{listing}\n")),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::DirectoryPurposeSummary,
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            auto_locator_path: Some("/tmp/docs".to_string()),
            ..Default::default()
        };
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                "base_skill_response_contract.md"
            ),
            Some(true)
        );
    }

    #[test]
    fn direct_scalar_finalize_prefers_presence_plus_path_for_fs_search_presence_queries() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_search".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent =
            "检查仓库工作区中是否存在 rustclaw.service 文件，如果存在则返回路径，如果不存在则返回不存在。回答格式只输出有或没有以及路径。"
                .to_string();
        route.output_contract.requires_content_evidence = false;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("presence+path fallback should succeed");
        assert_eq!(answer, "有，路径：rustclaw.service");
        assert_eq!(
            summary.disposition,
            Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn archive_exit_zero_passthrough_is_dropped_when_structured_answer_exists() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some("exit=0".to_string());
        loop_state.delivery_messages.push("exit=0".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "archive_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                "exit=0\nupdating: tmp/rustclaw-workspace/scripts/skill_calls/\n".to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            resolved_intent:
                "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
                locator_hint: "scripts/skill_calls -> tmp/nl_archive_case.zip".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        discard_raw_passthrough_delivery_when_structured_answer_available(
            &claimed_task("task-archive"),
            &mut loop_state,
            Some(&agent_run_context),
        );

        assert!(loop_state.delivery_messages.is_empty());
        assert!(loop_state.last_user_visible_respond.is_none());
    }

    #[test]
    fn raw_publishable_guard_rejects_structured_json_payloads() {
        assert!(looks_like_structured_machine_output(
            r#"{"hostname":"rustclaw-test-host.local","cwd":"/tmp/rustclaw-workspace"}"#
        ));
        assert!(looks_like_structured_machine_output(
            r#"[{"name":"README.md"},{"name":"Cargo.toml"}]"#
        ));
        assert!(!looks_like_structured_machine_output(
            "rustclaw-test-host.local"
        ));
        assert!(!looks_like_structured_machine_output(
            "package_manager=brew"
        ));
    }

    #[test]
    fn raw_publishable_guard_rejects_multi_line_command_snapshots() {
        assert!(looks_like_raw_command_snapshot(
            "exit=0\nCOMMAND PID USER\nclawd 4498 testuser TCP *:8787 (LISTEN)\n"
        ));
        assert!(!looks_like_raw_command_snapshot("testuser"));
    }

    #[test]
    fn file_token_auto_locator_wraps_bare_filename_under_directory() {
        let temp = TempDirGuard::new("file_token_dir");
        let file_path = temp.path().join("report.txt");
        fs::write(&file_path, "hello").expect("write");
        let expected = format!(
            "FILE:{}",
            file_path
                .canonicalize()
                .unwrap_or(file_path.clone())
                .display()
        );
        assert_eq!(
            resolve_file_token_from_auto_locator_answer(
                "report.txt",
                Some(temp.path().to_string_lossy().as_ref())
            )
            .as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn file_token_auto_locator_normalizes_delivery_messages() {
        let temp = TempDirGuard::new("file_token_messages");
        let file_path = temp.path().join("report.txt");
        fs::write(&file_path, "hello").expect("write");
        let expected = format!(
            "FILE:{}",
            file_path
                .canonicalize()
                .unwrap_or(file_path.clone())
                .display()
        );
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.last_user_visible_respond = Some("report.txt".to_string());
        loop_state.delivery_messages.push("report.txt".to_string());

        let mut route = scalar_route_result();
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            auto_locator_path: Some(temp.path().to_string_lossy().to_string()),
            ..Default::default()
        };

        normalize_file_token_delivery_from_auto_locator(&mut loop_state, Some(&agent_run_context));

        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(loop_state.delivery_messages, vec![expected]);
    }

    #[test]
    fn missing_file_search_evidence_detects_zero_match_fs_search() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_search".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "find_name",
                    "count": 0,
                    "results": [],
                    "root": ""
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        assert!(has_missing_file_search_evidence(&loop_state));
    }
}

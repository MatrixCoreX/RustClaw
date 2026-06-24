use tracing::info;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

#[path = "loop_reply_scalar_answer.rs"]
mod scalar_answer;
use scalar_answer::scalar_answer_from_json;

#[path = "loop_reply_delivery_record.rs"]
mod delivery_record;
use delivery_record::log_deterministic_delivery_record;

#[path = "loop_reply_delivery_text.rs"]
mod delivery_text;
use delivery_text::{
    delivery_is_single_line_text, final_answer_text_from_delivery,
    single_publishable_delivery_message,
};

#[path = "loop_reply_service_status.rs"]
mod service_status;
use service_status::service_status_system_basic_info_answer;

#[path = "loop_reply_quantity.rs"]
mod quantity;
#[cfg(test)]
use quantity::{
    compare_paths_size_ratio_answer, latest_delivery_preserves_observed_quantity_size_facts,
    path_batch_size_comparison_answer,
};
use quantity::{
    direct_quantity_comparison_from_compare_paths, inventory_ranked_size_list_answer,
    path_batch_size_facts, replace_delivery_with_deterministic_quantity_comparison_answer,
};

#[path = "loop_reply_directory_purpose.rs"]
mod directory_purpose;
#[cfg(test)]
use directory_purpose::direct_directory_purpose_summary_from_size_facts;
use directory_purpose::{
    compose_recent_artifacts_machine_field_delivery,
    direct_current_workspace_top_level_dirs_overview_answer,
    replace_delivery_with_deterministic_current_workspace_dirs_overview_answer,
    replace_delivery_with_deterministic_directory_purpose_answer,
    replace_delivery_with_deterministic_recent_artifacts_judgment_answer,
};

#[path = "loop_reply_config_edit.rs"]
mod config_edit;
pub(crate) use config_edit::direct_config_edit_observed_answer;
use config_edit::{delivery_matches_config_guard_answer, direct_rustclaw_config_risk_answer};

#[path = "loop_reply_structured_observation.rs"]
mod structured_observation;
#[cfg(test)]
use structured_observation::deterministic_structured_file_validation_from_read_range;
use structured_observation::{
    attach_deterministic_structured_file_validation_from_read_range,
    deterministic_structured_container_summary_answer, direct_db_basic_observed_answer,
    discard_non_answer_separator_delivery_for_broad_structured_read,
    latest_successful_synthesis_output_matches, message_is_non_answer_separator,
    replace_delivery_with_deterministic_rustclaw_config_risk_answer,
};

#[path = "loop_reply_execution_status.rs"]
mod execution_status;
use execution_status::{
    attach_deterministic_execution_failed_step_answer,
    attach_deterministic_observed_execution_status_answer, delivery_is_content_answer_candidate,
    deterministic_execution_failed_step_answer, deterministic_missing_observed_target_answer,
    deterministic_observed_execution_status_answer,
    deterministic_observed_execution_status_summary, path_display_label,
    planned_delivery_identifies_failed_observed_step,
    replace_delivery_with_deterministic_execution_failed_step_answer,
    replace_delivery_with_deterministic_observed_execution_status_answer,
    successful_content_observation_should_precede_status_summary,
};

#[path = "loop_reply_markdown_scalar.rs"]
mod markdown_scalar;
#[cfg(test)]
use markdown_scalar::markdown_heading_from_read_output;
use markdown_scalar::{
    deterministic_scalar_markdown_heading_answer_from_loop,
    first_markdown_heading_from_read_output, markdown_heading_from_line,
    observed_markdown_heading_scalar_answer_for_delivery,
    replace_delivery_with_observed_markdown_heading_scalar,
    route_allows_observed_markdown_heading_scalar_delivery,
};

#[path = "loop_reply_execution_summary.rs"]
mod execution_summary;
use execution_summary::{
    attach_execution_summary_to_delivery, build_execution_summary_messages,
    delivery_contract_suppresses_execution_summary, delivery_matches_latest_publishable_synthesis,
    delivery_messages_include_delivery_token, directory_entry_groups_prefers_observed_groups,
    execution_summary_arg_is_sensitive, execution_summary_value_to_string,
    latest_grounded_synthesis_for_mixed_listing_contract,
    latest_publishable_synthesis_matches_written_file_path,
    latest_publishable_synthesis_step_matches, output_text_from_execution_result,
    plan_step_for_execution, raw_command_arg_from_plan_step, truncate_with_ellipsis,
};
#[cfg(test)]
use execution_summary::{build_execution_summary_message, should_attach_execution_summary};

#[path = "loop_reply_raw_command.rs"]
mod raw_command;
#[cfg(test)]
use raw_command::shell_stdout_redirect_target_path;
use raw_command::{
    direct_raw_command_output_projection, looks_like_raw_command_snapshot,
    looks_like_structured_machine_output, output_contract_requests_exact_delivery,
    raw_command_output_needs_structural_projection, route_explicitly_requests_command_result,
};

#[path = "loop_reply_file_delivery.rs"]
mod file_delivery;
#[cfg(test)]
use file_delivery::resolve_file_token_from_auto_locator_answer;
use file_delivery::{
    direct_created_archive_path_from_observed_archive_pack,
    direct_file_token_from_observed_auto_locator_filename,
    direct_file_token_from_observed_find_entries, direct_file_token_from_observed_inventory,
    direct_file_token_from_observed_path_batch_facts,
    direct_generated_file_path_report_from_dry_run_payload,
    direct_generated_file_path_report_from_written_path, direct_path_from_active_bound_inventory,
    direct_scalar_path_candidate_list_from_observed_outputs,
    normalize_file_token_delivery_from_auto_locator,
    normalize_file_token_delivery_from_observed_paths,
};

#[path = "loop_reply_file_missing.rs"]
mod file_missing;
pub(crate) use file_missing::output_excerpt_has_missing_file_evidence;
use file_missing::{
    append_compound_file_delivery_token_from_route,
    generated_delivery_existing_file_content_synthesis_token,
    missing_file_delivery_reply_from_loop, missing_file_path_from_loop,
    route_allows_file_token_only_fallback, route_requires_compound_content_file_delivery,
    route_requires_file_token, should_return_missing_file_delivery_reply,
    step_error_has_missing_file_evidence,
};
#[cfg(test)]
use file_missing::{
    has_missing_file_search_evidence, latest_file_delivery_observation_is_missing,
    missing_file_path_from_output,
};

#[path = "loop_reply_tail_read.rs"]
mod tail_read;
use tail_read::{
    current_user_visible_delivery_text, latest_path_batch_facts_has_implicit_metadata_fields,
    latest_plan_requested_synthesis, latest_tail_read_range_answer_from_loop,
    latest_tail_read_range_observed_answer, replace_delivery_with_latest_tail_read_range_answer,
    route_allows_latest_tail_read_range_delivery, route_requires_raw_tail_read_passthrough,
};

#[path = "loop_reply_matrix_shape.rs"]
mod matrix_shape;
pub(crate) use matrix_shape::deterministic_matrix_observed_shape_answer;
#[cfg(test)]
use matrix_shape::{
    agent_context_allows_observed_output_language_fallback, matrix_strict_list_observed_answer,
};
use matrix_shape::{
    current_synthesis_satisfies_matrix_shape,
    finalizer_summary_requires_matrix_observed_replacement, matrix_candidate_satisfies_final_shape,
    matrix_grouped_name_list_observed_answer, matrix_observed_shape_summary,
    replace_delivery_with_matrix_observed_shape_answer, route_has_contract_matrix_final_shape,
    route_requires_matrix_deterministic_final_answer, route_requires_observed_semantic_projection,
    should_try_observed_output_language_fallback, synthetic_task_for_matrix_shape_check,
};

#[path = "loop_reply_machine_envelope.rs"]
mod machine_envelope;
use machine_envelope::{
    attach_machine_envelope_delivery_from_loop, mark_machine_envelope_delivery_complete,
};

#[path = "loop_reply_delivery_backfill.rs"]
mod delivery_backfill;
use delivery_backfill::{
    backfill_delivery_from_last_outputs, candidate_matches_successful_external_observation,
    current_delivery_is_latest_publishable_synthesis, delivery_is_raw_read_observation,
    last_respond_matches_single_line_observation, latest_contractual_synthesis_output,
    latest_publishable_synthesis_step_output, replace_placeholder_delivery_with_synthesis,
    replace_raw_observation_delivery_with_synthesis, replace_raw_read_delivery_with_synthesis,
    route_expects_synthesis_over_raw_observation, step_output_is_read_range,
    strict_raw_command_output_exact_observation_answer, valid_publishable_synthesis_output,
};

#[path = "loop_reply_contract_enforce.rs"]
mod contract_enforce;
#[cfg(test)]
use contract_enforce::{
    content_evidence_terminal_respond_is_contractual_answer,
    should_drop_passthrough_delivery_for_content_evidence,
};
use contract_enforce::{
    discard_meta_respond_placeholder_for_content_evidence,
    discard_raw_passthrough_delivery_when_structured_answer_available,
    enforce_delivery_output_contract, route_accepts_filesystem_mutation_synthesis,
};

#[path = "loop_reply_filesystem_mutation.rs"]
mod filesystem_mutation;
use filesystem_mutation::filesystem_mutation_synthesis_reply;

#[path = "loop_reply_observed_contract.rs"]
mod observed_contract;
use observed_contract::{
    direct_non_builtin_skill_raw_answer, direct_publishable_observed_answer,
    direct_scalar_observed_answer, direct_structured_observed_answer,
    direct_structured_observed_answer_allowing_implicit_metadata_path_facts,
    latest_successful_observation_body, latest_successful_raw_observation_body,
    replace_delivery_with_direct_scalar_observed_answer,
    replace_delivery_with_direct_structured_observed_answer,
    replace_delivery_with_loop_contract_observed_answer,
    replace_structured_delivery_with_grounded_synthesis,
    replace_structured_delivery_with_grounded_terminal_respond,
    route_allows_direct_scalar_observed_answer,
};

#[path = "loop_reply_exact_contract.rs"]
mod exact_contract;
use exact_contract::{prefer_observed_answer_for_exact_contract, route_prefers_observed_answer};

#[path = "loop_reply_git_state.rs"]
mod git_state;
use git_state::replace_git_repository_state_machine_delivery_with_observed_synthesis;

#[path = "loop_reply_language_closeout.rs"]
mod language_closeout;
use language_closeout::{
    attach_execution_recipe_closeout_to_delivery, auto_requested_success_marker,
    ensure_requested_success_marker_visible, execution_recipe_budget_exhausted_message,
    execution_recipe_closeout_note, execution_recipe_missing_success_marker_message,
    execution_summary_language, execution_summary_prefix, execution_summary_status_label,
    final_reply_language_hint, missing_requested_success_marker,
    planned_delivery_is_publishable_model_language_answer,
    prefer_english_for_agent_contextual_user_text, prefer_english_for_user_text,
    route_allows_model_language_final_answer,
    route_prefers_language_rendered_execution_failed_step, route_resolved_intent,
    ExecutionSummaryLanguage,
};

#[path = "loop_reply_missing_delivery.rs"]
mod missing_delivery;
use missing_delivery::{
    build_finalizer_clarify_reason, build_missing_delivery_clarify_reason,
    build_pending_user_input_clarify_reason, finalizer_requires_clarify,
    observed_execution_without_publishable_delivery_reply, observed_synthesis_unavailable_reply,
    pending_confirmation_resume_payload, successful_delivery_final_status,
};
#[cfg(test)]
use missing_delivery::{
    language_rendered_failed_step_finalizer_summary,
    observed_execution_without_publishable_delivery_outcome,
    verify_summary_requires_resume_confirmation,
};

#[path = "loop_reply_route_helpers.rs"]
mod route_helpers;
use route_helpers::{
    delivery_message_is_json_container, delivery_message_is_json_object,
    preferred_route_clarify_question, route_requires_content_evidence,
    route_structured_clarify_context, structured_json_values_from_output,
};

#[path = "loop_reply_content_evidence_failure.rs"]
mod content_evidence_failure;
#[cfg(test)]
use content_evidence_failure::{
    content_evidence_failure_suppresses_execution_summary, content_evidence_missing_target_answer,
    content_evidence_step_failure_answer,
};
use content_evidence_failure::{
    content_evidence_step_failure_reply_from_loop, structured_extra_string,
};

// Stage 3.1：build_loop_journal 已搬移到 `crate::finalize::build_from_loop_state`，
// 行为零变化。本文件保留 thin alias 以最小化 diff。
use crate::finalize::build_from_loop_state as build_loop_journal;

fn attach_execution_recipe_done_machine_closeout(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if !loop_state.delivery_messages.is_empty()
        || !matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Done
        )
    {
        return false;
    }
    let Some(answer) = execution_recipe_closeout_note(None, user_text, loop_state) else {
        return false;
    };
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    loop_state.last_user_visible_respond = Some(answer.clone());
    append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
    log_deterministic_delivery_record(
        &task.task_id,
        "execution_recipe_done_machine_closeout",
        "attached",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn replace_delivery_with_service_status_observed_answer(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    let Some(answer) = service_status_system_basic_info_answer(route, loop_state) else {
        return false;
    };
    if answer.trim().is_empty() {
        return false;
    }
    let summary = crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    };
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer.trim())
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = Some(answer.clone());
    append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "service_status_observed_fields",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn replace_raw_passthrough_delivery_with_publishable_synthesis(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_expects_synthesis_over_raw_observation(route) {
        return false;
    }
    let Some(synthesis) = valid_publishable_synthesis_output(loop_state)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return false;
    };
    if crate::finalize::looks_like_planner_artifact(&synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(&synthesis)
        || crate::finalize::parse_delivery_token(&synthesis).is_some()
    {
        return false;
    }
    let has_raw_passthrough = delivery_messages.iter().any(|message| {
        let candidate = message.trim();
        !candidate.is_empty()
            && candidate != synthesis
            && (crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
                candidate, loop_state,
            ) || candidate_matches_successful_external_observation(loop_state, candidate))
    });
    if !has_raw_passthrough {
        return false;
    }
    info!(
        "final_result_replace_raw_passthrough_delivery_with_synthesis task_id={} synthesis={}",
        task.task_id,
        crate::truncate_for_log(&synthesis)
    );
    delivery_messages.clear();
    delivery_messages.push(synthesis.clone());
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        synthesis.clone(),
    );
    loop_state.last_user_visible_respond = Some(synthesis);
    log_deterministic_delivery_record(
        &task.task_id,
        "final_result_replace_raw_passthrough_delivery_with_synthesis",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn prefer_latest_synthesis_for_compound_observation_delivery(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::RawCommandOutput
        )
    {
        return false;
    }
    if output_contract_requests_exact_delivery(route) {
        return false;
    }
    if !route.output_contract.requires_content_evidence
        && !route_expects_synthesis_over_raw_observation(route)
    {
        return false;
    }
    let current = final_answer_text_from_delivery(delivery_messages)
        .trim()
        .to_string();
    let Some(synthesis) = latest_publishable_synthesis_step_output(loop_state)
        .or_else(|| latest_contractual_synthesis_output(loop_state))
        .or_else(|| latest_publishable_terminal_language_output(loop_state))
        .map(str::trim)
        .filter(|text| {
            planned_delivery_is_publishable_model_language_answer(text)
                || structured_compound_synthesis_can_replace_current_delivery(
                    route, loop_state, &current, text,
                )
        })
        .map(str::to_string)
    else {
        return false;
    };
    if current.is_empty() || current == synthesis {
        return false;
    }
    let synthesis_is_structured_json = delivery_message_is_json_container(&synthesis);
    let current_chars = current.chars().count();
    let synthesis_chars = synthesis.chars().count();
    if !synthesis_is_structured_json
        && synthesis_chars <= current_chars + 80
        && synthesis_chars.saturating_mul(4) <= current_chars.saturating_mul(5)
    {
        return false;
    }

    delivery_messages.clear();
    delivery_messages.push(synthesis.clone());
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        synthesis.clone(),
    );
    loop_state.last_user_visible_respond = Some(synthesis);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
    log_deterministic_delivery_record(
        &task.task_id,
        "compound_observation_latest_synthesis",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn structured_compound_synthesis_can_replace_current_delivery(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    current: &str,
    synthesis: &str,
) -> bool {
    let current = current.trim();
    let synthesis = synthesis.trim();
    if current.is_empty()
        || synthesis.is_empty()
        || delivery_message_is_json_container(current)
        || !delivery_message_is_json_container(synthesis)
        || crate::finalize::looks_like_planner_artifact(synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(synthesis)
        || crate::finalize::is_execution_summary_message(synthesis)
        || output_contract_requests_exact_delivery(route)
        || route.output_contract.delivery_required
    {
        return false;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        return false;
    }
    let observation_count = loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think" | "answer_verifier"
                )
        })
        .count();
    observation_count >= 2
}

fn latest_publishable_terminal_language_output(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter(|step| matches!(step.skill.as_str(), "synthesize_answer" | "respond"))
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .find(|output| {
            !output.is_empty()
                && planned_delivery_is_publishable_model_language_answer(output)
                && !crate::finalize::is_execution_summary_message(output)
        })
}

pub(crate) async fn finalize_loop_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    mut loop_state: LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<AskReply, String> {
    // §3.3 Stage 3.2 invariant：进入 LOOP REPLY finalize 子层时，
    // ask_state 必须处于 Executing 或 Finalizing 之一。Executing 表示
    // agent loop 刚跑完一轮、本函数即将做最后归约；Finalizing 表示
    // 主路径已经在 ResumeExecuting 分支预先标记过 finalize 阶段。
    // 注：测试环境与未启用 §3.1 注册（registry 未 set）时返回 None，
    // 此时不触发 panic（相当于运行期 noop），release build 完全无开销。
    debug_assert!(
        matches!(
            state.current_ask_state(&task.task_id),
            None | Some(crate::AskState::Executing) | Some(crate::AskState::Finalizing)
        ),
        "finalize_loop_reply invariant: ask_state must be Executing|Finalizing, got {:?} (task_id={})",
        state.current_ask_state(&task.task_id),
        task.task_id,
    );

    backfill_delivery_from_last_outputs(task, &mut loop_state, agent_run_context);

    if let Some((user_error, resume_context)) =
        pending_confirmation_resume_payload(state, task, user_text, &loop_state).await
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
        let message = execution_recipe_budget_exhausted_message(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
        )
        .await;
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
    backfill_delivery_from_last_outputs(task, &mut loop_state, agent_run_context);
    let mut finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary> = None;
    attach_machine_envelope_delivery_from_loop(
        task,
        &mut loop_state,
        &mut finalizer_summary,
        agent_run_context,
    );
    if let Some(reply) =
        filesystem_mutation_synthesis_reply(task, user_text, &loop_state, agent_run_context)
    {
        return Ok(reply);
    }
    if should_return_missing_file_delivery_reply(&loop_state, agent_run_context) {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
        )
        .await
        {
            return Ok(reply);
        }
    }
    let should_try_observed_scalar_fallback = crate::finalize::should_attempt_observed_fallback(
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
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_observed_scalar",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_config_edit_observed_answer(state, user_text, &loop_state)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_config_edit_observed",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_rustclaw_config_risk_answer(state, user_text, &loop_state)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_rustclaw_config_risk_observed",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_db_basic_observed_answer(state, user_text, &loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_db_basic_observed",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
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
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_non_builtin_skill_raw",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
            if let Some((answer, summary)) =
                direct_raw_command_output_projection(state, route, &loop_state)
            {
                finalizer_summary = Some(summary);
                loop_state.last_user_visible_respond = Some(answer.clone());
                append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
                log_deterministic_delivery_record(
                    &task.task_id,
                    "fallback_from_raw_command_projection",
                    "attached",
                    agent_run_context,
                    loop_state.executed_step_results.len(),
                );
            }
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_structured_observed_answer_allowing_implicit_metadata_path_facts(
                Some(state),
                &loop_state,
                agent_run_context,
            )
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_observed_structured",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) = direct_quantity_comparison_from_compare_paths(
            state,
            user_text,
            &loop_state,
            agent_run_context,
        ) {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_compare_paths_quantity",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
        )
        .await
        {
            return Ok(reply);
        }
    }

    if loop_state.delivery_messages.is_empty()
        && route_allows_file_token_only_fallback(agent_run_context)
    {
        if let Some((answer, summary)) =
            direct_file_token_from_observed_auto_locator_filename(&loop_state, agent_run_context)
                .or_else(|| {
                    direct_file_token_from_observed_path_batch_facts(&loop_state, agent_run_context)
                })
                .or_else(|| {
                    direct_file_token_from_observed_find_entries(
                        state,
                        &loop_state,
                        agent_run_context,
                    )
                })
                .or_else(|| {
                    direct_file_token_from_observed_inventory(&loop_state, agent_run_context)
                })
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_observed_file_token",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        attach_deterministic_execution_failed_step_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        );
    }

    if loop_state.delivery_messages.is_empty() {
        if !route_prefers_language_rendered_execution_failed_step(agent_run_context)
            && !successful_content_observation_should_precede_status_summary(
                agent_run_context,
                &loop_state,
            )
        {
            attach_deterministic_observed_execution_status_answer(
                state,
                task,
                user_text,
                &mut loop_state,
                &mut finalizer_summary,
            );
        }
    }

    if let Some(reply) = content_evidence_step_failure_reply_from_loop(
        state,
        task,
        user_text,
        &loop_state,
        agent_run_context,
    )
    .await
    {
        return Ok(reply);
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_path_from_active_bound_inventory(&loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_active_bound_inventory_path",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty()
        && valid_publishable_synthesis_output(&loop_state).is_none()
        && should_try_observed_output_language_fallback(&loop_state, agent_run_context)
    {
        match crate::agent_engine::observed_output::try_synthesize_answer_from_observed_output(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
        )
        .await
        {
            Ok(Some((answer, summary))) => {
                if matches!(
                    summary.disposition,
                    Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
                ) && !answer.trim().is_empty()
                {
                    finalizer_summary = Some(summary);
                    loop_state.last_user_visible_respond = Some(answer.clone());
                    append_delivery_message(
                        &task.task_id,
                        &mut loop_state.delivery_messages,
                        answer,
                    );
                    log_deterministic_delivery_record(
                        &task.task_id,
                        "fallback_from_observed_answer",
                        "attached",
                        agent_run_context,
                        loop_state.executed_step_results.len(),
                    );
                } else if finalizer_summary.is_none() {
                    finalizer_summary = Some(summary);
                }
            }
            Ok(None) => {}
            Err(err) => {
                if !attach_execution_recipe_done_machine_closeout(
                    task,
                    user_text,
                    &mut loop_state,
                    agent_run_context,
                    &mut finalizer_summary,
                ) {
                    return Ok(observed_synthesis_unavailable_reply(
                        state,
                        task,
                        user_text,
                        &loop_state,
                        agent_run_context,
                        &err,
                    ));
                }
            }
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if !route_prefers_language_rendered_execution_failed_step(agent_run_context) {
            attach_deterministic_observed_execution_status_answer(
                state,
                task,
                user_text,
                &mut loop_state,
                &mut finalizer_summary,
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_publishable_observed_answer(state, task, &loop_state, agent_run_context).await
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_observed_raw",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    attach_execution_recipe_done_machine_closeout(
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
    );

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
        log_deterministic_delivery_record(
            &task.task_id,
            "auto_requested_success_marker",
            "attached",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
    }

    normalize_file_token_delivery_from_auto_locator(&mut loop_state, agent_run_context);
    normalize_file_token_delivery_from_observed_paths(state, &mut loop_state, agent_run_context);
    enforce_delivery_output_contract(state, task, user_text, &mut loop_state, agent_run_context)
        .await;
    replace_placeholder_delivery_with_synthesis(task, &mut loop_state);
    replace_raw_read_delivery_with_synthesis(task, &mut loop_state, agent_run_context);
    replace_raw_observation_delivery_with_synthesis(task, &mut loop_state, agent_run_context);
    let replaced_service_status = replace_delivery_with_service_status_observed_answer(
        task,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
    );
    let replaced_grounded_answer = if !replaced_service_status {
        replace_structured_delivery_with_grounded_synthesis(
            task,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        ) || replace_structured_delivery_with_grounded_terminal_respond(
            task,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    let replaced_quantity_comparison = if !replaced_grounded_answer && !replaced_service_status {
        replace_delivery_with_deterministic_quantity_comparison_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    let replaced_directory_purpose =
        if !replaced_grounded_answer && !replaced_service_status && !replaced_quantity_comparison {
            replace_delivery_with_deterministic_directory_purpose_answer(
                state,
                task,
                user_text,
                &mut loop_state,
                agent_run_context,
                &mut finalizer_summary,
            )
        } else {
            false
        };
    let replaced_current_workspace_dirs = if !replaced_grounded_answer
        && !replaced_service_status
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
    {
        replace_delivery_with_deterministic_current_workspace_dirs_overview_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    let replaced_recent_artifacts = if !replaced_grounded_answer
        && !replaced_service_status
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_current_workspace_dirs
    {
        replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
            task,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    let replaced_direct_scalar = if !replaced_grounded_answer
        && !replaced_service_status
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_current_workspace_dirs
        && !replaced_recent_artifacts
    {
        replace_delivery_with_direct_scalar_observed_answer(
            state,
            task,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    let replaced_direct_structured = if !replaced_grounded_answer
        && !replaced_service_status
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_current_workspace_dirs
        && !replaced_recent_artifacts
        && !replaced_direct_scalar
    {
        replace_delivery_with_direct_structured_observed_answer(
            state,
            task,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    let replaced_contract_answer = if !replaced_grounded_answer
        && !replaced_service_status
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_current_workspace_dirs
        && !replaced_recent_artifacts
        && !replaced_direct_scalar
        && !replaced_direct_structured
    {
        replace_delivery_with_loop_contract_observed_answer(
            task,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    let replaced_failed_step = if !replaced_grounded_answer
        && !replaced_service_status
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_current_workspace_dirs
        && !replaced_recent_artifacts
        && !replaced_direct_scalar
        && !replaced_direct_structured
        && !replaced_contract_answer
    {
        replace_delivery_with_deterministic_execution_failed_step_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    let replaced_matrix_observed_shape = if !replaced_grounded_answer
        && !replaced_service_status
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_current_workspace_dirs
        && !replaced_recent_artifacts
        && !replaced_direct_scalar
        && !replaced_direct_structured
        && !replaced_contract_answer
        && !replaced_failed_step
        && finalizer_summary_requires_matrix_observed_replacement(finalizer_summary.as_ref())
    {
        let mut delivery_messages = std::mem::take(&mut loop_state.delivery_messages);
        let replaced = replace_delivery_with_matrix_observed_shape_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut delivery_messages,
            &mut finalizer_summary,
        );
        loop_state.delivery_messages = delivery_messages;
        replaced
    } else {
        false
    };
    if !replaced_grounded_answer
        && !replaced_service_status
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_current_workspace_dirs
        && !replaced_recent_artifacts
        && !replaced_direct_scalar
        && !replaced_direct_structured
        && !replaced_contract_answer
        && !replaced_failed_step
        && !replaced_matrix_observed_shape
        && !delivery_is_content_answer_candidate(
            agent_run_context,
            &loop_state,
            &loop_state.delivery_messages,
        )
        && !route_prefers_language_rendered_execution_failed_step(agent_run_context)
    {
        replace_delivery_with_deterministic_observed_execution_status_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            &mut finalizer_summary,
        );
    }
    if !replaced_grounded_answer
        || route_requires_raw_tail_read_passthrough(
            agent_run_context.and_then(|ctx| ctx.route_result.as_ref()),
        )
    {
        replace_delivery_with_latest_tail_read_range_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        );
    }
    replace_delivery_with_deterministic_rustclaw_config_risk_answer(
        state,
        task,
        user_text,
        &mut loop_state,
        &mut finalizer_summary,
    );
    append_compound_file_delivery_token_from_route(state, task, &mut loop_state, agent_run_context);
    replace_git_repository_state_machine_delivery_with_observed_synthesis(
        state,
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
    )
    .await;
    discard_non_answer_separator_delivery_for_broad_structured_read(&task.task_id, &mut loop_state);
    if loop_state.delivery_messages.is_empty() {
        attach_deterministic_structured_file_validation_from_read_range(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        );
    }

    if let Some(reply) = content_evidence_step_failure_reply_from_loop(
        state,
        task,
        user_text,
        &loop_state,
        agent_run_context,
    )
    .await
    {
        return Ok(reply);
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
        )
        .await
        {
            return Ok(reply);
        }
    }

    let has_authoritative_delivery = !loop_state.delivery_messages.is_empty();
    if finalizer_requires_clarify(
        finalizer_summary.as_ref(),
        requires_content_evidence,
        has_authoritative_delivery,
    ) {
        let clarify_reason = build_finalizer_clarify_reason(finalizer_summary.as_ref());
        if let Some(reply) = observed_execution_without_publishable_delivery_reply(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
            &clarify_reason,
        )
        .await
        {
            return Ok(reply);
        }
        let structured_clarify_context = route_structured_clarify_context(agent_run_context);
        let clarify = crate::finalize::render_clarify_question(
            state,
            task,
            crate::finalize::ClarifyRenderRequest {
                user_request: user_text,
                resolver_reason: &clarify_reason,
                candidate_context: structured_clarify_context.as_deref(),
                preferred_question: preferred_route_clarify_question(agent_run_context),
                policy: crate::intent_router::ClarifyQuestionPolicy::SafeFallback,
                // §7.2: finalize 触发 requires_clarify（无 evidence 可合成）→ SynthesisEmpty。
                fallback_source: crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
            },
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

    let synthesis_is_publishable = valid_publishable_synthesis_output(&loop_state).is_some();
    let priority_last_respond = if synthesis_is_publishable {
        None
    } else {
        loop_state.last_user_visible_respond.as_ref()
    };
    let (mut delivery_deduped, _, used_last_respond) =
        crate::finalize::build_final_delivery_with_priority(
            &loop_state.delivery_messages,
            priority_last_respond,
        );

    if delivery_deduped.is_empty() {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
        )
        .await
        {
            return Ok(reply);
        }
    }

    if delivery_deduped.is_empty() {
        let clarify_reason = build_pending_user_input_clarify_reason(
            &loop_state,
            build_missing_delivery_clarify_reason(finalizer_summary.as_ref()),
        );
        if let Some(reply) = observed_execution_without_publishable_delivery_reply(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
            &clarify_reason,
        )
        .await
        {
            return Ok(reply);
        }
        let structured_clarify_context = route_structured_clarify_context(agent_run_context);
        let clarify = crate::finalize::render_clarify_question(
            state,
            task,
            crate::finalize::ClarifyRenderRequest {
                user_request: user_text,
                resolver_reason: &clarify_reason,
                candidate_context: structured_clarify_context.as_deref(),
                preferred_question: preferred_route_clarify_question(agent_run_context),
                policy: crate::intent_router::ClarifyQuestionPolicy::SafeFallback,
                // §7.2: 执行结束但 delivery 全空（最常见的"我需要确认一下..."触发点之一）→ SynthesisEmpty。
                fallback_source: crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
            },
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

    replace_raw_passthrough_delivery_with_publishable_synthesis(
        task,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
    );

    if let Some((answer, summary)) =
        direct_generated_file_path_report_from_dry_run_payload(&loop_state, agent_run_context)
    {
        let current = delivery_deduped.last().map(|message| message.trim());
        if current != Some(answer.as_str()) {
            delivery_deduped = vec![answer.clone()];
            loop_state.last_user_visible_respond = Some(answer);
            finalizer_summary = Some(summary);
            log_deterministic_delivery_record(
                &task.task_id,
                "generated_file_path_report_dry_run_payload",
                "replaced",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if let Some(marker) =
        missing_requested_success_marker(agent_run_context, &loop_state, &delivery_deduped)
    {
        let message = execution_recipe_missing_success_marker_message(
            state,
            task,
            user_text,
            marker,
            agent_run_context,
        )
        .await;
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

    prefer_observed_answer_for_exact_contract(
        state,
        &task.task_id,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
        &mut finalizer_summary,
    );
    replace_delivery_with_observed_markdown_heading_scalar(
        &task.task_id,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
        &mut finalizer_summary,
    );
    replace_delivery_with_matrix_observed_shape_answer(
        state,
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
        &mut finalizer_summary,
    );
    prefer_latest_synthesis_for_compound_observation_delivery(
        task,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
        &mut finalizer_summary,
    );
    mark_machine_envelope_delivery_complete(
        task,
        &mut loop_state,
        &delivery_deduped,
        &mut finalizer_summary,
        agent_run_context,
    );
    if let Some(rendered) = compose_recent_artifacts_machine_field_delivery(
        state,
        task,
        user_text,
        agent_run_context,
        &final_answer_text_from_delivery(&delivery_deduped),
    )
    .await
    {
        delivery_deduped = vec![rendered.clone()];
        loop_state.last_user_visible_respond = Some(rendered);
    }
    let exact_delivery_requested = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(output_contract_requests_exact_delivery)
        .unwrap_or(false);
    if !exact_delivery_requested {
        attach_execution_recipe_closeout_to_delivery(
            Some(state),
            user_text,
            &loop_state,
            agent_run_context,
            &mut delivery_deduped,
        );
        ensure_requested_success_marker_visible(agent_run_context, &mut delivery_deduped);
    }
    attach_execution_summary_to_delivery(
        &loop_state,
        agent_run_context,
        Some(user_text),
        &mut delivery_deduped,
    );
    prefer_latest_synthesis_for_compound_observation_delivery(
        task,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
        &mut finalizer_summary,
    );

    let final_text = final_answer_text_from_delivery(&delivery_deduped);

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

    let mut journal = build_loop_journal(
        task,
        user_text,
        &loop_state,
        agent_run_context,
        finalizer_summary.clone(),
        delivery_consistent,
        &final_text,
        successful_delivery_final_status(&loop_state, finalizer_summary.as_ref()),
    );
    if let Some(route_result) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if let Some(answer_verifier) = crate::answer_verifier::verify_answer_observe_only(
            state,
            task,
            user_text,
            route_result,
            &journal,
            &final_text,
        )
        .await
        {
            journal.record_answer_verifier_summary(answer_verifier);
        }
    }

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
    Ok(AskReply::non_llm(final_text)
        .with_messages(delivery_deduped)
        .with_task_journal(journal))
}

#[cfg(test)]
#[path = "loop_reply_tests.rs"]
mod tests;

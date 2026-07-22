use tracing::info;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

#[path = "loop_reply_renderer_registry.rs"]
mod renderer_registry;
#[cfg(test)]
#[path = "loop_reply_renderer_registry_tests.rs"]
mod renderer_registry_tests;
#[path = "loop_reply_task_lifecycle_renderers.rs"]
mod task_lifecycle_renderers;
use task_lifecycle_renderers::run_task_lifecycle_renderer_registry;

#[path = "loop_reply_deterministic_fallback_renderers.rs"]
mod deterministic_fallback_renderers;
use deterministic_fallback_renderers::run_deterministic_fallback_renderer_registry;

#[path = "loop_reply_capability_synthesis.rs"]
mod capability_synthesis;
use capability_synthesis::finalize_capability_synthesis;

#[path = "loop_reply_artifact_renderers.rs"]
mod artifact_renderers;
use artifact_renderers::normalize_file_token_delivery_from_observed_paths;

#[path = "loop_reply_final_answer_renderers.rs"]
mod final_answer_renderers;
use final_answer_renderers::{
    replace_delivery_with_matrix_observed_shape_answer,
    replace_delivery_with_requested_machine_kv_summary,
    replace_final_delivery_with_exact_observation_machine_field_projection,
};

#[path = "loop_reply_scalar_answer.rs"]
mod scalar_answer;
use scalar_answer::scalar_answer_from_json;

#[path = "loop_reply_scalar_count_projection.rs"]
mod scalar_count_projection;
use scalar_count_projection::direct_observed_count_answer_for_scalar_contract;

#[path = "loop_reply_scalar_placeholder.rs"]
mod scalar_placeholder;

#[path = "loop_reply_delivery_record.rs"]
mod delivery_record;
use delivery_record::log_deterministic_delivery_record;

#[path = "loop_reply_delivery_text.rs"]
mod delivery_text;
#[cfg(test)]
#[path = "loop_reply_delivery_text_tests.rs"]
mod delivery_text_tests;
#[cfg(test)]
use delivery_text::single_publishable_delivery_message;
use delivery_text::{delivery_is_single_line_text, final_answer_text_from_delivery};

#[path = "loop_reply_machine_projections.rs"]
mod machine_projections;
use machine_projections::inventory_ranked_size_list_answer;

#[path = "loop_reply_compare_paths_metadata.rs"]
mod compare_paths_metadata;
use compare_paths_metadata::replace_final_delivery_with_compare_paths_required_metadata;

#[path = "loop_reply_structured_observation.rs"]
mod structured_observation;
use structured_observation::{
    deterministic_structured_container_summary_answer,
    discard_non_answer_separator_delivery_for_broad_structured_read,
    message_is_non_answer_separator,
};

#[path = "loop_reply_execution_status.rs"]
mod execution_status;
#[cfg(test)]
use execution_status::planned_delivery_identifies_failed_observed_step;
use execution_status::{
    attach_deterministic_observed_execution_status_answer, delivery_is_content_answer_candidate,
    deterministic_missing_observed_target_answer, deterministic_observed_execution_status_answer,
    deterministic_observed_execution_status_summary,
    replace_delivery_with_deterministic_observed_execution_status_answer,
    successful_content_observation_should_precede_status_summary,
};

#[path = "loop_reply_execution_summary.rs"]
mod execution_summary;
use execution_summary::{
    attach_execution_summary_to_delivery, delivery_matches_latest_publishable_synthesis,
    delivery_messages_include_delivery_token, exact_observation_arg_from_plan_step,
    execution_summary_arg_is_sensitive, latest_publishable_synthesis_matches_written_file_path,
    latest_publishable_synthesis_step_matches, output_text_from_execution_result,
    plan_step_for_execution, truncate_with_ellipsis,
};
#[cfg(test)]
use execution_summary::{
    build_execution_summary_message, build_execution_summary_messages,
    delivery_contract_suppresses_execution_summary,
};

#[path = "loop_reply_exact_observation.rs"]
mod exact_observation;
use exact_observation::{
    direct_exact_observation_output_projection,
    exact_observation_output_needs_structural_projection, looks_like_structured_machine_output,
    output_contract_requests_exact_delivery,
};
pub(crate) use exact_observation::{
    exact_observation_machine_field_delivery_satisfies_request,
    exact_observation_machine_field_projection_from_journal,
};

#[path = "loop_reply_file_delivery.rs"]
mod file_delivery;
#[cfg(test)]
use file_delivery::resolve_file_token_from_auto_locator_answer;
use file_delivery::{
    direct_async_poll_result_report_from_payload, direct_exact_scalar_path_from_dry_run_payload,
    direct_exact_scalar_path_from_written_path,
    direct_file_token_from_observed_auto_locator_filename,
    direct_file_token_from_observed_find_entries, direct_file_token_from_observed_inventory,
    direct_file_token_from_observed_path_batch_facts, direct_path_from_active_bound_inventory,
    direct_scalar_path_candidate_list_from_observed_outputs,
    normalize_file_token_delivery_from_auto_locator,
};

#[path = "loop_reply_file_missing.rs"]
mod file_missing;
pub(crate) use file_missing::output_excerpt_has_missing_file_evidence;
pub(crate) use file_missing::preserve_compound_content_summary_with_file_token;
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
    current_user_visible_delivery_text, latest_bounded_read_range_answer_from_loop,
    latest_plan_requested_synthesis, latest_tail_read_range_answer_from_loop,
    latest_tail_read_range_observed_answer, replace_delivery_with_latest_tail_read_range_answer,
    route_allows_latest_tail_read_range_delivery, route_requires_exact_tail_read_delivery,
    tail_read_directory_inventory_projection_available,
};

#[path = "loop_reply_matrix_shape.rs"]
mod matrix_shape;
pub(crate) use matrix_shape::deterministic_matrix_observed_shape_answer;
#[cfg(test)]
use matrix_shape::route_has_evidence_policy_final_shape;
#[cfg(test)]
use matrix_shape::{
    agent_context_allows_observed_output_language_fallback, matrix_strict_list_observed_answer,
};
use matrix_shape::{
    current_synthesis_satisfies_evidence_policy_shape,
    evidence_policy_candidate_satisfies_final_shape,
    finalizer_summary_requires_matrix_observed_replacement,
    generic_observed_machine_projection_answer, matrix_observed_shape_summary,
    route_requires_evidence_policy_deterministic_final_answer,
    route_requires_observed_output_projection, should_try_observed_output_language_fallback,
    synthetic_task_for_evidence_policy_shape_check,
};

#[path = "loop_reply_machine_envelope.rs"]
mod machine_envelope;
use machine_envelope::{
    attach_machine_envelope_delivery_from_loop, mark_machine_envelope_delivery_complete,
};

#[path = "loop_reply_machine_kv.rs"]
mod machine_kv;

#[path = "loop_reply_machine_payload.rs"]
mod machine_payload;
use machine_payload::render_machine_payload_delivery_if_needed;
#[cfg(test)]
use machine_payload::{
    visible_answer_is_machine_payload, visible_answer_is_observed_machine_projection,
    visible_machine_payload_should_remain_structured,
};

#[path = "loop_reply_clarify_envelope.rs"]
mod clarify_envelope;

#[path = "loop_reply_control_envelope.rs"]
mod control_envelope;

#[path = "loop_reply_delivery_backfill.rs"]
mod delivery_backfill;
use delivery_backfill::{
    backfill_delivery_from_last_outputs, current_delivery_is_latest_publishable_synthesis,
    current_delivery_is_latest_terminal_respond, delivery_is_direct_read_observation,
    last_respond_matches_single_line_observation, publishable_summary_has_multi_source_observation,
    replace_direct_observation_delivery_with_synthesis,
    replace_direct_read_observation_with_synthesis, replace_placeholder_delivery_with_synthesis,
    step_output_is_read_range, strict_exact_observation_output_exact_observation_answer,
    valid_publishable_synthesis_output,
};
pub(crate) use delivery_backfill::{
    latest_contractual_synthesis_output, latest_publishable_respond_step_output,
    route_expects_synthesis_over_direct_observation,
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
    discard_observed_passthrough_delivery_when_structured_answer_available,
    enforce_delivery_output_contract,
};

#[path = "loop_reply_observed_contract.rs"]
mod observed_contract;
use observed_contract::{
    direct_non_builtin_skill_raw_answer, direct_publishable_observed_answer,
    direct_scalar_observed_answer, direct_structured_observed_answer,
    direct_structured_observed_answer_allowing_implicit_metadata_path_facts,
    latest_successful_observation_body, latest_successful_step_output,
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

#[path = "loop_reply_language_closeout.rs"]
mod language_closeout;
#[cfg(test)]
use language_closeout::execution_recipe_closeout_note;
pub(crate) use language_closeout::planned_delivery_is_publishable_model_language_answer;
use language_closeout::{
    attach_execution_recipe_closeout_to_delivery, attach_execution_recipe_done_machine_closeout,
    auto_requested_success_marker, ensure_requested_success_marker_visible,
    execution_recipe_budget_exhausted_message, execution_recipe_missing_success_marker_message,
    final_reply_language_hint, missing_requested_success_marker,
    route_allows_model_language_final_answer, route_resolved_intent,
};

#[path = "loop_reply_local_code_projection.rs"]
mod local_code_projection;
use local_code_projection::{
    attach_local_code_strict_json_projection, sync_final_delivery_with_local_code_projection,
    sync_latest_synthesis_local_code_projection_if_needed,
    sync_recorded_local_code_projection_if_needed,
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
    observed_delivery_has_complete_contract_evidence,
    observed_execution_without_publishable_delivery_outcome,
    pre_execution_confirmation_checkpoint_seed, verify_summary_requires_resume_confirmation,
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
use content_evidence_failure::content_evidence_step_failure_reply_from_loop;
#[cfg(test)]
use content_evidence_failure::{
    content_evidence_failure_suppresses_execution_summary, content_evidence_missing_target_answer,
    content_evidence_step_failure_answer,
};

#[path = "loop_reply_synthesis_preference.rs"]
mod synthesis_preference;
#[cfg(test)]
use synthesis_preference::structured_compound_synthesis_can_replace_current_delivery;
use synthesis_preference::{
    prefer_content_evidence_synthesis_for_final_delivery,
    prefer_latest_synthesis_for_compound_observation_delivery,
    replace_observed_passthrough_delivery_with_publishable_synthesis,
};

use crate::finalize::build_terminal_from_loop_state as build_loop_journal;

fn priority_last_respond_for_final_delivery<'a>(
    loop_state: &'a LoopState,
    _finalizer_summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
    synthesis_is_publishable: bool,
) -> Option<&'a String> {
    if synthesis_is_publishable {
        return None;
    }
    let last_respond = loop_state.last_user_visible_respond.as_ref()?;
    if !loop_state.delivery_messages.is_empty()
        && !latest_executed_step_is_respond(loop_state)
        && !delivery_messages_contain_last_respond(&loop_state.delivery_messages, last_respond)
    {
        return None;
    }
    Some(last_respond)
}

fn latest_executed_step_is_respond(loop_state: &LoopState) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.is_ok())
        .is_some_and(|step| step.skill == "respond")
}

fn delivery_messages_contain_last_respond(
    delivery_messages: &[String],
    last_respond: &str,
) -> bool {
    let last = crate::finalize::normalize_user_visible_text(last_respond).trim();
    delivery_messages
        .iter()
        .map(|message| crate::finalize::normalize_user_visible_text(message).trim())
        .any(|message| message == last)
}

pub(crate) async fn finalize_loop_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    mut loop_state: LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<AskReply, String> {
    let effective_agent_run_context =
        effective_agent_run_context_for_finalization(agent_run_context, &loop_state);
    let agent_run_context = effective_agent_run_context.as_ref();
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
        pending_confirmation_resume_payload(state, task, user_text, &mut loop_state).await?
    {
        let delivery_messages = vec![user_error.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&user_error, &delivery_messages);
        let journal = build_loop_journal(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            None,
            delivery_consistent,
            &user_error,
            crate::task_journal::TaskJournalFinalStatus::ResumeFailure,
        )
        .await;
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
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            None,
            delivery_consistent,
            &message,
            crate::task_journal::TaskJournalFinalStatus::Failure,
        )
        .await;
        return Ok(AskReply::non_llm(message.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal)
            .with_failure(message));
    }

    if !machine_envelope::loop_has_machine_envelope(&loop_state) {
        if let Some(reply) = finalize_capability_synthesis(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
        )
        .await
        {
            return Ok(reply);
        }
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
    discard_observed_passthrough_delivery_when_structured_answer_available(
        task,
        &mut loop_state,
        agent_run_context,
    );
    backfill_delivery_from_last_outputs(task, &mut loop_state, agent_run_context);
    let mut finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary> = None;
    let attached_machine_envelope = attach_machine_envelope_delivery_from_loop(
        task,
        &mut loop_state,
        &mut finalizer_summary,
        agent_run_context,
    );
    if attached_machine_envelope
        && machine_envelope::loop_has_subagent_machine_envelope(&loop_state)
    {
        let delivery_messages = loop_state.delivery_messages.clone();
        let message = loop_state
            .last_user_visible_respond
            .clone()
            .unwrap_or_default();
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
        let journal = build_loop_journal(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &message,
            crate::task_journal::TaskJournalFinalStatus::Success,
        )
        .await;
        return Ok(AskReply::non_llm(message)
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }
    if should_return_missing_file_delivery_reply(&loop_state, agent_run_context) {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &mut loop_state,
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
        if let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) {
            if let Some((answer, summary)) =
                direct_exact_observation_output_projection(state, route, &loop_state)
            {
                finalizer_summary = Some(summary);
                loop_state.last_user_visible_respond = Some(answer.clone());
                append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
                log_deterministic_delivery_record(
                    &task.task_id,
                    "fallback_from_exact_observation_projection",
                    "attached",
                    agent_run_context,
                    loop_state.executed_step_results.len(),
                );
            }
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_exact_scalar_path_from_dry_run_payload(&loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_dry_run_generated_file_payload",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
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
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &mut loop_state,
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
        if !successful_content_observation_should_precede_status_summary(
            agent_run_context,
            &loop_state,
        ) {
            attach_deterministic_observed_execution_status_answer(
                state,
                task,
                user_text,
                &mut loop_state,
                &mut finalizer_summary,
            );
        }
    }

    let replaced_scalar_placeholder_before_failure = run_deterministic_fallback_renderer_registry(
        state,
        task,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
    );

    if !replaced_scalar_placeholder_before_failure {
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

    if attach_local_code_strict_json_projection(
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
    ) {
        log_deterministic_delivery_record(
            &task.task_id,
            "fallback_from_local_code_strict_json_projection",
            "attached",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) = generic_observed_machine_projection_answer(&loop_state) {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "generic_observed_machine_projection",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty()
        && valid_publishable_synthesis_output(&loop_state).is_none()
        && should_try_observed_output_language_fallback(&loop_state, agent_run_context)
        && !crate::agent_engine::local_code_strict_json_projection_should_defer_finalizer_fallback(
            user_text,
            &loop_state,
            agent_run_context,
        )
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
        attach_deterministic_observed_execution_status_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            &mut finalizer_summary,
        );
    }

    if loop_state.delivery_messages.is_empty() {
        attach_local_code_strict_json_projection(
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        );
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
    replace_direct_read_observation_with_synthesis(task, &mut loop_state, agent_run_context);
    replace_direct_observation_delivery_with_synthesis(task, &mut loop_state, agent_run_context);
    let replaced_deterministic_fallback = run_deterministic_fallback_renderer_registry(
        state,
        task,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
    );
    let replaced_grounded_answer = if !replaced_deterministic_fallback {
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
    let replaced_direct_scalar = if !replaced_grounded_answer && !replaced_deterministic_fallback {
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
    let replaced_direct_structured =
        if !replaced_grounded_answer && !replaced_deterministic_fallback && !replaced_direct_scalar
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
        && !replaced_deterministic_fallback
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
    let replaced_matrix_observed_shape = if !replaced_grounded_answer
        && !replaced_deterministic_fallback
        && !replaced_direct_scalar
        && !replaced_direct_structured
        && !replaced_contract_answer
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
        && !replaced_deterministic_fallback
        && !replaced_direct_scalar
        && !replaced_direct_structured
        && !replaced_contract_answer
        && !replaced_matrix_observed_shape
        && !delivery_is_content_answer_candidate(
            agent_run_context,
            &loop_state,
            &loop_state.delivery_messages,
        )
        && !successful_content_observation_should_precede_status_summary(
            agent_run_context,
            &loop_state,
        )
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
        || route_requires_exact_tail_read_delivery(
            agent_run_context.and_then(|ctx| ctx.output_contract()),
        )
        || tail_read_directory_inventory_projection_available(&loop_state, agent_run_context)
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
    append_compound_file_delivery_token_from_route(state, task, &mut loop_state, agent_run_context);
    discard_non_answer_separator_delivery_for_broad_structured_read(&task.task_id, &mut loop_state);
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
            &mut loop_state,
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
                policy: crate::finalize::ClarifyQuestionPolicy::SafeFallback,
                // §7.2: finalize 触发 requires_clarify（无 evidence 可合成）→ SynthesisEmpty。
                fallback_source: crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
            },
        )
        .await;
        let delivery_messages = vec![clarify.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&clarify, &delivery_messages);
        let journal = build_loop_journal(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &clarify,
            crate::task_journal::TaskJournalFinalStatus::Clarify,
        )
        .await;
        return Ok(AskReply::non_llm(clarify.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }

    let synthesis_is_publishable = valid_publishable_synthesis_output(&loop_state).is_some();
    let priority_last_respond = priority_last_respond_for_final_delivery(
        &loop_state,
        finalizer_summary.as_ref(),
        synthesis_is_publishable,
    );
    let (mut delivery_deduped, _, used_last_respond) =
        crate::finalize::build_final_delivery_with_priority(
            &loop_state.delivery_messages,
            priority_last_respond,
        );
    if attach_machine_envelope_delivery_from_loop(
        task,
        &mut loop_state,
        &mut finalizer_summary,
        agent_run_context,
    ) {
        delivery_deduped = loop_state.delivery_messages.clone();
    }
    if run_task_lifecycle_renderer_registry(
        task,
        &mut loop_state,
        &mut delivery_deduped,
        &mut finalizer_summary,
        agent_run_context,
    ) {
        delivery_deduped = loop_state.delivery_messages.clone();
    }

    if delivery_deduped.is_empty() {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &mut loop_state,
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
                policy: crate::finalize::ClarifyQuestionPolicy::SafeFallback,
                // §7.2: 执行结束但 delivery 全空（最常见的"我需要确认一下..."触发点之一）→ SynthesisEmpty。
                fallback_source: crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
            },
        )
        .await;
        let delivery_messages = vec![clarify.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&clarify, &delivery_messages);
        let journal = build_loop_journal(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &clarify,
            crate::task_journal::TaskJournalFinalStatus::Clarify,
        )
        .await;
        return Ok(AskReply::non_llm(clarify.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }

    replace_observed_passthrough_delivery_with_publishable_synthesis(
        task,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
    );

    if let Some((answer, summary)) =
        direct_async_poll_result_report_from_payload(&loop_state, agent_run_context)
    {
        let current = delivery_deduped.last().map(|message| message.trim());
        if current != Some(answer.as_str()) {
            delivery_deduped = vec![answer.clone()];
            loop_state.last_user_visible_respond = Some(answer);
            finalizer_summary = Some(summary);
            log_deterministic_delivery_record(
                &task.task_id,
                "async_poll_result_report_payload",
                "replaced",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if let Some((answer, summary)) =
        direct_exact_scalar_path_from_dry_run_payload(&loop_state, agent_run_context)
    {
        let current = delivery_deduped.last().map(|message| message.trim());
        if current != Some(answer.as_str()) {
            delivery_deduped = vec![answer.clone()];
            loop_state.last_user_visible_respond = Some(answer);
            finalizer_summary = Some(summary);
            log_deterministic_delivery_record(
                &task.task_id,
                "exact_scalar_path_dry_run_payload",
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
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &message,
            crate::task_journal::TaskJournalFinalStatus::Failure,
        )
        .await;
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
    run_task_lifecycle_renderer_registry(
        task,
        &mut loop_state,
        &mut delivery_deduped,
        &mut finalizer_summary,
        agent_run_context,
    );
    let exact_delivery_requested = agent_run_context
        .and_then(|ctx| ctx.output_contract())
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
    replace_delivery_with_requested_machine_kv_summary(
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
        &mut delivery_deduped,
    );
    render_machine_payload_delivery_if_needed(
        state,
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        finalizer_summary.clone(),
        &mut delivery_deduped,
    )
    .await;
    prefer_content_evidence_synthesis_for_final_delivery(
        task,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
        &mut finalizer_summary,
    );
    replace_final_delivery_with_compare_paths_required_metadata(
        task,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
        &mut finalizer_summary,
    );
    replace_final_delivery_with_exact_observation_machine_field_projection(
        state,
        task,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
        &mut delivery_deduped,
    );
    if sync_final_delivery_with_local_code_projection(
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
        &mut delivery_deduped,
    ) {
        log_deterministic_delivery_record(
            &task.task_id,
            "final_delivery_from_local_code_strict_json_projection",
            "attached",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
    }
    render_machine_payload_delivery_if_needed(
        state,
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        finalizer_summary.clone(),
        &mut delivery_deduped,
    )
    .await;
    if tail_read_directory_inventory_projection_available(&loop_state, agent_run_context)
        && replace_delivery_with_latest_tail_read_range_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    {
        delivery_deduped = loop_state.delivery_messages.clone();
    }
    preserve_compound_content_summary_with_file_token(
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
    );
    if sync_latest_synthesis_local_code_projection_if_needed(
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
        &mut delivery_deduped,
    ) {
        log_deterministic_delivery_record(
            &task.task_id,
            "final_delivery_from_synthesis_local_code_strict_json_projection",
            "synced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
    }
    if sync_recorded_local_code_projection_if_needed(
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
        &mut delivery_deduped,
    ) {
        log_deterministic_delivery_record(
            &task.task_id,
            "final_delivery_from_recorded_local_code_strict_json_projection",
            "synced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
    }

    // Renderers may append or replace messages after the initial priority pass.
    // Normalize once more at the delivery boundary so repeated machine payloads
    // cannot leak into API/UI message arrays.
    let (normalized_delivery, _, _) =
        crate::finalize::build_final_delivery_with_priority(&delivery_deduped, None);
    delivery_deduped = normalized_delivery;

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

    let final_status = successful_delivery_final_status(
        &loop_state,
        finalizer_summary.as_ref(),
        &delivery_deduped,
    );
    let mut journal = build_loop_journal(
        state,
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        finalizer_summary.clone(),
        delivery_consistent,
        &final_text,
        final_status,
    )
    .await;
    if let Some(route_result) = agent_run_context.and_then(|ctx| ctx.output_contract()) {
        let defer_to_post_write_readback =
            crate::answer_verifier::post_write_content_evidence_missing_before_verifier(
                &journal,
                &final_text,
            );
        if defer_to_post_write_readback {
            crate::append_act_plan_log(
                state,
                task,
                "answer_verifier_deferred_post_write_readback",
                loop_state.total_steps_executed,
                loop_state.subtask_results.len(),
                loop_state.tool_calls_total,
                "reason=post_write_content_evidence_required",
            );
        }
        if !defer_to_post_write_readback {
            let answer_contract =
                crate::answer_verifier::AnswerContract::new(user_text, route_result.clone());
            if let Some(answer_verifier) = crate::answer_verifier::verify_answer_observe_only(
                state,
                task,
                user_text,
                &answer_contract,
                &journal,
                &final_text,
            )
            .await
            {
                journal.record_answer_verifier_summary(answer_verifier);
            }
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

fn effective_agent_run_context_for_finalization(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
) -> Option<AgentRunContext> {
    if agent_run_context.is_none() && loop_state.output_contract.is_none() {
        return None;
    }
    let mut context = agent_run_context.cloned().unwrap_or_default();
    if let Some(output_contract) = loop_state.output_contract.as_ref() {
        context.output_contract = Some(output_contract.clone());
    }
    Some(context)
}

#[cfg(test)]
#[path = "loop_reply_tests.rs"]
mod tests;

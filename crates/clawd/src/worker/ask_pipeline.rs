use anyhow::Result;
use serde_json::Value;
use tracing::info;

use super::*;

#[path = "ask_pipeline_active_binding.rs"]
mod active_binding;
#[path = "ask_pipeline_agent_context.rs"]
mod agent_context;
#[path = "ask_pipeline_auto_locator_binding.rs"]
mod auto_locator_binding;
#[path = "ask_pipeline_background_locator_guard.rs"]
mod background_locator_guard;
#[path = "ask_pipeline_bare_topic_guard.rs"]
mod bare_topic_guard;
#[path = "ask_pipeline_clarify_context.rs"]
mod clarify_context;
#[path = "ask_pipeline_contract_repair.rs"]
mod contract_repair;
#[path = "ask_pipeline_default_config.rs"]
mod default_config;
#[path = "ask_pipeline_deictic_guard.rs"]
mod deictic_guard;
#[path = "ask_pipeline_execution_context.rs"]
mod execution_context;
#[path = "ask_pipeline_file_delivery.rs"]
mod file_delivery;
#[path = "ask_pipeline_locator_hint_binding.rs"]
mod locator_hint_binding;
#[path = "ask_pipeline_locator_resolution.rs"]
mod locator_resolution;
#[path = "ask_pipeline_locatorless_observation_guard.rs"]
mod locatorless_observation_guard;
#[path = "ask_pipeline_ordered_entry_binding.rs"]
mod ordered_entry_binding;
#[path = "ask_pipeline_post_route_binding.rs"]
mod post_route_binding;
#[path = "ask_pipeline_quantity_pair_binding.rs"]
mod quantity_pair_binding;
#[path = "ask_pipeline_runtime_status.rs"]
mod runtime_status;
#[path = "ask_pipeline_structured_anchor_guard.rs"]
mod structured_anchor_guard;
#[path = "ask_pipeline_unbound_context_guard.rs"]
mod unbound_context_guard;
#[path = "ask_pipeline_workspace_locator_binding.rs"]
mod workspace_locator_binding;
use active_binding::{
    active_observed_facts_have_bound_target,
    prebind_active_bound_target_for_locatorless_content_evidence,
    prebind_active_bound_target_from_matching_locator_hint,
    prebind_active_listing_target_for_locatorless_scalar_count,
    prebind_current_workspace_root_hint_for_scalar_count,
    prebind_session_alias_locator_from_current_request,
    repair_service_status_file_locator_to_content_excerpt, single_component_locator_hint,
    SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST,
};
pub(super) use agent_context::build_agent_run_context_from_prepared_flow;
use background_locator_guard::{
    background_only_locator_route_should_force_clarify,
    downgrade_background_locator_clarify_to_recent_observed_chat, locator_identity_candidates,
    recent_execution_result_segments, text_mentions_locator_identity,
};
use bare_topic_guard::{
    bare_topic_clarify_question_should_drop_context_target,
    bare_topic_memory_expansion_route_should_force_clarify,
    bare_topic_model_supplied_locator_route_should_force_clarify, is_bare_topic_only_prompt,
    route_introduces_unmentioned_distinctive_context_target,
    route_introduces_unmentioned_distinctive_context_target_except_workspace_root,
};
use clarify_context::{
    build_locator_fuzzy_clarify_context, route_clarify_reason_code,
    should_reuse_route_clarify_question, should_suppress_recent_execution_in_clarify_context,
    structured_missing_locator_clarify_context,
};
use contract_repair::{
    repair_compound_file_names_plus_content_summary_contract,
    repair_config_validation_findings_contract,
    repair_generic_path_content_grounded_summary_contract,
    repair_session_alias_listing_plus_content_summary_contract,
    repair_sqlite_path_excerpt_judgment_contract, repair_sqlite_structured_table_listing_contract,
    repair_sqlite_structured_version_contract,
    repair_summary_only_content_excerpt_with_summary_contract,
};
use default_config::{
    prebind_config_contract_default_main_config_locator,
    promote_config_contract_default_main_config_to_execute,
};
use deictic_guard::{
    deictic_bare_locator_should_force_clarify, deictic_missing_locator_reason_code,
    mark_deictic_missing_locator_clarify, route_locator_hint_matches_active_ordered_entry,
    state_patch_allows_deictic_locator_guard_bypass, state_patch_requires_deictic_locator_clarify,
};
pub(super) use execution_context::execution_user_request;
use execution_context::{
    sanitize_untrusted_normalizer_answer_candidate_for_execution,
    sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution,
};
use file_delivery::{
    active_anchor_file_delivery_without_structured_reference_should_force_clarify,
    direct_existing_file_delivery_token,
    directory_file_delivery_without_structured_selection_should_force_clarify,
    generated_file_delivery_uses_runtime_target,
    prebind_direct_file_delivery_locator_before_deictic_guard,
    prebind_file_delivery_locator_from_recent_ordered_resolved_prompt,
    prebind_file_delivery_locator_from_resolved_prompt_path,
    prebind_file_delivery_missing_locator_from_resolved_prompt_path,
    promote_unresolved_file_delivery_with_current_request_locator,
    unbound_existing_file_delivery_route_should_force_clarify,
};
use locator_hint_binding::{
    locator_component_token, locator_hint_token_ambiguous_in_workspace,
    locator_hint_token_present_in_prompt, prebind_workspace_root_locator_from_resolved_prompt,
    resolve_direct_child_stem_workspace_locator_hint, resolve_existing_workspace_locator_hint,
    resolved_prompt_existing_workspace_locator, text_contains_workspace_root_locator,
};
use locator_resolution::{
    current_workspace_locator_resolution, effective_auto_locator_kind,
    locator_hint_names_workspace_root, locator_hint_points_to_workspace_root,
    normalize_locator_identity_token, normalize_workspace_locator_path,
    should_attempt_auto_locator,
};
use locatorless_observation_guard::{
    command_observation_route_has_runtime_evidence,
    current_request_has_self_contained_structured_payload,
    locatorless_observation_route_should_force_clarify,
    promote_locatorless_git_capability_to_repository_state,
    promote_locatorless_scalar_child_metadata_to_quantity_comparison,
    raw_command_output_has_explicit_command, raw_command_request_has_structural_input_locator,
    semantic_kind_can_execute_without_locator,
};
use ordered_entry_binding::{
    prebind_content_evidence_locator_from_active_ordered_resolved_prompt,
    promote_clarify_observation_to_execute_with_locator,
    resolve_recent_ordered_entry_target_from_resolved_prompt,
};
use post_route_binding::{
    auto_locator_scalar_file_without_current_locator_should_force_clarify,
    direct_auto_locator_path, post_route_promote_resolved_multifile_targets_to_execute,
};
use quantity_pair_binding::{
    prebind_quantity_compare_directory_pair_from_current_request,
    route_has_single_existing_directory_locator_hint, structural_locator_token_candidates,
    workspace_directory_pair_from_current_request,
};
use runtime_status::{
    prebind_runtime_status_scalar_path_to_current_workspace,
    promote_locatorless_scalar_status_query_to_runtime_info,
    promote_locatorless_status_query_to_service_status, turn_analysis_has_runtime_status_query,
};
use structured_anchor_guard::{
    active_session_has_structured_observation_anchor, answer_candidate_is_compact_scalar_shape,
    direct_answer_from_structured_anchor_requires_evidence, embedded_normalizer_answer_candidate,
    followup_frame_has_matching_target,
    normalizer_answer_candidate_is_grounded_in_structured_observation,
    normalizer_answer_candidate_matches_recent_execution_context,
    observed_facts_have_matching_target,
    preserve_scalar_shape_from_normalizer_candidate_for_clarify,
    promote_structured_anchor_direct_answer_to_evidence, session_has_authoritative_deictic_anchor,
};
use unbound_context_guard::{
    deictic_memory_only_route_should_force_clarify,
    execute_route_without_input_locator_should_plan,
    promote_broad_current_workspace_content_summary_to_directory_purpose,
    promote_runtime_surface_contract_to_command_summary,
    raw_command_output_without_locator_can_plan_via_contract,
    repair_directory_purpose_command_summary_contract,
    repair_directory_purpose_quantity_comparison_contract,
    restore_explicit_extension_assess_gap_to_command_summary,
    runtime_status_query_route_can_plan_without_locator,
    task_control_route_can_plan_without_locator,
    unbound_model_context_target_route_should_force_clarify,
    unbound_targeted_evidence_route_should_force_clarify,
};
use workspace_locator_binding::{
    current_request_has_concrete_locator_surface,
    current_request_has_structural_locator_surface_for_route,
    current_request_resolves_workspace_child_locator,
    implicit_workspace_file_locator_route_should_force_clarify,
    inferred_missing_workspace_locator_hint_should_force_clarify,
    locator_hint_full_file_name_token_present_in_prompt,
    model_completed_workspace_file_locator_hint_should_force_clarify,
    path_scoped_locator_guard_can_defer_to_prompt_targets,
    prebind_clarify_workspace_child_locator_from_current_request,
    prebind_existing_workspace_locator_hint_from_current_request,
    prebind_workspace_child_locator_from_current_request,
    prebind_workspace_child_locator_from_resolved_prompt,
    prebind_workspace_root_locator_from_current_request,
    promote_clarify_path_scoped_filename_targets_to_execute,
    promote_clarify_resolved_multifile_targets_to_execute,
    recent_artifacts_judgment_can_use_recent_execution_context,
    structured_field_route_has_current_locator_surface,
    WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST,
};

pub(super) struct PreparedAskFlow {
    pub(super) context_bundle_summary: String,
    pub(super) memory_trace: Option<Value>,
    pub(super) route_result: crate::RouteResult,
    pub(super) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(super) execution_recipe_plan_hint: Option<crate::intent_router::ExecutionRecipePlanHint>,
    pub(super) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
    pub(super) clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    pub(super) auto_locator_path: Option<String>,
    pub(super) has_authoritative_deictic_anchor: bool,
    pub(super) chat_prompt_context: String,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) memory_context_for_execution: String,
    pub(super) semantic_answer_candidate_draft: Option<String>,
    pub(super) recent_execution_context: String,
    pub(super) session_alias_bindings: Vec<crate::conversation_state::SessionAliasBinding>,
    pub(super) agent_mode: bool,
    /// Final runtime ask mode after routing and post-route refinements.
    /// Dispatch decisions must use ask_mode predicates.
    pub(super) ask_mode: crate::AskMode,
    pub(super) clarify_reason: String,
    pub(super) clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    pub(super) fuzzy_locator_suggestions: Vec<String>,
    pub(super) should_route_schedule_direct: bool,
}

struct AppliedAskPostRoute {
    execution_route_result: crate::RouteResult,
    auto_locator_path: Option<String>,
    #[cfg(test)]
    gate_record: crate::post_route_policy::PostRouteGateRecord,
    has_authoritative_deictic_anchor: bool,
    resolved_prompt_for_execution: String,
    prompt_with_memory_for_execution: String,
    session_alias_bindings: Vec<crate::conversation_state::SessionAliasBinding>,
    clarify_reason: String,
    clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    fuzzy_locator_suggestions: Vec<String>,
}

fn clarify_fallback_source_or_default(
    source: Option<crate::fallback::ClarifyFallbackSource>,
) -> crate::fallback::ClarifyFallbackSource {
    crate::finalize::clarify_fallback_source_or_default(source)
}

fn log_route_guard_record(
    task: &crate::ClaimedTask,
    owner_layer: &'static str,
    reason_code: &'static str,
    outcome: &'static str,
    before_gate_kind: crate::RouteGateKind,
    route_result: &crate::RouteResult,
) {
    info!(
        "route_guard_record task_id={} owner_layer={} reason_code={} outcome={} before_gate_kind={} after_gate_kind={} needs_clarify={} locator_kind={} semantic_kind={} response_shape={} delivery_required={} content_evidence={}",
        task.task_id,
        owner_layer,
        reason_code,
        outcome,
        before_gate_kind.as_str(),
        route_result.gate_kind().as_str(),
        route_result.needs_clarify,
        route_result.output_contract.locator_kind.as_str(),
        route_result.output_contract.semantic_kind.as_str(),
        route_result.output_contract.response_shape.as_str(),
        route_result.output_contract.delivery_required,
        route_result.output_contract.requires_content_evidence,
    );
}

fn ask_reply_with_visible_process(
    _state: &crate::AppState,
    _task: &crate::ClaimedTask,
    _prompt: &str,
    text: String,
) -> crate::AskReply {
    let answer = text.trim().to_string();
    if answer.is_empty() || crate::finalize::is_execution_summary_message(&answer) {
        crate::AskReply::non_llm(text)
    } else {
        crate::AskReply::non_llm(answer)
    }
}

fn with_agent_decides_shadow_snapshot(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    mut reply: crate::AskReply,
    route_result: &crate::RouteResult,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> crate::AskReply {
    let Some(attribution) = crate::agent_engine::agent_decides_shadow_snapshot_for_route(
        state,
        task,
        agent_run_context,
        route_result,
    ) else {
        return reply;
    };
    let journal = reply.task_journal.get_or_insert_with(|| {
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", prompt)
    });
    journal.record_route_result(route_result);
    journal.record_rollout_attribution(attribution);
    reply
}

fn with_dispatch_boundary_attribution(
    task: &crate::ClaimedTask,
    prompt: &str,
    mut reply: crate::AskReply,
    route_result: &crate::RouteResult,
    event: &str,
    old_owner: &str,
    new_owner: &str,
    chosen_path: &str,
    rollback_token: &str,
) -> crate::AskReply {
    let journal = reply.task_journal.get_or_insert_with(|| {
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", prompt)
    });
    journal.record_route_result(route_result);
    journal.record_rollout_attribution(
        crate::task_journal::TaskJournalRolloutAttribution::dispatch_boundary_attribution(
            route_result,
            event,
            old_owner,
            new_owner,
            chosen_path,
            rollback_token,
        ),
    );
    reply
}

fn resume_discussion_uses_direct_chat_renderer(route_result: &crate::RouteResult) -> bool {
    crate::ask_flow::route_allows_agent_loop_pure_chat_submode(route_result)
}

fn ordinary_clarify_should_enter_agent_loop(
    _state: &crate::AppState,
    clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
) -> bool {
    !clarify_reason_kind.is_boundary_clarify()
}

fn subagent_boundary_clarify_should_enter_agent_loop(
    state: &crate::AppState,
    route_result: &crate::RouteResult,
) -> bool {
    if !route_result.needs_clarify
        || !route_result.ask_mode.is_clarify_only()
        || route_result.risk_ceiling == crate::RiskCeiling::High
        || route_result.schedule_kind != crate::ScheduleKind::None
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Strict
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    let hint = route_result
        .agent_display_name_hint
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.trim().is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let review_role = hint
        .iter()
        .any(|part| matches!(part.as_str(), "review" | "reviewer"));
    if !review_role {
        return false;
    }
    current_top_level_plan_markdown_path(state).is_some()
}

fn current_top_level_plan_markdown_path(state: &crate::AppState) -> Option<String> {
    let plan_dir = state.skill_rt.workspace_root.join("plan");
    let mut files = std::fs::read_dir(&plan_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file()
                || path.extension().and_then(|value| value.to_str()) != Some("md")
            {
                return None;
            }
            let modified = metadata.modified().ok();
            let name = path.file_name()?.to_str()?.to_string();
            Some((modified, name))
        })
        .collect::<Vec<_>>();
    files.sort_by(|(left_time, left_name), (right_time, right_name)| {
        right_time
            .cmp(left_time)
            .then_with(|| left_name.cmp(right_name))
    });
    files
        .into_iter()
        .next()
        .map(|(_, name)| format!("plan/{name}"))
}

fn apply_ask_post_route(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    resolved_prompt: &str,
    recent_execution_context: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    mut route_result: crate::RouteResult,
    mut resolved_prompt_for_execution: String,
    mut prompt_with_memory_for_execution: String,
) -> AppliedAskPostRoute {
    let session_snapshot = crate::conversation_state::load_active_session_snapshot(state, task);
    let has_authoritative_deictic_anchor =
        session_has_authoritative_deictic_anchor(prompt, &route_result, &session_snapshot);
    repair_compound_file_names_plus_content_summary_contract(&mut route_result);
    repair_session_alias_listing_plus_content_summary_contract(
        state,
        prompt,
        &session_snapshot,
        &mut route_result,
    );
    promote_locatorless_scalar_status_query_to_runtime_info(&mut route_result, turn_analysis);
    promote_locatorless_status_query_to_service_status(
        state,
        prompt,
        &mut route_result,
        turn_analysis,
    );
    if deictic_memory_only_route_should_force_clarify(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        mark_deictic_missing_locator_clarify(&mut route_result);
        append_route_reason(&mut route_result, "deictic_memory_only_requires_clarify");
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "deictic_memory_only_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if unbound_model_context_target_route_should_force_clarify(
        state,
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        route_result.clarify_question.clear();
        append_route_reason(
            &mut route_result,
            "unbound_model_context_target_requires_clarify",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "unbound_model_context_target_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if bare_topic_model_supplied_locator_route_should_force_clarify(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        route_result.clarify_question.clear();
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "bare_topic_model_supplied_locator_requires_clarify",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "bare_topic_model_supplied_locator_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if implicit_workspace_file_locator_route_should_force_clarify(
        state,
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        mark_deictic_missing_locator_clarify(&mut route_result);
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "implicit_workspace_file_locator_requires_clarify",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "implicit_workspace_file_locator_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    prebind_config_contract_default_main_config_locator(state, prompt, &mut route_result);
    prebind_workspace_root_locator_from_current_request(state, prompt, &mut route_result);
    prebind_workspace_child_locator_from_current_request(state, prompt, &mut route_result);
    prebind_clarify_workspace_child_locator_from_current_request(state, prompt, &mut route_result);
    prebind_workspace_child_locator_from_resolved_prompt(state, resolved_prompt, &mut route_result);
    repair_summary_only_content_excerpt_with_summary_contract(&mut route_result);
    repair_generic_path_content_grounded_summary_contract(&mut route_result);
    repair_sqlite_path_excerpt_judgment_contract(state, prompt, resolved_prompt, &mut route_result);
    repair_sqlite_structured_version_contract(state, prompt, resolved_prompt, &mut route_result);
    repair_sqlite_structured_table_listing_contract(
        state,
        prompt,
        resolved_prompt,
        &mut route_result,
    );
    repair_config_validation_findings_contract(state, prompt, resolved_prompt, &mut route_result);
    prebind_file_delivery_locator_from_recent_ordered_resolved_prompt(
        state,
        resolved_prompt,
        recent_execution_context,
        &mut route_result,
    );
    prebind_content_evidence_locator_from_active_ordered_resolved_prompt(
        state,
        resolved_prompt,
        &mut route_result,
        &session_snapshot,
    );
    prebind_file_delivery_locator_from_resolved_prompt_path(
        state,
        resolved_prompt,
        &mut route_result,
    );
    prebind_file_delivery_missing_locator_from_resolved_prompt_path(
        state,
        resolved_prompt,
        &mut route_result,
    );
    prebind_workspace_root_locator_from_resolved_prompt(state, resolved_prompt, &mut route_result);
    prebind_quantity_compare_directory_pair_from_current_request(
        state,
        resolved_prompt,
        &mut route_result,
    );
    prebind_existing_workspace_locator_hint_from_current_request(state, prompt, &mut route_result);
    prebind_session_alias_locator_from_current_request(
        prompt,
        &mut route_result,
        &session_snapshot,
    );
    promote_clarify_resolved_multifile_targets_to_execute(state, prompt, &mut route_result);
    promote_clarify_path_scoped_filename_targets_to_execute(prompt, &mut route_result);
    prebind_active_bound_target_from_matching_locator_hint(&mut route_result, &session_snapshot);
    prebind_active_bound_target_for_locatorless_content_evidence(
        &mut route_result,
        &session_snapshot,
    );
    repair_service_status_file_locator_to_content_excerpt(state, &mut route_result);
    prebind_active_listing_target_for_locatorless_scalar_count(
        &mut route_result,
        &session_snapshot,
    );
    prebind_current_workspace_root_hint_for_scalar_count(state, prompt, &mut route_result);
    if model_completed_workspace_file_locator_hint_should_force_clarify(
        state,
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        mark_deictic_missing_locator_clarify(&mut route_result);
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "model_completed_workspace_file_locator_requires_clarify",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "model_completed_workspace_file_locator_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if inferred_missing_workspace_locator_hint_should_force_clarify(
        state,
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        mark_deictic_missing_locator_clarify(&mut route_result);
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "inferred_missing_workspace_locator_requires_clarify",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "inferred_missing_workspace_locator_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if active_anchor_file_delivery_without_structured_reference_should_force_clarify(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        mark_deictic_missing_locator_clarify(&mut route_result);
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "active_anchor_file_delivery_requires_structured_reference",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "active_anchor_file_delivery_requires_structured_reference",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if bare_topic_model_supplied_locator_route_should_force_clarify(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        route_result.clarify_question.clear();
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "bare_topic_model_supplied_locator_requires_clarify",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "bare_topic_model_supplied_locator_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    prebind_quantity_compare_directory_pair_from_current_request(state, prompt, &mut route_result);
    if background_only_locator_route_should_force_clarify(
        state,
        prompt,
        resolved_prompt,
        recent_execution_context,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        mark_deictic_missing_locator_clarify(&mut route_result);
        append_route_reason(&mut route_result, "background_locator_requires_clarify");
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "background_locator_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    downgrade_background_locator_clarify_to_recent_observed_chat(
        &mut route_result,
        recent_execution_context,
    );
    promote_locatorless_scalar_status_query_to_runtime_info(&mut route_result, turn_analysis);
    promote_locatorless_status_query_to_service_status(
        state,
        prompt,
        &mut route_result,
        turn_analysis,
    );
    prebind_runtime_status_scalar_path_to_current_workspace(
        &mut route_result,
        turn_analysis,
        &session_snapshot,
    );
    promote_broad_current_workspace_content_summary_to_directory_purpose(prompt, &mut route_result);
    repair_directory_purpose_command_summary_contract(state, prompt, &mut route_result);
    repair_directory_purpose_quantity_comparison_contract(state, prompt, &mut route_result);
    restore_explicit_extension_assess_gap_to_command_summary(&mut route_result);
    promote_runtime_surface_contract_to_command_summary(prompt, &mut route_result);
    promote_locatorless_git_capability_to_repository_state(&mut route_result);
    promote_locatorless_scalar_child_metadata_to_quantity_comparison(
        state,
        prompt,
        &mut route_result,
    );
    if locatorless_observation_route_should_force_clarify(
        state,
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        route_result.clarify_question.clear();
        append_route_reason(
            &mut route_result,
            "locatorless_observation_requires_clarify",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "locatorless_observation_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if unbound_targeted_evidence_route_should_force_clarify(
        prompt,
        &route_result,
        &session_snapshot,
        recent_execution_context,
    ) {
        let before_gate_kind = route_result.gate_kind();
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        route_result.clarify_question.clear();
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "unbound_targeted_evidence_requires_clarify",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "unbound_targeted_evidence_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if bare_topic_memory_expansion_route_should_force_clarify(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        route_result.clarify_question.clear();
        append_route_reason(
            &mut route_result,
            "bare_topic_context_expansion_requires_clarify",
        );
    }
    if bare_topic_clarify_question_should_drop_context_target(prompt, &route_result) {
        route_result.clarify_question.clear();
        append_route_reason(&mut route_result, "bare_topic_contextual_clarify_sanitized");
    }
    if unbound_existing_file_delivery_route_should_force_clarify(
        state,
        prompt,
        &route_result,
        has_authoritative_deictic_anchor,
    ) {
        let before_gate_kind = route_result.gate_kind();
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        route_result.clarify_question.clear();
        route_result.wants_file_delivery = true;
        route_result.output_contract.delivery_required = true;
        route_result.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "unbound_existing_file_delivery_requires_clarify",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "unbound_existing_file_delivery_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    prebind_direct_file_delivery_locator_before_deictic_guard(
        state,
        recent_execution_context,
        &mut route_result,
    );
    if directory_file_delivery_without_structured_selection_should_force_clarify(
        state,
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        mark_deictic_missing_locator_clarify(&mut route_result);
        route_result.wants_file_delivery = true;
        route_result.output_contract.delivery_required = true;
        route_result.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "directory_file_delivery_requires_structured_selection",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "directory_file_delivery_requires_structured_selection",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if deictic_bare_locator_should_force_clarify(&route_result, turn_analysis, &session_snapshot) {
        let before_gate_kind = route_result.gate_kind();
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        if route_result.clarify_question.trim().is_empty() {
            mark_deictic_missing_locator_clarify(&mut route_result);
        }
        append_route_reason(&mut route_result, "deictic_bare_locator_requires_clarify");
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "deictic_bare_locator_requires_clarify",
            "blocked",
            before_gate_kind,
            &route_result,
        );
    }
    if direct_answer_from_structured_anchor_requires_evidence(
        prompt,
        &route_result,
        &session_snapshot,
        recent_execution_context,
        has_authoritative_deictic_anchor,
        turn_analysis,
    ) {
        let before_gate_kind = route_result.gate_kind();
        promote_structured_anchor_direct_answer_to_evidence(&mut route_result);
        log_route_guard_record(
            task,
            "worker_active_task_guard",
            "structured_anchor_direct_answer_requires_evidence",
            "promoted",
            before_gate_kind,
            &route_result,
        );
    }
    let multiple_explicit_local_paths = super::has_multiple_distinct_explicit_local_path_locators(
        state,
        &format!("{prompt}\n{resolved_prompt}"),
        Some(recent_execution_context),
    );
    if multiple_explicit_local_paths {
        append_route_reason(
            &mut route_result,
            "auto_locator_suppressed_multiple_explicit_paths",
        );
    }
    let locator_resolution =
        if should_attempt_auto_locator(&route_result) && !multiple_explicit_local_paths {
            current_workspace_locator_resolution(&state.skill_rt.workspace_root, &route_result)
                .unwrap_or_else(|| {
                    let locator_hint = route_result.output_contract.locator_hint.trim();
                    if locator_hint.is_empty() {
                        return crate::post_route_policy::LocatorResolution::None;
                    }
                    let locator_kind = effective_auto_locator_kind(&route_result);
                    match super::try_resolve_implicit_locator_path(
                        state,
                        locator_hint,
                        locator_hint,
                        locator_kind,
                        Some(recent_execution_context),
                    )
                    .map(|resolution| match resolution {
                        super::LocatorAutoResolution::Direct(path) => {
                            crate::post_route_policy::LocatorResolution::Direct(path)
                        }
                        super::LocatorAutoResolution::Fuzzy(candidates) => {
                            crate::post_route_policy::LocatorResolution::Fuzzy(candidates)
                        }
                    }) {
                        Some(resolution) => resolution,
                        None => crate::post_route_policy::LocatorResolution::None,
                    }
                })
        } else {
            crate::post_route_policy::LocatorResolution::None
        };
    let mut post_route =
        crate::post_route_policy::apply_post_route_policy(route_result.clone(), locator_resolution);
    if promote_unresolved_file_delivery_with_current_request_locator(prompt, &mut post_route) {
        info!(
            "{} worker_once: ask file_delivery_current_request_locator_to_planner task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
    }
    if auto_locator_scalar_file_without_current_locator_should_force_clarify(
        state,
        prompt,
        &post_route.execution_route_result,
        post_route.auto_locator_path.as_deref(),
    ) {
        post_route.execution_route_result.needs_clarify = true;
        post_route.execution_route_result.set_clarify_gate();
        mark_deictic_missing_locator_clarify(&mut post_route.execution_route_result);
        post_route
            .execution_route_result
            .output_contract
            .locator_kind = crate::OutputLocatorKind::None;
        post_route
            .execution_route_result
            .output_contract
            .locator_hint
            .clear();
        post_route.auto_locator_path = None;
        post_route.auto_locator_hint = None;
        post_route.auto_locator_resolved_direct = false;
        post_route.missing_locator_for_path_scoped_content = true;
        post_route.clarify_reason =
            deictic_missing_locator_reason_code(&post_route.execution_route_result).to_string();
        post_route.clarify_reason_kind =
            crate::post_route_policy::ClarifyReasonKind::MissingPathScopedLocator;
        post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_auto_locator_scalar_file_requires_current_locator",
            crate::post_route_policy::PostRoutePolicyOutcome::Clarify,
        );
        append_route_reason(
            &mut post_route.execution_route_result,
            "auto_locator_scalar_file_requires_current_locator",
        );
    }
    if post_route.missing_locator_for_path_scoped_content
        && !route_reason_has_marker(
            &post_route.execution_route_result,
            "directory_file_delivery_requires_structured_selection",
        )
        && path_scoped_locator_guard_can_defer_to_prompt_targets(
            prompt,
            &post_route.execution_route_result,
        )
    {
        post_route.missing_locator_for_path_scoped_content = false;
        post_route.execution_route_result.needs_clarify = false;
        post_route.execution_route_result.clarify_question.clear();
        let finalize = crate::post_route_policy::content_evidence_execution_finalize_style(
            &post_route.execution_route_result.output_contract,
            false,
        )
        .unwrap_or(crate::ActFinalizeStyle::ChatWrapped);
        post_route
            .execution_route_result
            .set_planner_execute_finalize(finalize);
        post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_locator_guard_deferred_to_prompt_targets",
            crate::post_route_policy::PostRoutePolicyOutcome::Execute,
        );
        append_route_reason(
            &mut post_route.execution_route_result,
            "locator_guard_deferred_to_prompt_targets",
        );
    }
    if post_route_promote_resolved_multifile_targets_to_execute(
        state,
        prompt,
        resolved_prompt,
        &mut post_route,
    ) {
        info!(
            "{} worker_once: ask resolved_multifile_targets_to_planner task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
    }
    if promote_config_contract_default_main_config_to_execute(state, prompt, &mut post_route) {
        info!(
            "{} worker_once: ask config_contract_default_main_config_to_planner task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
    }
    if super::ask_prepare::repair_structural_file_delivery_resolution_for_turn(
        &mut post_route.execution_route_result,
        &session_snapshot,
        turn_analysis,
    ) && !post_route.execution_route_result.needs_clarify
    {
        let target = post_route
            .execution_route_result
            .output_contract
            .locator_hint
            .trim()
            .to_string();
        if !target.is_empty() {
            post_route.auto_locator_path = Some(target);
            post_route.auto_locator_resolved_direct = true;
            post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
                "post_route_structural_file_delivery_bound_target",
                crate::post_route_policy::PostRoutePolicyOutcome::Execute,
            );
        }
    }
    auto_locator_binding::bind_structured_field_read_to_auto_locator(&mut post_route);
    if route_reason_has_marker(
        &post_route.execution_route_result,
        "directory_file_delivery_requires_structured_selection",
    ) {
        post_route.execution_route_result.needs_clarify = true;
        post_route.execution_route_result.set_clarify_gate();
        post_route
            .execution_route_result
            .output_contract
            .locator_kind = crate::OutputLocatorKind::None;
        post_route
            .execution_route_result
            .output_contract
            .locator_hint
            .clear();
        post_route.auto_locator_path = None;
        post_route.auto_locator_hint = None;
        post_route.auto_locator_resolved_direct = false;
        post_route.missing_locator_for_path_scoped_content = true;
        post_route.clarify_reason =
            deictic_missing_locator_reason_code(&post_route.execution_route_result).to_string();
        post_route.clarify_reason_kind =
            crate::post_route_policy::ClarifyReasonKind::MissingPathScopedLocator;
        post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_directory_file_delivery_requires_structured_selection",
            crate::post_route_policy::PostRoutePolicyOutcome::Clarify,
        );
    }
    if let Some(hint) = post_route.auto_locator_hint.as_deref() {
        resolved_prompt_for_execution.push_str(hint);
        prompt_with_memory_for_execution.push_str(hint);
    }
    if let Some(path) = post_route.auto_locator_path.as_deref() {
        info!(
            "{} worker_once: ask auto_locator_resolved task_id={} path={} raw_text={} resolved_text={}",
            crate::highlight_tag("routing"),
            task.task_id,
            path,
            crate::truncate_for_log(prompt),
            crate::truncate_for_log(resolved_prompt)
        );
    }
    if !post_route.fuzzy_locator_suggestions.is_empty() {
        info!(
            "{} worker_once: ask auto_locator_fuzzy_candidates task_id={} candidates={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&post_route.fuzzy_locator_suggestions.join(" | "))
        );
    }
    if post_route.missing_locator_for_path_scoped_content {
        info!(
            "{} worker_once: ask force_clarify_by_locator_guard task_id={} reason=locator_required_for_path_scoped_content raw_text={} resolved_text={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(prompt),
            crate::truncate_for_log(resolved_prompt)
        );
    }
    info!(
        "{} worker_once: ask post_route_policy_decision task_id={} owner_layer={} reason_code={} outcome={}",
        crate::highlight_tag("routing"),
        task.task_id,
        post_route.gate_record.owner_layer,
        post_route.gate_record.reason_code,
        post_route.gate_record.outcome.as_str()
    );
    if post_route.execution_route_result.gate_kind() != route_result.gate_kind() {
        info!(
            "{} worker_once: ask route_gate_override_by_auto_locator task_id={} gate={:?}->{:?}",
            crate::highlight_tag("routing"),
            task.task_id,
            route_result.gate_kind(),
            post_route.execution_route_result.gate_kind()
        );
    } else if post_route.execution_route_result.ask_mode != route_result.ask_mode {
        info!(
            "{} worker_once: ask ask_mode_refined_by_auto_locator task_id={} ask_mode={} -> {} legacy_route_label={} -> {}",
            crate::highlight_tag("routing"),
            task.task_id,
            route_result.ask_mode.as_str(),
            post_route.execution_route_result.ask_mode.as_str(),
            route_result.legacy_route_label_for_trace(),
            post_route
                .execution_route_result
                .legacy_route_label_for_trace()
        );
    }
    sanitize_untrusted_normalizer_answer_candidate_for_execution(
        &mut post_route.execution_route_result,
        prompt,
        recent_execution_context,
        &session_snapshot,
        &mut resolved_prompt_for_execution,
        &mut prompt_with_memory_for_execution,
    );
    sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution(
        &mut post_route.execution_route_result,
        prompt,
        turn_analysis,
        &mut resolved_prompt_for_execution,
        &mut prompt_with_memory_for_execution,
    );
    if subagent_boundary_clarify_should_enter_agent_loop(state, &post_route.execution_route_result)
    {
        let before_gate_kind = post_route.execution_route_result.gate_kind();
        post_route.execution_route_result.needs_clarify = false;
        post_route.execution_route_result.clarify_question.clear();
        post_route
            .execution_route_result
            .set_planner_execute_finalize(crate::ActFinalizeStyle::ChatWrapped);
        post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_subagent_boundary_clarify_deferred_to_agent_loop",
            crate::post_route_policy::PostRoutePolicyOutcome::Execute,
        );
        append_route_reason(
            &mut post_route.execution_route_result,
            "subagent_boundary_clarify_deferred_to_agent_loop",
        );
        log_route_guard_record(
            task,
            "worker_agent_loop_boundary",
            "subagent_boundary_clarify_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            &post_route.execution_route_result,
        );
    }
    AppliedAskPostRoute {
        execution_route_result: post_route.execution_route_result,
        auto_locator_path: post_route.auto_locator_path,
        #[cfg(test)]
        gate_record: post_route.gate_record,
        has_authoritative_deictic_anchor,
        resolved_prompt_for_execution,
        prompt_with_memory_for_execution,
        session_alias_bindings: session_snapshot
            .conversation_state
            .as_ref()
            .map(|conversation_state| conversation_state.alias_bindings.clone())
            .unwrap_or_default(),
        clarify_reason: post_route.clarify_reason,
        clarify_reason_kind: post_route.clarify_reason_kind,
        fuzzy_locator_suggestions: post_route.fuzzy_locator_suggestions,
    }
}

fn route_reason_has_marker_prefix(route_result: &crate::RouteResult, marker_prefix: &str) -> bool {
    route_result
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| part == marker_prefix || part.starts_with(&format!("{marker_prefix}:")))
}

fn route_reason_has_marker(route_result: &crate::RouteResult, marker: &str) -> bool {
    if marker.trim().is_empty() {
        return false;
    }
    route_result
        .route_reason
        .split(';')
        .any(|part| route_reason_part_has_marker(part.trim(), marker))
}

fn route_reason_part_has_marker(part: &str, marker: &str) -> bool {
    part.match_indices(marker).any(|(start, _)| {
        let before = part[..start].chars().next_back();
        let after = part[start + marker.len()..].chars().next();
        !is_route_reason_marker_char(before) && !is_route_reason_marker_char(after)
    })
}

fn is_route_reason_marker_char(ch: Option<char>) -> bool {
    ch.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn append_route_reason(route_result: &mut crate::RouteResult, reason: &'static str) {
    if route_result.route_reason.trim().is_empty() {
        route_result.route_reason = reason.to_string();
    } else if !route_result.route_reason.contains(reason) {
        route_result.route_reason.push_str("; ");
        route_result.route_reason.push_str(reason);
    }
}

pub(super) async fn prepare_ask_flow(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    source: &str,
) -> Result<PreparedAskFlow> {
    let prepared_routing = super::prepare_ask_routing(state, task, payload, prompt, source).await?;
    let semantic_answer_candidate_draft =
        embedded_normalizer_answer_candidate(&prepared_routing.route_result.resolved_intent)
            .map(ToOwned::to_owned);
    let prepared_execution = super::prepare_ask_execution_context(
        state,
        task,
        payload,
        &prepared_routing.route_result,
        &prepared_routing.resolved_prompt,
        prepared_routing.turn_analysis.as_ref(),
    )
    .await?;
    let applied_post_route = apply_ask_post_route(
        state,
        task,
        prompt,
        &prepared_routing.resolved_prompt,
        &prepared_execution.recent_execution_context,
        prepared_routing.turn_analysis.as_ref(),
        prepared_routing.route_result,
        prepared_execution.resolved_prompt_for_execution,
        prepared_execution.prompt_with_memory_for_execution,
    );
    let has_schedule_intent =
        applied_post_route.execution_route_result.schedule_kind != crate::ScheduleKind::None;
    let final_ask_mode = applied_post_route.execution_route_result.ask_mode.clone();
    let should_route_schedule_direct = has_schedule_intent
        && !final_ask_mode.resume_execution()
        && !final_ask_mode.is_resume_discussion();
    Ok(PreparedAskFlow {
        context_bundle_summary: prepared_execution.context_bundle.summary(),
        memory_trace: prepared_execution.context_bundle.memory_trace(),
        route_result: applied_post_route.execution_route_result,
        execution_recipe_hint: prepared_routing.execution_recipe_hint,
        execution_recipe_plan_hint: prepared_routing.execution_recipe_plan_hint,
        turn_analysis: prepared_routing.turn_analysis,
        clarify_fallback_source: prepared_routing.clarify_fallback_source,
        auto_locator_path: applied_post_route.auto_locator_path,
        has_authoritative_deictic_anchor: applied_post_route.has_authoritative_deictic_anchor,
        chat_prompt_context: prepared_execution.chat_prompt_context,
        resolved_prompt_for_execution: applied_post_route.resolved_prompt_for_execution,
        prompt_with_memory_for_execution: applied_post_route.prompt_with_memory_for_execution,
        memory_context_for_execution: prepared_execution.memory_context_for_execution,
        semantic_answer_candidate_draft,
        recent_execution_context: prepared_execution.recent_execution_context,
        session_alias_bindings: applied_post_route.session_alias_bindings,
        agent_mode: prepared_routing.agent_mode,
        ask_mode: final_ask_mode,
        clarify_reason: applied_post_route.clarify_reason,
        clarify_reason_kind: applied_post_route.clarify_reason_kind,
        fuzzy_locator_suggestions: applied_post_route.fuzzy_locator_suggestions,
        should_route_schedule_direct,
    })
}

pub(super) async fn execute_ask_dispatch(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    recent_execution_context: &str,
    resolved_prompt_for_execution: &str,
    prompt_with_memory_for_execution: &str,
    chat_prompt_context: &str,
    route_result: &crate::RouteResult,
    agent_mode: bool,
    clarify_reason: &str,
    clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    fuzzy_locator_suggestions: &[String],
    ask_mode: &crate::AskMode,
    should_route_schedule_direct: bool,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<Option<Result<crate::AskReply, String>>> {
    let execution_user_request = execution_user_request(prompt, resolved_prompt_for_execution);
    if let Some(selected_class) =
        crate::agent_engine::agent_loop_authority_selected_migration_class(state, route_result)
    {
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::Executing,
            "agent_loop_authority_selected_migration_class",
            None,
        );
        tracing::info!(
            "agent_loop_authority_selected_migration_class task_id={} selected_migration_class={} previous_gate_kind={}",
            task.task_id,
            selected_class,
            route_result.gate_kind().as_str(),
        );
        return Ok(Some(
            crate::agent_engine::run_agent_with_tools(
                state,
                task,
                prompt_with_memory_for_execution,
                execution_user_request,
                agent_run_context.clone(),
            )
            .await,
        ));
    }
    if let Some(candidate) = crate::ask_flow::active_ordered_entries_count_direct_answer_candidate(
        prompt,
        agent_run_context.as_ref(),
    ) {
        return Ok(Some(Ok(with_agent_decides_shadow_snapshot(
            state,
            task,
            prompt,
            ask_reply_with_visible_process(state, task, prompt, candidate),
            route_result,
            agent_run_context.as_ref(),
        ))));
    }
    if let Some(delivery_token) = direct_existing_file_delivery_token(route_result) {
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::Executing,
            "direct_existing_file_delivery",
            None,
        );
        let path = delivery_token
            .strip_prefix("FILE:")
            .unwrap_or(delivery_token.as_str())
            .to_string();
        let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", prompt);
        journal.record_route_result(route_result);
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "direct_file_delivery".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    serde_json::json!({
                        "action": "direct_file_delivery",
                        "execution_surface": "worker/ask_pipeline::direct_existing_file_delivery",
                        "execution_surface_owner": "delivery_boundary",
                        "path": path.clone(),
                        "resolved_path": path,
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });
        return Ok(Some(Ok(with_agent_decides_shadow_snapshot(
            state,
            task,
            prompt,
            crate::AskReply::non_llm(delivery_token).with_task_journal(journal),
            route_result,
            agent_run_context.as_ref(),
        ))));
    }
    if route_result.ask_mode.is_clarify_only() {
        if ordinary_clarify_should_enter_agent_loop(state, clarify_reason_kind) {
            crate::log_ask_transition(
                state,
                &task.task_id,
                Some(crate::AskState::Routing),
                crate::AskState::Executing,
                "ordinary_clarify_deferred_to_agent_loop",
                None,
            );
            let mut loop_ctx = agent_run_context.clone();
            if let Some(route) = loop_ctx.as_mut().and_then(|ctx| ctx.route_result.as_mut()) {
                route.set_planner_execute_finalize(crate::ActFinalizeStyle::ChatWrapped);
                append_route_reason(route, "ordinary_clarify_deferred_to_agent_loop");
            }
            return Ok(Some(
                crate::agent_engine::run_agent_with_tools(
                    state,
                    task,
                    prompt_with_memory_for_execution,
                    execution_user_request,
                    loop_ctx,
                )
                .await,
            ));
        }
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::Clarifying,
            "ask_mode_is_clarify_only",
            None,
        );
        let suppress_recent_execution_context = should_suppress_recent_execution_in_clarify_context(
            route_result,
            fuzzy_locator_suggestions,
        );
        let clarify_context = build_locator_fuzzy_clarify_context(
            recent_execution_context,
            fuzzy_locator_suggestions,
            !suppress_recent_execution_context,
        );
        let structured_clarify_context =
            structured_missing_locator_clarify_context(route_result, fuzzy_locator_suggestions);
        let clarify_context = match structured_clarify_context.as_deref() {
            Some(context)
                if clarify_context.trim().is_empty() || clarify_context.trim() == "<none>" =>
            {
                format!("### STRUCTURED_CLARIFY_CONTEXT\n{context}")
            }
            Some(context) => {
                format!("{clarify_context}\n\n### STRUCTURED_CLARIFY_CONTEXT\n{context}")
            }
            None => clarify_context,
        };
        let preferred_clarify_question = if should_reuse_route_clarify_question(
            route_result,
            clarify_reason_kind,
            fuzzy_locator_suggestions,
        ) {
            let route_question = route_result.clarify_question.trim();
            (!route_question.is_empty()).then_some(route_question)
        } else {
            None
        };
        let structured_context_requires_llm =
            structured_clarify_context.is_some() && preferred_clarify_question.is_none();
        let clarify_policy = if structured_context_requires_llm
            || (preferred_clarify_question.is_none()
                && route_result.clarify_question.trim().is_empty()
                && !matches!(
                    clarify_reason_kind,
                    crate::post_route_policy::ClarifyReasonKind::FuzzyLocatorCandidates
                )) {
            crate::intent_router::ClarifyQuestionPolicy::SafeFallback
        } else {
            crate::intent_router::ClarifyQuestionPolicy::AllowModel
        };
        let fallback_source = clarify_fallback_source_or_default(clarify_fallback_source);
        let clarify = crate::finalize::render_clarify_question(
            state,
            task,
            crate::finalize::ClarifyRenderRequest {
                user_request: prompt,
                resolver_reason: clarify_reason,
                candidate_context: Some(&clarify_context),
                preferred_question: preferred_clarify_question,
                policy: clarify_policy,
                // §7.2: 路由阶段没拿到可用 clarify_question + 非 fuzzy_locator 触发的 SafeFallback。
                // normalizer LLM 失败必须暴露为 LlmUnavailable，不能伪装成“我没看懂”。
                fallback_source,
            },
        )
        .await;
        let reply = with_dispatch_boundary_attribution(
            task,
            prompt,
            ask_reply_with_visible_process(state, task, prompt, clarify),
            route_result,
            clarify_reason_kind.dispatch_event(),
            clarify_reason_kind.dispatch_old_owner(),
            clarify_reason_kind.dispatch_new_owner(),
            clarify_reason_kind.dispatch_chosen_path(),
            "semantic_route_authority:legacy_pre_agent",
        );
        return Ok(Some(Ok(with_agent_decides_shadow_snapshot(
            state,
            task,
            prompt,
            reply,
            route_result,
            agent_run_context.as_ref(),
        ))));
    }
    if ask_mode.is_resume_discussion() {
        if !resume_discussion_uses_direct_chat_renderer(route_result) {
            crate::log_ask_transition(
                state,
                &task.task_id,
                Some(crate::AskState::Routing),
                crate::AskState::Executing,
                "resume_discussion_requires_agent_loop",
                None,
            );
            let mut loop_ctx = agent_run_context.clone();
            if let Some(route) = loop_ctx.as_mut().and_then(|ctx| ctx.route_result.as_mut()) {
                route.set_planner_execute_finalize(crate::ActFinalizeStyle::ChatWrapped);
                append_route_reason(route, "resume_discussion_requires_agent_loop");
            }
            return Ok(Some(
                crate::agent_engine::run_agent_with_tools(
                    state,
                    task,
                    prompt_with_memory_for_execution,
                    execution_user_request,
                    loop_ctx,
                )
                .await,
            ));
        }
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::ResumeDiscussing,
            "ask_mode_resume_discussion",
            None,
        );
        let resume_prompt_source = crate::resolve_prompt_rel_path_for_vendor(
            &state.skill_rt.workspace_root,
            &crate::active_prompt_vendor_name(state),
            crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_LOGICAL_PATH,
        );
        crate::log_prompt_render(
            state,
            &task.task_id,
            "resume_followup_discussion_prompt",
            &resume_prompt_source,
            None,
        );
        let reply = crate::llm_gateway::run_with_fallback_with_prompt_source(
            state,
            task,
            resolved_prompt_for_execution,
            &resume_prompt_source,
        )
        .await
        .map(|s| crate::AskReply::llm(s.trim().to_string()));
        return Ok(Some(reply));
    }
    if ask_mode.resume_execution() {
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::ResumeExecuting,
            "ask_mode_resume_execution",
            None,
        );
        return Ok(Some(
            crate::agent_engine::run_agent_with_tools(
                state,
                task,
                prompt_with_memory_for_execution,
                execution_user_request,
                agent_run_context.clone(),
            )
            .await,
        ));
    }
    if should_route_schedule_direct {
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::ScheduleDirect,
            "schedule_direct_route",
            None,
        );
        if crate::finalize::try_finalize_schedule_direct_success(
            state,
            task,
            payload,
            prompt,
            resolved_prompt_for_execution,
            route_result,
        )
        .await?
        {
            return Ok(None);
        }
        let routed_to_execute = route_result.is_execute_gate();
        let target_state = if routed_to_execute {
            crate::AskState::Executing
        } else {
            crate::AskState::Chatting
        };
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            target_state,
            "execute_ask_routed_in_schedule_branch",
            None,
        );
        return Ok(Some(
            crate::execute_ask_routed(
                state,
                task,
                chat_prompt_context,
                prompt_with_memory_for_execution,
                resolved_prompt_for_execution,
                execution_user_request,
                agent_mode,
                ask_mode.is_resume_discussion(),
                Some(route_result.ask_mode.clone()),
                agent_run_context.clone(),
            )
            .await,
        ));
    }
    let routed_to_execute = route_result.is_execute_gate();
    let target_state = if routed_to_execute {
        crate::AskState::Executing
    } else {
        crate::AskState::Chatting
    };
    crate::log_ask_transition(
        state,
        &task.task_id,
        Some(crate::AskState::Routing),
        target_state,
        if routed_to_execute {
            "execute_ask_routed_act"
        } else {
            "execute_ask_routed_chat"
        },
        None,
    );
    Ok(Some(
        crate::execute_ask_routed(
            state,
            task,
            chat_prompt_context,
            prompt_with_memory_for_execution,
            resolved_prompt_for_execution,
            execution_user_request,
            agent_mode,
            false,
            Some(route_result.ask_mode.clone()),
            agent_run_context,
        )
        .await,
    ))
}

#[cfg(test)]
#[path = "ask_pipeline_agent_context_tests.rs"]
mod agent_context_tests;
#[cfg(test)]
#[path = "ask_pipeline_clarify_tests.rs"]
mod clarify_tests;
#[cfg(test)]
#[path = "ask_pipeline_resume_tests.rs"]
mod resume_tests;
#[cfg(test)]
#[path = "ask_pipeline_scalar_count_tests.rs"]
mod scalar_count_tests;
#[cfg(test)]
#[path = "ask_pipeline_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "ask_pipeline_tests.rs"]
mod tests;

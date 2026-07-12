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
#[path = "ask_pipeline_boundary_preflight.rs"]
mod boundary_preflight;
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
#[path = "ask_pipeline_post_route_binding.rs"]
mod post_route_binding;
#[path = "ask_pipeline_post_route_refinement.rs"]
mod post_route_refinement;
#[path = "ask_pipeline_quantity_pair_binding.rs"]
mod quantity_pair_binding;
#[path = "ask_pipeline_runtime_status.rs"]
mod runtime_status;
#[path = "ask_pipeline_state_patch_ack.rs"]
mod state_patch_ack;
#[path = "ask_pipeline_structured_anchor_guard.rs"]
mod structured_anchor_guard;
#[path = "ask_pipeline_unbound_context_guard.rs"]
mod unbound_context_guard;
#[path = "ask_pipeline_workspace_locator_binding.rs"]
mod workspace_locator_binding;
use active_binding::{
    active_observed_facts_have_bound_target, active_session_has_bound_target,
    single_component_locator_hint, SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST,
};
pub(super) use agent_context::build_agent_run_context_from_prepared_flow;
use background_locator_guard::{
    background_only_locator_route_should_defer_to_agent_loop, recent_execution_result_segments,
    text_mentions_locator_identity,
};
use bare_topic_guard::{
    bare_topic_clarify_question_should_drop_context_target,
    bare_topic_memory_expansion_route_should_defer_to_agent_loop,
    bare_topic_model_supplied_locator_route_should_defer_to_agent_loop, is_bare_topic_only_prompt,
    route_introduces_unmentioned_distinctive_context_target,
    route_introduces_unmentioned_distinctive_context_target_except_workspace_root,
};
use boundary_preflight::{
    boundary_context_locator_preflight, boundary_post_binding_locator_preflight,
    boundary_safety_preflight, defer_locator_binding_to_agent_loop,
};
use contract_repair::{
    contract_repair_candidate_observations, registry_capability_contract_observation,
};
use default_config::{
    default_main_config_contract_observation,
    defer_config_contract_default_main_config_after_locator_policy,
};
use deictic_guard::{
    deictic_bare_locator_should_defer_to_agent_loop,
    route_locator_hint_matches_active_ordered_entry,
    state_patch_allows_deictic_locator_guard_bypass, state_patch_requires_deictic_locator_clarify,
};
pub(super) use execution_context::execution_user_request;
use execution_context::{
    sanitize_untrusted_normalizer_answer_candidate_for_execution,
    sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution,
    sanitize_untrusted_normalizer_locator_completion_for_loop_boundary,
};
use file_delivery::{
    active_anchor_file_delivery_without_structured_reference_should_defer_to_agent_loop,
    directory_file_delivery_without_structured_selection_should_defer_to_agent_loop,
    generated_file_delivery_uses_runtime_target, refine_unresolved_file_delivery_boundary_contract,
    reject_direct_file_delivery_workspace_root_locator, route_has_structured_list_selector,
    unbound_existing_file_delivery_route_should_defer_to_agent_loop,
};
use locator_hint_binding::{locator_component_token, resolve_existing_workspace_locator_hint};
use locator_resolution::{
    current_workspace_locator_resolution, effective_auto_locator_kind,
    locator_hint_names_workspace_root, locator_hint_points_to_workspace_root,
    normalize_locator_identity_token, normalize_workspace_locator_path,
    should_attempt_auto_locator,
};
use locatorless_observation_guard::{
    command_observation_marker_present, command_observation_route_has_runtime_evidence,
    current_request_has_self_contained_structured_payload,
    locatorless_observation_route_should_defer_to_agent_loop,
    raw_command_output_has_explicit_command, raw_command_request_has_structural_input_locator,
    route_can_execute_without_locator,
};
use post_route_binding::{
    auto_locator_scalar_file_without_current_locator_should_defer_to_agent_loop,
    auto_locator_unbound_workspace_child_without_current_locator_should_defer_to_agent_loop,
    direct_auto_locator_path,
};
use post_route_refinement::apply_post_route_refinements;
use quantity_pair_binding::{
    current_request_quantity_pair_evidence, structural_locator_token_candidates,
    workspace_directory_pair_from_current_request,
};
use runtime_status::{
    append_runtime_status_capability_context, turn_analysis_has_runtime_status_query,
};
use state_patch_ack::{
    alias_state_patch_ack_reply, apply_alias_state_patch_ack_route, session_binding_value_reply,
};
use structured_anchor_guard::{
    active_session_has_structured_observation_anchor, apply_structured_anchor_evidence_repair,
    followup_frame_has_matching_target, observed_facts_have_matching_target,
    session_has_authoritative_deictic_anchor, structured_anchor_route_requires_evidence_repair,
};
use unbound_context_guard::{
    current_workspace_summary_repair_without_bound_locator_should_defer_to_agent_loop,
    deictic_memory_only_route_should_defer_to_agent_loop,
    execute_route_without_input_locator_should_plan,
    runtime_status_query_route_can_plan_without_locator,
    task_control_route_can_plan_without_locator,
    unbound_model_context_target_route_should_defer_to_agent_loop,
    unbound_targeted_evidence_route_should_defer_to_agent_loop,
};
use workspace_locator_binding::{
    current_request_has_concrete_locator_surface,
    current_request_has_structural_locator_surface_for_route,
    current_request_resolves_workspace_child_locator,
    implicit_workspace_file_locator_route_should_defer_to_agent_loop,
    inferred_missing_workspace_locator_hint_should_defer_to_agent_loop,
    locator_hint_full_file_name_token_present_in_prompt,
    model_completed_workspace_file_locator_hint_should_defer_to_agent_loop,
    path_scoped_locator_guard_can_defer_to_prompt_targets,
    recent_artifacts_judgment_can_use_recent_execution_context,
    structured_field_route_has_current_locator_surface, workspace_root_name_token_present,
    workspace_root_topic_route_should_require_evidence,
};

pub(super) struct PreparedAskFlow {
    pub(super) context_bundle_summary: String,
    pub(super) memory_trace: Option<Value>,
    pub(super) route_result: crate::RouteResult,
    pub(super) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(super) execution_recipe_plan_hint: Option<crate::intent_router::ExecutionRecipePlanHint>,
    pub(super) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
    pub(super) boundary_envelope: Option<crate::intent_router::BoundaryEnvelope>,
    pub(super) clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    pub(super) auto_locator_path: Option<String>,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) recent_execution_context: String,
    pub(super) session_alias_bindings: Vec<crate::conversation_state::SessionAliasBinding>,
    /// Final runtime ask mode after routing and post-route refinements.
    /// Dispatch decisions must use ask_mode predicates.
    pub(super) ask_mode: crate::AskMode,
    pub(super) fuzzy_locator_suggestions: Vec<String>,
    pub(super) should_route_schedule_direct: bool,
}

struct BuiltLoopContext {
    execution_route_result: crate::RouteResult,
    auto_locator_path: Option<String>,
    #[cfg(test)]
    gate_record: crate::post_route_policy::PostRouteGateRecord,
    resolved_prompt_for_execution: String,
    prompt_with_memory_for_execution: String,
    session_alias_bindings: Vec<crate::conversation_state::SessionAliasBinding>,
    fuzzy_locator_suggestions: Vec<String>,
}

fn log_route_guard_record(
    task: &crate::ClaimedTask,
    owner_layer: &'static str,
    reason_code: &'static str,
    outcome: &'static str,
    before_gate_kind: crate::RouteGateKind,
    route_result: &crate::RouteResult,
) {
    let final_answer_shape = crate::evidence_policy::final_answer_shape_for_route(route_result);
    let final_answer_shape_token = final_answer_shape
        .map(crate::evidence_policy::FinalAnswerShape::as_str)
        .unwrap_or("none");
    let final_answer_shape_class = final_answer_shape
        .map(|shape| shape.class().as_str())
        .unwrap_or("none");
    info!(
        "route_guard_record task_id={} owner_layer={} reason_code={} outcome={} before_gate_kind={} after_gate_kind={} needs_clarify={} locator_kind={} final_answer_shape={} final_answer_shape_class={} response_shape={} delivery_required={} content_evidence={}",
        task.task_id,
        owner_layer,
        reason_code,
        outcome,
        before_gate_kind.as_str(),
        route_result.gate_kind().as_str(),
        route_result.needs_clarify,
        route_result.output_contract.locator_kind.as_str(),
        final_answer_shape_token,
        final_answer_shape_class,
        route_result.output_contract.response_shape.as_str(),
        route_result.output_contract.delivery_required,
        route_result.output_contract.requires_content_evidence,
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerRouteMarker {
    AgentLoopDefaultEntry,
    BareTopicContextualClarifySanitized,
    AutoLocatorSuppressedMultipleExplicitPaths,
}

impl WorkerRouteMarker {
    fn route_reason(self) -> &'static str {
        match self {
            Self::AgentLoopDefaultEntry => "agent_loop_default_entry",
            Self::BareTopicContextualClarifySanitized => "bare_topic_contextual_clarify_sanitized",
            Self::AutoLocatorSuppressedMultipleExplicitPaths => {
                "auto_locator_suppressed_multiple_explicit_paths"
            }
        }
    }

    fn record(self, route_result: &mut crate::RouteResult) {
        append_route_reason(route_result, self.route_reason());
    }
}

fn agent_loop_default_context(
    mut agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Option<crate::agent_engine::AgentRunContext> {
    if let Some(route) = agent_run_context
        .as_mut()
        .and_then(|ctx| ctx.route_result.as_mut())
    {
        route.needs_clarify = false;
        route.clarify_question.clear();
        route.set_act_finalize(crate::ActFinalizeStyle::ChatWrapped);
        WorkerRouteMarker::AgentLoopDefaultEntry.record(route);
    }
    agent_run_context
}

fn push_pre_loop_clarify_candidate(candidates: &mut Vec<&'static str>, candidate: &'static str) {
    if !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

pub(super) fn pre_loop_candidates_redact_untrusted_workspace_child(
    candidates: &[&'static str],
) -> bool {
    candidates.iter().any(|candidate| {
        matches!(
            *candidate,
            "auto_locator_unbound_workspace_child_without_current_locator"
                | "unbound_targeted_evidence"
                | "implicit_workspace_file_locator"
                | "model_completed_workspace_file_locator"
                | "inferred_missing_workspace_locator"
                | "background_only_locator"
                | "bare_topic_model_supplied_locator"
                | "unbound_model_context_target"
        )
    })
}

fn post_route_redacts_untrusted_workspace_child(
    post_route: &crate::post_route_policy::PostRoutePolicyResult,
) -> bool {
    post_route.gate_record.reason_code == "post_route_non_boundary_clarify_deferred_to_agent_loop"
        || route_reason_has_marker(
            &post_route.execution_route_result,
            "standalone_freeform_clarify_loop_context",
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerLoopBoundaryDeferral {
    BareTopicContextExpansion,
    UnboundExistingFileDelivery,
    DirectoryFileDeliveryWithoutStructuredSelection,
    DeicticBareLocator,
}

impl WorkerLoopBoundaryDeferral {
    fn observation_token(self) -> &'static str {
        match self {
            Self::BareTopicContextExpansion => "bare_topic_context_expansion",
            Self::UnboundExistingFileDelivery => "unbound_existing_file_delivery",
            Self::DirectoryFileDeliveryWithoutStructuredSelection => {
                "directory_file_delivery_without_structured_selection"
            }
            Self::DeicticBareLocator => "deictic_bare_locator",
        }
    }

    fn guard_reason_code(self) -> Option<&'static str> {
        match self {
            Self::BareTopicContextExpansion => None,
            Self::UnboundExistingFileDelivery => {
                Some("unbound_existing_file_delivery_deferred_to_agent_loop")
            }
            Self::DirectoryFileDeliveryWithoutStructuredSelection => {
                Some("directory_file_delivery_deferred_to_agent_loop")
            }
            Self::DeicticBareLocator => Some("deictic_bare_locator_deferred_to_agent_loop"),
        }
    }

    fn apply_boundary_contract(self, route_result: &mut crate::RouteResult) {
        match self {
            Self::BareTopicContextExpansion => {}
            Self::UnboundExistingFileDelivery
            | Self::DirectoryFileDeliveryWithoutStructuredSelection => {
                route_result.wants_file_delivery = true;
                route_result.output_contract.delivery_required = true;
                route_result.output_contract.response_shape = crate::OutputResponseShape::FileToken;
                route_result.output_contract.delivery_intent =
                    crate::OutputDeliveryIntent::FileSingle;
                route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
                route_result.output_contract.locator_hint.clear();
            }
            Self::DeicticBareLocator => defer_locator_binding_to_agent_loop(route_result),
        }
    }

    fn record(
        self,
        task: &crate::ClaimedTask,
        pre_loop_clarify_candidates: &mut Vec<&'static str>,
        route_result: &mut crate::RouteResult,
    ) {
        let before_gate_kind = route_result.gate_kind();
        self.apply_boundary_contract(route_result);
        push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, self.observation_token());
        if let Some(reason_code) = self.guard_reason_code() {
            log_route_guard_record(
                task,
                "worker_locator_guard",
                reason_code,
                "deferred",
                before_gate_kind,
                route_result,
            );
        }
    }
}

fn append_agent_loop_boundary_observations(
    state: &AppState,
    post_route: &crate::post_route_policy::PostRoutePolicyResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    prompt: &str,
    resolved_prompt: &str,
    pre_loop_clarify_candidates: &[&'static str],
    resolved_prompt_for_execution: &mut String,
    prompt_with_memory_for_execution: &mut String,
) {
    let Some(block) = agent_loop_boundary_observations_block(
        state,
        post_route,
        session_snapshot,
        turn_analysis,
        prompt,
        resolved_prompt,
        pre_loop_clarify_candidates,
    ) else {
        return;
    };
    append_agent_loop_boundary_observation_block(resolved_prompt_for_execution, &block);
    append_agent_loop_boundary_observation_block(prompt_with_memory_for_execution, &block);
}

fn append_agent_loop_boundary_observation_block(target: &mut String, block: &str) {
    if target.contains("### AGENT_LOOP_BOUNDARY_OBSERVATIONS") {
        return;
    }
    if !target.ends_with('\n') {
        target.push('\n');
    }
    target.push('\n');
    target.push_str(block);
    target.push('\n');
}

fn agent_loop_boundary_observations_block(
    state: &AppState,
    post_route: &crate::post_route_policy::PostRoutePolicyResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    prompt: &str,
    resolved_prompt: &str,
    pre_loop_clarify_candidates: &[&'static str],
) -> Option<String> {
    let route = &post_route.execution_route_result;
    let route_reason_codes = boundary_observation_route_reason_codes(route);
    let session_alias_bindings = session_alias_binding_observations(session_snapshot);
    let active_bound_targets = active_bound_target_observations(session_snapshot);
    let has_auto_locator = post_route
        .auto_locator_path
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty() || post_route.auto_locator_resolved_direct);
    let has_fuzzy_locator = !post_route.fuzzy_locator_suggestions.is_empty();
    let auto_locator_boundary_ready = has_auto_locator
        && matches!(
            post_route.gate_record.outcome,
            crate::post_route_policy::PostRoutePolicyOutcome::BoundaryReady
        );
    let missing_referent =
        missing_referent_observation(state, prompt, route, &active_bound_targets).or_else(|| {
            unbound_contextual_locator_missing_referent_observation(
                state,
                prompt,
                route,
                &active_bound_targets,
            )
        });
    let missing_referent = if auto_locator_boundary_ready {
        None
    } else {
        missing_referent
    };
    let file_delivery_target_candidates =
        file_delivery_target_candidate_observations(route, session_snapshot);
    let current_workspace_scope = current_workspace_scope_observation(state, route);
    let active_plan_files = if missing_referent.is_some()
        || !route_allows_active_plan_file_observations(route, turn_analysis)
    {
        Vec::new()
    } else {
        active_plan_file_observations(state)
    };
    let default_main_config_contract =
        default_main_config_contract_observation(state, prompt, route);
    let current_request_locator = if default_main_config_contract.is_some() {
        None
    } else {
        current_request_locator_observation(
            state,
            prompt,
            route,
            pre_loop_candidates_redact_untrusted_workspace_child(pre_loop_clarify_candidates)
                || post_route_redacts_untrusted_workspace_child(post_route),
        )
    };
    let registry_capability_contract =
        registry_capability_contract_observation(resolved_prompt, route);
    let contract_repair_candidates =
        contract_repair_candidate_observations(state, prompt, resolved_prompt, route);
    let runtime_session_state = runtime_session_state_observation(session_snapshot, turn_analysis);
    let has_boundary_gate = post_route.gate_record.outcome
        != crate::post_route_policy::PostRoutePolicyOutcome::NoChange
        || route.needs_clarify
        || post_route.missing_locator_for_path_scoped_content;

    if !has_boundary_gate
        && !has_auto_locator
        && !has_fuzzy_locator
        && route_reason_codes.is_empty()
        && session_alias_bindings.is_empty()
        && active_bound_targets.is_empty()
        && missing_referent.is_none()
        && file_delivery_target_candidates.is_empty()
        && current_workspace_scope.is_none()
        && active_plan_files.is_empty()
        && current_request_locator.is_none()
        && default_main_config_contract.is_none()
        && registry_capability_contract.is_none()
        && contract_repair_candidates.is_empty()
        && runtime_session_state.is_none()
        && pre_loop_clarify_candidates.is_empty()
    {
        return None;
    }

    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "needs_clarify": route.needs_clarify,
        "locator_kind": route.output_contract.locator_kind.as_str(),
        "delivery_required": route.output_contract.delivery_required,
        "content_evidence_required": route.output_contract.requires_content_evidence,
        "post_route_boundary_record": {
            "owner_layer": post_route.gate_record.owner_layer,
            "reason_code": post_route.gate_record.reason_code,
            "outcome": post_route.gate_record.outcome.as_str(),
            "missing_locator_for_path_scoped_content": post_route.missing_locator_for_path_scoped_content,
        },
        "auto_locator": {
            "resolved_direct": post_route.auto_locator_resolved_direct,
            "path": post_route.auto_locator_path.as_deref().unwrap_or(""),
            "fuzzy_candidates": post_route.fuzzy_locator_suggestions,
        },
        "session_alias_bindings": session_alias_bindings,
        "active_bound_targets": active_bound_targets,
        "missing_referent": missing_referent,
        "file_delivery_target_candidates": file_delivery_target_candidates,
        "current_workspace_scope": current_workspace_scope,
        "active_plan_files": active_plan_files,
        "current_request_locator": current_request_locator,
        "default_main_config_contract": default_main_config_contract,
        "registry_capability_contract": registry_capability_contract,
        "contract_repair_candidates": contract_repair_candidates,
        "runtime_session_state": runtime_session_state,
        "pre_loop_clarify_candidates": pre_loop_clarify_candidates,
        "route_reason_codes": route_reason_codes,
    });
    let encoded = serde_json::to_string(&observation).ok()?;
    Some(format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{encoded}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS"
    ))
}

fn missing_referent_observation(
    state: &AppState,
    prompt: &str,
    route: &crate::RouteResult,
    active_bound_targets: &[serde_json::Value],
) -> Option<serde_json::Value> {
    if !route.has_route_reason_machine_marker("standalone_freeform_clarify_loop_context")
        || !active_bound_targets.is_empty()
    {
        return None;
    }
    if !current_request_has_concrete_locator_surface(prompt)
        && current_request_resolves_workspace_child_locator(state, prompt).is_none()
    {
        return None;
    }
    Some(serde_json::json!({
        "owner_layer": "agent_loop_boundary",
        "reason_code": "unbound_deictic_reference",
        "status_code": "missing_referent",
        "missing_slot": "referent",
    }))
}

fn unbound_contextual_locator_missing_referent_observation(
    state: &AppState,
    prompt: &str,
    route: &crate::RouteResult,
    active_bound_targets: &[serde_json::Value],
) -> Option<serde_json::Value> {
    if !active_bound_targets.is_empty() || route.needs_clarify {
        return None;
    }
    if !route.has_route_reason_machine_marker("current_turn_locator_overrides_contextual_path")
        || !route.has_route_reason_machine_marker("executable_contract_preserved_for_agent_loop")
    {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if surface.has_explicit_path_or_url()
        || surface
            .filename_candidates_excluding_field_selectors()
            .is_empty()
    {
        return None;
    }
    if route.output_contract.locator_hint.trim().starts_with('/') {
        return None;
    }
    if current_request_resolves_workspace_child_locator(state, prompt).is_some() {
        return None;
    }
    Some(serde_json::json!({
        "owner_layer": "agent_loop_boundary",
        "reason_code": "unbound_deictic_reference",
        "status_code": "missing_referent",
        "missing_slot": "referent",
        "source": "unbound_contextual_locator",
    }))
}

fn runtime_session_state_observation(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<serde_json::Value> {
    let active_followup_present = session_snapshot.active_followup_frame.is_some()
        || session_snapshot
            .conversation_state
            .as_ref()
            .and_then(|state| state.active_followup_task_id.as_deref())
            .map(str::trim)
            .is_some_and(|task_id| !task_id.is_empty());
    let active_clarify_present = session_snapshot.active_clarify_state.is_some()
        || session_snapshot
            .conversation_state
            .as_ref()
            .and_then(|state| state.active_clarify_task_id.as_deref())
            .map(str::trim)
            .is_some_and(|task_id| !task_id.is_empty());
    let active_observed_facts_present = session_snapshot.active_observed_facts.is_some()
        || session_snapshot
            .conversation_state
            .as_ref()
            .and_then(|state| state.active_observed_facts_task_id.as_deref())
            .map(str::trim)
            .is_some_and(|task_id| !task_id.is_empty());
    let runtime_status_query_requested =
        turn_analysis.is_some_and(turn_analysis_has_runtime_status_query);
    if !runtime_status_query_requested
        && !active_followup_present
        && !active_clarify_present
        && !active_observed_facts_present
    {
        return None;
    }
    Some(serde_json::json!({
        "source": "active_session_snapshot",
        "runtime_status_query_requested": runtime_status_query_requested,
        "active_followup_present": active_followup_present,
        "active_clarify_present": active_clarify_present,
        "active_observed_facts_present": active_observed_facts_present,
        "active_task_present": active_followup_present || active_clarify_present || active_observed_facts_present,
        "pending_user_boundary_present": active_clarify_present,
    }))
}

fn current_request_locator_observation(
    state: &AppState,
    prompt: &str,
    route: &crate::RouteResult,
    redact_untrusted_workspace_child: bool,
) -> Option<serde_json::Value> {
    if default_main_config_contract_observation(state, prompt, route).is_some() {
        return None;
    }
    let explicit_locators =
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt);
    let mut explicit_locator_hints = explicit_locators
        .iter()
        .filter_map(|locator| {
            let hint = locator.locator_hint.trim();
            if hint.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "kind": locator.locator_kind.as_str(),
                "hint": hint,
            }))
        })
        .collect::<Vec<_>>();
    if explicit_locator_hints.is_empty() {
        if let Some(locator) =
            crate::intent::locator_extractor::extract_explicit_locator_for_fallback(prompt)
        {
            let hint = locator.locator_hint.trim();
            if !hint.is_empty() {
                explicit_locator_hints.push(serde_json::json!({
                    "kind": locator.locator_kind.as_str(),
                    "hint": hint,
                }));
            }
        }
    }
    let has_concrete_surface = current_request_has_concrete_locator_surface(prompt);
    let resolved_workspace_child = current_request_resolves_workspace_child_locator(state, prompt);
    let resolved_workspace_child_redacted = redact_untrusted_workspace_child
        && !has_concrete_surface
        && explicit_locator_hints.is_empty()
        && resolved_workspace_child
            .as_deref()
            .map(str::trim)
            .is_some_and(|path| !path.is_empty());
    let resolved_workspace_path_pair = current_request_quantity_pair_evidence(state, prompt, route)
        .map(|(left, right)| vec![left, right])
        .unwrap_or_default();
    let mentions_workspace_root =
        workspace_root_name_token_present(&state.skill_rt.workspace_root, prompt);
    let resolved_workspace_root = mentions_workspace_root.then(|| {
        state
            .skill_rt
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| state.skill_rt.workspace_root.clone())
            .display()
            .to_string()
    });
    let has_multiple_local_paths =
        has_multiple_distinct_explicit_local_path_locators(state, prompt, None);
    if !has_concrete_surface
        && explicit_locator_hints.is_empty()
        && resolved_workspace_child.is_none()
        && !resolved_workspace_child_redacted
        && resolved_workspace_path_pair.is_empty()
        && resolved_workspace_root.is_none()
        && !has_multiple_local_paths
    {
        return None;
    }
    Some(serde_json::json!({
        "source": "current_request",
        "has_concrete_surface": has_concrete_surface,
        "explicit_locator_hints": explicit_locator_hints,
        "resolved_workspace_child": if resolved_workspace_child_redacted {
            ""
        } else {
            resolved_workspace_child.as_deref().unwrap_or("")
        },
        "resolved_workspace_child_redacted": resolved_workspace_child_redacted,
        "resolved_workspace_path_pair": resolved_workspace_path_pair,
        "mentions_workspace_root": mentions_workspace_root,
        "resolved_workspace_root": resolved_workspace_root.as_deref().unwrap_or(""),
        "has_multiple_local_paths": has_multiple_local_paths,
    }))
}

fn current_workspace_scope_observation(
    state: &AppState,
    route: &crate::RouteResult,
) -> Option<serde_json::Value> {
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.wants_file_delivery
        || !current_workspace_scope_has_count_shape(route)
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
    {
        return None;
    }
    let has_current_workspace_scope = route.output_contract.locator_kind
        == crate::OutputLocatorKind::CurrentWorkspace
        || route_reason_has_marker(route, "current_workspace_scope_from_current_request");
    if !has_current_workspace_scope {
        return None;
    }
    let final_answer_shape = crate::evidence_policy::final_answer_shape_for_route(route);
    Some(serde_json::json!({
        "source": "current_workspace_scope",
        "target": state.skill_rt.workspace_root.display().to_string(),
        "task_shape": "scalar_count",
        "final_answer_shape": final_answer_shape.map(crate::evidence_policy::FinalAnswerShape::as_str),
        "final_answer_shape_class": final_answer_shape.map(|shape| shape.class().as_str()),
        "response_shape": route.output_contract.response_shape.as_str(),
    }))
}

fn current_workspace_scope_has_count_shape(route: &crate::RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
    ) || (route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.exact_sentence_count == Some(1))
}

fn route_allows_active_plan_file_observations(
    route: &crate::RouteResult,
    _turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.wants_file_delivery
        || route.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
    {
        return false;
    }

    route.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary)
}

fn active_plan_file_observations(state: &AppState) -> Vec<serde_json::Value> {
    let plan_root = state.skill_rt.workspace_root.join("plan");
    let Ok(entries) = std::fs::read_dir(&plan_root) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let file_name = path.file_name()?.to_str()?.trim();
            if file_name.is_empty() || file_name.starts_with('.') {
                return None;
            }
            let logical_path = path
                .strip_prefix(&state.skill_rt.workspace_root)
                .ok()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            Some((
                logical_path.clone(),
                serde_json::json!({
                    "source": "workspace_plan_directory",
                    "logical_path": logical_path,
                    "workspace_path": path.display().to_string(),
                    "bytes": metadata.len(),
                }),
            ))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files.into_iter().take(8).map(|(_, value)| value).collect()
}

fn session_alias_binding_observations(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<serde_json::Value> {
    session_snapshot
        .conversation_state
        .as_ref()
        .map(|state| {
            state
                .alias_bindings
                .iter()
                .filter_map(|binding| {
                    let alias = binding.alias.trim();
                    let target = binding.target.trim();
                    if alias.is_empty() || target.is_empty() {
                        return None;
                    }
                    Some(serde_json::json!({
                        "alias": alias,
                        "target": target,
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn active_bound_target_observations(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        let expose_followup_target = matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Read
                | crate::followup_frame::FollowupOpKind::List
                | crate::followup_frame::FollowupOpKind::CodeWorkspace
                | crate::followup_frame::FollowupOpKind::Delivery
                | crate::followup_frame::FollowupOpKind::ClarifyPending
        );
        if expose_followup_target {
            let target = frame
                .bound_target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty());
            let ordered_targets = active_followup_ordered_targets(frame);
            if target.is_some() || !ordered_targets.is_empty() {
                out.push(serde_json::json!({
                    "source": "active_followup_frame",
                    "op_kind": followup_op_kind_token(frame.op_kind),
                    "target": target.unwrap_or(""),
                    "ordered_targets": ordered_targets,
                    "ordered_entry_count": frame.ordered_entries.len(),
                }));
            }
        } else if !frame.source_request.trim().is_empty() {
            out.push(serde_json::json!({
                "source": "active_followup_frame",
                "op_kind": followup_op_kind_token(frame.op_kind),
                "target": "",
                "ordered_targets": Vec::<String>::new(),
                "ordered_entry_count": 0,
            }));
        }
    }
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts
            .bound_target
            .as_deref()
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            out.push(serde_json::json!({
                "source": "active_observed_facts",
                "target": target,
                "ordered_entry_count": facts.ordered_entries.len(),
                "observed_entry_count": facts.observed_entry_count,
            }));
        }
    }
    out
}

fn file_delivery_target_candidate_observations(
    route: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<serde_json::Value> {
    let route_requests_file_delivery = route.wants_file_delivery
        || route.output_contract.delivery_required
        || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
        || route.output_contract.delivery_intent == crate::OutputDeliveryIntent::FileSingle;
    if !route_requests_file_delivery || !route.output_contract.locator_hint.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        if matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Read
                | crate::followup_frame::FollowupOpKind::Delivery
        ) {
            if let Some(target) = frame
                .bound_target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty())
            {
                out.push(serde_json::json!({
                    "source": "active_followup_frame",
                    "op_kind": followup_op_kind_token(frame.op_kind),
                    "target": target,
                }));
            }
        }
    }
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts
            .bound_target
            .as_deref()
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            out.push(serde_json::json!({
                "source": "active_observed_facts",
                "target": target,
            }));
        }
    }
    out
}

fn active_followup_ordered_targets(frame: &crate::followup_frame::FollowupFrame) -> Vec<String> {
    frame
        .ordered_entries
        .iter()
        .enumerate()
        .filter_map(|(index, _)| crate::followup_frame::ordered_entry_target_at(frame, index))
        .map(|target| target.trim().to_string())
        .filter(|target| !target.is_empty())
        .collect()
}

fn followup_op_kind_token(kind: crate::followup_frame::FollowupOpKind) -> &'static str {
    match kind {
        crate::followup_frame::FollowupOpKind::Generic => "generic",
        crate::followup_frame::FollowupOpKind::Read => "read",
        crate::followup_frame::FollowupOpKind::List => "list",
        crate::followup_frame::FollowupOpKind::CodeWorkspace => "code_workspace",
        crate::followup_frame::FollowupOpKind::Delivery => "delivery",
        crate::followup_frame::FollowupOpKind::ClarifyPending => "clarify_pending",
    }
}

fn route_reason_machine_codes(route_result: &crate::RouteResult) -> Vec<String> {
    let mut codes = route_result
        .route_reason
        .split(';')
        .filter_map(|part| {
            let token = part.trim();
            valid_route_reason_machine_code(token).then_some(token.to_string())
        })
        .collect::<Vec<_>>();
    codes.sort();
    codes.dedup();
    codes
}

fn boundary_observation_route_reason_codes(route_result: &crate::RouteResult) -> Vec<String> {
    route_reason_machine_codes(route_result)
        .into_iter()
        .filter(|code| !is_route_trace_reason_code(code))
        .collect()
}

fn is_route_trace_reason_code(code: &str) -> bool {
    matches!(
        code,
        "executionless_finalize_trace_plain"
            | "respond_trace_inferred"
            | "act_trace_inferred"
            | "clarify_trace_inferred"
            | "agent_loop_default_entry"
    )
}

fn valid_route_reason_machine_code(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 96
        && token.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
        && token.bytes().any(|byte| byte == b'_')
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

fn defer_subagent_boundary_clarify_to_agent_loop(
    task: &crate::ClaimedTask,
    post_route: &mut crate::post_route_policy::PostRoutePolicyResult,
) {
    let before_gate_kind = post_route.execution_route_result.gate_kind();
    post_route.execution_route_result.needs_clarify = false;
    post_route.execution_route_result.clarify_question.clear();
    post_route
        .execution_route_result
        .set_act_finalize(crate::ActFinalizeStyle::ChatWrapped);
    post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
        "post_route_subagent_boundary_clarify_deferred_to_agent_loop",
        crate::post_route_policy::PostRoutePolicyOutcome::BoundaryReady,
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

fn build_loop_context_after_boundary_preflight(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    resolved_prompt: &str,
    recent_execution_context: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    mut route_result: crate::RouteResult,
    mut resolved_prompt_for_execution: String,
    mut prompt_with_memory_for_execution: String,
) -> BuiltLoopContext {
    let session_snapshot = crate::conversation_state::load_active_session_snapshot(state, task);
    let has_authoritative_deictic_anchor =
        session_has_authoritative_deictic_anchor(prompt, &route_result, &session_snapshot);
    append_runtime_status_capability_context(&mut route_result, turn_analysis);
    let mut pre_loop_clarify_candidates: Vec<&'static str> = Vec::new();
    boundary_safety_preflight(
        state,
        task,
        prompt,
        turn_analysis,
        &session_snapshot,
        &mut pre_loop_clarify_candidates,
        &mut route_result,
    );
    boundary_post_binding_locator_preflight(
        state,
        task,
        prompt,
        turn_analysis,
        &session_snapshot,
        &mut pre_loop_clarify_candidates,
        &mut route_result,
    );
    boundary_context_locator_preflight(
        state,
        task,
        prompt,
        resolved_prompt,
        recent_execution_context,
        turn_analysis,
        &session_snapshot,
        &mut pre_loop_clarify_candidates,
        &mut route_result,
    );
    if workspace_root_topic_route_should_require_evidence(
        &state.skill_rt.workspace_root,
        prompt,
        &route_result,
    ) {
        let before_gate_kind = route_result.gate_kind();
        apply_workspace_root_topic_evidence_contract(state, &mut route_result);
        log_route_guard_record(
            task,
            "worker_workspace_scope_guard",
            "workspace_root_topic_requires_evidence",
            "repaired",
            before_gate_kind,
            &route_result,
        );
    }
    if bare_topic_memory_expansion_route_should_defer_to_agent_loop(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        WorkerLoopBoundaryDeferral::BareTopicContextExpansion.record(
            task,
            &mut pre_loop_clarify_candidates,
            &mut route_result,
        );
    }
    if bare_topic_clarify_question_should_drop_context_target(prompt, &route_result) {
        route_result.clarify_question.clear();
        WorkerRouteMarker::BareTopicContextualClarifySanitized.record(&mut route_result);
    }
    if unbound_existing_file_delivery_route_should_defer_to_agent_loop(
        state,
        prompt,
        &route_result,
        has_authoritative_deictic_anchor,
    ) {
        WorkerLoopBoundaryDeferral::UnboundExistingFileDelivery.record(
            task,
            &mut pre_loop_clarify_candidates,
            &mut route_result,
        );
    }
    reject_direct_file_delivery_workspace_root_locator(
        state,
        recent_execution_context,
        &mut route_result,
    );
    if directory_file_delivery_without_structured_selection_should_defer_to_agent_loop(
        state,
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        WorkerLoopBoundaryDeferral::DirectoryFileDeliveryWithoutStructuredSelection.record(
            task,
            &mut pre_loop_clarify_candidates,
            &mut route_result,
        );
    }
    if deictic_bare_locator_should_defer_to_agent_loop(
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        WorkerLoopBoundaryDeferral::DeicticBareLocator.record(
            task,
            &mut pre_loop_clarify_candidates,
            &mut route_result,
        );
    }
    if structured_anchor_route_requires_evidence_repair(
        prompt,
        &route_result,
        &session_snapshot,
        recent_execution_context,
        has_authoritative_deictic_anchor,
        turn_analysis,
    ) {
        let before_gate_kind = route_result.gate_kind();
        apply_structured_anchor_evidence_repair(&mut route_result);
        log_route_guard_record(
            task,
            "worker_active_task_guard",
            "structured_anchor_requires_evidence",
            "repaired",
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
        WorkerRouteMarker::AutoLocatorSuppressedMultipleExplicitPaths.record(&mut route_result);
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
    apply_post_route_refinements(
        state,
        task,
        prompt,
        turn_analysis,
        &session_snapshot,
        &mut pre_loop_clarify_candidates,
        &mut post_route,
    );
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
            "{} worker_once: ask locator_boundary_needs_loop_decision task_id={} reason=locator_required_for_path_scoped_content raw_text={} resolved_text={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(prompt),
            crate::truncate_for_log(resolved_prompt)
        );
    }
    info!(
        "{} worker_once: ask post_route_boundary_record task_id={} owner_layer={} reason_code={} outcome={}",
        crate::highlight_tag("routing"),
        task.task_id,
        post_route.gate_record.owner_layer,
        post_route.gate_record.reason_code,
        post_route.gate_record.outcome.as_str()
    );
    if post_route.execution_route_result.gate_kind() != route_result.gate_kind() {
        info!(
            "{} worker_once: ask boundary_mode_refined_by_auto_locator task_id={} gate={:?}->{:?}",
            crate::highlight_tag("routing"),
            task.task_id,
            route_result.gate_kind(),
            post_route.execution_route_result.gate_kind()
        );
    } else if post_route.execution_route_result.ask_mode != route_result.ask_mode {
        info!(
            "{} worker_once: ask boundary_dispatch_hint_refined_by_auto_locator task_id={} ask_mode={} -> {} route_trace_label={} -> {}",
            crate::highlight_tag("routing"),
            task.task_id,
            route_result.ask_mode.as_str(),
            post_route.execution_route_result.ask_mode.as_str(),
            route_result.route_trace_label_for_log(),
            post_route
                .execution_route_result
                .route_trace_label_for_log()
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
    sanitize_untrusted_normalizer_locator_completion_for_loop_boundary(
        &mut post_route.execution_route_result,
        prompt,
        &pre_loop_clarify_candidates,
        &mut resolved_prompt_for_execution,
        &mut prompt_with_memory_for_execution,
    );
    if subagent_boundary_clarify_should_enter_agent_loop(state, &post_route.execution_route_result)
    {
        defer_subagent_boundary_clarify_to_agent_loop(task, &mut post_route);
    }
    append_agent_loop_boundary_observations(
        state,
        &post_route,
        &session_snapshot,
        turn_analysis,
        prompt,
        resolved_prompt,
        &pre_loop_clarify_candidates,
        &mut resolved_prompt_for_execution,
        &mut prompt_with_memory_for_execution,
    );
    BuiltLoopContext {
        execution_route_result: post_route.execution_route_result,
        auto_locator_path: post_route.auto_locator_path,
        #[cfg(test)]
        gate_record: post_route.gate_record,
        resolved_prompt_for_execution,
        prompt_with_memory_for_execution,
        session_alias_bindings: session_snapshot
            .conversation_state
            .as_ref()
            .map(|conversation_state| conversation_state.alias_bindings.clone())
            .unwrap_or_default(),
        fuzzy_locator_suggestions: post_route.fuzzy_locator_suggestions,
    }
}

fn apply_workspace_root_topic_evidence_contract(
    state: &AppState,
    route_result: &mut crate::RouteResult,
) {
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.delivery_required = false;
    route_result.wants_file_delivery = false;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route_result.output_contract.locator_hint = state.skill_rt.workspace_root.display().to_string();
    append_route_reason(route_result, "contract:workspace_project_summary");
    let finalize = crate::post_route_policy::content_evidence_execution_finalize_style(
        &route_result.output_contract,
        false,
    )
    .unwrap_or(crate::ActFinalizeStyle::ChatWrapped);
    route_result.set_act_finalize(finalize);
    append_route_reason(route_result, "current_workspace_scope_from_current_request");
    append_route_reason(route_result, "workspace_root_topic_requires_evidence");
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

pub(super) fn route_has_capability_ref_machine_signal(route_result: &crate::RouteResult) -> bool {
    route_machine_tokens(route_result).any(|token| {
        token
            .strip_prefix("capability_ref=")
            .is_some_and(crate::machine_capability_ref::is_valid_capability_ref_value)
    })
}

pub(super) fn route_reason_has_capability_ref_prefix(
    route_result: &crate::RouteResult,
    prefix: &str,
) -> bool {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return false;
    }
    route_result
        .route_reason
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | '(' | ')'))
        .map(str::trim)
        .filter_map(|token| token.strip_prefix("capability_ref="))
        .any(|capability_ref| {
            crate::machine_capability_ref::is_valid_capability_ref_value(capability_ref)
                && capability_ref.starts_with(prefix)
        })
}

fn route_machine_tokens(route_result: &crate::RouteResult) -> impl Iterator<Item = &str> {
    route_result
        .route_reason
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | '(' | ')'))
        .map(str::trim)
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
    } else if !route_reason_has_marker(route_result, reason) {
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
    let prepared_execution = super::prepare_ask_execution_context(
        state,
        task,
        payload,
        &prepared_routing.route_result,
        &prepared_routing.resolved_prompt,
        prepared_routing.turn_analysis.as_ref(),
    )
    .await?;
    let mut loop_context = build_loop_context_after_boundary_preflight(
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
    apply_alias_state_patch_ack_route(
        &mut loop_context.execution_route_result,
        prepared_routing.turn_analysis.as_ref(),
        prepared_routing.boundary_envelope.as_ref(),
    );
    let has_schedule_intent =
        loop_context.execution_route_result.schedule_kind != crate::ScheduleKind::None;
    let final_ask_mode = loop_context.execution_route_result.ask_mode.clone();
    let should_route_schedule_direct = has_schedule_intent
        && !final_ask_mode.resume_execution()
        && !final_ask_mode.is_resume_discussion();
    Ok(PreparedAskFlow {
        context_bundle_summary: prepared_execution.context_bundle.summary(),
        memory_trace: prepared_execution.context_bundle.memory_trace(),
        route_result: loop_context.execution_route_result,
        execution_recipe_hint: prepared_routing.execution_recipe_hint,
        execution_recipe_plan_hint: prepared_routing.execution_recipe_plan_hint,
        turn_analysis: prepared_routing.turn_analysis,
        boundary_envelope: prepared_routing.boundary_envelope,
        clarify_fallback_source: prepared_routing.clarify_fallback_source,
        auto_locator_path: loop_context.auto_locator_path,
        resolved_prompt_for_execution: loop_context.resolved_prompt_for_execution,
        prompt_with_memory_for_execution: loop_context.prompt_with_memory_for_execution,
        recent_execution_context: prepared_execution.recent_execution_context,
        session_alias_bindings: loop_context.session_alias_bindings,
        ask_mode: final_ask_mode,
        fuzzy_locator_suggestions: loop_context.fuzzy_locator_suggestions,
        should_route_schedule_direct,
    })
}

pub(super) async fn execute_ask_dispatch(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    resolved_prompt_for_execution: &str,
    prompt_with_memory_for_execution: &str,
    route_result: &crate::RouteResult,
    ask_mode: &crate::AskMode,
    should_route_schedule_direct: bool,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    boundary_envelope: Option<&crate::intent_router::BoundaryEnvelope>,
) -> Result<Option<Result<crate::AskReply, String>>> {
    let execution_user_request = execution_user_request(prompt, resolved_prompt_for_execution);
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
    }
    if let Some(reply) = crate::self_extension::maybe_handle_ask_self_extension(
        state,
        task,
        resolved_prompt_for_execution,
        &execution_user_request,
        agent_run_context.as_ref(),
    )
    .await
    .map_err(anyhow::Error::msg)?
    {
        return Ok(Some(Ok(reply)));
    }
    if let Some(reply) = alias_state_patch_ack_reply(
        state,
        task,
        prompt,
        route_result,
        turn_analysis,
        boundary_envelope,
    ) {
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::Executing,
            "alias_state_patch_ack_direct",
            None,
        );
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Executing),
            crate::AskState::Finalizing,
            "alias_state_patch_ack",
            None,
        );
        return Ok(Some(Ok(reply)));
    }
    if let Some(reply) = session_binding_value_reply(route_result, boundary_envelope) {
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::Executing,
            "session_binding_value_direct",
            None,
        );
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Executing),
            crate::AskState::Finalizing,
            "session_binding_value_direct",
            None,
        );
        return Ok(Some(Ok(reply)));
    }
    let loop_ctx = agent_loop_default_context(agent_run_context);
    crate::log_ask_transition(
        state,
        &task.task_id,
        Some(crate::AskState::Routing),
        crate::AskState::Executing,
        "agent_loop_default_entry",
        None,
    );
    tracing::info!(
        "{} worker_once: ask agent_loop_default_entry task_id={} previous_gate_kind={} ask_mode={}",
        crate::highlight_tag("routing"),
        task.task_id,
        route_result.gate_kind().as_str(),
        ask_mode.as_str(),
    );
    Ok(Some(
        crate::agent_engine::run_agent_with_tools(
            state,
            task,
            prompt_with_memory_for_execution,
            execution_user_request,
            loop_ctx,
        )
        .await,
    ))
}

#[cfg(test)]
#[path = "ask_pipeline_agent_context_tests.rs"]
mod agent_context_tests;
#[cfg(test)]
#[path = "ask_pipeline_route_reason_tests.rs"]
mod route_reason_tests;
#[cfg(test)]
#[path = "ask_pipeline_scalar_count_tests.rs"]
mod scalar_count_tests;
#[cfg(test)]
#[path = "ask_pipeline_test_support.rs"]
mod test_support;

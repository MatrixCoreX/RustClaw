//! Intent routing and unified normalizer for ask tasks.
//!
//! **Ask main path:** Only `run_intent_normalizer` is used (resolved intent, resume_behavior,
//! schedule_kind, route trace, needs_clarify, and output contract in one LLM call).
//!
//! **Fallback when normalizer LLM fails / parse fails:** stay on AskClarify unless the current
//! request contains an explicit structured tool/capability domain with no competing locator.
//! Those narrow fallbacks keep semantic routing owned by explicit contracts instead of
//! natural-language hard-match recovery code.

use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use tracing::{info, warn};

use crate::{ActFinalizeStyle, AppState, ClaimedTask, FirstLayerDecision};

#[path = "intent_router_scalar_count_filter.rs"]
mod scalar_count_filter;
use scalar_count_filter::normalize_scalar_count_filter_contract_field;

#[path = "intent_router_workspace_locator.rs"]
mod workspace_locator;
use workspace_locator::workspace_direct_child_stem_locator_from_text;

#[path = "intent_router_missing_target.rs"]
mod missing_target;
use missing_target::apply_missing_read_target_mutation_clarify;

#[path = "intent_router_contract_repair_report.rs"]
mod contract_repair_report;
use contract_repair_report::ContractRepairReport;

#[path = "intent_router_contract_repair_judge.rs"]
mod contract_repair_judge;
#[cfg(test)]
use contract_repair_judge::apply_contract_repair_judge_output;
use contract_repair_judge::clear_spurious_generated_file_delivery_attachment_processing;
#[cfg(test)]
use contract_repair_judge::ContractRepairJudgeOut;

#[path = "intent_router_route_trace.rs"]
mod route_trace;
use route_trace::{push_unique_repair_code, route_trace_record};

#[path = "intent_router_turn_analysis.rs"]
mod turn_analysis;
pub(crate) use turn_analysis::{TargetTaskPolicy, TurnAnalysis, TurnType};

#[path = "intent_router_output_types.rs"]
mod output_types;
pub(crate) use output_types::{
    ClarifyQuestionPolicy, ContextResolution, ExecutionRecipePlanHint, IntentNormalizerOutput,
};

#[path = "intent_router_clarify.rs"]
mod clarify;
#[cfg(test)]
use clarify::safe_fallback_source_should_try_llm;
pub(crate) use clarify::{generate_or_reuse_clarify_question, try_handle_schedule_request};

#[path = "intent_router_parse_failed_fallback.rs"]
mod parse_failed_fallback;
#[cfg(test)]
use parse_failed_fallback::{
    parse_failed_explicit_capability_fallback_decision,
    parse_failed_explicit_existing_path_observation_fallback_decision,
};

#[path = "intent_router_path_tokens.rs"]
mod path_tokens;
use path_tokens::{
    compare_path_targets_current_anchor, current_request_mentions_workspace_identity,
    first_compare_path_from_text, locator_hint_compare_path, locator_hint_is_unset_or_broad,
    locator_hint_points_to_workspace_root, scope_patch_hint_value,
    workspace_identity_semantic_repair_context,
};

#[path = "intent_router_schema_parse.rs"]
mod schema_parse;
use schema_parse::{
    infer_missing_turn_type_from_policy, parse_execution_recipe_plan_hint, parse_output_contract,
    parse_output_delivery_intent, parse_output_locator_kind, parse_output_response_shape,
    parse_output_semantic_kind, parse_positive_usize_value, parse_resume_behavior,
    parse_runtime_async_job_start_plan_hint, parse_schedule_kind, parse_target_task_policy,
    parse_turn_type, IntentExecutionRecipeOut, IntentOutputContractOut,
};
#[cfg(test)]
use schema_parse::{parse_self_extension_mode, parse_self_extension_trigger};

#[path = "intent_router_schema_tokens.rs"]
mod schema_tokens;
#[cfg(test)]
use schema_tokens::parse_first_layer_decision_text;
use schema_tokens::{
    contract_value_token, execution_finalize_style_for_contract,
    looks_like_current_workspace_path_alias, machine_context_has_capability_ref,
    normalize_output_delivery_intent_for_schema, normalize_output_locator_kind_for_schema,
    normalize_output_response_shape_for_schema, normalize_output_semantic_kind_for_schema,
    normalize_schema_token, route_label_from_first_layer_decision,
};

#[path = "intent_router_schema_report.rs"]
mod schema_report;
use schema_report::{
    answer_like_normalizer_payload_text, contract_repair_report_from_before_after,
    parse_top_level_json_object_preserving_meaningful_duplicates, scalar_json_value_text,
};

#[path = "intent_router_normalizer_schema_core.rs"]
mod normalizer_schema_core;
use normalizer_schema_core::{
    normalize_bool_field_with_default, normalize_execution_recipe_for_schema,
    normalize_intent_normalizer_scalar_types_for_schema,
    normalize_intent_normalizer_top_level_for_schema, normalize_optional_string_field,
    normalize_plain_intent_normalizer_text_for_schema, sync_compat_decision_trace_for_schema,
};

#[path = "intent_router_normalizer_raw_schema.rs"]
mod normalizer_raw_schema;
#[cfg(test)]
use normalizer_raw_schema::normalize_intent_normalizer_raw_for_schema;
use normalizer_raw_schema::normalize_intent_normalizer_raw_for_schema_with_report;

#[path = "intent_router_output_contract_schema.rs"]
mod output_contract_schema;
use output_contract_schema::{
    apply_raw_output_explicit_locator_repair, coerce_output_contract_value_for_schema,
    normalize_output_contract_for_schema,
};

#[path = "intent_router_runtime_status_recipe.rs"]
mod runtime_status_recipe;
use runtime_status_recipe::{
    execution_recipe_value_declares_command_payload,
    execution_recipe_value_declares_scalar_runtime_tool_observation,
    scalar_runtime_status_kind_from_execution_recipe,
    scalar_runtime_status_kind_from_output_contract, upsert_runtime_status_query_state_patch,
};

#[path = "intent_router_execution_recipe_schema.rs"]
mod execution_recipe_schema;
use execution_recipe_schema::{
    execution_recipe_value_declares_health_check_observation,
    execution_recipe_value_declares_package_detect_manager_capability,
    execution_recipe_value_declares_service_status_observation,
    execution_recipe_value_declares_structured_read_observation,
    execution_recipe_value_declares_structured_scalar_extraction,
    execution_recipe_value_locator_hint, execution_recipe_value_structured_locator_hint,
    normalizer_object_declares_tool_action_payload, output_recipe_value_declares_execution,
    schema_text_declares_execution_recipe, value_has_nonempty_scalar_text, value_has_schema_token,
};

#[path = "intent_router_execution_recipe_contract.rs"]
mod execution_recipe_contract;
use execution_recipe_contract::{
    force_output_contract_semantic_kind, mark_output_contract_requires_content_evidence,
    normalize_output_contract_for_command_payload,
    normalize_output_contract_for_package_detect_manager_capability,
    normalize_output_contract_for_service_status_recipe,
    normalize_output_contract_for_structured_read_recipe,
    promote_misnested_turn_analysis_from_execution_recipe,
};

#[path = "intent_router_state_patch_tokens.rs"]
mod state_patch_tokens;
use state_patch_tokens::{
    request_uses_filename_only_schema_token, state_patch_deictic_reference_is_resolved,
    state_patch_deictic_reference_requires_clarify,
};

#[path = "intent_router_state_patch_fields.rs"]
mod state_patch_fields;
use state_patch_fields::{
    append_state_patch_slice_tokens_to_resolved_intent,
    append_state_patch_structured_field_selector_to_resolved_intent,
    apply_state_patch_structured_field_selector, normalize_structured_field_selector,
    schema_key_is_structured_scalar_field_selector, state_patch_targets_task_lifecycle_fields,
};

#[path = "intent_router_unbound_scope_tokens.rs"]
mod unbound_scope_tokens;
use unbound_scope_tokens::surface_has_unbound_scope_plus_single_filename_target;

#[path = "intent_router_archive_contract.rs"]
mod archive_contract;
use archive_contract::{
    apply_archive_unpack_missing_archive_locator_clarify, archive_list_contract_from_surface,
    archive_pair_contract_from_surface, archive_read_contract_from_surface,
};

#[path = "intent_router_explicit_path_facts.rs"]
mod explicit_path_facts;
use explicit_path_facts::{
    ascii_token_present, explicit_surface_path_fact_targets,
    explicit_surface_path_facts_clarify_repair_decision,
    explicit_surface_path_facts_fallback_decision,
    explicit_surface_path_metadata_clarify_repair_decision, is_bare_path_only_input_for_clarify,
};

#[path = "intent_router_structural_contracts.rs"]
mod structural_contracts;
use structural_contracts::{
    config_mutation_contract_from_surface,
    current_turn_extension_inventory_file_paths_repair_applies,
    current_workspace_generic_summary_needs_semantic_contract,
    existence_with_path_mixed_locator_summary_repair,
    extension_inventory_locator_hint_should_use_workspace,
    file_paths_missing_file_locator_parent_dir,
    generated_file_delivery_existing_content_summary_repair,
    generated_file_delivery_filename_only_existing_target_repair,
    inline_structured_payload_contract_context, inline_structured_transform_contract_context,
    output_contract_structured_config_path, quoted_literal_content_presence_contract_repair,
    structural_config_value_after_field, structured_config_keys_contract_from_surface,
    structured_field_pair_contract_from_quantity_comparison,
    structured_field_value_contract_from_quantity_comparison,
    structured_identifier_presence_contract_from_surface,
    surface_has_directory_scoped_filename_lookup,
};

#[path = "intent_router_contract_hint.rs"]
mod contract_hint;
use contract_hint::{apply_structured_contract_hint_repair, contract_hint_fallback_decision};
pub(crate) use contract_hint::{
    contract_test_hint_semantic_kind, contract_test_hint_value, request_without_contract_test_hint,
};

#[path = "intent_router_inline_transform.rs"]
mod inline_transform;
#[cfg(test)]
use inline_transform::inline_json_transform_fallback_decision;
use inline_transform::parsed_inline_json_transform_repair_decision;

#[path = "intent_router_observation_repair.rs"]
mod observation_repair;
use observation_repair::{
    apply_deictic_missing_locator_state_patch_clarify_repair,
    apply_locatorless_observation_clarify_repair,
    apply_spurious_structured_observation_clarify_repair,
    apply_workspace_default_observation_clarify_repair,
    should_preserve_existing_observed_context_synthesis_contract,
};

#[path = "intent_router_directory_observation.rs"]
mod directory_observation;
use directory_observation::apply_resolved_directory_observation_clarify_repair;
#[cfg(test)]
use directory_observation::directory_pair_fallback_decision;

#[path = "intent_router_execution_contract.rs"]
mod execution_contract;
use execution_contract::{
    apply_command_payload_contract_repair, apply_explicit_command_execution_contract_repair,
    apply_file_delivery_contract_repair, cleanup_executionless_route_finalize_style,
    output_semantic_kind_requires_fresh_evidence, parse_execution_recipe_hint,
    route_has_structured_execution_signal, structured_execution_signal_for_effective_route,
};

#[path = "intent_router_current_turn_anchor.rs"]
mod current_turn_anchor;
use current_turn_anchor::{
    apply_current_turn_anchor_drift_repair, bare_path_only_input_can_fill_active_observable_task,
    current_request_mentions_session_alias, current_turn_anchor_drift_repair_allowed,
    resolve_current_turn_anchor_path, sanitize_resolved_intent_for_current_turn_locator,
};

#[path = "intent_router_current_turn_structural_repair.rs"]
mod current_turn_structural_repair;
#[cfg(test)]
use current_turn_structural_repair::apply_media_generation_path_report_machine_contract_repair;
use current_turn_structural_repair::{
    apply_current_turn_structural_contract_repair,
    apply_fs_basic_lifecycle_machine_contract_repair,
    apply_unbound_workspace_generic_content_clarify_repair,
    apply_workspace_scope_patch_to_contract, infer_missing_target_policy_from_contract,
    is_meaningful_state_patch, should_detach_bare_acknowledgement_from_active_task,
    should_downgrade_orphan_output_shape_clarify_to_direct_answer,
    should_downgrade_standalone_freeform_clarify_to_direct_answer,
};

#[path = "intent_router_active_observation.rs"]
mod active_observation;
#[cfg(test)]
use active_observation::active_ordered_scalar_path_missing_state_patch_context;
#[cfg(test)]
use active_observation::prompt_has_concrete_fileish_cue;
use active_observation::{
    active_clarify_locator_task_prompt, active_observable_task_prompt,
    active_observed_output_loop_context_hint, active_ordered_scalar_path_loop_context_hint,
    active_primary_task_prompt, active_session_has_structured_execution_target,
    active_task_turn_can_reuse_semantic_patch, active_text_followup_surface_is_chat_only,
};

#[path = "intent_router_active_task_repair.rs"]
mod active_task_repair;
#[cfg(test)]
use active_task_repair::unresolved_deictic_observable_target_should_clarify;
use active_task_repair::{
    active_context_has_structured_observation_anchor, active_primary_text_context,
    active_task_mutation_loop_context_hint, apply_active_task_scope_refinement_repair,
    apply_active_task_structured_patch_repair, apply_missing_active_task_reuse_clarify,
    repair_state_patch_replacement_literal_conflicts,
    should_resolve_task_append_clarify_with_active_task,
    should_resolve_task_replace_clarify_with_active_task,
    should_resolve_task_scope_update_clarify_with_active_task,
};

#[path = "intent_router_semantic_suspect.rs"]
mod semantic_suspect;
#[cfg(test)]
use semantic_suspect::{
    semantic_suspect_detail_for_normalizer_output,
    semantic_suspect_detail_for_normalizer_output_with_command_runtime,
};

#[path = "intent_router_prompt_render.rs"]
mod prompt_render;
#[cfg(test)]
use prompt_render::{compact_prompt_slot, render_compact_intent_normalizer_prompt};
use prompt_render::{
    render_intent_normalizer_prompt_for_route, retry_intent_normalizer_json_parse,
};

#[path = "intent_router_structural_schedule.rs"]
mod structural_schedule;
use structural_schedule::{
    apply_schedule_route_contract_repair, normalize_schedule_intent_from_normalizer,
    structural_alias_binding_fallback_decision,
};

#[path = "intent_router_answer_candidate_binding.rs"]
mod answer_candidate_binding;
#[cfg(test)]
use answer_candidate_binding::active_task_invalid_turn_binding_context;

#[path = "intent_router_route_output.rs"]
mod route_output;
pub(crate) use route_output::route_result_from_normalizer;
use route_output::{
    normalizer_output_from_fallback, normalizer_output_from_fallback_with_turn_analysis,
    render_auth_policy_context, render_self_extension_runtime,
};

#[path = "intent_router_normalizer_failure.rs"]
mod normalizer_failure;
use normalizer_failure::{
    normalizer_llm_failed_fallback_output, normalizer_parse_failed_fallback_output,
    normalizer_prompt_missing_fallback_output,
};

#[path = "intent_router_normalizer_answer_repair.rs"]
mod normalizer_answer_repair;
use normalizer_answer_repair::apply_answer_candidate_and_contract_judge_repair;

#[path = "intent_router_normalizer_model.rs"]
mod normalizer_model;
use normalizer_model::{run_intent_normalizer_model_step, NormalizerModelOutcome};

#[path = "intent_router_normalizer_final_gate.rs"]
mod normalizer_final_gate;
use normalizer_final_gate::build_normalizer_output_with_final_gate;

pub(crate) use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputScalarCountTargetKind, OutputSemanticKind, ResumeBehavior, ScheduleKind,
    SelfExtensionMode, SelfExtensionTrigger,
};

const CLARIFY_QUESTION_PROMPT_LOGICAL_PATH: &str = "prompts/clarify_question_prompt.md";
const INTENT_NORMALIZER_PROMPT_LOGICAL_PATH: &str = "prompts/intent_normalizer_prompt.md";
const ROUTING_POLICY_PERSONA_PROMPT: &str = "Neutral routing policy classifier. Ignore style/persona preferences and optimize for correct intent resolution, clarification, and guard decisions.";

#[derive(Debug)]
struct RouteDecision {
    resolved_user_intent: String,
    needs_clarify: bool,
    clarify_question: String,
    reason: String,
    confidence: Option<f64>,
    schedule_kind: ScheduleKind,
    schedule_intent: Option<crate::ScheduleIntentOutput>,
    wants_file_delivery: bool,
    should_refresh_long_term_memory: bool,
    agent_display_name_hint: String,
    output_contract: IntentOutputContract,
}

#[derive(Debug, Deserialize)]
struct IntentNormalizerOut {
    #[serde(default)]
    resolved_user_intent: String,
    #[serde(default)]
    #[allow(dead_code)]
    answer_candidate: String,
    #[serde(default)]
    resume_behavior: String,
    #[serde(default)]
    schedule_kind: String,
    #[serde(default)]
    wants_file_delivery: bool,
    #[serde(default)]
    should_refresh_long_term_memory: bool,
    #[serde(default)]
    agent_display_name_hint: String,
    #[serde(default)]
    needs_clarify: bool,
    #[serde(default)]
    clarify_question: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    decision: String,
    #[serde(default)]
    schedule_intent: Option<crate::ScheduleIntentOutput>,
    #[serde(default)]
    output_contract: Option<IntentOutputContractOut>,
    #[serde(default)]
    execution_recipe: Option<IntentExecutionRecipeOut>,
    #[serde(default)]
    turn_type: String,
    #[serde(default)]
    target_task_policy: String,
    #[serde(default)]
    should_interrupt_active_run: bool,
    #[serde(default)]
    state_patch: Option<Value>,
    #[serde(default)]
    attachment_processing_required: bool,
}

fn append_route_reason(reason: &mut String, addition: &str) {
    let addition = addition.trim();
    if addition.is_empty() || reason.contains(addition) {
        return;
    }
    if reason.trim().is_empty() {
        *reason = addition.to_string();
    } else {
        reason.push_str("; ");
        reason.push_str(addition);
    }
}

#[path = "intent_router_normalizer_run.rs"]
mod normalizer_run;
pub(crate) use normalizer_run::run_intent_normalizer;

#[cfg(test)]
#[path = "intent_router_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "intent_router_contract_repair_judge_tests.rs"]
mod contract_repair_judge_tests;

#[cfg(test)]
#[path = "intent_router_prompt_render_tests.rs"]
mod prompt_render_tests;

#[cfg(test)]
#[path = "intent_router_semantic_suspect_tests.rs"]
mod semantic_suspect_tests;

#[cfg(test)]
#[path = "intent_router_normalizer_schema_basic_tests.rs"]
mod normalizer_schema_basic_tests;

#[cfg(test)]
#[path = "intent_router_execution_recipe_contract_tests.rs"]
mod execution_recipe_contract_tests;

#[cfg(test)]
#[path = "intent_router_current_turn_anchor_tests.rs"]
mod current_turn_anchor_tests;

#[cfg(test)]
#[path = "intent_router_current_turn_structural_repair_tests.rs"]
mod current_turn_structural_repair_tests;

#[cfg(test)]
#[path = "intent_router_normalizer_schema_guard_tests.rs"]
mod normalizer_schema_guard_tests;

#[cfg(test)]
#[path = "intent_router_normalizer_turn_policy_tests.rs"]
mod normalizer_turn_policy_tests;

#[cfg(test)]
#[path = "intent_router_normalizer_schema_tail_tests.rs"]
mod normalizer_schema_tail_tests;

#[cfg(test)]
#[path = "intent_router_structural_contract_repair_tests.rs"]
mod structural_contract_repair_tests;

#[cfg(test)]
#[path = "intent_router_execution_contract_repair_tests.rs"]
mod execution_contract_repair_tests;

#[cfg(test)]
#[path = "intent_router_observation_clarify_tests.rs"]
mod observation_clarify_tests;

#[cfg(test)]
#[path = "intent_router_archive_contract_tests.rs"]
mod archive_contract_tests;

#[cfg(test)]
#[path = "intent_router_parse_failed_fallback_tests.rs"]
mod parse_failed_fallback_tests;

#[cfg(test)]
#[path = "intent_router_active_task_reuse_tests.rs"]
mod active_task_reuse_tests;

#[cfg(test)]
#[path = "intent_router_active_text_followup_tests.rs"]
mod active_text_followup_tests;

#[cfg(test)]
#[path = "intent_router_tests.rs"]
mod tests;

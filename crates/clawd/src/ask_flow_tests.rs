use super::{
    active_file_basename_direct_answer_candidate,
    active_ordered_entries_count_direct_answer_candidate,
    active_task_factual_rewrite_review_needs_repair, active_task_factual_rewrite_review_prompt,
    apply_direct_answer_gate_outcome, ask_reply_with_chat_process,
    chat_prompt_context_with_route_resolution, chat_request_for_prompt, chat_user_request,
    contract_test_hint_should_enter_planner_loop, current_request_mentions_resolvable_gate_locator,
    direct_answer_chat_user_request,
    direct_answer_gate_can_skip_for_active_observed_output_chat_repair,
    direct_answer_gate_can_skip_for_active_task_text_mutation,
    direct_answer_gate_can_skip_for_boundary_clean_chat,
    direct_answer_gate_can_skip_for_pure_chat_draft,
    direct_answer_gate_can_skip_for_recent_count_context,
    direct_answer_gate_can_skip_for_recent_execution_judgment_context,
    direct_answer_gate_can_skip_for_self_contained_payload,
    direct_answer_gate_can_skip_for_standalone_freeform_repair,
    direct_answer_gate_candidate_needs_unbound_context_clarify,
    direct_answer_gate_planner_needs_unbound_locator_clarify,
    direct_answer_gate_promotion_depends_only_on_background_context,
    direct_answer_gate_promotion_needs_unbound_deictic_clarify,
    direct_answer_gate_recent_execution_context, direct_answer_gate_route_context,
    direct_chat_answer_needs_repair, direct_chat_answer_repair_prompt,
    ensure_active_task_required_visible_literals, forbidden_visible_literals_from_state_patch,
    locator_hint_mentions_current_request, normalizer_answer_candidate_from_resolved_prompt,
    normalizer_chat_direct_answer_candidate,
    normalizer_chat_direct_answer_candidate_with_context_summary,
    normalizer_runtime_fact_direct_answer_candidate, output_contract_from_direct_answer_gate,
    preferred_route_clarify_question, promote_active_anchor_observed_judgment_to_chat,
    promote_clarify_config_risk_assessment_default_config_to_planner,
    promote_clarify_recent_execution_judgment_context_to_chat,
    promote_inline_json_transform_context_to_planner, recent_count_comparison_direct_answer,
    replacement_pairs_from_state_patch, required_visible_literals_from_state_patch,
    resolved_intent_declares_structured_scalar_extraction,
    route_allows_agent_loop_pure_chat_submode, route_contract_requests_filename_only_output,
    route_structured_clarify_context, runtime_approval_wait_status_direct_answer_candidate,
    runtime_scalar_path_direct_answer_candidate, session_alias_target_direct_answer_candidate,
    structural_alias_binding_ack, task_payload_text, token_looks_like_pathlike_locator,
    ActiveTaskFactualRewriteReview, DirectAnswerGateContractOut, DirectAnswerGateOut,
    DirectAnswerGateReferenceResolutionOut, DirectAnswerGateSelfExtensionOut,
    DirectAnswerPreflight,
};

fn schema_enum_strings(schema: &serde_json::Value, path: &[&str]) -> Vec<String> {
    let mut node = schema;
    for part in path {
        node = node
            .get(*part)
            .unwrap_or_else(|| panic!("schema path `{}` not found", path.join(".")));
    }
    node.get("enum")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("schema path `{}.enum` not found", path.join(".")))
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

include!("ask_flow_tests/gate_core.rs");
include!("ask_flow_tests/agent_loop_submode.rs");
include!("ask_flow_tests/gate_policy.rs");
include!("ask_flow_tests/chat_context.rs");
include!("ask_flow_tests/active_task_literals.rs");
include!("ask_flow_tests/alias_ack_locale.rs");
include!("ask_flow_tests/runtime_alias_recent.rs");

use serde_json::Value;

use super::{
    active_task_repair::{
        active_context_has_structured_observation_anchor, active_primary_text_context,
    },
    parse_output_locator_kind, parse_target_task_policy, parse_turn_type, scalar_json_value_text,
    OutputLocatorKind,
};

const EXISTING_OBSERVED_CONTEXT_MARKERS: &[&str] = &[
    "content_excerpt_summary",
    "content_presence_check",
    "excerpt_kind_judgment",
    "recent_artifacts_judgment",
    "execution_failed_step",
];

pub(super) fn append_contract_repair_context(context: &mut String, block: String) {
    if block.trim().is_empty() {
        return;
    }
    if context.trim().is_empty() || context == "none" {
        *context = block;
    } else {
        context.push_str("\n\n");
        context.push_str(&block);
    }
}

pub(super) fn active_task_invalid_turn_binding_context(
    raw_normalizer_output: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    should_refresh_long_term_memory: bool,
) -> Option<String> {
    if should_refresh_long_term_memory
        || req_surface.has_explicit_path_or_url()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_structured_target_refinement()
        || req_surface.has_delivery_token_reference()
        || req_surface.inline_json_shape.is_some()
    {
        return None;
    }
    let (prior_prompt, prior_output) = active_primary_text_context(session_snapshot)?;
    prior_output?;
    let raw_value =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw_normalizer_output)?;
    let obj = raw_value.as_object()?;
    if raw_normalizer_output_uses_existing_observed_context_contract(obj)
        && active_context_has_structured_observation_anchor(session_snapshot)
    {
        return None;
    }
    let raw_turn_type = obj
        .get("turn_type")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let raw_target_task_policy = obj
        .get("target_task_policy")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let turn_type_invalid =
        !raw_turn_type.trim().is_empty() && parse_turn_type(&raw_turn_type).is_none();
    let target_policy_invalid = !raw_target_task_policy.trim().is_empty()
        && parse_target_task_policy(&raw_target_task_policy).is_none();
    if !(turn_type_invalid || target_policy_invalid) {
        return None;
    }
    Some(format!(
        "active_task_invalid_turn_binding:\n\
         raw_turn_type: {}\n\
         raw_target_task_policy: {}\n\
         turn_type_invalid: {}\n\
         target_task_policy_invalid: {}\n\
         active_task_prompt: {}\n\
         active_task_has_output: true",
        crate::truncate_for_log(raw_turn_type.trim()),
        crate::truncate_for_log(raw_target_task_policy.trim()),
        turn_type_invalid,
        target_policy_invalid,
        crate::truncate_for_log(prior_prompt)
    ))
}

fn raw_normalizer_output_uses_existing_observed_context_contract(
    obj: &serde_json::Map<String, Value>,
) -> bool {
    let Some(contract) = obj.get("output_contract").and_then(Value::as_object) else {
        return false;
    };
    let route_reason = obj
        .get("reason")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    if !route_reason_has_any_machine_marker(&route_reason, EXISTING_OBSERVED_CONTEXT_MARKERS) {
        return false;
    }
    let requires_content_evidence = contract
        .get("requires_content_evidence")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            contract
                .get("requires_content_evidence")
                .and_then(scalar_json_value_text)
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        });
    let delivery_required = contract
        .get("delivery_required")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            contract
                .get("delivery_required")
                .and_then(scalar_json_value_text)
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        });
    let locator_kind = contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .map(|token| parse_output_locator_kind(&token))
        .unwrap_or(OutputLocatorKind::None);

    requires_content_evidence
        && !delivery_required
        && matches!(
            locator_kind,
            OutputLocatorKind::None
                | OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
}

fn route_reason_has_machine_marker(route_reason: &str, marker: &str) -> bool {
    route_reason.split(';').map(str::trim).any(|part| {
        part == marker
            || part
                .rsplit_once(':')
                .is_some_and(|(_, suffix)| suffix.trim() == marker)
    })
}

fn route_reason_has_any_machine_marker(route_reason: &str, markers: &[&str]) -> bool {
    markers
        .iter()
        .any(|marker| route_reason_has_machine_marker(route_reason, marker))
}

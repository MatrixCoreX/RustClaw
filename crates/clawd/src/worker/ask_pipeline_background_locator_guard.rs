use super::*;

#[cfg(test)]
#[path = "ask_pipeline_background_locator_guard_tests.rs"]
mod tests;

pub(super) fn text_mentions_locator_identity(text: &str, locator: &str) -> bool {
    let normalized_text = text.to_ascii_lowercase();
    locator_identity_candidates(locator)
        .into_iter()
        .any(|identity| identity.chars().count() >= 3 && normalized_text.contains(&identity))
}

pub(super) fn route_has_model_supplied_concrete_locator(
    route_result: &crate::RouteResult,
    resolved_prompt: &str,
) -> bool {
    let contract = &route_result.output_contract;
    let contract_locator = matches!(
        contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::Url
    ) && !contract.locator_hint.trim().is_empty();
    let resolved_prompt_without_answer_candidate =
        strip_embedded_answer_candidate_lines(resolved_prompt);
    let resolved_intent_without_answer_candidate =
        strip_embedded_answer_candidate_lines(&route_result.resolved_intent);
    contract_locator
        || crate::worker::has_explicit_path_or_url_locator_hint(
            &resolved_prompt_without_answer_candidate,
        )
        || crate::worker::has_explicit_path_or_url_locator_hint(
            &resolved_intent_without_answer_candidate,
        )
}

fn strip_embedded_answer_candidate_lines(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("answer_candidate:"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn background_only_locator_route_should_force_clarify(
    state: &AppState,
    prompt: &str,
    resolved_prompt: &str,
    recent_execution_context: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if execute_route_without_input_locator_should_plan(route_result)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || recent_execution_context_has_ordered_entry_target(recent_execution_context, route_result)
        || route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        || generated_file_delivery_uses_runtime_target(route_result)
        || route_can_execute_without_locator(route_result)
        || task_control_route_can_plan_without_locator(route_result)
        || !route_has_model_supplied_concrete_locator(route_result, resolved_prompt)
    {
        return false;
    }
    if !state_patch_requires_deictic_locator_clarify(turn_analysis)
        && current_request_has_structural_locator_surface_for_route(state, prompt, route_result)
    {
        return false;
    }
    if route_has_quantity_comparison_machine_signal(route_result)
        && workspace_directory_pair_from_current_request(state, prompt, false).is_some()
    {
        return false;
    }

    route_result.is_execute_gate()
        || route_result.output_contract.requires_content_evidence
        || route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

fn route_has_quantity_comparison_machine_signal(route_result: &crate::RouteResult) -> bool {
    route_reason_has_marker(route_result, "quantity_comparison")
        || route_reason_has_marker(route_result, "quantity_compare")
}

#[cfg(test)]
pub(super) fn recover_background_locator_clarify_to_agent_loop(
    route_result: &mut crate::RouteResult,
    recent_execution_context: &str,
) -> bool {
    let recent_segments = recent_execution_result_segments(recent_execution_context);
    let existing_observed_synthesis_with_recent_context =
        route_reason_has_marker(route_result, "existing_observed_context_synthesis")
            && !recent_segments.is_empty();
    if !route_result.needs_clarify
        || !route_reason_has_marker(route_result, "background_locator_requires_clarify")
        || route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || !existing_observed_synthesis_with_recent_context
    {
        return false;
    }
    route_result.needs_clarify = false;
    route_result.set_execute_gate();
    route_result.clarify_question.clear();
    let response_shape =
        language_response_shape_or_default(route_result.output_contract.response_shape);
    route_result.output_contract = crate::IntentOutputContract {
        response_shape,
        ..Default::default()
    };
    append_route_reason(route_result, "active_observed_output_loop_recovery");
    append_route_reason(
        route_result,
        "recent_observed_results_background_locator_loop_recovery",
    );
    true
}

#[cfg(test)]
fn language_response_shape_or_default(
    response_shape: crate::OutputResponseShape,
) -> crate::OutputResponseShape {
    match response_shape {
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence => {
            response_shape
        }
        _ => crate::OutputResponseShape::OneSentence,
    }
}

fn recent_execution_context_has_ordered_entry_target(
    recent_execution_context: &str,
    route_result: &crate::RouteResult,
) -> bool {
    let context = recent_execution_context.trim();
    if context.is_empty() || context == "<none>" {
        return false;
    }
    let target_identities =
        locator_identity_candidates(route_result.output_contract.locator_hint.trim());
    if target_identities.is_empty() {
        return false;
    }
    let mut sources = recent_execution_result_segments(context);
    sources.push(context.to_string());
    sources.into_iter().any(|source| {
        crate::followup_frame::extract_ordered_entries_from_text(&source)
            .into_iter()
            .any(|entry| {
                locator_identity_candidates(&entry)
                    .into_iter()
                    .any(|entry_identity| target_identities.contains(&entry_identity))
            })
    })
}

pub(super) fn recent_execution_result_segments(context: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current: Option<String> = None;
    for line in context.lines() {
        if let Some((_, result)) = line.split_once(" result=") {
            if let Some(segment) = current.take().filter(|value| !value.trim().is_empty()) {
                segments.push(segment);
            }
            current = Some(result.trim().to_string());
            continue;
        }
        if line.trim_start().starts_with("- ts=") || line.trim_start().starts_with("### ") {
            if let Some(segment) = current.take().filter(|value| !value.trim().is_empty()) {
                segments.push(segment);
            }
            continue;
        }
        if let Some(segment) = current.as_mut() {
            if !line.trim().is_empty() {
                segment.push('\n');
                segment.push_str(line.trim());
            }
        }
    }
    if let Some(segment) = current.take().filter(|value| !value.trim().is_empty()) {
        segments.push(segment);
    }
    segments
}

pub(super) fn locator_identity_candidates(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    push_locator_identity(&mut out, trimmed);
    if let Some(name) = std::path::Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
    {
        push_locator_identity(&mut out, name);
    }
    out
}

fn push_locator_identity(out: &mut Vec<String>, value: &str) {
    let normalized = normalize_locator_identity_token(value);
    if !normalized.is_empty() && !out.contains(&normalized) {
        out.push(normalized);
    }
}

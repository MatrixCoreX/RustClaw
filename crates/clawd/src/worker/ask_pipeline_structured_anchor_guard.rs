use super::*;

#[cfg(test)]
#[path = "ask_pipeline_structured_anchor_guard_tests.rs"]
mod tests;

fn active_structured_observation_values<'a>(
    session_snapshot: &'a crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<&'a str> {
    let mut values = Vec::new();
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        if matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Read
                | crate::followup_frame::FollowupOpKind::List
        ) {
            values.push(frame.source_request.as_str());
        }
        if let Some(target) = frame.bound_target.as_deref() {
            values.push(target);
        }
        values.extend(frame.ordered_entries.iter().map(String::as_str));
    }
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts.bound_target.as_deref() {
            values.push(target);
        }
        values.extend(facts.ordered_entries.iter().map(String::as_str));
        values.extend(facts.delivery_targets.iter().map(String::as_str));
    }
    values
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect()
}

pub(super) fn active_session_has_structured_observation_anchor(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    !active_structured_observation_values(session_snapshot).is_empty()
        || session_snapshot
            .active_followup_frame
            .as_ref()
            .is_some_and(|frame| frame.selected_entry_index.is_some() || frame.slice_spec.is_some())
        || session_snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(|facts| {
                facts.selected_entry_index.is_some()
                    || facts.observed_entry_count.is_some()
                    || facts.slice_spec.is_some()
            })
}

fn route_output_contract_requires_planner_execution(
    contract: &crate::IntentOutputContract,
) -> bool {
    contract.requires_content_evidence
        || contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
}

fn prompt_surface_has_current_turn_concrete_target(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_explicit_path_or_url()
        || surface.locator_target_pair.is_some()
        || surface.field_selector_count > 0
        || surface.dotted_field_selector.is_some()
        || surface.has_delivery_token_reference()
        || surface.has_deictic_reference()
        || surface.inline_json_shape.is_some()
        || !surface
            .filename_candidates_excluding_field_selectors()
            .is_empty()
}

fn active_text_mutation_can_stay_without_structured_anchor_evidence(
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if route_result.needs_clarify
        || route_result.is_execute_gate()
        || route_result.wants_file_delivery
        || !matches!(route_result.schedule_kind, crate::ScheduleKind::None)
        || route_output_contract_requires_planner_execution(&route_result.output_contract)
    {
        return false;
    }
    let Some(analysis) = turn_analysis else {
        return false;
    };
    if !matches!(
        analysis.turn_type,
        Some(
            crate::intent_router::TurnType::TaskAppend
                | crate::intent_router::TurnType::TaskCorrect
                | crate::intent_router::TurnType::TaskReplace
                | crate::intent_router::TurnType::TaskScopeUpdate
        )
    ) || !matches!(
        analysis.target_task_policy,
        Some(
            crate::intent_router::TargetTaskPolicy::ReuseActive
                | crate::intent_router::TargetTaskPolicy::ReplaceActive
        )
    ) {
        return false;
    }
    if analysis.attachment_processing_required {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    !prompt_surface_has_current_turn_concrete_target(&surface)
}

fn state_patch_can_stay_without_structured_anchor_evidence(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if route_result.needs_clarify
        || route_result.is_execute_gate()
        || route_result.wants_file_delivery
        || !matches!(route_result.schedule_kind, crate::ScheduleKind::None)
        || route_output_contract_requires_planner_execution(&route_result.output_contract)
    {
        return false;
    }
    let Some(analysis) = turn_analysis else {
        return false;
    };
    if analysis.attachment_processing_required || analysis.should_interrupt_active_run {
        return false;
    }
    analysis
        .state_patch
        .as_ref()
        .is_some_and(state_patch_json_is_meaningful)
}

fn state_patch_json_is_meaningful(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Object(map) => map.values().any(state_patch_json_is_meaningful),
        serde_json::Value::Array(items) => items.iter().any(state_patch_json_is_meaningful),
        serde_json::Value::String(text) => !text.trim().is_empty(),
        _ => true,
    }
}

pub(super) fn structured_anchor_route_requires_evidence_repair(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    recent_execution_context: &str,
    has_authoritative_deictic_anchor: bool,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if !has_authoritative_deictic_anchor
        || route_result.needs_clarify
        || route_result.is_execute_gate()
        || route_result.wants_file_delivery
        || !matches!(route_result.schedule_kind, crate::ScheduleKind::None)
        || route_output_contract_requires_planner_execution(&route_result.output_contract)
        || !active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    if active_text_mutation_can_stay_without_structured_anchor_evidence(
        prompt,
        route_result,
        turn_analysis,
    ) || state_patch_can_stay_without_structured_anchor_evidence(route_result, turn_analysis)
    {
        return false;
    }
    if current_request_has_self_contained_structured_payload(prompt) {
        return false;
    }
    if resolved_intent_mentions_active_target_basename(route_result, session_snapshot) {
        return false;
    }
    let _ = recent_execution_context;
    true
}

fn resolved_intent_mentions_active_target_basename(
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.resolved_intent.trim().is_empty() {
        return false;
    }
    active_session_target_basenames(session_snapshot)
        .into_iter()
        .any(|basename| {
            let normalized = normalize_locator_identity_token(&basename);
            normalized.chars().count() >= 3
                && resolved_intent_has_normalized_structural_value(
                    &route_result.resolved_intent,
                    &normalized,
                )
        })
}

fn resolved_intent_has_normalized_structural_value(resolved_intent: &str, value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && resolved_intent
            .lines()
            .any(|line| structural_text_value_matches_normalized(line, value))
}

fn structural_text_value_matches_normalized(text: &str, normalized_value: &str) -> bool {
    let text = text.trim();
    normalize_locator_identity_token(text) == normalized_value
        || text
            .rsplit_once(':')
            .is_some_and(|(_, suffix)| normalize_locator_identity_token(suffix) == normalized_value)
}

fn structural_text_value_matches_exact(text: &str, value: &str) -> bool {
    let text = text.trim();
    text == value
        || text
            .rsplit_once(':')
            .is_some_and(|(_, suffix)| suffix.trim() == value)
}

fn resolved_intent_has_structural_value(resolved_intent: &str, value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && resolved_intent
            .lines()
            .any(|line| structural_text_value_matches_exact(line, value))
}

fn locator_hint_has_structural_value(locator_hint: &str, value: &str) -> bool {
    let value = value.trim();
    let normalized_value = normalize_locator_identity_token(value);
    !value.is_empty()
        && locator_hint
            .split(|ch| matches!(ch, '\n' | ';' | '|'))
            .any(|part| {
                structural_text_value_matches_exact(part, value)
                    || (normalized_value.chars().count() >= 3
                        && structural_path_basename_matches_normalized(part, &normalized_value))
            })
}

fn structural_path_basename_matches_normalized(text: &str, normalized_value: &str) -> bool {
    let text = text.trim();
    path_basename_matches_normalized(text, normalized_value)
        || text
            .rsplit_once(':')
            .is_some_and(|(_, suffix)| path_basename_matches_normalized(suffix, normalized_value))
}

fn path_basename_matches_normalized(text: &str, normalized_value: &str) -> bool {
    std::path::Path::new(text)
        .file_name()
        .and_then(|name| name.to_str())
        .map(normalize_locator_identity_token)
        .is_some_and(|basename| basename == normalized_value)
}

fn active_session_target_basenames(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        push_target_basename(&mut out, frame.bound_target.as_deref());
    }
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        push_target_basename(&mut out, facts.bound_target.as_deref());
        for target in &facts.delivery_targets {
            push_target_basename(&mut out, Some(target));
        }
    }
    out
}

fn push_target_basename(out: &mut Vec<String>, target: Option<&str>) {
    let Some(target) = target.map(str::trim).filter(|target| !target.is_empty()) else {
        return;
    };
    let Some(name) = std::path::Path::new(target)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return;
    };
    if !out.iter().any(|existing| existing == name) {
        out.push(name.to_string());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StructuredAnchorEvidenceMarker {
    RequiresEvidence,
}

impl StructuredAnchorEvidenceMarker {
    fn route_reason(self) -> &'static str {
        match self {
            Self::RequiresEvidence => "structured_anchor_requires_evidence",
        }
    }

    fn record(self, route_result: &mut crate::RouteResult) {
        append_route_reason(route_result, self.route_reason());
    }
}

pub(super) fn apply_structured_anchor_evidence_repair(route_result: &mut crate::RouteResult) {
    route_result.needs_clarify = false;
    route_result.set_planner_execute_finalize(crate::ActFinalizeStyle::ChatWrapped);
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    if matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    ) {
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    }
    if matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        route_result.output_contract.response_shape = crate::OutputResponseShape::Strict;
    }
    StructuredAnchorEvidenceMarker::RequiresEvidence.record(route_result);
}

pub(super) fn session_has_authoritative_deictic_anchor(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if session_snapshot.active_clarify_state.is_some() {
        return true;
    }
    if followup_frame_has_matching_target(
        session_snapshot.active_followup_frame.as_ref(),
        route_result,
    ) || observed_facts_have_matching_target(
        session_snapshot.active_observed_facts.as_ref(),
        route_result,
    ) {
        return true;
    }
    session_snapshot
        .conversation_state
        .as_ref()
        .is_some_and(|state| {
            state.alias_bindings.iter().any(|binding| {
                let alias = binding.alias.trim();
                !alias.is_empty()
                    && crate::conversation_state::alias_surface_matches_prompt(prompt, alias)
            })
        })
}

fn route_context_contains_target(route_result: &crate::RouteResult, target: &str) -> bool {
    let target = target.trim();
    !target.is_empty()
        && (resolved_intent_has_structural_value(&route_result.resolved_intent, target)
            || locator_hint_has_structural_value(
                &route_result.output_contract.locator_hint,
                target,
            ))
}

pub(super) fn followup_frame_has_matching_target(
    frame: Option<&crate::followup_frame::FollowupFrame>,
    route_result: &crate::RouteResult,
) -> bool {
    frame.is_some_and(|frame| {
        frame
            .bound_target
            .as_deref()
            .is_some_and(|target| route_context_contains_target(route_result, target))
            || frame
                .ordered_entries
                .iter()
                .any(|target| route_context_contains_target(route_result, target))
    })
}

pub(super) fn observed_facts_have_matching_target(
    facts: Option<&crate::observed_facts::ObservedFacts>,
    route_result: &crate::RouteResult,
) -> bool {
    facts.is_some_and(|facts| {
        facts
            .bound_target
            .as_deref()
            .is_some_and(|target| route_context_contains_target(route_result, target))
            || facts
                .ordered_entries
                .iter()
                .any(|target| route_context_contains_target(route_result, target))
            || facts
                .delivery_targets
                .iter()
                .any(|target| route_context_contains_target(route_result, target))
    })
}

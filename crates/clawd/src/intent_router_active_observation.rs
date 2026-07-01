use serde_json::Value;

use super::{
    parse_output_contract, parse_output_semantic_kind,
    state_patch_deictic_reference_requires_clarify, ActFinalizeStyle, IntentNormalizerOut,
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ScheduleKind, TargetTaskPolicy, TurnType,
};

const ACTIVE_OBSERVATION_CONTRACT_MARKERS: &[&str] = &[
    "content_excerpt_summary",
    "content_excerpt_with_summary",
    "excerpt_kind_judgment",
    "scalar_path_only",
];

pub(super) fn active_primary_task_prompt<'a>(
    session_snapshot: Option<&'a crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<&'a str> {
    session_snapshot
        .and_then(|snapshot| snapshot.conversation_state.as_ref())
        .and_then(|state| state.last_primary_task_prompt.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(super) fn active_clarify_locator_task_prompt<'a>(
    session_snapshot: Option<&'a crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<&'a str> {
    session_snapshot
        .and_then(|snapshot| snapshot.active_clarify_state.as_ref())
        .filter(|state| {
            matches!(
                state.missing_slot,
                crate::clarify_state::ClarifyMissingSlot::Locator
            )
        })
        .filter(|state| {
            state.delivery_required
                || state.output_shape.is_some()
                || clarify_state_has_known_semantic_marker(state)
                || !state.candidate_targets.is_empty()
        })
        .map(|state| state.source_request.trim())
        .filter(|value| !value.is_empty())
}

pub(super) fn active_observable_task_prompt<'a>(
    session_snapshot: Option<&'a crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<&'a str> {
    active_primary_task_prompt(session_snapshot)
        .or_else(|| active_clarify_locator_task_prompt(session_snapshot))
}

fn clarify_state_has_known_semantic_marker(state: &crate::clarify_state::ClarifyState) -> bool {
    state
        .semantic_kind
        .as_deref()
        .is_some_and(|value| parse_output_semantic_kind(value) != OutputSemanticKind::None)
}

fn route_reason_has_contract_marker(route_reason: &str, marker: &str) -> bool {
    route_reason.split(';').map(str::trim).any(|part| {
        part == marker
            || part
                .rsplit_once(':')
                .is_some_and(|(_, suffix)| suffix.trim() == marker)
    })
}

fn route_reason_has_any_active_observation_contract_marker(route_reason: &str) -> bool {
    ACTIVE_OBSERVATION_CONTRACT_MARKERS
        .iter()
        .any(|marker| route_reason_has_contract_marker(route_reason, marker))
}

fn route_reason_has_repairable_active_observed_output_contract(route_reason: &str) -> bool {
    !route_reason_has_any_active_observation_contract_marker(route_reason)
        || route_reason_has_contract_marker(route_reason, "content_excerpt_summary")
        || route_reason_has_contract_marker(route_reason, "content_excerpt_with_summary")
        || route_reason_has_contract_marker(route_reason, "excerpt_kind_judgment")
}

pub(super) fn prompt_has_concrete_fileish_cue(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_explicit_path_or_url()
        || surface.field_selector_count > 0
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference()
}

pub(super) fn active_task_turn_can_reuse_semantic_patch(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    state_patch: Option<&Value>,
) -> bool {
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return false;
    }
    active_text_followup_surface_is_chat_only(surface)
}

pub(super) fn active_text_followup_surface_is_chat_only(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    !prompt_has_concrete_fileish_cue(surface)
        && !surface.is_structural_locator_only_reply()
        && surface.inline_json_shape.is_none()
}

fn active_prompt_surface_has_structured_execution_target(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_explicit_path_or_url()
        || surface.locator_target_pair.is_some()
        || surface.has_structured_target_refinement()
        || surface.inline_json_shape.is_some()
        || surface.has_delivery_token_reference()
}

fn active_followup_frame_has_structured_target(
    frame: &crate::followup_frame::FollowupFrame,
) -> bool {
    let has_bound_target = frame
        .bound_target
        .as_deref()
        .map(str::trim)
        .is_some_and(|target| !target.is_empty());
    if has_bound_target
        && matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Read
                | crate::followup_frame::FollowupOpKind::List
                | crate::followup_frame::FollowupOpKind::Delivery
                | crate::followup_frame::FollowupOpKind::ClarifyPending
        )
    {
        return true;
    }
    frame.selected_entry_index.is_some() || frame.slice_spec.is_some()
}

fn active_observed_facts_have_structured_target(
    facts: &crate::observed_facts::ObservedFacts,
) -> bool {
    facts
        .bound_target
        .as_deref()
        .map(str::trim)
        .is_some_and(|target| !target.is_empty())
        || !facts.delivery_targets.is_empty()
        || facts.selected_entry_index.is_some()
        || facts.observed_entry_count.is_some()
        || facts.slice_spec.is_some()
}

pub(super) fn active_session_has_structured_execution_target(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let Some(snapshot) = session_snapshot else {
        return false;
    };
    if let Some(active_prompt) = active_primary_task_prompt(Some(snapshot)) {
        let active_surface = crate::intent::surface_signals::analyze_prompt_surface(active_prompt);
        if active_prompt_surface_has_structured_execution_target(&active_surface) {
            return true;
        }
    }
    snapshot
        .active_followup_frame
        .as_ref()
        .is_some_and(active_followup_frame_has_structured_target)
        || snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(active_observed_facts_have_structured_target)
}

pub(super) fn active_session_has_ordered_entries(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let Some(snapshot) = session_snapshot else {
        return false;
    };
    snapshot
        .active_followup_frame
        .as_ref()
        .is_some_and(|frame| !frame.ordered_entries.is_empty())
        || snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(|facts| !facts.ordered_entries.is_empty())
}

fn active_session_has_recent_primary_output(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    session_snapshot
        .and_then(|snapshot| snapshot.conversation_state.as_ref())
        .and_then(|state| state.last_primary_task_output.as_deref())
        .map(str::trim)
        .is_some_and(|output| !output.is_empty())
}

fn contract_locator_matches_active_observation(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    output_contract: &IntentOutputContract,
) -> bool {
    let locator_hint = output_contract.locator_hint.trim();
    if locator_hint.is_empty() {
        return true;
    }
    let Some(snapshot) = session_snapshot else {
        return false;
    };
    let mut values = Vec::new();
    if let Some(frame) = snapshot.active_followup_frame.as_ref() {
        if let Some(target) = frame.bound_target.as_deref() {
            values.push(target.trim());
        }
        values.extend(frame.ordered_entries.iter().map(|value| value.trim()));
    }
    if let Some(facts) = snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts.bound_target.as_deref() {
            values.push(target.trim());
        }
        values.extend(facts.ordered_entries.iter().map(|value| value.trim()));
        values.extend(facts.delivery_targets.iter().map(|value| value.trim()));
    }
    values
        .into_iter()
        .filter(|value| !value.is_empty())
        .any(|value| {
            value == locator_hint
                || std::path::Path::new(value)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == locator_hint)
                || std::path::Path::new(locator_hint)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == value)
        })
}

fn state_patch_has_ordered_entry_ref(state_patch: Option<&Value>) -> bool {
    state_patch.is_some_and(|patch| {
        patch.get("ordered_entry_ref").is_some() || patch.get("ordered_entry_reference").is_some()
    })
}

pub(super) fn active_ordered_scalar_path_missing_state_patch_context(
    out: &IntentNormalizerOut,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<String> {
    if out.needs_clarify || state_patch_has_ordered_entry_ref(out.state_patch.as_ref()) {
        return None;
    }
    let output_contract =
        parse_output_contract(out.output_contract.clone(), out.wants_file_delivery);
    if output_contract.response_shape != OutputResponseShape::Scalar
        || !route_reason_has_contract_marker(&out.reason, "scalar_path_only")
        || output_contract.locator_kind != OutputLocatorKind::None
        || !output_contract.locator_hint.trim().is_empty()
        || output_contract.delivery_required
        || output_contract.delivery_intent != OutputDeliveryIntent::None
    {
        return None;
    }
    let snapshot = session_snapshot?;
    let frame = snapshot.active_followup_frame.as_ref()?;
    if frame.ordered_entries.is_empty() {
        return None;
    }
    let entries = frame
        .ordered_entries
        .iter()
        .take(crate::followup_frame::MAX_ORDERED_ENTRIES)
        .enumerate()
        .map(|(index, entry)| format!("{}:{}", index + 1, entry.trim()))
        .collect::<Vec<_>>()
        .join(" | ");
    let bound_target = frame
        .bound_target
        .as_deref()
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .unwrap_or("<none>");
    let selected_entry_index = frame
        .selected_entry_index
        .map(|index| (index + 1).to_string())
        .unwrap_or_else(|| "<none>".to_string());
    Some(format!(
        "active_ordered_scalar_path_missing_ref: active_bound_target={bound_target}; active_selected_entry_index_base1={selected_entry_index}; active_ordered_entries={entries}; required_patch=state_patch.ordered_entry_ref"
    ))
}

pub(super) fn apply_active_ordered_scalar_path_chat_repair(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    state_patch: Option<&Value>,
    route_reason: &str,
    needs_clarify: bool,
    execution_finalize_style: &mut ActFinalizeStyle,
    output_contract: &mut IntentOutputContract,
) -> Option<&'static str> {
    if needs_clarify
        || !active_session_has_ordered_entries(session_snapshot)
        || state_patch_has_ordered_entry_ref(state_patch)
        || output_contract.response_shape != OutputResponseShape::Scalar
        || !route_reason_has_contract_marker(route_reason, "scalar_path_only")
        || output_contract.locator_kind != OutputLocatorKind::None
        || !output_contract.locator_hint.trim().is_empty()
        || output_contract.delivery_required
        || output_contract.delivery_intent != OutputDeliveryIntent::None
    {
        return None;
    }
    output_contract.response_shape = OutputResponseShape::Strict;
    output_contract.requires_content_evidence = false;
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("active_ordered_scalar_path_chat_repair_without_structured_ref")
}

pub(super) fn apply_active_observed_output_chat_repair(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    should_refresh_long_term_memory: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    wants_file_delivery: bool,
    needs_clarify: bool,
    route_reason: &str,
    execution_finalize_style: &mut ActFinalizeStyle,
    output_contract: &mut IntentOutputContract,
) -> Option<&'static str> {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let current_turn_has_concrete_target = surface.has_concrete_locator_hint()
        || surface.inline_json_shape.is_some()
        || surface.field_selector_count > 0
        || surface.dotted_field_selector.is_some()
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference();
    let conversation_observation_contract = !output_contract.requires_content_evidence
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Scalar
                | OutputResponseShape::Strict
        )
        && !route_reason_has_any_active_observation_contract_marker(route_reason);
    if attachment_processing_required
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || execution_recipe_hint.is_some()
        || wants_file_delivery
        || needs_clarify
        || !matches!(
            turn_type,
            None | Some(TurnType::TaskRequest | TurnType::TaskScopeUpdate)
        )
        || !matches!(
            target_task_policy,
            None | Some(TargetTaskPolicy::ReuseActive)
        )
        || !active_session_has_structured_execution_target(session_snapshot)
        || !active_session_has_recent_primary_output(session_snapshot)
        || current_turn_has_concrete_target
        || !(output_contract.requires_content_evidence || conversation_observation_contract)
        || output_contract.delivery_required
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None
                | OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
        || !contract_locator_matches_active_observation(session_snapshot, output_contract)
        || output_contract.delivery_intent != OutputDeliveryIntent::None
        || !route_reason_has_repairable_active_observed_output_contract(route_reason)
    {
        return None;
    }

    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("active_observed_output_chat_repair")
}

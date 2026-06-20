use serde_json::Value;
use std::path::Path;

use super::{
    active_primary_text_context, parse_output_contract,
    state_patch_deictic_reference_requires_clarify, state_patch_deictic_reference_target,
    ActFinalizeStyle, AppState, FirstLayerDecision, IntentNormalizerOut, IntentOutputContract,
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind, ScheduleKind,
    TargetTaskPolicy, TurnType,
};

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
                || state.semantic_kind.is_some()
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

pub(super) fn chat_only_active_text_missing_locator_followup(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    matches!(legacy_normalizer_decision, FirstLayerDecision::Clarify)
        && active_primary_text_context(session_snapshot).is_some()
        && !active_session_has_structured_execution_target(session_snapshot)
        && state_patch_deictic_reference_target(state_patch) == Some("missing_locator")
        && matches!(
            turn_type,
            Some(
                TurnType::TaskAppend
                    | TurnType::TaskCorrect
                    | TurnType::TaskReplace
                    | TurnType::TaskScopeUpdate
            )
        )
        && matches!(
            target_task_policy,
            Some(TargetTaskPolicy::ReuseActive | TargetTaskPolicy::ReplaceActive)
        )
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
        && active_text_followup_surface_is_chat_only(surface)
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

fn single_component_answer_candidate(candidate: &str) -> Option<&str> {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.contains('\n')
        || candidate.contains('/')
        || candidate.contains('\\')
        || Path::new(candidate).components().count() != 1
    {
        return None;
    }
    Some(candidate)
}

fn existing_file_basename_for_session_target(state: &AppState, target: &str) -> Option<String> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let path = Path::new(target);
    let is_existing_file = path.is_file()
        || path
            .canonicalize()
            .ok()
            .is_some_and(|canonical| canonical.is_file());
    let is_workspace_file = !is_existing_file && state.skill_rt.workspace_root.join(path).is_file();
    if !is_existing_file && !is_workspace_file {
        return None;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
}

fn active_session_bound_file_basename_candidate(
    state: &AppState,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    answer_candidate: &str,
) -> Option<String> {
    let candidate = single_component_answer_candidate(answer_candidate)?;
    let snapshot = session_snapshot?;
    let mut targets = Vec::new();
    if let Some(frame) = snapshot.active_followup_frame.as_ref() {
        if matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Delivery
                | crate::followup_frame::FollowupOpKind::Read
        ) {
            if let Some(target) = frame.bound_target.as_deref() {
                targets.push(target);
            }
        }
    }
    if let Some(facts) = snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts.bound_target.as_deref() {
            targets.push(target);
        }
        targets.extend(facts.delivery_targets.iter().map(String::as_str));
    }
    targets.into_iter().find_map(|target| {
        let basename = existing_file_basename_for_session_target(state, target)?;
        basename.eq_ignore_ascii_case(candidate).then_some(basename)
    })
}

fn existing_path_for_answer_candidate(state: &AppState, answer_candidate: &str) -> Option<String> {
    let candidate = answer_candidate.trim();
    if candidate.is_empty()
        || candidate.contains('\n')
        || candidate.starts_with('{')
        || candidate.starts_with('[')
    {
        return None;
    }
    let path = Path::new(candidate);
    if path.is_absolute() && path.exists() {
        return Some(path.display().to_string());
    }
    let workspace_path = state.skill_rt.workspace_root.join(path);
    workspace_path
        .exists()
        .then(|| workspace_path.display().to_string())
}

fn paths_match_existing_location(left: &str, right: &str) -> bool {
    let left = Path::new(left.trim());
    let right = Path::new(right.trim());
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn active_session_bound_targets(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> Vec<&str> {
    let Some(snapshot) = session_snapshot else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    if let Some(frame) = snapshot.active_followup_frame.as_ref() {
        if let Some(target) = frame.bound_target.as_deref() {
            targets.push(target);
        }
        targets.extend(frame.ordered_entries.iter().map(String::as_str));
    }
    if let Some(facts) = snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts.bound_target.as_deref() {
            targets.push(target);
        }
        targets.extend(facts.ordered_entries.iter().map(String::as_str));
        targets.extend(facts.delivery_targets.iter().map(String::as_str));
    }
    targets
}

fn active_session_bound_path_answer_candidate(
    state: &AppState,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    answer_candidate: &str,
) -> Option<String> {
    let candidate_path = existing_path_for_answer_candidate(state, answer_candidate)?;
    active_session_bound_targets(session_snapshot)
        .into_iter()
        .filter(|target| !target.trim().is_empty())
        .any(|target| paths_match_existing_location(&candidate_path, target))
        .then_some(candidate_path)
}

pub(super) fn apply_active_file_basename_answer_candidate_direct_repair(
    state: &AppState,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    answer_candidate: &str,
    needs_clarify: bool,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    wants_file_delivery: &mut bool,
    output_contract: &mut IntentOutputContract,
) -> Option<&'static str> {
    if needs_clarify || !matches!(legacy_normalizer_decision, FirstLayerDecision::DirectAnswer) {
        return None;
    }
    active_session_bound_file_basename_candidate(state, session_snapshot, answer_candidate)?;
    *legacy_normalizer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    *wants_file_delivery = false;
    output_contract.response_shape = OutputResponseShape::Scalar;
    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::FileBasename;
    output_contract.locator_hint.clear();
    Some("active_file_basename_answer_candidate_direct")
}

pub(super) fn apply_active_bound_path_answer_candidate_direct_repair(
    state: &AppState,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    answer_candidate: &str,
    needs_clarify: bool,
    schedule_kind: ScheduleKind,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    wants_file_delivery: &mut bool,
    output_contract: &mut IntentOutputContract,
) -> Option<&'static str> {
    if needs_clarify
        || !matches!(legacy_normalizer_decision, FirstLayerDecision::DirectAnswer)
        || !matches!(schedule_kind, ScheduleKind::None)
        || *wants_file_delivery
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
    {
        return None;
    }
    active_session_bound_path_answer_candidate(state, session_snapshot, answer_candidate)?;
    *legacy_normalizer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    *wants_file_delivery = false;
    output_contract.response_shape = OutputResponseShape::Scalar;
    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_hint.clear();
    Some("active_bound_path_answer_candidate_direct")
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
        || output_contract.semantic_kind != OutputSemanticKind::ScalarPathOnly
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
    answer_candidate: &str,
    needs_clarify: bool,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    output_contract: &mut IntentOutputContract,
) -> Option<&'static str> {
    if needs_clarify
        || !matches!(
            legacy_normalizer_decision,
            FirstLayerDecision::DirectAnswer | FirstLayerDecision::PlannerExecute
        )
        || !answer_candidate.trim().is_empty()
        || !active_session_has_ordered_entries(session_snapshot)
        || state_patch_has_ordered_entry_ref(state_patch)
        || output_contract.response_shape != OutputResponseShape::Scalar
        || output_contract.semantic_kind != OutputSemanticKind::ScalarPathOnly
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
    *legacy_normalizer_decision = FirstLayerDecision::DirectAnswer;
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
    answer_candidate: &str,
    needs_clarify: bool,
    legacy_normalizer_decision: &mut FirstLayerDecision,
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
        && output_contract.semantic_kind == OutputSemanticKind::None;
    if attachment_processing_required
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || execution_recipe_hint.is_some()
        || wants_file_delivery
        || needs_clarify
        || !matches!(
            legacy_normalizer_decision,
            FirstLayerDecision::DirectAnswer | FirstLayerDecision::PlannerExecute
        )
        || !matches!(
            turn_type,
            None | Some(TurnType::TaskRequest | TurnType::TaskScopeUpdate)
        )
        || !matches!(
            target_task_policy,
            None | Some(TargetTaskPolicy::ReuseActive)
        )
        || !answer_candidate.trim().is_empty()
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
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::None
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::ContentExcerptWithSummary
                | OutputSemanticKind::ExcerptKindJudgment
        )
    {
        return None;
    }

    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    *legacy_normalizer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("active_observed_output_chat_repair")
}

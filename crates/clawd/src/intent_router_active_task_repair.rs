use serde_json::Value;
use std::collections::BTreeSet;

use super::{
    active_primary_task_prompt, active_session_has_structured_execution_target,
    active_task_turn_can_reuse_semantic_patch, active_text_followup_surface_is_chat_only,
    is_meaningful_state_patch, output_semantic_kind_requires_fresh_evidence,
    semantic_kind_can_use_existing_observed_context, state_patch_deictic_reference_is_resolved,
    state_patch_deictic_reference_requires_clarify, ActFinalizeStyle, AnswerCandidateBindingReport,
    FirstLayerDecision, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, ScheduleKind, TargetTaskPolicy, TurnType,
};

pub(super) fn unresolved_deictic_observable_target_should_clarify(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return true;
    }
    if state_patch_deictic_reference_is_resolved(state_patch) {
        return false;
    }
    if surface.has_deictic_reference()
        && output_contract.requires_content_evidence
        && !surface.has_explicit_path_or_url()
    {
        return true;
    }
    false
}

pub(super) fn should_resolve_task_scope_update_clarify_with_active_task(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if attachment_processing_required
        || !matches!(legacy_normalizer_decision, FirstLayerDecision::Clarify)
        || active_primary_task_prompt(session_snapshot).is_none()
        || !matches!(turn_type, Some(TurnType::TaskScopeUpdate))
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch) {
        return false;
    }
    active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
}

pub(super) fn should_resolve_task_append_clarify_with_active_task(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if attachment_processing_required
        || !matches!(legacy_normalizer_decision, FirstLayerDecision::Clarify)
        || active_primary_task_prompt(session_snapshot).is_none()
        || !matches!(
            turn_type,
            Some(TurnType::TaskAppend | TurnType::TaskCorrect | TurnType::TaskScopeUpdate)
        )
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch) {
        return false;
    }
    active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
}

pub(super) fn should_resolve_task_replace_clarify_with_active_task(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if attachment_processing_required
        || !matches!(legacy_normalizer_decision, FirstLayerDecision::Clarify)
        || active_primary_task_prompt(session_snapshot).is_none()
        || !matches!(turn_type, Some(TurnType::TaskReplace))
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReplaceActive))
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch) {
        return false;
    }
    active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
}

pub(super) fn should_route_active_task_mutation_to_direct_answer(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if attachment_processing_required
        || !matches!(
            legacy_normalizer_decision,
            FirstLayerDecision::PlannerExecute
        )
        || active_primary_task_prompt(session_snapshot).is_none()
        || !output_contract_allows_chat_only_task_mutation(output_contract)
    {
        return false;
    }
    if output_contract.requires_content_evidence
        && !matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && active_primary_text_context(session_snapshot)
            .and_then(|(_, output)| output)
            .is_some()
    {
        return false;
    }
    let turn_type = match turn_type {
        Some(value) => value,
        None => return false,
    };
    let target_task_policy = match target_task_policy {
        Some(value) => value,
        None => return false,
    };
    if !matches!(
        turn_type,
        TurnType::TaskAppend
            | TurnType::TaskCorrect
            | TurnType::TaskReplace
            | TurnType::TaskScopeUpdate
    ) {
        return false;
    }
    if !matches!(
        target_task_policy,
        TargetTaskPolicy::ReuseActive | TargetTaskPolicy::ReplaceActive
    ) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
}

pub(super) fn apply_missing_active_task_reuse_clarify(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    answer_candidate: Option<&str>,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    output_contract: &mut IntentOutputContract,
) -> Option<&'static str> {
    if !matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        || active_primary_task_prompt(session_snapshot).is_some()
    {
        return None;
    }
    if missing_active_reuse_has_standalone_execution_contract(
        turn_type,
        *legacy_normalizer_decision,
        output_contract,
    ) {
        return None;
    }
    if missing_active_reuse_has_standalone_direct_answer_candidate(
        *legacy_normalizer_decision,
        output_contract,
        answer_candidate,
    ) {
        return None;
    }
    if missing_active_text_followup_can_continue_as_chat(
        prompt,
        turn_type,
        target_task_policy,
        *legacy_normalizer_decision,
        output_contract,
        state_patch,
    ) {
        *needs_clarify = false;
        clarify_question.clear();
        *legacy_normalizer_decision = FirstLayerDecision::DirectAnswer;
        *execution_finalize_style = ActFinalizeStyle::Plain;
        clear_output_contract_for_active_text_followup(output_contract);
        return Some("missing_active_task_reuse_continues_as_chat");
    }
    *needs_clarify = true;
    clarify_question.clear();
    *legacy_normalizer_decision = FirstLayerDecision::Clarify;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    clear_output_contract_for_active_text_followup(output_contract);
    Some("missing_active_task_reuse_requires_clarify")
}

fn missing_active_text_followup_can_continue_as_chat(
    prompt: &str,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if !matches!(legacy_normalizer_decision, FirstLayerDecision::Clarify)
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        || !matches!(
            turn_type,
            Some(TurnType::TaskAppend | TurnType::TaskCorrect | TurnType::TaskScopeUpdate)
        )
        || !output_contract_looks_like_contextual_text_followup(output_contract)
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch) {
        return false;
    }
    active_text_followup_surface_is_chat_only(&surface)
}

fn missing_active_reuse_has_standalone_execution_contract(
    turn_type: Option<TurnType>,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
) -> bool {
    if !matches!(
        turn_type,
        Some(TurnType::TaskRequest | TurnType::StatusQuery)
    ) || !matches!(
        legacy_normalizer_decision,
        FirstLayerDecision::PlannerExecute
    ) {
        return false;
    }
    let requires_observation = output_contract.requires_content_evidence
        || output_semantic_kind_requires_fresh_evidence(output_contract.semantic_kind);
    if !requires_observation {
        return false;
    }
    if output_contract.delivery_required
        && !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename | OutputLocatorKind::Url
        )
        && output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    true
}

fn missing_active_reuse_has_standalone_direct_answer_candidate(
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    answer_candidate: Option<&str>,
) -> bool {
    if !matches!(legacy_normalizer_decision, FirstLayerDecision::DirectAnswer) {
        return false;
    }
    if answer_candidate
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .is_none()
    {
        return false;
    }
    !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && !output_semantic_kind_requires_fresh_evidence(output_contract.semantic_kind)
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
}

fn state_patch_has_semantic_update(state_patch: Option<&Value>) -> bool {
    let Some(Value::Object(map)) = state_patch else {
        return false;
    };
    !map.is_empty() && map.values().any(is_meaningful_state_patch)
}

fn state_patch_selects_ordered_entry_or_execution_scope(state_patch: Option<&Value>) -> bool {
    let Some(Value::Object(map)) = state_patch else {
        return false;
    };
    if map.contains_key("ordered_entry_ref") || map.contains_key("ordered_entry_reference") {
        return true;
    }
    map.get("active_task_scope")
        .and_then(Value::as_object)
        .and_then(|scope| scope.get("operation"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|operation| !operation.is_empty())
}

fn normalized_replacement_literal_from_map(
    map: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    let literal = keys
        .iter()
        .find_map(|key| map.get(*key))
        .and_then(Value::as_str)?
        .trim()
        .trim_matches(|c: char| c.is_ascii_whitespace() || matches!(c, '"' | '\'' | '`'))
        .trim();
    if literal.is_empty()
        || literal.contains('\n')
        || literal.chars().count() > 80
        || state_patch_replacement_placeholder_literal(literal)
    {
        return None;
    }
    Some(literal.to_string())
}

fn state_patch_replacement_placeholder_literal(value: &str) -> bool {
    let Some(inner) = value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    inner.starts_with("pending_")
        && inner
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

fn collect_state_patch_replacements_from_values(
    value: Option<&Value>,
    from_out: &mut BTreeSet<String>,
    to_out: &mut BTreeSet<String>,
) {
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                collect_state_patch_replacements_from_values(Some(item), from_out, to_out);
            }
        }
        Some(Value::Object(map)) => {
            let from = normalized_replacement_literal_from_map(map, &["from", "old", "source"]);
            let to =
                normalized_replacement_literal_from_map(map, &["to", "new", "target", "value"]);
            if let (Some(from), Some(to)) = (from, to) {
                if from != to {
                    from_out.insert(from);
                    to_out.insert(to);
                }
            }
        }
        _ => {}
    }
}

fn state_patch_replacement_literals(
    map: &serde_json::Map<String, Value>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut from_literals = BTreeSet::new();
    let mut to_literals = BTreeSet::new();
    for key in [
        "replacement_pairs",
        "active_task_replacement_pairs",
        "replacements",
        "active_task_replacements",
        "replace",
    ] {
        collect_state_patch_replacements_from_values(
            map.get(key),
            &mut from_literals,
            &mut to_literals,
        );
    }
    if let Some(constraints) = map.get("visible_constraints").and_then(Value::as_object) {
        for key in ["replacement_pairs", "replacements", "replace"] {
            collect_state_patch_replacements_from_values(
                constraints.get(key),
                &mut from_literals,
                &mut to_literals,
            );
        }
    }
    (from_literals, to_literals)
}

fn append_required_content_literals(
    map: &mut serde_json::Map<String, Value>,
    required: &BTreeSet<String>,
) -> bool {
    if required.is_empty() {
        return false;
    }
    let entry = map
        .entry("required_content_literals".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    let items = entry.as_array_mut().expect("array after repair");
    let mut existing = items
        .iter()
        .filter_map(Value::as_str)
        .map(|value| value.trim().to_string())
        .collect::<BTreeSet<_>>();
    let mut changed = false;
    for value in required {
        if existing.insert(value.clone()) {
            items.push(Value::String(value.clone()));
            changed = true;
        }
    }
    changed
}

fn append_forbidden_visible_literals(
    map: &mut serde_json::Map<String, Value>,
    forbidden: &BTreeSet<String>,
) -> bool {
    if forbidden.is_empty() {
        return false;
    }
    let entry = map
        .entry("forbidden_visible_literals".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    let items = entry.as_array_mut().expect("array after repair");
    let mut existing = items
        .iter()
        .filter_map(Value::as_str)
        .map(|value| value.trim().to_string())
        .collect::<BTreeSet<_>>();
    let mut changed = false;
    for value in forbidden {
        if existing.insert(value.clone()) {
            items.push(Value::String(value.clone()));
            changed = true;
        }
    }
    changed
}

fn remove_required_literals_that_match_replacements(
    value: Option<&mut Value>,
    replacement_from_literals: &BTreeSet<String>,
    removed: &mut BTreeSet<String>,
) -> bool {
    let Some(Value::Array(items)) = value else {
        return false;
    };
    let before = items.len();
    items.retain(|item| {
        let Some(text) = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return true;
        };
        if replacement_from_literals.contains(text) {
            removed.insert(text.to_string());
            false
        } else {
            true
        }
    });
    before != items.len()
}

pub(super) fn repair_state_patch_replacement_literal_conflicts(
    state_patch: &mut Option<Value>,
) -> Option<&'static str> {
    let Some(Value::Object(map)) = state_patch.as_mut() else {
        return None;
    };
    let (replacement_from_literals, replacement_to_literals) =
        state_patch_replacement_literals(map);
    if replacement_from_literals.is_empty() && replacement_to_literals.is_empty() {
        return None;
    }

    let mut removed = BTreeSet::new();
    let mut changed = false;
    for key in [
        "required_content_literals",
        "active_task_required_content_literals",
    ] {
        changed |= remove_required_literals_that_match_replacements(
            map.get_mut(key),
            &replacement_from_literals,
            &mut removed,
        );
    }
    if let Some(constraints) = map
        .get_mut("visible_constraints")
        .and_then(Value::as_object_mut)
    {
        for key in [
            "required_content_literals",
            "active_task_required_content_literals",
        ] {
            changed |= remove_required_literals_that_match_replacements(
                constraints.get_mut(key),
                &replacement_from_literals,
                &mut removed,
            );
        }
    }
    changed |= append_required_content_literals(map, &replacement_to_literals);
    changed |= append_forbidden_visible_literals(map, &replacement_from_literals);
    if !changed {
        return None;
    }

    Some("state_patch_replacement_literal_conflict_repair")
}

fn prompt_has_concrete_locator_for_patch_repair(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_explicit_path_or_url()
        || surface.locator_target_pair.is_some()
        || surface.field_selector_count > 0
        || surface.dotted_field_selector.is_some()
        || surface.has_delivery_token_reference()
        || !surface
            .filename_candidates_excluding_field_selectors()
            .is_empty()
}

pub(super) fn active_primary_text_context(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<(&str, Option<&str>)> {
    let state = session_snapshot.and_then(|snapshot| snapshot.conversation_state.as_ref())?;
    let prompt = state
        .last_primary_task_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let output = state
        .last_primary_task_output
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !crate::finalize::is_execution_summary_message(value));
    Some((prompt, output))
}

fn active_text_patch_locator_context_is_safe(
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    output_contract: &IntentOutputContract,
) -> bool {
    if output_contract.locator_hint.trim().is_empty() {
        return true;
    }
    matches!(
        (turn_type, target_task_policy),
        (
            Some(
                TurnType::TaskAppend
                    | TurnType::TaskCorrect
                    | TurnType::TaskReplace
                    | TurnType::TaskScopeUpdate
            ),
            Some(TargetTaskPolicy::ReuseActive | TargetTaskPolicy::ReplaceActive)
        )
    )
}

pub(super) fn apply_active_task_structured_patch_repair(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: &mut Option<TurnType>,
    target_task_policy: &mut Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    needs_clarify: &mut bool,
    schedule_kind: ScheduleKind,
    should_refresh_long_term_memory: bool,
    output_contract: &mut IntentOutputContract,
    state_patch: Option<&Value>,
) -> Option<&'static str> {
    active_primary_text_context(session_snapshot)?;
    if attachment_processing_required
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || crate::conversation_state::state_patch_is_alias_bindings_only(state_patch?)
        || state_patch_selects_ordered_entry_or_execution_scope(state_patch)
        || !state_patch_has_semantic_update(state_patch)
        || !matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        || !active_text_patch_locator_context_is_safe(
            *turn_type,
            *target_task_policy,
            output_contract,
        )
    {
        return None;
    }

    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if prompt_has_concrete_locator_for_patch_repair(&surface)
        || unresolved_deictic_observable_target_should_clarify(
            &surface,
            output_contract,
            state_patch,
        )
        || !active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
    {
        return None;
    }

    if !matches!(
        turn_type,
        None | Some(TurnType::TaskRequest | TurnType::TaskCorrect | TurnType::TaskAppend)
    ) || !matches!(
        target_task_policy,
        None | Some(TargetTaskPolicy::Standalone | TargetTaskPolicy::ReuseActive)
    ) {
        return None;
    }

    *turn_type = Some(TurnType::TaskCorrect);
    *target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    *needs_clarify = false;
    *legacy_normalizer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_hint.clear();
    Some("active_task_structured_patch_repair")
}

pub(super) fn apply_active_task_scope_refinement_repair(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: &mut Option<TurnType>,
    target_task_policy: &mut Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    needs_clarify: &mut bool,
    schedule_kind: ScheduleKind,
    should_refresh_long_term_memory: bool,
    output_contract: &mut IntentOutputContract,
    state_patch: Option<&Value>,
    current_request_has_resolved_workspace_child_locator: bool,
) -> Option<&'static str> {
    if attachment_processing_required
        || active_primary_task_prompt(session_snapshot).is_none()
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || !matches!(turn_type, None | Some(TurnType::TaskRequest))
        || !matches!(
            target_task_policy,
            None | Some(TargetTaskPolicy::Standalone)
        )
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
    {
        return None;
    }

    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch) {
        return None;
    }
    if active_task_explicit_locator_clarify_should_preserve_binding(output_contract, *needs_clarify)
    {
        *turn_type = Some(TurnType::TaskCorrect);
        return None;
    }
    if active_task_scope_refinement_should_preserve_fresh_execution_contract(output_contract) {
        return None;
    }
    if current_request_has_resolved_workspace_child_locator
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
    {
        return None;
    }
    if !active_task_turn_can_reuse_semantic_patch(&surface, state_patch) {
        return None;
    }

    let unresolved_observation_missing_locator = *needs_clarify
        && output_contract.requires_content_evidence
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None);
    if unresolved_observation_missing_locator {
        return None;
    }

    let standalone_observation_without_missing_slot = !*needs_clarify
        && output_contract.requires_content_evidence
        && !output_contract.delivery_required;
    if standalone_observation_without_missing_slot {
        return None;
    }

    let model_lifted_prompt_into_execution_target = matches!(
        legacy_normalizer_decision,
        FirstLayerDecision::Clarify | FirstLayerDecision::PlannerExecute
    ) && (output_contract
        .requires_content_evidence
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        || !matches!(output_contract.semantic_kind, OutputSemanticKind::None));

    if !*needs_clarify && !model_lifted_prompt_into_execution_target {
        return None;
    }

    let repair_reason = if active_session_has_structured_execution_target(session_snapshot) {
        *turn_type = None;
        *target_task_policy = None;
        "active_task_scope_refinement_detached_from_structured_anchor"
    } else {
        *turn_type = Some(TurnType::TaskScopeUpdate);
        *target_task_policy = Some(TargetTaskPolicy::ReuseActive);
        "active_task_scope_refinement_repair"
    };
    *needs_clarify = false;
    *legacy_normalizer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_hint.clear();
    Some(repair_reason)
}

fn active_task_scope_refinement_should_preserve_fresh_execution_contract(
    output_contract: &IntentOutputContract,
) -> bool {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
    {
        return false;
    }
    let has_concrete_observable_locator = matches!(
        output_contract.locator_kind,
        OutputLocatorKind::Path | OutputLocatorKind::Filename | OutputLocatorKind::Url
    ) && !output_contract.locator_hint.trim().is_empty();
    has_concrete_observable_locator
        && (output_contract.requires_content_evidence
            || output_semantic_kind_requires_fresh_evidence(output_contract.semantic_kind))
}

fn active_task_explicit_locator_clarify_should_preserve_binding(
    output_contract: &IntentOutputContract,
    needs_clarify: bool,
) -> bool {
    needs_clarify
        && output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        )
        && !output_contract.locator_hint.trim().is_empty()
}

fn output_contract_allows_chat_only_task_mutation(output_contract: &IntentOutputContract) -> bool {
    let chat_only_contract = !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None);
    let active_scope_text_contract = output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
        && matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::WorkspaceProjectSummary
                | OutputSemanticKind::DirectoryPurposeSummary
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::ContentExcerptWithSummary
                | OutputSemanticKind::ExcerptKindJudgment
                | OutputSemanticKind::None
        );
    chat_only_contract || active_scope_text_contract
}

pub(super) fn clear_output_contract_for_active_text_followup(
    output_contract: &mut IntentOutputContract,
) {
    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_hint.clear();
}

fn output_contract_looks_like_contextual_text_followup(
    output_contract: &IntentOutputContract,
) -> bool {
    let contextual_semantic = matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        || semantic_kind_can_use_existing_observed_context(output_contract.semantic_kind);
    !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        && contextual_semantic
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
}

pub(super) fn active_context_has_structured_observation_anchor(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let Some(snapshot) = session_snapshot else {
        return false;
    };

    let followup_anchor = snapshot
        .active_followup_frame
        .as_ref()
        .is_some_and(|frame| {
            matches!(
                frame.op_kind,
                crate::followup_frame::FollowupOpKind::Read
                    | crate::followup_frame::FollowupOpKind::List
            ) || frame
                .bound_target
                .as_deref()
                .is_some_and(|target| !target.trim().is_empty())
                || !frame.ordered_entries.is_empty()
                || frame.selected_entry_index.is_some()
                || frame.slice_spec.is_some()
        });
    if followup_anchor {
        return true;
    }

    snapshot
        .active_observed_facts
        .as_ref()
        .is_some_and(|facts| {
            facts
                .bound_target
                .as_deref()
                .is_some_and(|target| !target.trim().is_empty())
                || !facts.ordered_entries.is_empty()
                || facts.selected_entry_index.is_some()
                || facts.observed_entry_count.is_some()
                || facts.slice_spec.is_some()
        })
}

pub(super) fn answer_candidate_can_conflict_with_active_text_followup(
    binding: Option<&AnswerCandidateBindingReport>,
) -> bool {
    binding.is_some_and(|binding| {
        binding.is_distinctive()
            && !binding.in_current_request
            && !binding.in_recent_execution_context
            && (binding.in_recent_assistant_replies
                || binding.in_recent_turns_full
                || binding.in_last_turn_full
                || binding.in_memory_context)
    })
}

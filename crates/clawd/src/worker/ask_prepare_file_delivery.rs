use serde_json::Value;
use std::path::Path;

const UNRESOLVED_FILE_DELIVERY_REQUIRES_LOCATOR_MARKER: &str =
    "unresolved_file_delivery_requires_locator";
const ACTIVE_DELIVERY_CONTENT_TARGET_MARKER: &str = "active_delivery_content_target_bound";
const ACTIVE_DELIVERY_CONTENT_TARGET_KEY: &str = "active_delivery_content_target";

pub(super) fn route_requests_file_delivery(route_result: &crate::RouteResult) -> bool {
    route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

pub(super) fn route_reason_has_structural_marker(
    route_result: &crate::RouteResult,
    marker: &str,
) -> bool {
    route_result
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| {
            part == marker
                || part
                    .rsplit_once(':')
                    .is_some_and(|(_, suffix)| suffix.trim() == marker)
        })
}

fn remove_route_reason_structural_marker(route_result: &mut crate::RouteResult, marker: &str) {
    route_result.route_reason = route_result
        .route_reason
        .split(';')
        .map(str::trim)
        .filter(|part| {
            !part.is_empty()
                && *part != marker
                && !part
                    .rsplit_once(':')
                    .is_some_and(|(_, suffix)| suffix.trim() == marker)
        })
        .collect::<Vec<_>>()
        .join("; ");
}

fn replace_route_reason_structural_value(
    route_reason: &str,
    old_value: &str,
    new_value: &str,
) -> String {
    let old_value = old_value.trim();
    if old_value.is_empty() {
        return route_reason.to_string();
    }
    route_reason
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part == old_value {
                return new_value.to_string();
            }
            if let Some((prefix, suffix)) = part.rsplit_once(':') {
                if suffix.trim() == old_value {
                    return format!("{}: {}", prefix.trim_end(), new_value);
                }
            }
            part.to_string()
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn append_route_reason_structural_marker(route_result: &mut crate::RouteResult, marker: &str) {
    if route_result.route_reason.trim().is_empty() {
        route_result.route_reason = marker.to_string();
    } else if !route_reason_has_structural_marker(route_result, marker) {
        route_result.route_reason.push_str("; ");
        route_result.route_reason.push_str(marker);
    }
}

fn resolved_intent_has_structural_value(resolved_intent: &str, value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && resolved_intent.lines().map(str::trim).any(|line| {
            line == value
                || line
                    .rsplit_once(':')
                    .is_some_and(|(_, suffix)| suffix.trim() == value)
        })
}

fn replace_resolved_intent_structural_value(
    resolved_intent: &str,
    old_value: &str,
    new_value: &str,
) -> String {
    let old_value = old_value.trim();
    if old_value.is_empty() {
        return resolved_intent.to_string();
    }
    resolved_intent
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed == old_value {
                return new_value.to_string();
            }
            if let Some((prefix, suffix)) = trimmed.rsplit_once(':') {
                if suffix.trim() == old_value {
                    return format!("{}: {}", prefix.trim_end(), new_value);
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn active_delivery_target_line(target: &str) -> String {
    let mut line =
        String::with_capacity(ACTIVE_DELIVERY_CONTENT_TARGET_KEY.len() + ": ".len() + target.len());
    line.push_str(ACTIVE_DELIVERY_CONTENT_TARGET_KEY);
    line.push_str(": ");
    line.push_str(target);
    line
}

fn append_active_delivery_resolved_target(route_result: &mut crate::RouteResult, target: &str) {
    if route_result.resolved_intent.trim().is_empty() {
        route_result.resolved_intent = active_delivery_target_line(target);
    } else if !resolved_intent_has_structural_value(&route_result.resolved_intent, target) {
        route_result.resolved_intent.push('\n');
        route_result
            .resolved_intent
            .push_str(&active_delivery_target_line(target));
    }
}

fn file_delivery_has_concrete_locator(route_result: &crate::RouteResult) -> bool {
    !route_result.output_contract.locator_hint.trim().is_empty()
}

fn file_delivery_locator_hint_points_to_existing_directory(
    route_result: &crate::RouteResult,
) -> bool {
    if !route_requests_file_delivery(route_result)
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
    {
        return false;
    }
    if !matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
    ) {
        return false;
    }
    let hint = route_result.output_contract.locator_hint.trim();
    !hint.is_empty() && Path::new(hint).is_dir()
}

fn route_has_structured_list_selector(route_result: &crate::RouteResult) -> bool {
    let selector = &route_result.output_contract.self_extension.list_selector;
    selector.target_kind_specified
        || selector.limit.is_some()
        || selector
            .sort_by
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || selector.include_metadata.is_some()
        || selector.include_hidden.is_some()
        || route_reason_has_structural_marker(
            route_result,
            "normalizer_emitted_directory_file_selector",
        )
}

fn generated_file_delivery_can_choose_target(route_result: &crate::RouteResult) -> bool {
    if !route_requests_file_delivery(route_result)
        || !route_reason_has_structural_marker(route_result, "generated_file_delivery")
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || route_result.output_contract.response_shape != crate::OutputResponseShape::FileToken
    {
        return false;
    }
    if !route_result.output_contract.locator_hint.trim().is_empty() {
        return true;
    }
    if route_result.needs_clarify {
        return true;
    }
    matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    ) || [
        "file_token_delivery_contract_repair",
        "structured_contract_hint_repair",
        "explicit_command_preserves_generated_file_delivery_execution",
    ]
    .iter()
    .any(|marker| route_reason_has_structural_marker(route_result, marker))
}

fn preserve_filename_locator_as_existing_file_delivery(
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_reason_has_structural_marker(route_result, "generated_file_delivery") {
        return false;
    }
    let contract = &mut route_result.output_contract;
    if contract.locator_kind != crate::OutputLocatorKind::Filename
        || contract.locator_hint.trim().is_empty()
        || contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || contract.response_shape != crate::OutputResponseShape::FileToken
        || !contract.delivery_required
    {
        return false;
    }
    contract.semantic_kind = crate::OutputSemanticKind::None;
    contract.requires_content_evidence = true;
    route_result.wants_file_delivery = true;
    remove_route_reason_structural_marker(
        route_result,
        "generated_file_delivery_allows_runtime_target",
    );
    route_result
        .route_reason
        .push_str("; filename_locator_preserved_as_existing_file_delivery");
    true
}

fn normalize_output_shape_text(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn json_value_requests_filename_only_output(value: &Value) -> bool {
    match value {
        Value::String(text) => matches!(
            normalize_output_shape_text(text).as_str(),
            "filename"
                | "file_name"
                | "basename"
                | "filename_only"
                | "file_name_only"
                | "basename_only"
        ),
        Value::Array(items) => items.iter().any(json_value_requests_filename_only_output),
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                normalize_output_shape_text(key).as_str(),
                "output_format" | "output_shape" | "format" | "answer_format" | "delivery_format"
            ) && json_value_requests_filename_only_output(value)
        }),
        _ => false,
    }
}

fn turn_analysis_requests_filename_only_output(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(json_value_requests_filename_only_output)
}

pub(super) fn clear_file_delivery_contract_for_filename_only(
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) {
    if !turn_analysis_requests_filename_only_output(turn_analysis) {
        return;
    }
    route_result.wants_file_delivery = false;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    if matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) {
        route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    }
    route_result
        .route_reason
        .push_str("; filename_only_output_clears_file_delivery_contract");
}

fn active_delivery_bound_target(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Option<String> {
    session_snapshot
        .active_followup_frame
        .as_ref()
        .filter(|frame| {
            matches!(
                frame.op_kind,
                crate::followup_frame::FollowupOpKind::Delivery
            )
        })
        .and_then(|frame| frame.bound_target.as_deref())
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map(ToString::to_string)
}

fn prompt_has_current_turn_locator(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    surface.has_explicit_path_or_url()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_structured_target_refinement()
}

fn active_delivery_content_binding_allows_ordered_entry_override(
    route_result: &crate::RouteResult,
) -> bool {
    route_reason_has_structural_marker(
        route_result,
        "active_task_invalid_turn_binding_repaired_to_canonical_active_task_continuation_for_file_slice_correction",
    )
}

pub(super) fn bind_content_read_to_active_delivery_target(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    current_prompt: &str,
) -> bool {
    let has_ordered_entry_binding = super::has_ordered_entry_state_patch(turn_analysis)
        || route_reason_has_structural_marker(
            route_result,
            "ordered_entry_reference_bound_from_active_frame",
        );
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::None
        || route_result.output_contract.response_shape == crate::OutputResponseShape::FileToken
        || !route_result.output_contract.requires_content_evidence
        || prompt_has_current_turn_locator(current_prompt)
        || (has_ordered_entry_binding
            && !active_delivery_content_binding_allows_ordered_entry_override(route_result))
    {
        return false;
    }
    let Some(target) = active_delivery_bound_target(session_snapshot) else {
        return false;
    };
    let previous_hint = route_result.output_contract.locator_hint.trim().to_string();
    if previous_hint == target {
        if route_result.output_contract.locator_kind != crate::OutputLocatorKind::Path {
            route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        }
        return false;
    }

    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = target.clone();
    if !previous_hint.is_empty()
        && !resolved_intent_has_structural_value(&route_result.resolved_intent, &target)
        && resolved_intent_has_structural_value(&route_result.resolved_intent, &previous_hint)
    {
        route_result.resolved_intent = replace_resolved_intent_structural_value(
            &route_result.resolved_intent,
            &previous_hint,
            &target,
        );
    }
    append_active_delivery_resolved_target(route_result, &target);
    if !previous_hint.is_empty() {
        route_result.route_reason = replace_route_reason_structural_value(
            &route_result.route_reason,
            &previous_hint,
            &target,
        );
    }
    append_route_reason_structural_marker(route_result, ACTIVE_DELIVERY_CONTENT_TARGET_MARKER);
    true
}

pub(super) fn append_active_delivery_content_target_token(
    runtime_prompt: &mut String,
    route_result: &crate::RouteResult,
) {
    if !route_reason_has_structural_marker(route_result, ACTIVE_DELIVERY_CONTENT_TARGET_MARKER) {
        return;
    }
    let target = route_result.output_contract.locator_hint.trim();
    if target.is_empty() || runtime_prompt.contains(target) {
        return;
    }
    if !runtime_prompt.ends_with(char::is_whitespace) && !runtime_prompt.is_empty() {
        runtime_prompt.push('\n');
    }
    runtime_prompt.push_str("active_delivery_content_target: ");
    runtime_prompt.push_str(target);
}

fn record_unresolved_file_delivery_loop_candidate(route_result: &mut crate::RouteResult) {
    route_result.needs_clarify = false;
    route_result.clarify_question.clear();
    route_result.set_planner_execute_finalize(crate::ActFinalizeStyle::ChatWrapped);
    route_result.wants_file_delivery = true;
    route_result.output_contract.delivery_required = true;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route_result.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route_result.output_contract.locator_hint.clear();
    route_result.route_reason.push_str("; ");
    route_result
        .route_reason
        .push_str(UNRESOLVED_FILE_DELIVERY_REQUIRES_LOCATOR_MARKER);
}

fn allow_generated_file_delivery_without_locator(route_result: &mut crate::RouteResult) {
    route_result.needs_clarify = false;
    route_result.clarify_question.clear();
    route_result.wants_file_delivery = true;
    route_result.output_contract.delivery_required = true;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route_result.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route_result.output_contract.requires_content_evidence = true;
    if matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    ) {
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    }
    route_result
        .route_reason
        .push_str("; generated_file_delivery_allows_runtime_target");
}

#[cfg(test)]
pub(in crate::worker) fn repair_structural_file_delivery_resolution(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    repair_structural_file_delivery_resolution_for_turn(route_result, session_snapshot, None)
}

pub(in crate::worker) fn repair_structural_file_delivery_resolution_for_turn(
    route_result: &mut crate::RouteResult,
    _session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    _turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if !route_requests_file_delivery(route_result) {
        return false;
    }
    if file_delivery_locator_hint_points_to_existing_directory(route_result) {
        if route_has_structured_list_selector(route_result) {
            remove_route_reason_structural_marker(
                route_result,
                "directory_file_delivery_requires_structured_selection",
            );
            return false;
        }
        route_result
            .route_reason
            .push_str("; directory_file_delivery_requires_structured_selection");
        record_unresolved_file_delivery_loop_candidate(route_result);
        return true;
    }
    if preserve_filename_locator_as_existing_file_delivery(route_result) {
        return true;
    }
    if generated_file_delivery_can_choose_target(route_result) {
        allow_generated_file_delivery_without_locator(route_result);
        return true;
    }
    if file_delivery_has_concrete_locator(route_result) {
        return false;
    }
    record_unresolved_file_delivery_loop_candidate(route_result);
    true
}

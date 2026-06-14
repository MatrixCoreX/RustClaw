fn should_preserve_original_inline_structured_input(
    prompt: &str,
    resolved_prompt_for_execution: &str,
) -> bool {
    let prompt_trimmed = prompt.trim();
    let resolved_trimmed = resolved_prompt_for_execution.trim();
    if prompt_trimmed.is_empty()
        || resolved_trimmed.is_empty()
        || prompt_trimmed == resolved_trimmed
    {
        return false;
    }
    let Some(inline_value) = crate::extract_first_json_value_any(prompt_trimmed) else {
        return false;
    };
    !resolved_trimmed.contains(inline_value.trim())
}

pub(super) fn execution_user_request<'a>(
    prompt: &'a str,
    resolved_prompt_for_execution: &'a str,
) -> &'a str {
    if should_preserve_original_inline_structured_input(prompt, resolved_prompt_for_execution) {
        prompt
    } else {
        resolved_prompt_for_execution
    }
}

pub(super) fn sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution(
    route_result: &mut crate::RouteResult,
    prompt: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    resolved_prompt_for_execution: &mut String,
    prompt_with_memory_for_execution: &mut String,
) {
    if !pure_direct_chat_current_request_route(route_result, turn_analysis)
        || prompt.trim().is_empty()
        || route_result.resolved_intent.trim() == prompt.trim()
    {
        return;
    }
    route_result.resolved_intent = prompt.to_string();
    *resolved_prompt_for_execution = prompt.to_string();
    *prompt_with_memory_for_execution = prompt.to_string();
    super::append_route_reason(
        route_result,
        "untrusted_normalizer_freeform_rewrite_removed_from_execution_context",
    );
}

fn pure_direct_chat_current_request_route(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if !turn_analysis_allows_current_request_only_freeform_rewrite(turn_analysis)
        || route_result.needs_clarify
        || route_result.is_execute_gate()
        || route_result.wants_file_delivery
        || route_result.should_refresh_long_term_memory
        || !matches!(route_result.schedule_kind, crate::ScheduleKind::None)
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
        || route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        )
        || !matches!(
            route_result.output_contract.self_extension.mode,
            crate::SelfExtensionMode::None
        )
        || !matches!(
            route_result.output_contract.self_extension.trigger,
            crate::SelfExtensionTrigger::None
        )
        || route_result.output_contract.self_extension.execute_now
    {
        return false;
    }
    matches!(
        route_result.first_layer_decision(),
        crate::FirstLayerDecision::DirectAnswer
    )
}

fn turn_analysis_allows_current_request_only_freeform_rewrite(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(analysis) = turn_analysis else {
        return true;
    };
    matches!(
        analysis.turn_type,
        Some(crate::intent_router::TurnType::TaskRequest)
    ) && analysis.target_task_policy.is_none()
        && !analysis.should_interrupt_active_run
        && analysis.state_patch.is_none()
        && !analysis.attachment_processing_required
}

pub(super) fn sanitize_untrusted_normalizer_answer_candidate_for_execution(
    route_result: &mut crate::RouteResult,
    prompt: &str,
    recent_execution_context: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    resolved_prompt_for_execution: &mut String,
    prompt_with_memory_for_execution: &mut String,
) {
    let Some(candidate) = embedded_normalizer_answer_candidate_block(&route_result.resolved_intent)
    else {
        return;
    };
    if normalizer_answer_candidate_allowed_in_execution_context(
        &candidate,
        prompt,
        recent_execution_context,
        session_snapshot,
    ) {
        return;
    }
    route_result.resolved_intent =
        strip_embedded_answer_candidate_block(&route_result.resolved_intent);
    *resolved_prompt_for_execution =
        strip_embedded_answer_candidate_block(resolved_prompt_for_execution);
    *prompt_with_memory_for_execution =
        strip_embedded_answer_candidate_block(prompt_with_memory_for_execution);
    super::append_route_reason(
        route_result,
        "untrusted_normalizer_answer_candidate_removed_from_execution_context",
    );
}

fn embedded_normalizer_answer_candidate_block(text: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut collecting = false;
    for line in text.lines() {
        if let Some(candidate) = line.trim_start().strip_prefix("answer_candidate:") {
            collecting = true;
            let candidate = candidate.trim();
            if !candidate.is_empty() {
                lines.push(candidate.to_string());
            }
            continue;
        }
        if collecting {
            if line.trim_start().starts_with("### ") {
                break;
            }
            lines.push(line.to_string());
        }
    }
    let candidate = lines.join("\n").trim().to_string();
    (!candidate.is_empty()).then_some(candidate)
}

fn normalizer_answer_candidate_allowed_in_execution_context(
    candidate: &str,
    prompt: &str,
    recent_execution_context: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if super::answer_candidate_is_compact_scalar_shape(candidate) {
        return true;
    }
    if super::normalizer_answer_candidate_is_grounded_in_structured_observation(
        candidate,
        session_snapshot,
    ) {
        return true;
    }
    if super::normalizer_answer_candidate_matches_recent_execution_context(
        candidate,
        recent_execution_context,
    ) {
        return true;
    }
    answer_candidate_preserves_current_turn_machine_literals(candidate, prompt)
}

fn answer_candidate_preserves_current_turn_machine_literals(candidate: &str, prompt: &str) -> bool {
    if candidate.chars().count() > 300 || candidate.lines().count() > 2 {
        return false;
    }
    let literals = current_turn_machine_literals(prompt);
    if literals.is_empty() {
        return false;
    }
    let candidate = candidate.to_ascii_lowercase();
    literals
        .iter()
        .all(|literal| candidate.contains(&literal.to_ascii_lowercase()))
}

fn current_turn_machine_literals(text: &str) -> Vec<String> {
    let mut literals = Vec::new();
    for token in text.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '.' | ':' | '\\'))
    }) {
        let token = token.trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '.' | ':' | '\\'));
        if current_turn_machine_literal(token) && !literals.iter().any(|item| item == token) {
            literals.push(token.to_string());
        }
    }
    literals
}

fn current_turn_machine_literal(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    if token.starts_with("http://") || token.starts_with("https://") {
        return token.len() <= 240;
    }
    if std::path::Path::new(token).extension().is_some() {
        return true;
    }
    token.contains('/')
        || token.contains('\\')
        || token.contains('_')
        || token.contains('-')
        || token.chars().any(|ch| ch.is_ascii_digit())
}

fn strip_embedded_answer_candidate_block(text: &str) -> String {
    let mut lines = Vec::new();
    let mut skipping_candidate = false;
    for line in text.lines() {
        if line.trim_start().starts_with("answer_candidate:") {
            skipping_candidate = true;
            continue;
        }
        if skipping_candidate && line.trim_start().starts_with("### ") {
            skipping_candidate = false;
        }
        if !skipping_candidate {
            lines.push(line);
        }
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "ask_pipeline_execution_context_tests.rs"]
mod tests;

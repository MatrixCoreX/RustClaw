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

#[derive(Clone, Copy)]
enum ExecutionContextSanitization {
    FreeformRewrite,
    AnswerCandidate,
    LocatorCompletion,
}

impl ExecutionContextSanitization {
    fn route_reason(self) -> &'static str {
        match self {
            Self::FreeformRewrite => {
                "untrusted_normalizer_freeform_rewrite_removed_from_execution_context"
            }
            Self::AnswerCandidate => {
                "untrusted_normalizer_answer_candidate_removed_from_execution_context"
            }
            Self::LocatorCompletion => {
                "untrusted_normalizer_locator_completion_removed_from_execution_context"
            }
        }
    }

    fn record(self, route_result: &mut crate::RouteResult) {
        super::append_route_reason(route_result, self.route_reason());
    }
}

pub(in crate::worker) fn execution_user_request<'a>(
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
    let resolved_context_suffix = trusted_execution_context_suffix(resolved_prompt_for_execution);
    let prompt_context_suffix = trusted_execution_context_suffix(prompt_with_memory_for_execution);
    route_result.resolved_intent = prompt.to_string();
    let mut resolved_with_context =
        String::with_capacity(prompt.len() + resolved_context_suffix.len());
    resolved_with_context.push_str(prompt);
    resolved_with_context.push_str(&resolved_context_suffix);
    *resolved_prompt_for_execution = resolved_with_context;
    let mut prompt_with_context = String::with_capacity(prompt.len() + prompt_context_suffix.len());
    prompt_with_context.push_str(prompt);
    prompt_with_context.push_str(&prompt_context_suffix);
    *prompt_with_memory_for_execution = prompt_with_context;
    ExecutionContextSanitization::FreeformRewrite.record(route_result);
}

pub(super) fn sanitize_untrusted_normalizer_locator_completion_for_loop_boundary(
    route_result: &mut crate::RouteResult,
    prompt: &str,
    pre_loop_clarify_candidates: &[&'static str],
    resolved_prompt_for_execution: &mut String,
    prompt_with_memory_for_execution: &mut String,
) {
    let should_sanitize_locator_completion =
        super::pre_loop_candidates_redact_untrusted_workspace_child(pre_loop_clarify_candidates)
            || super::route_reason_has_marker(
                route_result,
                "standalone_freeform_clarify_loop_context",
            );
    if prompt.trim().is_empty()
        || !should_sanitize_locator_completion
        || route_result.resolved_intent.trim() == prompt.trim()
    {
        return;
    }
    let resolved_context_suffix = trusted_execution_context_suffix(resolved_prompt_for_execution);
    let prompt_context_suffix = trusted_execution_context_suffix(prompt_with_memory_for_execution);
    route_result.resolved_intent = prompt.to_string();
    let mut resolved_with_context =
        String::with_capacity(prompt.len() + resolved_context_suffix.len());
    resolved_with_context.push_str(prompt);
    resolved_with_context.push_str(&resolved_context_suffix);
    *resolved_prompt_for_execution = resolved_with_context;
    let mut prompt_with_context = String::with_capacity(prompt.len() + prompt_context_suffix.len());
    prompt_with_context.push_str(prompt);
    prompt_with_context.push_str(&prompt_context_suffix);
    *prompt_with_memory_for_execution = prompt_with_context;
    ExecutionContextSanitization::LocatorCompletion.record(route_result);
}

fn trusted_execution_context_suffix(text: &str) -> String {
    const MARKERS: [&str; 4] = [
        "\n\n### RUNTIME_CONTEXT",
        "\n\n### SESSION_ALIAS_BINDINGS",
        "\n\n### ACTIVE_EXECUTION_ANCHOR",
        "\n\n### RECENT_EXECUTION_CONTEXT",
    ];
    MARKERS
        .iter()
        .filter_map(|marker| text.find(marker))
        .min()
        .map(|idx| text[idx..].to_string())
        .unwrap_or_default()
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
    route_result.is_resume_discussion_mode()
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
    if embedded_normalizer_answer_candidate_block(&route_result.resolved_intent).is_none() {
        return;
    }
    let _ = (prompt, recent_execution_context, session_snapshot);
    route_result.resolved_intent =
        strip_embedded_answer_candidate_block(&route_result.resolved_intent);
    *resolved_prompt_for_execution =
        strip_embedded_answer_candidate_block(resolved_prompt_for_execution);
    *prompt_with_memory_for_execution =
        strip_embedded_answer_candidate_block(prompt_with_memory_for_execution);
    ExecutionContextSanitization::AnswerCandidate.record(route_result);
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

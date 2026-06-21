use super::*;

pub(super) fn ask_reply_with_chat_process(text: String, _language_hint: &str) -> AskReply {
    let answer = text.trim().to_string();
    if answer.is_empty() || crate::finalize::is_execution_summary_message(&answer) {
        AskReply::llm(text)
    } else {
        AskReply::llm(answer)
    }
}

pub(super) fn with_agent_decides_shadow_snapshot(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    mut reply: AskReply,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> AskReply {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return reply;
    };
    let Some(attribution) = crate::agent_engine::agent_decides_shadow_snapshot_for_route(
        state,
        task,
        agent_run_context,
        route,
    ) else {
        return reply;
    };
    let journal = reply.task_journal.get_or_insert_with(|| {
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", user_request)
    });
    journal.record_route_result(route);
    journal.record_rollout_attribution(attribution);
    reply
}

pub(super) fn with_pre_planner_exit_snapshot(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    reply: AskReply,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    reason_code: &str,
) -> AskReply {
    let Some(exit) = pre_planner_exit_for_reason(reason_code) else {
        return with_agent_decides_shadow_snapshot(
            state,
            task,
            user_request,
            reply,
            agent_run_context,
        );
    };
    let mut reply =
        with_agent_decides_shadow_snapshot(state, task, user_request, reply, agent_run_context);
    let journal = reply.task_journal.get_or_insert_with(|| {
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", user_request)
    });
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        journal.record_route_result(route);
    }
    journal.record_rollout_attribution(crate::task_journal::TaskJournalRolloutAttribution {
        switch_name: "pre_planner_exit_inventory".to_string(),
        event: "pre_planner_exit".to_string(),
        outcome: "observed".to_string(),
        reason_code: Some(exit.reason_code.to_string()),
        owner_layer: Some(exit.owner_layer.to_string()),
        decision: Some(exit.kind.as_str().to_string()),
        boundary_context: Some(exit.trace_context()),
        ..Default::default()
    });
    reply
}

pub(super) fn ask_reply_with_clarify_process(
    task: &ClaimedTask,
    user_request: &str,
    text: String,
    route_result: Option<&crate::RouteResult>,
) -> AskReply {
    let answer = text.trim().to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", user_request);
    if let Some(route_result) = route_result {
        journal.record_route_result(route_result);
    }
    journal.record_final_answer(&answer);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);
    AskReply::llm(answer).with_task_journal(journal)
}

pub(super) fn schema_value_requests_filename_only_output(value: &Value) -> bool {
    match value {
        Value::String(text) => matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "basename" | "filename_only" | "file_name_only" | "basename_only"
        ),
        Value::Bool(value) => *value,
        Value::Array(items) => items.iter().any(schema_value_requests_filename_only_output),
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.trim(),
                "filename_only" | "file_name_only" | "basename_only" | "output_format" | "format"
            ) && schema_value_requests_filename_only_output(value)
        }),
        _ => false,
    }
}

pub(super) fn request_uses_filename_only_schema_token(prompt: &str) -> bool {
    let normalized = prompt.trim().to_ascii_lowercase();
    [
        "basename",
        "filename_only",
        "file_name_only",
        "basename_only",
    ]
    .iter()
    .any(|token| normalized.contains(token))
}

pub(super) fn route_contract_requests_filename_only_output(
    route: Option<&crate::RouteResult>,
) -> bool {
    route.is_some_and(|route| {
        matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
        )
    })
}

pub(super) fn turn_analysis_requests_filename_only_output(
    analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(schema_value_requests_filename_only_output)
}

pub(super) fn session_alias_target_direct_answer_candidate(
    state: &AppState,
    task: &ClaimedTask,
    current_user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref();
    if route.is_some_and(|route| route.needs_clarify || route.output_contract.delivery_required) {
        return None;
    }
    let current_request_declares_filename_only =
        request_uses_filename_only_schema_token(current_user_request);
    let route_resolved_intent_declares_filename_only = route
        .and_then(|route| {
            let resolved = route.resolved_intent.trim();
            (!resolved.is_empty()).then_some(resolved)
        })
        .is_some_and(request_uses_filename_only_schema_token);
    let turn_analysis_declares_filename_only =
        turn_analysis_requests_filename_only_output(ctx.turn_analysis.as_ref());
    let route_contract_declares_filename_only = route_contract_requests_filename_only_output(route);
    let wants_filename_only = current_request_declares_filename_only
        || route_resolved_intent_declares_filename_only
        || turn_analysis_declares_filename_only
        || route_contract_declares_filename_only;
    if !wants_filename_only {
        return None;
    }
    if route.is_some_and(|route| route.output_contract.requires_content_evidence)
        && !current_request_declares_filename_only
        && !route_resolved_intent_declares_filename_only
        && !turn_analysis_declares_filename_only
    {
        return None;
    }
    let conversation_state =
        crate::conversation_state::load_active_conversation_state(state, task)?;
    let binding = crate::conversation_state::single_alias_binding_mentioned_in_prompt(
        &conversation_state.alias_bindings,
        current_user_request,
    )?;
    let target = binding.target.trim();
    if target.is_empty() {
        return None;
    }
    Path::new(target)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn structural_alias_binding_ack(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    prompt: &str,
    resolved_prompt_for_execution: &str,
    language_hint: &str,
) -> Option<AskReply> {
    let ctx = agent_run_context?;
    let analysis = ctx.turn_analysis.as_ref()?;
    let route_result = ctx.route_result.as_ref()?;
    if !route_allows_memory_ack_shape(route_result) {
        return None;
    }
    let has_alias_only_state_patch = analysis
        .state_patch
        .as_ref()
        .is_some_and(crate::conversation_state::state_patch_is_alias_bindings_only);
    if !has_alias_only_state_patch
        && analysis.turn_type != Some(crate::intent_router::TurnType::PreferenceOrMemory)
    {
        return None;
    }
    if has_alias_only_state_patch {
        if let Some(answer) = normalizer_memory_ack_answer_candidate(route_result) {
            return Some(ask_reply_with_chat_process(answer, language_hint));
        }
    }
    if let Some(answer) = alias_state_patch_ack_answer(state, ctx, language_hint) {
        return Some(AskReply::non_llm(answer));
    }
    let Some(_binding) = crate::conversation_state::structural_alias_binding_from_memory_prompt(
        prompt,
        route_result,
        resolved_prompt_for_execution,
    ) else {
        return None;
    };
    normalizer_memory_ack_answer_candidate(route_result)
        .map(|answer| ask_reply_with_chat_process(answer, language_hint))
}

pub(super) fn normalizer_memory_ack_answer_candidate(
    route_result: &crate::RouteResult,
) -> Option<String> {
    let candidate =
        normalizer_answer_candidate_from_resolved_prompt(&route_result.resolved_intent)?;
    let trimmed = candidate.trim();
    if trimmed.is_empty()
        || trimmed.contains(['\n', '\r'])
        || trimmed.chars().count() > 120
        || crate::finalize::is_execution_summary_message(trimmed)
    {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(trimmed);
    if surface.has_concrete_locator_hint()
        || crate::intent::locator_extractor::extract_explicit_locator_for_fallback(trimmed)
            .is_some()
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(super) fn alias_state_patch_ack_answer(
    state: &AppState,
    ctx: &crate::agent_engine::AgentRunContext,
    language_hint: &str,
) -> Option<String> {
    let state_patch = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())?;
    let bindings =
        crate::conversation_state::session_alias_bindings_from_state_patch(Some(state_patch));
    if bindings.is_empty() {
        return None;
    }
    let key = alias_binding_ack_message_key(&bindings, &ctx.session_alias_bindings);
    Some(localized_alias_binding_ack(state, key, language_hint))
}

pub(super) fn alias_binding_ack_message_key(
    bindings: &[crate::conversation_state::SessionAliasBinding],
    prior_bindings: &[crate::conversation_state::SessionAliasBinding],
) -> &'static str {
    let has_prior_alias = bindings.iter().any(|binding| {
        prior_bindings
            .iter()
            .any(|prior| prior.alias == binding.alias)
    });
    if has_prior_alias {
        "clawd.msg.memory.alias_updated"
    } else {
        "clawd.msg.memory.alias_remembered"
    }
}

pub(super) fn localized_alias_binding_ack(
    state: &AppState,
    key: &str,
    language_hint: &str,
) -> String {
    let fallback = format!("message_key={key}");
    crate::app_helpers::localized_t_with_default(state, key, &fallback, language_hint)
}

pub(crate) fn build_resume_continue_execute_prompt(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    fallback_user_text: &str,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let user_text = payload
        .get("resume_user_text")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_user_text);
    let resume_context = payload
        .get("resume_context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let resume_instruction = payload
        .get("resume_instruction")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let resume_steps = payload.get("resume_steps");
    build_resume_continue_execute_prompt_from_parts(
        state,
        task,
        user_text,
        &resume_context,
        resume_instruction,
        resume_steps,
    )
}

pub(crate) fn build_resume_continue_execute_prompt_from_context(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    build_resume_continue_execute_prompt_from_parts(
        state,
        task,
        user_text,
        resume_context,
        "",
        None,
    )
}

pub(super) fn build_resume_followup_discussion_prompt_from_parts(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let resume_context_json =
        serde_json::to_string_pretty(resume_context).unwrap_or_else(|_| resume_context.to_string());
    let (prompt_template, _) = crate::bootstrap::load_required_prompt_template_for_state(
        state,
        crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_LOGICAL_PATH,
    )?;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    Ok(crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text.trim()),
            ("__RESUME_CONTEXT__", &resume_context_json),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
        ],
    ))
}

pub(crate) fn build_resume_followup_discussion_prompt(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    fallback_user_text: &str,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let user_text = payload
        .get("resume_user_text")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_user_text)
        .trim();
    let resume_context = payload
        .get("resume_context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    build_resume_followup_discussion_prompt_from_parts(state, task, user_text, &resume_context)
}

pub(crate) fn build_resume_followup_discussion_prompt_from_context(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    build_resume_followup_discussion_prompt_from_parts(state, task, user_text, resume_context)
}

pub(super) fn chat_wrapped_execution_goal_from_prompt(prompt_with_memory: &str) -> String {
    format!(
        "{}\n\nFinalize hint: complete required actions first, then return a concise user-facing reply that confirms results naturally.",
        prompt_with_memory
    )
}

pub(super) fn fuzzy_locator_clarify_context(candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let candidate_block = candidates
        .iter()
        .enumerate()
        .map(|(idx, value)| format!("candidate_{}: {}", idx + 1, value))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "clarify_case: fuzzy_locator_candidates\nexact_target_found: false\ncandidate_count: {}\n{}",
        candidates.len(),
        candidate_block
    ))
}

pub(super) fn preferred_route_clarify_question(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    // Reuse the normalizer's clarify_question as the single clarify entry point.
    // Post-route policy may promote a route to first-layer Clarify after locator
    // checks, so preserving the existing question avoids an extra LLM round.
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    let question = route.clarify_question.trim();
    if !question.is_empty() {
        return Some(question.to_string());
    }
    None
}

pub(super) fn route_structured_clarify_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if let Some(context) = fuzzy_locator_clarify_context(&ctx.fuzzy_locator_suggestions) {
        return Some(context);
    }
    if !route.needs_clarify || !route.output_contract.locator_hint.trim().is_empty() {
        return None;
    }
    let clarify_case = route_clarify_reason_code(&route.route_reason).or_else(|| {
        if route.output_contract.delivery_required {
            Some("missing_file_locator")
        } else if route.output_contract.requires_content_evidence
            && matches!(
                route.output_contract.response_shape,
                crate::OutputResponseShape::Scalar
            )
        {
            Some("missing_read_target")
        } else {
            None
        }
    })?;
    Some(
        [
            format!("clarify_case: {clarify_case}"),
            format!(
                "locator_kind: {}",
                route.output_contract.locator_kind.as_str()
            ),
            format!(
                "response_shape: {}",
                route.output_contract.response_shape.as_str()
            ),
            format!(
                "semantic_kind: {}",
                route.output_contract.semantic_kind.as_str()
            ),
            format!(
                "requires_content_evidence: {}",
                route.output_contract.requires_content_evidence
            ),
            format!(
                "delivery_required: {}",
                route.output_contract.delivery_required
            ),
        ]
        .join("\n"),
    )
}

pub(super) fn route_clarify_reason_code(route_reason: &str) -> Option<&'static str> {
    for token in route_reason.split(|ch: char| {
        ch.is_whitespace() || matches!(ch, ';' | ',' | '|' | '[' | ']' | '(' | ')')
    }) {
        let token = token.trim();
        let Some(code) = token.strip_prefix("clarify_reason_code:") else {
            continue;
        };
        return match code {
            "missing_count_target" => Some("missing_count_target"),
            "missing_delivery_locator" => Some("missing_delivery_locator"),
            "missing_service_target" => Some("missing_service_target"),
            "missing_search_locator" => Some("missing_search_locator"),
            "missing_read_target" => Some("missing_read_target"),
            "missing_target" => Some("missing_target"),
            _ => None,
        };
    }
    None
}

pub(super) fn chat_route_resolution_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    let mut lines = Vec::new();
    let resolved_intent = route.resolved_intent.trim();
    if !resolved_intent.is_empty() {
        lines.push(format!("resolved_user_intent: {resolved_intent}"));
    }
    if let Some(draft) = active_task_semantic_answer_candidate_draft(ctx) {
        lines.push(format!("active_task_semantic_draft: {draft}"));
        lines.push("active_task_semantic_draft_note: Non-evidence writing draft from routing. Use it only as a semantic anchor for active-task rewriting; the current user's output shape, length, language, and correction constraints still win.".to_string());
    }
    let required_visible_literals = active_task_required_visible_literals(ctx);
    if !required_visible_literals.is_empty() {
        lines.push(format!(
            "active_task_required_visible_literals: {}",
            required_visible_literals.join(" | ")
        ));
        lines.push("active_task_required_visible_literals_note: These are exact user-supplied correction/refinement literals from structured turn state. The revised deliverable must visibly contain them unless the current user explicitly asks to omit them.".to_string());
    }
    let replacement_pairs = active_task_replacement_pairs(ctx);
    if !replacement_pairs.is_empty() {
        let rendered = replacement_pairs
            .iter()
            .map(|pair| format!("{} -> {}", pair.from, pair.to))
            .collect::<Vec<_>>()
            .join(" | ");
        lines.push(format!("active_task_replacement_pairs: {rendered}"));
    }
    let forbidden_visible_literals = active_task_forbidden_visible_literals(ctx);
    if !forbidden_visible_literals.is_empty() {
        lines.push(format!(
            "active_task_forbidden_visible_literals: {}",
            forbidden_visible_literals.join(" | ")
        ));
    }
    let locator_hint = route.output_contract.locator_hint.trim();
    if !locator_hint.is_empty() {
        lines.push(format!("locator_hint: {locator_hint}"));
    }
    if route_reason_has_exact_marker(route, "active_anchor_observed_judgment_to_chat") {
        if let Some(anchor_lines) = ctx
            .context_bundle_summary
            .as_deref()
            .map(active_execution_anchor_evidence_lines)
            .filter(|items| !items.is_empty())
        {
            lines.push("active_execution_anchor_evidence:".to_string());
            lines.extend(anchor_lines.into_iter().map(|line| format!("  {line}")));
        }
    }
    lines.push(format!(
        "response_shape: {}",
        route.output_contract.response_shape.as_str()
    ));
    lines.push(format!(
        "semantic_kind: {}",
        route.output_contract.semantic_kind.as_str()
    ));
    lines.push(format!(
        "requires_content_evidence: {}",
        route.output_contract.requires_content_evidence
    ));
    lines.push(format!(
        "delivery_required: {}",
        route.output_contract.delivery_required
    ));
    let route_reason = route.route_reason.trim();
    let route_reason_markers = route_reason_machine_markers(route_reason);
    if !route_reason_markers.is_empty() {
        lines.push(format!(
            "route_reason_markers: {}",
            route_reason_markers.join(" | ")
        ));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "### ROUTE_RESOLUTION\nTreat the following route resolution as authoritative for this turn. It is resolved context, not missing context. If older memory or unrelated assistant history conflicts with it, prefer this resolution unless the user explicitly asks about older history.\n{}\n",
        lines.join("\n")
    ))
}

pub(super) fn route_reason_machine_markers(route_reason: &str) -> Vec<String> {
    let mut markers = Vec::new();
    for token in route_reason.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ';' | ',' | '，' | '。' | '|' | '[' | ']' | '(' | ')' | '{' | '}'
            )
    }) {
        let token = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '`' | ':' | '=' | '.' | '-' | '_' | '/' | '\\'
            )
        });
        if token.chars().count() < 3 || !token.is_ascii() {
            continue;
        }
        if !token.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '=' | '-' | '.' | '/' | '\\')
        }) {
            continue;
        }
        let looks_machine = token.contains('_')
            || token.contains(':')
            || token.contains('=')
            || token.contains('/')
            || token.contains('\\');
        if looks_machine && !markers.iter().any(|marker| marker == token) {
            markers.push(token.to_string());
        }
        if markers.len() >= 12 {
            break;
        }
    }
    markers
}

pub(super) fn active_task_text_mutation_context(
    ctx: &crate::agent_engine::AgentRunContext,
) -> bool {
    let Some(route) = ctx.route_result.as_ref() else {
        return false;
    };
    if route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(&route.output_contract)
    {
        return false;
    }
    let Some(analysis) = ctx.turn_analysis.as_ref() else {
        return false;
    };
    matches!(
        analysis.turn_type,
        Some(
            crate::intent_router::TurnType::TaskAppend
                | crate::intent_router::TurnType::TaskCorrect
                | crate::intent_router::TurnType::TaskReplace
                | crate::intent_router::TurnType::TaskScopeUpdate
        )
    ) && matches!(
        analysis.target_task_policy,
        Some(
            crate::intent_router::TargetTaskPolicy::ReuseActive
                | crate::intent_router::TargetTaskPolicy::ReplaceActive
        )
    )
}

pub(super) fn active_task_semantic_answer_candidate_draft(
    ctx: &crate::agent_engine::AgentRunContext,
) -> Option<String> {
    if !active_task_text_mutation_context(ctx) {
        return None;
    }
    let draft = ctx.semantic_answer_candidate_draft.as_deref()?.trim();
    if draft.is_empty() || route_draft_is_compact_scalar_shape(draft) {
        return None;
    }
    let max_bytes = 1600;
    if draft.len() <= max_bytes {
        return Some(draft.to_string());
    }
    let mut out = crate::utf8_safe_prefix(draft, max_bytes).to_string();
    out.push_str("...(truncated)");
    Some(out)
}

pub(super) fn route_draft_is_compact_scalar_shape(draft: &str) -> bool {
    let trimmed = draft.trim();
    if trimmed.is_empty()
        || trimmed.contains('\n')
        || trimmed.chars().count() > 80
        || trimmed.chars().any(|c| {
            matches!(
                c,
                ',' | '，'
                    | ';'
                    | '；'
                    | '。'
                    | '！'
                    | '？'
                    | '!'
                    | '?'
                    | '|'
                    | '['
                    | ']'
                    | '{'
                    | '}'
            )
        })
    {
        return false;
    }
    let token_count = trimmed.split_whitespace().count();
    (1..=4).contains(&token_count)
}

pub(super) fn active_task_required_visible_literals(
    ctx: &crate::agent_engine::AgentRunContext,
) -> Vec<String> {
    if !active_task_text_mutation_context(ctx) {
        return Vec::new();
    }
    let Some(state_patch) = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
    else {
        return Vec::new();
    };
    trusted_required_visible_literals_from_state_patch(
        state_patch,
        ctx.original_user_request
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActiveTaskReplacementPair {
    pub(super) from: String,
    pub(super) to: String,
}

#[cfg(test)]
pub(super) fn required_visible_literals_from_state_patch(
    state_patch: &serde_json::Value,
) -> Vec<String> {
    let mut literals = Vec::new();
    for key in [
        "required_visible_literals",
        "active_task_required_visible_literals",
        "visible_literals",
    ] {
        collect_required_visible_literals(state_patch.get(key), &mut literals);
    }
    if let Some(constraints) = state_patch.get("visible_constraints") {
        collect_required_visible_literals(Some(constraints), &mut literals);
        collect_required_visible_literals(
            constraints.get("required_visible_literals"),
            &mut literals,
        );
        collect_required_visible_literals(constraints.get("literals"), &mut literals);
    }
    for pair in replacement_pairs_from_state_patch(state_patch) {
        push_required_visible_literal(&pair.to, &mut literals);
    }
    literals
}

pub(super) fn trusted_required_visible_literals_from_state_patch(
    state_patch: &serde_json::Value,
    current_user_request: Option<&str>,
) -> Vec<String> {
    let mut literals = Vec::new();
    let mut source_bound_literals = Vec::new();
    for key in [
        "required_content_literals",
        "active_task_required_content_literals",
        "content_literals",
    ] {
        collect_required_visible_literals(state_patch.get(key), &mut source_bound_literals);
    }
    if let Some(constraints) = state_patch.get("visible_constraints") {
        collect_required_visible_literals(
            constraints.get("required_content_literals"),
            &mut source_bound_literals,
        );
        collect_required_visible_literals(
            constraints.get("content_literals"),
            &mut source_bound_literals,
        );
    }
    for key in [
        "required_visible_literals",
        "active_task_required_visible_literals",
        "visible_literals",
    ] {
        collect_typed_content_visible_literals(state_patch.get(key), &mut source_bound_literals);
    }
    if let Some(constraints) = state_patch.get("visible_constraints") {
        collect_typed_content_visible_literals(
            constraints.get("required_visible_literals"),
            &mut source_bound_literals,
        );
        collect_typed_content_visible_literals(
            constraints.get("literals"),
            &mut source_bound_literals,
        );
    }
    for literal in source_bound_literals {
        if required_literal_has_current_request_source(current_user_request, &literal) {
            push_required_visible_literal(&literal, &mut literals);
        }
    }
    for pair in replacement_pairs_from_state_patch(state_patch) {
        push_required_visible_literal(&pair.to, &mut literals);
    }
    literals
}

pub(super) fn required_literal_has_current_request_source(
    current_user_request: Option<&str>,
    literal: &str,
) -> bool {
    let Some(current_user_request) = current_user_request else {
        return true;
    };
    let literal = literal.trim();
    if literal.is_empty() {
        return false;
    }
    current_user_request.contains(literal)
        || (literal.is_ascii()
            && current_user_request
                .to_ascii_lowercase()
                .contains(&literal.to_ascii_lowercase()))
}

pub(super) fn active_task_forbidden_visible_literals(
    ctx: &crate::agent_engine::AgentRunContext,
) -> Vec<String> {
    if !active_task_text_mutation_context(ctx) {
        return Vec::new();
    }
    let Some(state_patch) = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
    else {
        return Vec::new();
    };
    forbidden_visible_literals_from_state_patch(state_patch)
}

pub(super) fn forbidden_visible_literals_from_state_patch(
    state_patch: &serde_json::Value,
) -> Vec<String> {
    let mut literals = Vec::new();
    for key in [
        "forbidden_visible_literals",
        "active_task_forbidden_visible_literals",
    ] {
        collect_required_visible_literals(state_patch.get(key), &mut literals);
    }
    if let Some(constraints) = state_patch.get("visible_constraints") {
        collect_required_visible_literals(
            constraints.get("forbidden_visible_literals"),
            &mut literals,
        );
    }
    for pair in replacement_pairs_from_state_patch(state_patch) {
        push_required_visible_literal(&pair.from, &mut literals);
    }
    literals
}

pub(super) fn active_task_replacement_pairs(
    ctx: &crate::agent_engine::AgentRunContext,
) -> Vec<ActiveTaskReplacementPair> {
    if !active_task_text_mutation_context(ctx) {
        return Vec::new();
    }
    let Some(state_patch) = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
    else {
        return Vec::new();
    };
    replacement_pairs_from_state_patch(state_patch)
}

pub(super) fn replacement_pairs_from_state_patch(
    state_patch: &serde_json::Value,
) -> Vec<ActiveTaskReplacementPair> {
    let mut pairs = Vec::new();
    for key in ["replacement_pairs", "active_task_replacement_pairs"] {
        collect_replacement_pairs(state_patch.get(key), &mut pairs);
    }
    if let Some(constraints) = state_patch.get("visible_constraints") {
        collect_replacement_pairs(constraints.get("replacement_pairs"), &mut pairs);
    }
    pairs
}

pub(super) fn collect_replacement_pairs(
    value: Option<&serde_json::Value>,
    out: &mut Vec<ActiveTaskReplacementPair>,
) {
    match value {
        Some(serde_json::Value::Array(values)) => {
            for value in values {
                collect_replacement_pairs(Some(value), out);
            }
        }
        Some(serde_json::Value::Object(map)) => {
            let from = map
                .get("from")
                .or_else(|| map.get("old"))
                .or_else(|| map.get("source"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim();
            let to = map
                .get("to")
                .or_else(|| map.get("new"))
                .or_else(|| map.get("target"))
                .or_else(|| map.get("value"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim();
            if from.is_empty() || to.is_empty() {
                return;
            }
            if active_task_machine_placeholder_literal(to) {
                return;
            }
            if from.contains('\n')
                || to.contains('\n')
                || from.chars().count() > 80
                || to.chars().count() > 80
            {
                return;
            }
            if out.iter().any(|pair| pair.from == from && pair.to == to) {
                return;
            }
            out.push(ActiveTaskReplacementPair {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
        _ => {}
    }
}

pub(super) fn active_task_machine_placeholder_literal(value: &str) -> bool {
    let value = value
        .trim()
        .trim_matches(|c: char| c.is_ascii_whitespace() || matches!(c, '"' | '\'' | '`'))
        .trim();
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

pub(super) fn collect_required_visible_literals(
    value: Option<&serde_json::Value>,
    out: &mut Vec<String>,
) {
    match value {
        Some(serde_json::Value::String(value)) => push_required_visible_literal(value, out),
        Some(serde_json::Value::Array(values)) => {
            for value in values {
                collect_required_visible_literals(Some(value), out);
            }
        }
        Some(serde_json::Value::Object(map)) => {
            collect_required_visible_literals(map.get("literal"), out);
            collect_required_visible_literals(map.get("value"), out);
            collect_required_visible_literals(map.get("text"), out);
        }
        _ => {}
    }
}

pub(super) fn collect_typed_content_visible_literals(
    value: Option<&serde_json::Value>,
    out: &mut Vec<String>,
) {
    match value {
        Some(serde_json::Value::Array(values)) => {
            for value in values {
                collect_typed_content_visible_literals(Some(value), out);
            }
        }
        Some(serde_json::Value::Object(map)) => {
            let semantic_token = map
                .get("kind")
                .or_else(|| map.get("type"))
                .or_else(|| map.get("role"))
                .or_else(|| map.get("semantic"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if matches!(
                semantic_token.as_str(),
                "content" | "content_literal" | "visible_content" | "required_content"
            ) {
                collect_required_visible_literals(map.get("literal"), out);
                collect_required_visible_literals(map.get("value"), out);
                collect_required_visible_literals(map.get("text"), out);
            }
        }
        _ => {}
    }
}

pub(super) fn push_required_visible_literal(value: &str, out: &mut Vec<String>) {
    let value = value
        .trim()
        .trim_matches(|c: char| c.is_ascii_whitespace() || matches!(c, '"' | '\'' | '`'))
        .trim();
    if active_task_machine_placeholder_literal(value) {
        return;
    }
    if value.is_empty() || value.contains('\n') || value.chars().count() > 80 {
        return;
    }
    if out.iter().any(|existing| existing == value) {
        return;
    }
    out.push(value.to_string());
}

pub(super) fn answer_contains_required_visible_literal(answer: &str, literal: &str) -> bool {
    if answer.contains(literal) {
        return true;
    }
    literal.is_ascii()
        && answer
            .to_ascii_lowercase()
            .contains(&literal.to_ascii_lowercase())
}

pub(super) fn ensure_active_task_required_visible_literals(
    answer: String,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> String {
    let Some(ctx) = agent_run_context else {
        return answer;
    };
    let missing = active_task_required_visible_literals(ctx)
        .into_iter()
        .filter(|literal| !answer_contains_required_visible_literal(&answer, literal))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return answer;
    }
    let prefix = missing.join(" / ");
    let answer = answer.trim();
    if answer.is_empty() {
        prefix
    } else {
        format!("{prefix}: {answer}")
    }
}

pub(super) fn strip_embedded_answer_candidate_from_intent(resolved_intent: &str) -> (String, bool) {
    let mut stripped = Vec::new();
    let mut removed = false;
    for line in resolved_intent.lines() {
        if line.trim_start().starts_with("answer_candidate:") {
            removed = true;
            break;
        }
        stripped.push(line);
    }
    (stripped.join("\n").trim().to_string(), removed)
}

pub(super) fn chat_prompt_context_with_route_resolution(
    chat_prompt_context: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> String {
    let route_context = chat_route_resolution_context(agent_run_context);
    let recent_execution_context = chat_recent_execution_context(agent_run_context);
    if route_context.is_none() && recent_execution_context.is_none() {
        return chat_prompt_context.to_string();
    };
    let trimmed_context = chat_prompt_context.trim();
    let mut blocks = Vec::new();
    if !active_task_text_rewrite_context(agent_run_context)
        && !(trimmed_context.is_empty() || trimmed_context == "<none>")
    {
        blocks.push(chat_prompt_context.to_string());
    }
    if let Some(route_context) = route_context {
        blocks.push(route_context);
    }
    if let Some(recent_execution_context) = recent_execution_context {
        blocks.push(recent_execution_context);
    }
    blocks.join("\n\n")
}

pub(super) fn active_task_text_rewrite_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(analysis) = agent_run_context.and_then(|ctx| ctx.turn_analysis.as_ref()) else {
        return false;
    };
    if !matches!(
        analysis.target_task_policy,
        Some(crate::intent_router::TargetTaskPolicy::ReuseActive)
    ) {
        return false;
    }
    matches!(
        analysis.turn_type,
        Some(
            crate::intent_router::TurnType::TaskAppend
                | crate::intent_router::TurnType::TaskCorrect
                | crate::intent_router::TurnType::TaskReplace
                | crate::intent_router::TurnType::TaskScopeUpdate
        )
    )
}

pub(super) fn chat_recent_execution_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if active_task_text_rewrite_should_suppress_recent_execution_context(ctx) {
        return None;
    }
    if !chat_route_should_include_recent_execution_context(route) {
        return None;
    }
    let context = ctx
        .cross_turn_recent_execution_context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<none>")?;
    Some(format!(
        "### RECENT_EXECUTION_CONTEXT\nUse this observed execution context as evidence for this turn when the route contract or repaired route requires prior observed evidence. Do not invent details beyond it.\n{context}"
    ))
}

pub(super) fn active_task_text_rewrite_should_suppress_recent_execution_context(
    ctx: &crate::agent_engine::AgentRunContext,
) -> bool {
    let Some(route) = ctx.route_result.as_ref() else {
        return false;
    };
    active_task_text_rewrite_context(Some(ctx))
        && !route.output_contract.requires_content_evidence
        && !route_reason_has_exact_marker(route, "semantic_contract_requires_evidence")
        && !route.output_contract.delivery_required
        && route.output_contract.delivery_intent == crate::OutputDeliveryIntent::None
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
}

pub(super) fn chat_route_should_include_recent_execution_context(
    route: &crate::RouteResult,
) -> bool {
    route.output_contract.requires_content_evidence
        || route_reason_has_exact_marker(route, "semantic_contract_requires_evidence")
        || route_reason_has_exact_marker(route, "active_text_followup_route_repair")
}

pub(super) fn route_reason_has_exact_marker(route: &crate::RouteResult, marker: &str) -> bool {
    route
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| part == marker)
        || route.route_reason.trim() == marker
}

pub(super) fn chat_user_request<'a>(
    resolved_prompt: &'a str,
    execution_user_request: &'a str,
) -> &'a str {
    if execution_user_request.trim() != resolved_prompt.trim() {
        execution_user_request
    } else {
        resolved_prompt
    }
}

pub(super) fn direct_answer_chat_user_request(
    semantic_request: &str,
    original_user_request: &str,
    direct_answer_gate_approved: bool,
) -> String {
    if direct_answer_gate_approved {
        return semantic_request.to_string();
    }
    let (stripped, removed_answer_candidate) =
        strip_embedded_answer_candidate_from_intent(semantic_request);
    if removed_answer_candidate && !stripped.trim().is_empty() {
        stripped
    } else if removed_answer_candidate {
        original_user_request.to_string()
    } else {
        semantic_request.to_string()
    }
}

pub(super) fn chat_request_for_prompt(
    original_user_request: &str,
    semantic_request: &str,
) -> String {
    let original = original_user_request.trim();
    let semantic = semantic_request.trim();
    if original.is_empty() || original == semantic {
        return semantic.to_string();
    }
    format!(
        "Original user request:\n{original}\n\nResolved semantic intent / answer candidate:\n{semantic}\n\nUse the resolved semantic intent to answer the original request. If the original request asks for only a value, ID, path, name, or one short answer, output only the resolved value with no preamble."
    )
}

pub(super) fn direct_chat_answer_needs_repair(answer: &str) -> bool {
    let trimmed = answer.trim();
    trimmed.is_empty()
        || crate::finalize::looks_like_planner_artifact(trimmed)
        || crate::finalize::looks_like_internal_trace_artifact(trimmed)
        || direct_chat_answer_has_unclosed_code_fence(trimmed)
}

pub(super) fn direct_chat_answer_has_unclosed_code_fence(answer: &str) -> bool {
    let fence_count = answer
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("```"))
        .count();
    fence_count % 2 == 1
}

pub(super) fn direct_chat_answer_repair_prompt(chat_prompt: &str, rejected_answer: &str) -> String {
    format!(
        "{chat_prompt}\n\n### Previous Draft Rejected\nThe previous draft is malformed or incomplete and cannot be shown to the user:\n{rejected_answer}\n\nReturn only a complete final answer for the same user request. Do not use a code fence unless the user explicitly requested code."
    )
}

pub(super) fn active_task_rewrite_conservation_prompt(
    chat_prompt: &str,
    draft_answer: &str,
) -> String {
    format!(
        "{chat_prompt}\n\n### Active Task Rewrite Conservation\nThe previous draft may have added facts, instructions, examples, use cases, paths, docs/guides, setup steps, credential/setup detail categories, or operational claims that were not present in the most recent generated output:\n{draft_answer}\n\nRewrite the final answer for the same current user request using only the factual content already present in the most recent generated output plus the current style/length/audience constraint. Preserve any statement that concrete details were not observed. Return only the corrected final answer."
    )
}

pub(super) fn active_task_factual_rewrite_review_prompt(
    chat_prompt: &str,
    candidate_answer: &str,
) -> String {
    format!(
        "{chat_prompt}\n\n### Active Task Factual Rewrite Review\nReview the candidate answer against the active task context above, especially the most recent generated output. Determine whether the candidate adds concrete factual claims, setup/deployment/channel instructions, examples, paths, commands, product capabilities, operational guarantees, or policy/privacy claims that are not supported by the most recent generated output or other authoritative context in this prompt.\n\nCandidate answer:\n{candidate_answer}\n\nReturn only JSON with this shape:\n{{\"pass\":true|false,\"unsupported_claims\":[\"short machine-readable claim summaries\"],\"confidence\":0.0}}\nUse pass=true when the candidate only reformats, shortens, translates, changes audience, or changes medium while preserving supported facts. Use pass=false only when unsupported concrete claims are present."
    )
}

pub(super) fn active_task_factual_rewrite_repair_prompt(
    chat_prompt: &str,
    candidate_answer: &str,
    review: &ActiveTaskFactualRewriteReview,
) -> String {
    let claims = serde_json::to_string(&review.unsupported_claims).unwrap_or_else(|_| "[]".into());
    format!(
        "{chat_prompt}\n\n### Active Task Factual Rewrite Repair\nThe candidate answer was rejected by a structured factual-preservation review because it may add unsupported concrete claims.\n\nUnsupported claim summaries:\n{claims}\n\nCandidate answer:\n{candidate_answer}\n\nRewrite the final answer for the same current user request using only facts supported by the active task context above, especially the most recent generated output. Preserve the requested format, medium, length, and language. Return only the corrected final answer."
    )
}

pub(super) fn active_task_factual_rewrite_review_needs_repair(
    review: &ActiveTaskFactualRewriteReview,
) -> bool {
    !review.pass
        && review
            .unsupported_claims
            .iter()
            .map(|claim| claim.trim())
            .filter(|claim| !claim.is_empty())
            .take(8)
            .next()
            .is_some()
}

pub(super) async fn repair_active_task_factual_rewrite_if_needed(
    state: &AppState,
    task: &ClaimedTask,
    chat_prompt: &str,
    chat_prompt_source: &str,
    answer: String,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Result<String, String> {
    if !active_task_text_rewrite_context(agent_run_context) {
        return Ok(answer);
    }
    let review_prompt = active_task_factual_rewrite_review_prompt(chat_prompt, &answer);
    let raw_review = crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &review_prompt,
        chat_prompt_source,
    )
    .await
    .map_err(|e| e.to_string())?;
    let Some(review) =
        crate::parse_llm_json_raw_or_any::<ActiveTaskFactualRewriteReview>(&raw_review)
    else {
        tracing::warn!(
            "{} worker_once: ask active_task_factual_rewrite_review_parse_failed task_id={} raw={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&raw_review)
        );
        return Ok(answer);
    };
    if !active_task_factual_rewrite_review_needs_repair(&review) {
        return Ok(answer);
    }
    tracing::warn!(
        "{} worker_once: ask active_task_factual_rewrite_repair task_id={} unsupported_claims={}",
        crate::highlight_tag("routing"),
        task.task_id,
        crate::truncate_for_log(
            &serde_json::to_string(&review.unsupported_claims).unwrap_or_else(|_| "[]".into())
        )
    );
    let repair_prompt = active_task_factual_rewrite_repair_prompt(chat_prompt, &answer, &review);
    let repaired_answer = crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &repair_prompt,
        chat_prompt_source,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(ensure_active_task_required_visible_literals(
        repaired_answer,
        agent_run_context,
    ))
}

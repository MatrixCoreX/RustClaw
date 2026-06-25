use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActiveFileBasenameDirectAnswer {
    pub(super) answer: String,
    pub(super) evidence_path: String,
}

impl ActiveFileBasenameDirectAnswer {
    pub(super) fn observed_evidence(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "source": "active_execution_anchor",
            "action": "active_file_basename",
            "status": "ok",
            "path": self.evidence_path,
            "field_value": self.answer,
            "observed_evidence": {
                "extractor": {
                    "extractor_ref": "active_execution_anchor.file_basename.v1"
                },
                "items": [
                    {
                        "source": "active_execution_anchor",
                        "field": "field_value",
                        "value": self.answer
                    },
                    {
                        "source": "active_execution_anchor",
                        "field": "path",
                        "value": self.evidence_path
                    }
                ]
            }
        })
    }
}

pub(super) fn normalizer_answer_candidate_from_resolved_prompt(
    resolved_prompt: &str,
) -> Option<String> {
    let (_intent, candidate) = resolved_prompt.rsplit_once("\nanswer_candidate:")?;
    let candidate = crate::visible_text::strip_internal_context_sections(candidate).trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

pub(super) fn normalizer_answer_candidate_from_context_bundle_summary(
    summary: &str,
) -> Option<String> {
    summary
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("answer_candidate:").map(str::trim))
        .filter(|candidate| !candidate.is_empty())
        .map(ToString::to_string)
}

pub(super) fn paths_refer_to_same_existing_location(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

pub(super) fn normalizer_answer_candidate_matches_runtime_fact(
    state: &AppState,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() || candidate.contains('\n') {
        return false;
    }
    if normalizer_answer_candidate_matches_runtime_identity(candidate) {
        return true;
    }
    let candidate_path = Path::new(candidate);
    if !candidate_path.is_absolute() {
        return false;
    }
    if paths_refer_to_same_existing_location(candidate_path, &state.skill_rt.workspace_root) {
        return true;
    }
    std::env::current_dir()
        .ok()
        .is_some_and(|cwd| paths_refer_to_same_existing_location(candidate_path, &cwd))
}

pub(super) fn normalizer_answer_candidate_matches_runtime_identity(candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.contains('\n')
        || candidate.contains('/')
        || candidate.contains('\\')
    {
        return false;
    }
    ["USER", "LOGNAME", "USERNAME"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .any(|value| value == candidate)
}

pub(super) fn normalizer_answer_candidate_matches_runtime_memory_context(
    candidate: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() || candidate.contains('\n') || !distinctive_context_token(candidate) {
        return false;
    }
    agent_run_context
        .and_then(|ctx| ctx.memory_context_for_execution.as_deref())
        .map(str::trim)
        .filter(|memory_context| !memory_context.is_empty() && *memory_context != "<none>")
        .is_some_and(|memory_context| memory_context.contains(candidate))
}

pub(super) fn normalizer_answer_candidate_bound_anchor_basename(
    candidate: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let candidate = single_component_basename_candidate(candidate)?;
    let summary = agent_run_context?.context_bundle_summary.as_deref()?.trim();
    if summary.is_empty() {
        return None;
    }
    active_execution_anchor_targets(summary)
        .into_iter()
        .filter_map(|target| {
            Path::new(&target)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
        })
        .find(|basename| basename.eq_ignore_ascii_case(candidate))
}

pub(super) fn normalizer_answer_candidate_bound_recent_execution_basename(
    state: &AppState,
    candidate: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let candidate = single_component_basename_candidate(candidate)?;
    collect_recent_execution_request_file_targets(state, agent_run_context)
        .into_iter()
        .filter_map(|target| {
            Path::new(&target)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
        })
        .find(|basename| basename.eq_ignore_ascii_case(candidate))
}

pub(super) fn promote_clarify_recent_execution_judgment_context_to_chat(
    state: &AppState,
    agent_run_context: Option<&mut crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(ctx) = agent_run_context else {
        return false;
    };
    let route_allows_promotion = ctx.route_result.as_ref().is_some_and(|route| {
        route.needs_clarify
            && route.is_clarify_gate()
            && !route.wants_file_delivery
            && !route.output_contract.delivery_required
            && matches!(route.schedule_kind, crate::ScheduleKind::None)
            && matches!(
                route.output_contract.delivery_intent,
                crate::OutputDeliveryIntent::None
            )
            && route.output_contract.requires_content_evidence
            && matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::ExcerptKindJudgment
                    | crate::OutputSemanticKind::RecentArtifactsJudgment
            )
    });
    if !route_allows_promotion {
        return false;
    }
    if collect_recent_execution_request_file_targets(state, Some(&*ctx)).len() < 2 {
        return false;
    }
    let Some(route) = ctx.route_result.as_mut() else {
        return false;
    };
    route.set_chat_gate();
    route.needs_clarify = false;
    route.clarify_question.clear();
    append_route_reason(route, "clarify_recent_execution_judgment_to_chat");
    true
}

pub(crate) fn promote_active_anchor_observed_judgment_to_chat(
    current_user_request: &str,
    agent_run_context: Option<&mut crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(ctx) = agent_run_context else {
        return false;
    };
    let Some(summary) = ctx.context_bundle_summary.as_deref() else {
        return false;
    };
    if active_execution_anchor_ordered_entry_count(summary).is_none() {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_explicit_path_or_url()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference()
        || surface.has_structured_target_refinement()
    {
        return false;
    }
    let route_allows_promotion = ctx.route_result.as_ref().is_some_and(|route| {
        route.is_execute_gate()
            && !route.needs_clarify
            && !route.wants_file_delivery
            && !route.output_contract.delivery_required
            && matches!(route.schedule_kind, crate::ScheduleKind::None)
            && matches!(
                route.output_contract.delivery_intent,
                crate::OutputDeliveryIntent::None
            )
            && route.output_contract.requires_content_evidence
            && route.output_contract.locator_hint.trim().is_empty()
            && ask_route_reason_has_marker(
                route,
                "structured_anchor_direct_answer_requires_evidence",
            )
            && matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::None
                    | crate::OutputSemanticKind::ExcerptKindJudgment
                    | crate::OutputSemanticKind::RecentArtifactsJudgment
            )
    });
    if !route_allows_promotion {
        return false;
    }
    let Some(route) = ctx.route_result.as_mut() else {
        return false;
    };
    route.set_chat_gate();
    route.needs_clarify = false;
    route.clarify_question.clear();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    append_route_reason(route, "active_anchor_observed_judgment_to_chat");
    true
}

pub(super) fn promote_clarify_config_risk_assessment_default_config_to_planner(
    state: &AppState,
    current_user_request: &str,
    agent_run_context: Option<&mut crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(ctx) = agent_run_context else {
        return false;
    };
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_explicit_path_or_url() || surface.has_delivery_token_reference() {
        return false;
    }
    let default_config_path = state
        .skill_rt
        .workspace_root
        .join(RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH);
    if !default_config_path.is_file() {
        return false;
    }
    let route_allows_promotion = ctx.route_result.as_ref().is_some_and(|route| {
        route.needs_clarify
            && route.is_clarify_gate()
            && !route.wants_file_delivery
            && !route.output_contract.delivery_required
            && matches!(route.schedule_kind, crate::ScheduleKind::None)
            && matches!(
                route.output_contract.delivery_intent,
                crate::OutputDeliveryIntent::None
            )
            && route.output_contract.requires_content_evidence
            && route.output_contract.semantic_kind
                == crate::OutputSemanticKind::ConfigRiskAssessment
    });
    if !route_allows_promotion {
        return false;
    }
    let Some(route) = ctx.route_result.as_mut() else {
        return false;
    };
    route.set_execute_gate();
    route.needs_clarify = false;
    route.clarify_question.clear();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH.to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    append_route_reason(route, "config_risk_default_main_config_to_planner");
    true
}

pub(super) fn single_component_basename_candidate(candidate: &str) -> Option<&str> {
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

pub(super) fn active_execution_anchor_targets(summary: &str) -> Vec<String> {
    let mut in_active_anchor = false;
    let mut targets = Vec::new();
    for line in summary.lines() {
        let line = line.trim();
        if line == "### ACTIVE_EXECUTION_ANCHOR" {
            in_active_anchor = true;
            continue;
        }
        if in_active_anchor && line.starts_with("### ") {
            break;
        }
        if !in_active_anchor {
            continue;
        }
        if let Some(target) = line
            .strip_prefix("followup_bound_target:")
            .or_else(|| line.strip_prefix("observed_bound_target:"))
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            targets.push(target.to_string());
            continue;
        }
        if let Some(entries) = line
            .strip_prefix("followup_ordered_entries:")
            .or_else(|| line.strip_prefix("observed_ordered_entries:"))
        {
            targets.extend(active_anchor_ordered_entry_targets(entries));
        }
    }
    targets
}

pub(super) fn active_execution_anchor_bound_targets(summary: &str) -> Vec<String> {
    let mut in_active_anchor = false;
    let mut targets = Vec::new();
    for line in summary.lines() {
        let line = line.trim();
        if line == "### ACTIVE_EXECUTION_ANCHOR" {
            in_active_anchor = true;
            continue;
        }
        if in_active_anchor && line.starts_with("### ") {
            break;
        }
        if !in_active_anchor {
            continue;
        }
        if let Some(target) = line
            .strip_prefix("followup_bound_target:")
            .or_else(|| line.strip_prefix("observed_bound_target:"))
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            targets.push(target.to_string());
        }
    }
    targets
}

pub(super) fn active_execution_anchor_has_delivery_op(summary: &str) -> bool {
    let mut in_active_anchor = false;
    for line in summary.lines() {
        let line = line.trim();
        if line == "### ACTIVE_EXECUTION_ANCHOR" {
            in_active_anchor = true;
            continue;
        }
        if in_active_anchor && line.starts_with("### ") {
            break;
        }
        if !in_active_anchor {
            continue;
        }
        if line
            .strip_prefix("followup_op_kind:")
            .map(str::trim)
            .is_some_and(|op_kind| op_kind.eq_ignore_ascii_case("Delivery"))
        {
            return true;
        }
    }
    false
}

pub(super) fn ask_route_reason_has_marker(route: &crate::RouteResult, marker: &str) -> bool {
    route
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| part == marker || part.starts_with(&format!("{marker}:")))
}

pub(super) fn active_anchor_ordered_entry_targets(entries: &str) -> Vec<String> {
    entries
        .split(" | ")
        .filter_map(|entry| {
            let (ordinal, target) = entry.trim().split_once(':')?;
            ordinal
                .chars()
                .all(|ch| ch.is_ascii_digit())
                .then_some(target.trim())
        })
        .filter(|target| !target.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn active_execution_anchor_ordered_entry_count(summary: &str) -> Option<usize> {
    let mut in_active_anchor = false;
    let mut best_count = None;
    for line in summary.lines() {
        let line = line.trim();
        if line == "### ACTIVE_EXECUTION_ANCHOR" {
            in_active_anchor = true;
            continue;
        }
        if in_active_anchor && line.starts_with("### ") {
            break;
        }
        if !in_active_anchor {
            continue;
        }
        if let Some(entries) = line
            .strip_prefix("observed_ordered_entries:")
            .or_else(|| line.strip_prefix("followup_ordered_entries:"))
        {
            let count = active_anchor_ordered_entry_targets(entries).len();
            if count > 0 {
                best_count = Some(count);
            }
        }
    }
    best_count
}

pub(super) fn active_execution_anchor_evidence_lines(summary: &str) -> Vec<String> {
    let mut in_active_anchor = false;
    let mut lines = Vec::new();
    for line in summary.lines() {
        let line = line.trim();
        if line == "### ACTIVE_EXECUTION_ANCHOR" {
            in_active_anchor = true;
            continue;
        }
        if in_active_anchor && line.starts_with("### ") {
            break;
        }
        if !in_active_anchor || line.is_empty() {
            continue;
        }
        if line.starts_with("followup_")
            || line.starts_with("observed_")
            || line.starts_with("Active ordered-entry rule:")
        {
            lines.push(line.to_string());
        }
        if lines.len() >= 16 {
            break;
        }
    }
    lines
}

pub(crate) fn active_ordered_entries_count_direct_answer_candidate(
    current_user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if route.wants_file_delivery
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarCount
    {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_explicit_path_or_url()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference()
        || surface.has_structured_target_refinement()
    {
        return None;
    }
    let count = ctx
        .context_bundle_summary
        .as_deref()
        .and_then(active_execution_anchor_ordered_entry_count)?;
    Some(count.to_string())
}

pub(super) fn normalizer_bound_runtime_answer_candidate(
    state: &AppState,
    candidate: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let candidate = candidate.trim();
    if normalizer_answer_candidate_matches_runtime_fact(state, candidate)
        || normalizer_answer_candidate_matches_runtime_memory_context(candidate, agent_run_context)
    {
        return Some(candidate.to_string());
    }
    normalizer_answer_candidate_bound_anchor_basename(candidate, agent_run_context).or_else(|| {
        normalizer_answer_candidate_bound_recent_execution_basename(
            state,
            candidate,
            agent_run_context,
        )
    })
}

pub(super) fn normalizer_answer_candidate_matches_active_observation_synthesis(
    candidate: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.contains('\n')
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || candidate.contains("FILE:")
        || token_looks_like_pathlike_locator(candidate)
    {
        return false;
    }
    let Some(ctx) = agent_run_context else {
        return false;
    };
    if direct_answer_gate_has_recent_observed_result(ctx) {
        return true;
    }
    ctx.context_bundle_summary
        .as_deref()
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .is_some_and(|summary| {
            !active_execution_anchor_targets(summary).is_empty()
                || active_execution_anchor_ordered_entry_count(summary).is_some()
        })
}

pub(super) fn normalizer_answer_candidate_matches_bound_runtime_context(
    state: &AppState,
    candidate: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    normalizer_bound_runtime_answer_candidate(state, candidate, agent_run_context).is_some()
}

pub(super) fn normalizer_answer_candidate_matches_repaired_turn_binding(
    state: &AppState,
    route: &crate::RouteResult,
    candidate: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    const TURN_BINDING_REPAIR_MARKER: &str =
        "llm_semantic_contract_repair:contract_structurally_valid_but_turn_binding_invalid_active_task_context";
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.contains('\n')
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || !route_reason_has_exact_marker(route, TURN_BINDING_REPAIR_MARKER)
    {
        return false;
    }
    let pathlike_tokens = answer_candidate_pathlike_tokens(candidate);
    if pathlike_tokens.is_empty() {
        return false;
    }
    pathlike_tokens.into_iter().all(|token| {
        pathlike_token_matches_structured_context(state, route, agent_run_context, &token)
    })
}

pub(super) fn normalizer_answer_candidate_matches_context_turn_binding(
    state: &AppState,
    route: &crate::RouteResult,
    candidate: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.contains('\n')
        || candidate.starts_with('{')
        || candidate.starts_with('[')
    {
        return false;
    }
    let pathlike_tokens = answer_candidate_pathlike_tokens(candidate);
    if pathlike_tokens.is_empty() {
        return false;
    }
    pathlike_tokens.into_iter().all(|token| {
        pathlike_token_matches_structured_context(state, route, agent_run_context, &token)
    })
}

pub(super) fn pathlike_token_matches_structured_context(
    state: &AppState,
    route: &crate::RouteResult,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    token: &str,
) -> bool {
    pathlike_token_matches_target(token, route.output_contract.locator_hint.as_str())
        || agent_run_context
            .and_then(|ctx| ctx.auto_locator_path.as_deref())
            .is_some_and(|target| pathlike_token_matches_target(token, target))
        || agent_run_context
            .and_then(|ctx| ctx.context_bundle_summary.as_deref())
            .is_some_and(|summary| {
                active_execution_anchor_targets(summary)
                    .iter()
                    .any(|target| pathlike_token_matches_target(token, target))
            })
        || pathlike_token_is_existing_workspace_path(state, token)
}

pub(super) fn pathlike_token_matches_target(token: &str, target: &str) -> bool {
    let token = normalize_pathlike_binding_token(token);
    let target = normalize_pathlike_binding_token(target);
    if token.is_empty() || target.is_empty() {
        return false;
    }
    token == target
        || target.ends_with(&format!("/{token}"))
        || token
            .rsplit('/')
            .next()
            .filter(|basename| {
                basename.len() >= 3 && token_path_component_looks_structural(basename)
            })
            .is_some_and(|basename| target.ends_with(&format!("/{basename}")) || target == basename)
}

pub(super) fn normalize_pathlike_binding_token(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'))
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

pub(super) fn pathlike_token_is_existing_workspace_path(state: &AppState, token: &str) -> bool {
    let path = Path::new(token.trim());
    if !path.is_absolute() {
        return false;
    }
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    let Ok(workspace_root) = state.skill_rt.workspace_root.canonicalize() else {
        return false;
    };
    path.starts_with(workspace_root)
}

pub(super) fn normalizer_chat_direct_answer_candidate(
    state: &AppState,
    resolved_prompt: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    normalizer_chat_direct_answer_candidate_with_context_summary(
        state,
        resolved_prompt,
        agent_run_context,
        None,
        None,
    )
}

pub(super) fn normalizer_chat_direct_answer_candidate_with_context_summary(
    state: &AppState,
    resolved_prompt: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    context_bundle_summary_override: Option<&str>,
    current_user_request: Option<&str>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if route.needs_clarify || route.is_execute_gate() {
        return None;
    }
    if active_task_text_mutation_context(ctx) {
        return None;
    }
    let contract = &route.output_contract;
    if contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
    {
        return None;
    }
    let primary_candidate = normalizer_answer_candidate_from_resolved_prompt(resolved_prompt)
        .or_else(|| normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent));
    let context_candidate = context_bundle_summary_override
        .or_else(|| agent_run_context.and_then(|ctx| ctx.context_bundle_summary.as_deref()))
        .and_then(normalizer_answer_candidate_from_context_bundle_summary);
    let candidate = primary_candidate
        .clone()
        .or_else(|| context_candidate.clone())?;
    let bound_candidate =
        normalizer_bound_runtime_answer_candidate(state, &candidate, agent_run_context);
    if contract.requires_content_evidence {
        if bound_direct_answer_candidate_satisfies_output_contract(contract) {
            return bound_candidate;
        }
        return None;
    }
    if let Some(candidate) = normalizer_memory_ack_candidate_with_current_turn_token(
        &candidate,
        current_user_request,
        agent_run_context,
    ) {
        return Some(candidate);
    }
    if normalizer_answer_candidate_preserves_current_turn_machine_literals(
        &candidate,
        current_user_request,
    ) {
        return Some(candidate);
    }
    if normalizer_answer_candidate_matches_repaired_turn_binding(
        state,
        route,
        &candidate,
        agent_run_context,
    ) {
        return Some(candidate);
    }
    if context_candidate.as_deref() == Some(candidate.as_str())
        && normalizer_answer_candidate_matches_context_turn_binding(
            state,
            route,
            &candidate,
            agent_run_context,
        )
    {
        return Some(candidate);
    }
    bound_candidate.or_else(|| {
        normalizer_answer_candidate_matches_active_observation_synthesis(
            &candidate,
            agent_run_context,
        )
        .then_some(candidate)
    })
}

pub(super) fn normalizer_answer_candidate_preserves_current_turn_machine_literals(
    candidate: &str,
    current_user_request: Option<&str>,
) -> bool {
    let Some(request) = current_user_request else {
        return false;
    };
    let literals = current_turn_machine_literals(request);
    if literals.is_empty() {
        return false;
    }
    let candidate = candidate.to_ascii_lowercase();
    literals
        .iter()
        .all(|literal| candidate.contains(&literal.to_ascii_lowercase()))
}

pub(super) fn current_turn_machine_literals(text: &str) -> Vec<String> {
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

pub(super) fn current_turn_machine_literal(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    if token_looks_like_pathlike_locator(token) {
        return false;
    }
    if crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token) {
        return true;
    }
    if distinctive_context_token(token) {
        return true;
    }
    let has_alpha = token.chars().any(|ch| ch.is_ascii_alphabetic());
    let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
    let has_machine_separator = token.contains(['_', '-', '.', ':']);
    has_alpha && has_digit && has_machine_separator
}

pub(super) fn normalizer_memory_ack_candidate_with_current_turn_token(
    candidate: &str,
    current_user_request: Option<&str>,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.contains('\n')
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || !answer_candidate_pathlike_tokens(candidate).is_empty()
    {
        return None;
    }
    let ctx = agent_run_context?;
    let memory_ack = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.turn_type)
        == Some(crate::intent_router::TurnType::PreferenceOrMemory)
        || ctx
            .route_result
            .as_ref()
            .is_some_and(|route| route.should_refresh_long_term_memory);
    if !memory_ack {
        return None;
    }
    if ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|patch| patch.get("alias_bindings"))
        .is_some()
    {
        return Some(candidate.to_string());
    }
    let tokens = distinctive_context_tokens(current_user_request.unwrap_or_default())
        .into_iter()
        .filter(|token| !token_looks_like_pathlike_locator(token))
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return None;
    }
    let candidate_lower = candidate.to_ascii_lowercase();
    if tokens
        .iter()
        .any(|token| candidate_lower.contains(&token.to_ascii_lowercase()))
    {
        return Some(candidate.to_string());
    }
    let token = tokens
        .iter()
        .max_by_key(|token| token.chars().count())
        .map(String::as_str)?;
    Some(format!("{candidate} {token}"))
}

pub(super) fn normalizer_runtime_fact_direct_answer_candidate(
    state: &AppState,
    resolved_prompt: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    let contract = &route.output_contract;
    if route.is_execute_gate()
        || contract.requires_content_evidence
        || contract.delivery_required
        || route.wants_file_delivery
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
        || !matches!(contract.response_shape, crate::OutputResponseShape::Scalar)
        || !matches!(
            contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::ScalarPathOnly
        )
        || !matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return None;
    }
    let candidate = normalizer_answer_candidate_from_resolved_prompt(resolved_prompt)
        .or_else(|| normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent))?;
    normalizer_answer_candidate_matches_runtime_fact(state, &candidate).then_some(candidate)
}

pub(super) fn runtime_approval_wait_status_direct_answer_candidate(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    _language_hint: &str,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.wants_file_delivery
        || !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || route.schedule_kind != crate::ScheduleKind::None
        || route.risk_ceiling == crate::RiskCeiling::High
    {
        return None;
    }
    let status_query = ctx
        .turn_analysis
        .as_ref()
        .filter(|analysis| analysis.turn_type == Some(crate::intent_router::TurnType::StatusQuery))
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|state_patch| state_patch.get("runtime_status_query"))?;
    if status_query.get("kind").and_then(Value::as_str) != Some("approval_wait") {
        return None;
    }
    if status_query.get("scope").and_then(Value::as_str) != Some("current_task") {
        return None;
    }
    if !runtime_status_machine_delivery_requested(ctx) {
        return Some("approval_wait=false".to_string());
    }
    Some(
        serde_json::json!({
            "output_format": "machine_json",
            "owner_layer": "runtime_status_query",
            "runtime_status_query": {
                "kind": "approval_wait",
                "scope": "current_task"
            },
            "approval_wait": false,
            "state": "not_waiting_for_user_confirmation",
            "evidence_source": "turn_analysis.state_patch.runtime_status_query",
            "schema_version": 1
        })
        .to_string(),
    )
}

fn runtime_status_machine_delivery_requested(ctx: &crate::agent_engine::AgentRunContext) -> bool {
    ctx.turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|state_patch| state_patch.get("required_machine_fields"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(is_runtime_status_machine_field)
}

fn is_runtime_status_machine_field(raw: &str) -> bool {
    let field = raw
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '('));
    matches!(
        field,
        "runtime_status_query"
            | "runtime_status_query.kind"
            | "runtime_status_query.scope"
            | "approval_wait"
            | "state"
            | "not_waiting_for_user_confirmation"
    )
}

pub(super) fn runtime_scalar_path_direct_answer_candidate(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    if route.needs_clarify || !route.is_execute_gate() {
        return None;
    }
    let contract = &route.output_contract;
    if !matches!(
        contract.response_shape,
        crate::OutputResponseShape::Free
            | crate::OutputResponseShape::Scalar
            | crate::OutputResponseShape::Strict
    ) || contract.requires_content_evidence
        || !matches!(
            contract.semantic_kind,
            crate::OutputSemanticKind::ScalarPathOnly
        )
        || contract.delivery_required
        || route.wants_file_delivery
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
    {
        return None;
    }
    verified_runtime_scalar_locator_candidate(
        state,
        contract.locator_hint.trim(),
        contract.locator_kind,
    )
}

fn verified_runtime_scalar_locator_candidate(
    state: &AppState,
    locator: &str,
    locator_kind: crate::OutputLocatorKind,
) -> Option<String> {
    let locator = locator.trim();
    if locator.is_empty() {
        return None;
    }
    match locator_kind {
        crate::OutputLocatorKind::CurrentWorkspace => {
            if normalizer_answer_candidate_matches_runtime_fact(state, locator) {
                Some(locator.to_string())
            } else {
                verified_workspace_or_default_locator_path(state, locator)
            }
        }
        crate::OutputLocatorKind::Path => {
            verified_workspace_or_default_locator_path(state, locator)
        }
        _ => None,
    }
}

fn verified_workspace_or_default_locator_path(state: &AppState, locator: &str) -> Option<String> {
    let path = Path::new(locator.trim());
    if path.is_absolute() {
        return (path.exists()
            && (path_is_under_root(path, &state.skill_rt.workspace_root)
                || path_is_under_root(path, &state.skill_rt.default_locator_search_dir)))
        .then(|| locator.trim().to_string());
    }
    let workspace_path = state.skill_rt.workspace_root.join(path);
    if workspace_path.exists() {
        return Some(locator.trim().to_string());
    }
    let default_path = state.skill_rt.default_locator_search_dir.join(path);
    default_path.exists().then(|| locator.trim().to_string())
}

fn path_is_under_root(path: &Path, root: &Path) -> bool {
    match (path.canonicalize(), root.canonicalize()) {
        (Ok(path), Ok(root)) => path.starts_with(root),
        _ => false,
    }
}

#[cfg(test)]
pub(super) fn active_file_basename_direct_answer_candidate(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    active_file_basename_direct_answer(state, agent_run_context).map(|candidate| candidate.answer)
}

pub(super) fn active_file_basename_direct_answer(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<ActiveFileBasenameDirectAnswer> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    let summary = ctx.context_bundle_summary.as_deref().map(str::trim);
    let recent_execution_context = ctx
        .cross_turn_recent_execution_context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<none>");
    if summary.filter(|value| !value.is_empty()).is_none() && recent_execution_context.is_none() {
        return None;
    }
    let summary = summary.unwrap_or_default();
    let semantic_kind = route.output_contract.semantic_kind;
    let semantic_basename = semantic_kind == crate::OutputSemanticKind::FileBasename;
    let has_delivery_anchor = active_execution_anchor_has_delivery_op(summary);
    let active_delivery_direct_answer = has_delivery_anchor
        && ask_route_reason_has_marker(route, "active_task_mutation_to_direct_answer");
    let semantic_delivery_file_name =
        semantic_kind == crate::OutputSemanticKind::FileNames && has_delivery_anchor;
    let candidate_bound_basename = matches!(
        semantic_kind,
        crate::OutputSemanticKind::None | crate::OutputSemanticKind::ScalarPathOnly
    );
    let locator_requires_candidate_match = semantic_basename
        && has_delivery_anchor
        && route.output_contract.locator_kind == crate::OutputLocatorKind::Filename
        && !route.output_contract.locator_hint.trim().is_empty();
    let locator_ok = route.output_contract.locator_kind == crate::OutputLocatorKind::None
        || (semantic_basename
            && route.output_contract.locator_kind == crate::OutputLocatorKind::Filename
            && route.output_contract.locator_hint.trim().is_empty())
        || locator_requires_candidate_match
        || (candidate_bound_basename
            && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
            && route.output_contract.locator_hint.trim().is_empty());
    if ((route.wants_file_delivery || route.output_contract.delivery_required)
        && !active_delivery_direct_answer)
        || (route.output_contract.response_shape != crate::OutputResponseShape::Scalar
            && !(semantic_delivery_file_name
                && matches!(
                    route.output_contract.response_shape,
                    crate::OutputResponseShape::Free | crate::OutputResponseShape::Strict
                ))
            && !active_delivery_direct_answer)
        || (!semantic_basename
            && !candidate_bound_basename
            && !semantic_delivery_file_name
            && !active_delivery_direct_answer)
        || (!locator_ok && !active_delivery_direct_answer)
        || (route.output_contract.delivery_intent != crate::OutputDeliveryIntent::None
            && !active_delivery_direct_answer)
    {
        return None;
    }
    let mut candidates = Vec::new();
    for target in active_execution_anchor_bound_targets(summary) {
        push_existing_file_basename_answer_candidate(state, &mut candidates, &target);
    }
    if let Some(context) = recent_execution_context {
        for target in recent_execution_delivery_file_targets(state, context) {
            push_existing_file_basename_answer_candidate(state, &mut candidates, &target);
        }
    }
    if locator_requires_candidate_match {
        let Some(locator_basename) = route
            .output_contract
            .locator_hint
            .trim()
            .rsplit(['/', '\\'])
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return None;
        };
        candidates.retain(|candidate| candidate.answer.eq_ignore_ascii_case(locator_basename));
    }
    match candidates.as_slice() {
        [candidate]
            if semantic_basename
                || semantic_delivery_file_name
                || active_delivery_direct_answer =>
        {
            Some(candidate.clone())
        }
        _ => {
            let candidate =
                normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent)
                    .or_else(|| normalizer_answer_candidate_from_context_bundle_summary(summary))?;
            let candidate = single_component_basename_candidate(&candidate)?;
            candidates
                .into_iter()
                .find(|entry| entry.answer.eq_ignore_ascii_case(candidate))
        }
    }
}

fn push_existing_file_basename_answer_candidate(
    state: &AppState,
    candidates: &mut Vec<ActiveFileBasenameDirectAnswer>,
    target: &str,
) {
    let Some(candidate) = existing_file_basename_answer_candidate(state, target) else {
        return;
    };
    if !candidates
        .iter()
        .any(|existing| existing.answer.eq_ignore_ascii_case(&candidate.answer))
    {
        candidates.push(candidate);
    }
}

fn existing_file_basename_answer_candidate(
    state: &AppState,
    target: &str,
) -> Option<ActiveFileBasenameDirectAnswer> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let path = Path::new(target);
    let evidence_path = if path.is_file() {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    } else if let Ok(canonical) = path.canonicalize() {
        if !canonical.is_file() {
            return None;
        }
        canonical
    } else {
        let workspace_path = state.skill_rt.workspace_root.join(path);
        if !workspace_path.is_file() {
            return None;
        }
        workspace_path.canonicalize().unwrap_or(workspace_path)
    };
    let Some(basename) = evidence_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return None;
    };
    Some(ActiveFileBasenameDirectAnswer {
        answer: basename.to_string(),
        evidence_path: evidence_path.display().to_string(),
    })
}

pub(super) fn recent_execution_delivery_file_targets(
    state: &AppState,
    context: &str,
) -> Vec<String> {
    let mut targets = Vec::new();
    for value in recent_execution_result_values(context) {
        let Some(path) = crate::delivery_utils::extract_file_path_from_delivery_token(value.trim())
        else {
            continue;
        };
        let Some(resolved) = resolve_existing_recent_file_token(state, &path) else {
            continue;
        };
        if !targets.iter().any(|existing| existing == &resolved) {
            targets.push(resolved);
        }
    }
    targets
}

pub(super) fn recent_execution_result_values(context: &str) -> Vec<&str> {
    let mut values = Vec::new();
    for line in context
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some((_, value)) = line.split_once("latest_result=") {
            values.push(value.trim());
            continue;
        }
        if let Some((_, value)) = line.split_once(" result=") {
            values.push(value.trim());
        }
    }
    values
}

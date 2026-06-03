use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

use crate::{ActFinalizeStyle, AppState, AskReply, ClaimedTask};

const DIRECT_ANSWER_GATE_PROMPT_LOGICAL_PATH: &str = "prompts/direct_answer_gate_prompt.md";

#[derive(Debug, Clone, Deserialize)]
struct DirectAnswerGateOut {
    #[serde(default)]
    decision: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    clarify_question: String,
    #[serde(default)]
    resolved_user_intent: String,
    #[serde(default)]
    reference_resolution: DirectAnswerGateReferenceResolutionOut,
    output_contract: DirectAnswerGateContractOut,
}

#[derive(Debug, Clone, Deserialize)]
struct DirectAnswerGateContractOut {
    #[serde(default)]
    response_shape: String,
    #[serde(default)]
    exact_sentence_count: Option<usize>,
    #[serde(default)]
    requires_content_evidence: bool,
    #[serde(default)]
    delivery_required: bool,
    #[serde(default)]
    locator_kind: String,
    #[serde(default)]
    delivery_intent: String,
    #[serde(default)]
    semantic_kind: String,
    #[serde(default)]
    locator_hint: String,
    #[serde(default)]
    self_extension: DirectAnswerGateSelfExtensionOut,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DirectAnswerGateSelfExtensionOut {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    trigger: String,
    #[serde(default)]
    execute_now: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DirectAnswerGateReferenceResolutionOut {
    #[serde(default)]
    target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectAnswerGateDecision {
    DirectAnswer,
    PlannerExecute,
    Clarify,
}

enum DirectAnswerPreflight {
    DirectAnswer,
    PlannerExecute(crate::agent_engine::AgentRunContext),
    Clarify(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentCountObservation {
    target_label: String,
    total: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecentCountComparisonDirection {
    More,
    Less,
}

fn build_resume_continue_execute_prompt_from_parts(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
    resume_instruction: &str,
    resume_steps: Option<&Value>,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let resume_steps = resume_steps
        .cloned()
        .filter(|v| v.as_array().map(|arr| !arr.is_empty()).unwrap_or(false))
        .unwrap_or_else(|| {
            resume_context
                .get("remaining_actions")
                .cloned()
                .filter(|v| v.as_array().map(|arr| !arr.is_empty()).unwrap_or(false))
                .unwrap_or_else(|| {
                    resume_context
                        .get("remaining_steps")
                        .cloned()
                        .unwrap_or_else(|| json!([]))
                })
        });
    let resume_context_json =
        serde_json::to_string_pretty(resume_context).unwrap_or_else(|_| resume_context.to_string());
    let resume_steps_json =
        serde_json::to_string_pretty(&resume_steps).unwrap_or_else(|_| resume_steps.to_string());

    let (prompt_template, _) = crate::bootstrap::load_required_prompt_template_for_state(
        state,
        "prompts/resume_continue_execute_prompt.md",
    )?;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    Ok(crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text),
            ("__RESUME_CONTEXT__", &resume_context_json),
            ("__RESUME_STEPS__", &resume_steps_json),
            ("__RESUME_INSTRUCTION__", resume_instruction),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
        ],
    ))
}

fn normalizer_answer_candidate_from_resolved_prompt(resolved_prompt: &str) -> Option<String> {
    let (_intent, candidate) = resolved_prompt.rsplit_once("\nanswer_candidate:")?;
    let candidate = candidate.trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

fn normalizer_answer_candidate_from_context_bundle_summary(summary: &str) -> Option<String> {
    summary
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("answer_candidate:").map(str::trim))
        .filter(|candidate| !candidate.is_empty())
        .map(ToString::to_string)
}

fn paths_refer_to_same_existing_location(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn normalizer_answer_candidate_matches_runtime_fact(state: &AppState, candidate: &str) -> bool {
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

fn normalizer_answer_candidate_matches_runtime_identity(candidate: &str) -> bool {
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

fn normalizer_answer_candidate_matches_runtime_memory_context(
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

fn normalizer_answer_candidate_bound_anchor_basename(
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

fn single_component_basename_candidate(candidate: &str) -> Option<&str> {
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

fn active_execution_anchor_targets(summary: &str) -> Vec<String> {
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

fn active_execution_anchor_bound_targets(summary: &str) -> Vec<String> {
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

fn active_execution_anchor_has_delivery_op(summary: &str) -> bool {
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

fn ask_route_reason_has_marker(route: &crate::RouteResult, marker: &str) -> bool {
    route
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| part == marker || part.starts_with(&format!("{marker}:")))
}

fn active_anchor_ordered_entry_targets(entries: &str) -> Vec<String> {
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

fn active_execution_anchor_ordered_entry_count(summary: &str) -> Option<usize> {
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

fn normalizer_bound_runtime_answer_candidate(
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
    normalizer_answer_candidate_bound_anchor_basename(candidate, agent_run_context)
}

fn normalizer_answer_candidate_matches_active_observation_synthesis(
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

fn normalizer_answer_candidate_matches_bound_runtime_context(
    state: &AppState,
    candidate: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    normalizer_bound_runtime_answer_candidate(state, candidate, agent_run_context).is_some()
}

fn normalizer_answer_candidate_matches_repaired_turn_binding(
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

fn normalizer_answer_candidate_matches_context_turn_binding(
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

fn pathlike_token_matches_structured_context(
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

fn pathlike_token_matches_target(token: &str, target: &str) -> bool {
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

fn normalize_pathlike_binding_token(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'))
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn pathlike_token_is_existing_workspace_path(state: &AppState, token: &str) -> bool {
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

fn normalizer_chat_direct_answer_candidate(
    state: &AppState,
    resolved_prompt: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    normalizer_chat_direct_answer_candidate_with_context_summary(
        state,
        resolved_prompt,
        agent_run_context,
        None,
    )
}

fn normalizer_chat_direct_answer_candidate_with_context_summary(
    state: &AppState,
    resolved_prompt: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    context_bundle_summary_override: Option<&str>,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    if route.needs_clarify || route.is_execute_gate() {
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

fn normalizer_runtime_fact_direct_answer_candidate(
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

fn runtime_approval_wait_status_direct_answer_candidate(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    language_hint: &str,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if route.needs_clarify || route.is_execute_gate() {
        return None;
    }
    if route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
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
    Some(if language_hint == "en" {
        "No, I am not waiting for your approval.".to_string()
    } else {
        "不，我没有在等待你的批准。".to_string()
    })
}

fn runtime_scalar_path_direct_answer_candidate(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    if route.needs_clarify || !route.is_execute_gate() {
        return None;
    }
    let contract = &route.output_contract;
    if !matches!(contract.response_shape, crate::OutputResponseShape::Scalar)
        || !matches!(
            contract.semantic_kind,
            crate::OutputSemanticKind::ScalarPathOnly
        )
        || !matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        )
        || contract.delivery_required
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
    {
        return None;
    }
    let candidate = contract.locator_hint.trim();
    normalizer_answer_candidate_matches_runtime_fact(state, candidate)
        .then(|| candidate.to_string())
}

fn active_file_basename_direct_answer_candidate(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    let summary = ctx.context_bundle_summary.as_deref()?.trim();
    if summary.is_empty() {
        return None;
    }
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
    let locator_ok = route.output_contract.locator_kind == crate::OutputLocatorKind::None
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
    let mut basenames = Vec::new();
    for target in active_execution_anchor_bound_targets(summary) {
        let target = target.trim();
        if target.is_empty() {
            continue;
        }
        let path = Path::new(target);
        let is_existing_file = path.is_file()
            || path
                .canonicalize()
                .ok()
                .is_some_and(|canonical| canonical.is_file());
        let is_workspace_file =
            !is_existing_file && state.skill_rt.workspace_root.join(path).is_file();
        if !is_existing_file && !is_workspace_file {
            continue;
        }
        let Some(basename) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !basenames
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(basename))
        {
            basenames.push(basename.to_string());
        }
    }
    match basenames.as_slice() {
        [basename]
            if semantic_basename
                || semantic_delivery_file_name
                || active_delivery_direct_answer =>
        {
            Some(basename.clone())
        }
        _ => {
            let candidate =
                normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent)
                    .or_else(|| normalizer_answer_candidate_from_context_bundle_summary(summary))?;
            let candidate = single_component_basename_candidate(&candidate)?;
            basenames
                .into_iter()
                .find(|basename| basename.eq_ignore_ascii_case(candidate))
        }
    }
}

fn route_is_recent_count_comparison(
    current_user_request: &str,
    route: &crate::RouteResult,
    direction: RecentCountComparisonDirection,
) -> Option<RecentCountComparisonDirection> {
    if route.needs_clarify
        || route.wants_file_delivery
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_explicit_path_or_url()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference()
    {
        return None;
    }
    (route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison)
        .then_some(direction)
}

fn target_label_from_count_inventory_output(value: &Value) -> Option<String> {
    let raw = value
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| value.get("resolved_path").and_then(Value::as_str))?
        .trim();
    if raw.is_empty() || raw == "." {
        return None;
    }
    let trimmed = raw.trim_end_matches(['/', '\\']);
    let label = Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed);
    Some(label.to_string())
}

fn count_observation_from_output_excerpt(output_excerpt: &str) -> Option<RecentCountObservation> {
    let value: Value = serde_json::from_str(output_excerpt.trim()).ok()?;
    if value.get("action").and_then(Value::as_str) != Some("count_inventory") {
        return None;
    }
    let total = value
        .get("counts")
        .and_then(|counts| counts.get("total"))
        .and_then(Value::as_i64)?;
    let target_label = target_label_from_count_inventory_output(&value)?;
    Some(RecentCountObservation {
        target_label,
        total,
    })
}

fn count_observation_from_task_result_json(result_json: &str) -> Option<RecentCountObservation> {
    let value: Value = serde_json::from_str(result_json).ok()?;
    let steps = value
        .pointer("/task_journal/trace/step_results")
        .and_then(Value::as_array)?;
    steps.iter().rev().find_map(|step| {
        step.get("output_excerpt")
            .and_then(Value::as_str)
            .and_then(count_observation_from_output_excerpt)
    })
}

fn recent_count_observations_from_completed_tasks(
    state: &AppState,
    task: &ClaimedTask,
    limit: usize,
) -> Vec<RecentCountObservation> {
    let Ok(db) = state.core.db.get() else {
        return Vec::new();
    };
    let user_key = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("anon:{}:{}", task.user_id, task.chat_id));
    let Ok(mut stmt) = db.prepare(
        "SELECT result_json
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND COALESCE(user_key, '') = ?3
           AND kind = 'ask'
           AND status = 'succeeded'
           AND task_id != ?4
           AND result_json IS NOT NULL
         ORDER BY updated_at DESC
         LIMIT ?5",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map(
        rusqlite::params![
            task.user_id,
            task.chat_id,
            user_key,
            task.task_id,
            limit as i64
        ],
        |row| row.get::<_, String>(0),
    ) else {
        return Vec::new();
    };
    rows.filter_map(Result::ok)
        .filter_map(|result_json| count_observation_from_task_result_json(&result_json))
        .collect()
}

fn recent_count_comparison_direct_answer(
    state: &AppState,
    task: &ClaimedTask,
    current_user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    let direction = recent_count_selection_from_turn_analysis(ctx.turn_analysis.as_ref())?;
    let observations = recent_count_observations_from_completed_tasks(state, task, 8);
    let latest = observations.first()?;
    let previous = observations.get(1)?;
    let direction = route_is_recent_count_comparison(current_user_request, route, direction)?;
    recent_count_comparison_winner_label(latest, previous, direction)
}

fn direct_answer_gate_can_skip_for_recent_count_context(
    state: &AppState,
    task: &ClaimedTask,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
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
    let observations = recent_count_observations_from_completed_tasks(state, task, 4);
    let Some(latest) = observations.first() else {
        return false;
    };
    let Some(previous) = observations.get(1) else {
        return false;
    };
    !latest.target_label.trim().is_empty()
        && !previous.target_label.trim().is_empty()
        && latest.total != previous.total
}

fn recent_count_selection_from_turn_analysis(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<RecentCountComparisonDirection> {
    let quantity_comparison = turn_analysis?
        .state_patch
        .as_ref()?
        .get("quantity_comparison")?;
    if quantity_comparison.get("source").and_then(Value::as_str) != Some("recent_count_inventory") {
        return None;
    }
    let selection = quantity_comparison.get("selection")?.as_str()?;
    match selection {
        "max" => Some(RecentCountComparisonDirection::More),
        "min" => Some(RecentCountComparisonDirection::Less),
        _ => None,
    }
}

fn recent_count_comparison_winner_label(
    latest: &RecentCountObservation,
    previous: &RecentCountObservation,
    direction: RecentCountComparisonDirection,
) -> Option<String> {
    let winner = match direction {
        RecentCountComparisonDirection::More => match latest.total.cmp(&previous.total) {
            std::cmp::Ordering::Greater => latest,
            std::cmp::Ordering::Less => previous,
            std::cmp::Ordering::Equal => return None,
        },
        RecentCountComparisonDirection::Less => match latest.total.cmp(&previous.total) {
            std::cmp::Ordering::Less => latest,
            std::cmp::Ordering::Greater => previous,
            std::cmp::Ordering::Equal => return None,
        },
    };
    Some(winner.target_label.clone())
}

fn parse_direct_answer_gate_decision(raw: &str) -> DirectAnswerGateDecision {
    match raw.trim().to_ascii_lowercase().as_str() {
        "planner_execute" => DirectAnswerGateDecision::PlannerExecute,
        "clarify" => DirectAnswerGateDecision::Clarify,
        _ => DirectAnswerGateDecision::DirectAnswer,
    }
}

fn parse_gate_response_shape(raw: &str) -> crate::OutputResponseShape {
    match raw.trim().to_ascii_lowercase().as_str() {
        "one_sentence" => crate::OutputResponseShape::OneSentence,
        "strict" => crate::OutputResponseShape::Strict,
        "scalar" => crate::OutputResponseShape::Scalar,
        "file_token" => crate::OutputResponseShape::FileToken,
        _ => crate::OutputResponseShape::Free,
    }
}

fn parse_gate_locator_kind(raw: &str) -> crate::OutputLocatorKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "path" => crate::OutputLocatorKind::Path,
        "current_workspace" => crate::OutputLocatorKind::CurrentWorkspace,
        "url" => crate::OutputLocatorKind::Url,
        "filename" => crate::OutputLocatorKind::Filename,
        _ => crate::OutputLocatorKind::None,
    }
}

fn parse_gate_delivery_intent(raw: &str) -> crate::OutputDeliveryIntent {
    match raw.trim().to_ascii_lowercase().as_str() {
        "file_single" => crate::OutputDeliveryIntent::FileSingle,
        "directory_lookup" => crate::OutputDeliveryIntent::DirectoryLookup,
        "directory_batch_files" => crate::OutputDeliveryIntent::DirectoryBatchFiles,
        _ => crate::OutputDeliveryIntent::None,
    }
}

fn parse_gate_semantic_kind(raw: &str) -> crate::OutputSemanticKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "raw_command_output"
        | "raw_output"
        | "command_output"
        | "command_result"
        | "command_execution_result" => crate::OutputSemanticKind::RawCommandOutput,
        "command_output_summary" | "command_result_summary" | "command_output_synthesis" => {
            crate::OutputSemanticKind::CommandOutputSummary
        }
        "service_status" => crate::OutputSemanticKind::ServiceStatus,
        "hidden_entries_check" => crate::OutputSemanticKind::HiddenEntriesCheck,
        "file_names" => crate::OutputSemanticKind::FileNames,
        "directory_names" => crate::OutputSemanticKind::DirectoryNames,
        "directory_entry_groups" => crate::OutputSemanticKind::DirectoryEntryGroups,
        "file_paths" => crate::OutputSemanticKind::FilePaths,
        "directory_purpose_summary" => crate::OutputSemanticKind::DirectoryPurposeSummary,
        "content_excerpt_summary" => crate::OutputSemanticKind::ContentExcerptSummary,
        "content_excerpt_with_summary" => crate::OutputSemanticKind::ContentExcerptWithSummary,
        "content_presence_check" => crate::OutputSemanticKind::ContentPresenceCheck,
        "excerpt_kind_judgment" => crate::OutputSemanticKind::ExcerptKindJudgment,
        "recent_artifacts_judgment" => crate::OutputSemanticKind::RecentArtifactsJudgment,
        "workspace_project_summary" => crate::OutputSemanticKind::WorkspaceProjectSummary,
        "scalar_count" => crate::OutputSemanticKind::ScalarCount,
        "quantity_comparison" => crate::OutputSemanticKind::QuantityComparison,
        "execution_failed_step" => crate::OutputSemanticKind::ExecutionFailedStep,
        "generated_file_delivery" => crate::OutputSemanticKind::GeneratedFileDelivery,
        "filesystem_mutation_result" => crate::OutputSemanticKind::FilesystemMutationResult,
        "scalar_path_only" => crate::OutputSemanticKind::ScalarPathOnly,
        "file_basename" => crate::OutputSemanticKind::FileBasename,
        "existence_with_path" => crate::OutputSemanticKind::ExistenceWithPath,
        "existence_with_path_summary" => crate::OutputSemanticKind::ExistenceWithPathSummary,
        "recent_scalar_equality_check" => crate::OutputSemanticKind::RecentScalarEqualityCheck,
        "git_commit_subject" => crate::OutputSemanticKind::GitCommitSubject,
        "git_repository_state"
        | "git_workspace_state"
        | "git_state"
        | "git_status"
        | "git_branch"
        | "git_current_branch"
        | "git_remote"
        | "git_changed_files" => crate::OutputSemanticKind::GitRepositoryState,
        "structured_keys" => crate::OutputSemanticKind::StructuredKeys,
        "config_validation" | "structured_config_validation" => {
            crate::OutputSemanticKind::ConfigValidation
        }
        "config_mutation" | "config_write" | "config_set" | "structured_config_mutation" => {
            crate::OutputSemanticKind::ConfigMutation
        }
        "config_risk_assessment" | "config_risk" | "structured_config_risk" | "config_guard" => {
            crate::OutputSemanticKind::ConfigRiskAssessment
        }
        "sqlite_table_listing" => crate::OutputSemanticKind::SqliteTableListing,
        "sqlite_table_names_only" => crate::OutputSemanticKind::SqliteTableNamesOnly,
        "sqlite_database_kind_judgment" => crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
        "sqlite_schema_version" => crate::OutputSemanticKind::SqliteSchemaVersion,
        "rss_news_fetch" | "rss_latest_news" | "rss_feed_fetch" | "external_news_fetch" => {
            crate::OutputSemanticKind::RssNewsFetch
        }
        "web_page_summary"
        | "webpage_summary"
        | "web_content_summary"
        | "url_content_summary"
        | "browser_page_summary" => crate::OutputSemanticKind::WebPageSummary,
        "web_search_summary" | "web_search_results" | "search_results_summary" => {
            crate::OutputSemanticKind::WebSearchSummary
        }
        "weather_query" | "weather_current" | "weather_forecast" | "weather_report" => {
            crate::OutputSemanticKind::WeatherQuery
        }
        "market_quote" | "stock_quote" | "crypto_quote" | "asset_quote" | "market_price" => {
            crate::OutputSemanticKind::MarketQuote
        }
        "image_understanding"
        | "image_description"
        | "image_describe"
        | "image_vision"
        | "image_extract"
        | "image_compare"
        | "screenshot_summary" => crate::OutputSemanticKind::ImageUnderstanding,
        "publishing_preview" | "social_post_preview" | "channel_draft_preview" => {
            crate::OutputSemanticKind::PublishingPreview
        }
        "package_manager_detection" | "package_manager_detect" | "package_detect_manager" => {
            crate::OutputSemanticKind::PackageManagerDetection
        }
        "archive_list" => crate::OutputSemanticKind::ArchiveList,
        "archive_read" => crate::OutputSemanticKind::ArchiveRead,
        "archive_pack" => crate::OutputSemanticKind::ArchivePack,
        "archive_unpack" => crate::OutputSemanticKind::ArchiveUnpack,
        "docker_ps" => crate::OutputSemanticKind::DockerPs,
        "docker_images" => crate::OutputSemanticKind::DockerImages,
        "docker_logs" => crate::OutputSemanticKind::DockerLogs,
        "docker_container_lifecycle" => crate::OutputSemanticKind::DockerContainerLifecycle,
        _ => crate::OutputSemanticKind::None,
    }
}

fn parse_gate_self_extension(
    raw: DirectAnswerGateSelfExtensionOut,
) -> crate::SelfExtensionContract {
    let mode = match raw.mode.trim().to_ascii_lowercase().as_str() {
        "temporary_fix" => crate::SelfExtensionMode::TemporaryFix,
        "permanent_extension" => crate::SelfExtensionMode::PermanentExtension,
        _ => crate::SelfExtensionMode::None,
    };
    let trigger = match raw.trigger.trim().to_ascii_lowercase().as_str() {
        "explicit_user_request" => crate::SelfExtensionTrigger::ExplicitUserRequest,
        "capability_gap" => crate::SelfExtensionTrigger::CapabilityGap,
        _ => crate::SelfExtensionTrigger::None,
    };
    crate::SelfExtensionContract {
        mode,
        trigger,
        execute_now: raw.execute_now,
    }
}

fn output_contract_from_direct_answer_gate(
    raw: DirectAnswerGateContractOut,
    fallback: &crate::IntentOutputContract,
) -> crate::IntentOutputContract {
    crate::IntentOutputContract {
        response_shape: parse_gate_response_shape(&raw.response_shape),
        exact_sentence_count: raw.exact_sentence_count,
        requires_content_evidence: raw.requires_content_evidence,
        delivery_required: raw.delivery_required,
        locator_kind: parse_gate_locator_kind(&raw.locator_kind),
        delivery_intent: parse_gate_delivery_intent(&raw.delivery_intent),
        semantic_kind: parse_gate_semantic_kind(&raw.semantic_kind),
        locator_hint: raw.locator_hint.trim().to_string(),
        self_extension: parse_gate_self_extension(raw.self_extension),
    }
    .with_fallback_shape(fallback)
}

fn ordered_entry_looks_like_workspace_artifact(entry: &str) -> bool {
    let trimmed = entry.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return false;
    }
    let path = Path::new(trimmed);
    let has_filename_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.chars().any(|ch| ch.is_ascii_alphabetic()));
    has_filename_extension
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.starts_with('.')
}

fn direct_answer_candidate_looks_like_artifact_listing(resolved_prompt: &str) -> bool {
    let Some(candidate) = normalizer_answer_candidate_from_resolved_prompt(resolved_prompt) else {
        return false;
    };
    let entries = crate::followup_frame::extract_ordered_entries_from_text(&candidate);
    entries.len() >= 2
        && entries
            .iter()
            .all(|entry| ordered_entry_looks_like_workspace_artifact(entry))
}

fn trim_artifact_token(token: &str) -> &str {
    token.trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    })
}

fn text_mentions_artifact_locator(text: &str) -> bool {
    crate::delivery_utils::extract_filename_candidates(text)
        .iter()
        .any(|candidate| ordered_entry_looks_like_workspace_artifact(candidate))
        || text
            .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '，' | ';' | '；'))
            .map(trim_artifact_token)
            .any(ordered_entry_looks_like_workspace_artifact)
}

fn resolve_existing_recent_file_token(state: &AppState, token: &str) -> Option<String> {
    let token = trim_artifact_token(token);
    if token.is_empty() {
        return None;
    }
    let raw_path = Path::new(token);
    let mut candidates = Vec::new();
    if raw_path.is_absolute() {
        candidates.push(raw_path.to_path_buf());
    } else {
        candidates.push(state.skill_rt.workspace_root.join(raw_path));
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join(raw_path));
        }
    }
    for candidate in candidates {
        if candidate.is_file() {
            return Some(
                candidate
                    .canonicalize()
                    .unwrap_or(candidate)
                    .display()
                    .to_string(),
            );
        }
    }
    None
}

fn collect_recent_execution_request_file_targets(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Vec<String> {
    let Some(context) = agent_run_context
        .and_then(|ctx| ctx.cross_turn_recent_execution_context.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<none>")
    else {
        return Vec::new();
    };
    let section = context
        .split_once("### RECENT_EXECUTION_EVENTS")
        .map(|(_, tail)| tail)
        .unwrap_or(context);
    let mut targets = Vec::new();
    for line in section.lines() {
        let Some((_, request_tail)) = line.split_once(" request=") else {
            continue;
        };
        let request = request_tail
            .split(" result=")
            .next()
            .unwrap_or(request_tail)
            .trim();
        for token in request.split(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\''
                        | '`'
                        | ','
                        | '，'
                        | '。'
                        | ';'
                        | '；'
                        | '('
                        | ')'
                        | '（'
                        | '）'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | '《'
                        | '》'
                )
        }) {
            let Some(path) = resolve_existing_recent_file_token(state, token) else {
                continue;
            };
            if !targets.iter().any(|existing| existing == &path) {
                targets.push(path);
            }
        }
    }
    targets
}

fn direct_answer_gate_should_force_recent_file_context_execution(
    current_user_request: &str,
    resolved_prompt: &str,
    contract: &crate::IntentOutputContract,
    recent_request_file_target_count: usize,
) -> bool {
    if output_contract_requires_planner_execution(contract) {
        return false;
    }
    if recent_request_file_target_count < 2 {
        return false;
    }
    let Some(candidate) = normalizer_answer_candidate_from_resolved_prompt(resolved_prompt) else {
        return false;
    };
    if !text_mentions_artifact_locator(&candidate) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_concrete_locator_hint()
        || surface.has_structured_target_refinement()
        || surface.has_filename_candidates()
    {
        return false;
    }
    true
}

fn promote_artifact_listing_candidate_contract(
    resolved_prompt: &str,
    contract: &mut crate::IntentOutputContract,
) -> bool {
    if output_contract_requires_planner_execution(contract)
        || !direct_answer_candidate_looks_like_artifact_listing(resolved_prompt)
    {
        return false;
    }
    contract.requires_content_evidence = true;
    contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    if matches!(
        contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        contract.response_shape = crate::OutputResponseShape::Strict;
    }
    true
}

fn output_contract_requires_planner_execution(contract: &crate::IntentOutputContract) -> bool {
    contract.requires_content_evidence
        || contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
        || !matches!(contract.semantic_kind, crate::OutputSemanticKind::None)
}

fn bound_direct_answer_candidate_satisfies_output_contract(
    contract: &crate::IntentOutputContract,
) -> bool {
    !contract.delivery_required
        && matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        && matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
        && matches!(contract.semantic_kind, crate::OutputSemanticKind::None)
}

fn transform_skill_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("transform")
}

fn package_manager_skill_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("package_manager")
}

fn package_manager_skill_supports_detection(state: &AppState) -> bool {
    if !package_manager_skill_available_for_plan(state) {
        return false;
    }
    let Some(manifest) = state.skill_manifest("package_manager") else {
        return true;
    };
    manifest
        .semantic_tags
        .iter()
        .any(|tag| tag == "package_manager_detection")
        || manifest
            .planner_capabilities
            .iter()
            .any(|capability| capability.name == "package.detect_manager")
}

fn output_contract_requests_package_manager_detection(
    contract: &crate::IntentOutputContract,
) -> bool {
    matches!(
        contract.semantic_kind,
        crate::OutputSemanticKind::PackageManagerDetection
    )
}

fn route_has_package_manager_install_preview_candidate(route: &crate::RouteResult) -> bool {
    normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent).is_some_and(
        |candidate| {
            crate::package_commands::package_install_packages_from_commandish_text(&candidate)
                .is_some()
        },
    )
}

fn direct_answer_gate_can_skip_for_self_contained_payload(
    current_user_request: &str,
    route: Option<&crate::RouteResult>,
) -> bool {
    let Some(route) = route else {
        return false;
    };
    if normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent).is_none() {
        return false;
    }
    if route.needs_clarify
        || route.is_execute_gate()
        || route
            .route_confidence
            .is_none_or(|confidence| confidence < 0.80)
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(&route.output_contract)
        || !route.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route.output_contract.self_extension.mode,
            crate::SelfExtensionMode::None
        )
        || !matches!(
            route.output_contract.self_extension.trigger,
            crate::SelfExtensionTrigger::None
        )
        || route.output_contract.self_extension.execute_now
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if crate::intent::surface_signals::inline_json_transform_request(current_user_request) {
        return false;
    }
    surface.inline_json_shape.is_some()
        && !surface.has_explicit_path_or_url()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
}

fn normalized_workspace_identity_token(text: &str) -> String {
    text.chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
                Some(ch)
            } else {
                None
            }
        })
        .collect()
}

fn workspace_identity_tokens(state: &AppState) -> Vec<String> {
    let Some(root_name) = state
        .skill_rt
        .workspace_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return Vec::new();
    };
    let token = normalized_workspace_identity_token(root_name);
    if token.chars().count() < 4 {
        return Vec::new();
    }
    vec![token]
}

fn current_request_mentions_workspace_identity(
    state: &AppState,
    current_user_request: &str,
) -> bool {
    let request = normalized_workspace_identity_token(current_user_request);
    if request.is_empty() {
        return false;
    }
    workspace_identity_tokens(state)
        .into_iter()
        .any(|token| request.contains(&token))
}

fn direct_answer_gate_can_skip_for_pure_chat_draft(
    state: &AppState,
    current_user_request: &str,
    route: Option<&crate::RouteResult>,
) -> bool {
    let Some(route) = route else {
        return false;
    };
    if normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent).is_none()
        || route.needs_clarify
        || route.is_execute_gate()
        || route
            .route_confidence
            .is_none_or(|confidence| confidence < 0.80)
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
        || route.output_contract.requires_content_evidence
        || !direct_answer_gate_contract_is_pure_chat(&route.output_contract)
    {
        return false;
    }
    if current_request_mentions_workspace_identity(state, current_user_request) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.inline_json_shape.is_none()
        && !crate::intent::surface_signals::inline_json_transform_request(current_user_request)
        && !surface.has_any_locator_reference()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && !surface.has_structured_target_refinement()
        && !surface.has_deictic_reference()
        && surface.locator_target_pair.is_none()
}

fn direct_answer_gate_can_skip_for_active_task_text_mutation(
    current_user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(ctx) = agent_run_context else {
        return false;
    };
    let Some(route) = ctx.route_result.as_ref() else {
        return false;
    };
    let Some(analysis) = ctx.turn_analysis.as_ref() else {
        return false;
    };
    if route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(&route.output_contract)
        || !route.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route.output_contract.self_extension.mode,
            crate::SelfExtensionMode::None
        )
        || !matches!(
            route.output_contract.self_extension.trigger,
            crate::SelfExtensionTrigger::None
        )
        || route.output_contract.self_extension.execute_now
        || analysis.attachment_processing_required
    {
        return false;
    }
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

    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !surface.has_explicit_path_or_url()
        && surface.locator_target_pair.is_none()
        && surface.field_selector_count == 0
        && surface.dotted_field_selector.is_none()
        && !surface.has_delivery_token_reference()
        && surface
            .filename_candidates_excluding_field_selectors()
            .is_empty()
}

fn direct_answer_gate_can_skip_for_active_observed_output_chat_repair(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    route
        .route_reason
        .contains("active_observed_output_chat_repair")
        && !route.needs_clarify
        && !route.is_execute_gate()
        && !route.wants_file_delivery
        && matches!(route.schedule_kind, crate::ScheduleKind::None)
        && !output_contract_requires_planner_execution(&route.output_contract)
        && route.output_contract.locator_hint.trim().is_empty()
}

fn contract_test_hint_requests_planner_execution(current_user_request: &str) -> bool {
    if crate::intent_router::contract_test_hint_semantic_kind(current_user_request).is_some() {
        return true;
    }
    if crate::intent_router::contract_test_hint_value(current_user_request, "none_passthrough")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return false;
    }
    let allowed_actions = crate::intent_router::contract_test_hint_value(
        current_user_request,
        "allowed_actions_json",
    )
    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    .and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|item| !item.trim().is_empty())
        })
    })
    .unwrap_or(false);
    let required_evidence = crate::intent_router::contract_test_hint_value(
        current_user_request,
        "required_evidence_json",
    )
    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    .and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|item| !item.trim().is_empty())
        })
    })
    .unwrap_or(false);
    allowed_actions || required_evidence
}

fn contract_test_hint_should_enter_planner_loop(
    current_user_request: &str,
    ctx: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    if !contract_test_hint_requests_planner_execution(current_user_request) {
        return false;
    }
    ctx.and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            !route.needs_clarify
                && (route.is_execute_gate()
                    || route.output_contract.requires_content_evidence
                    || route.output_contract.delivery_required
                    || route.wants_file_delivery)
        })
}

fn contract_test_hint_forced_planner_preflight(
    ctx: &mut crate::agent_engine::AgentRunContext,
    current_user_request: &str,
    reason_tag: &str,
) -> Option<DirectAnswerPreflight> {
    if !contract_test_hint_should_enter_planner_loop(current_user_request, Some(ctx)) {
        return None;
    }
    if let Some(route) = ctx.route_result.as_mut() {
        let finalize_style = planner_finalize_style_for_output_contract(&route.output_contract);
        route.set_planner_execute_finalize(finalize_style);
        route.needs_clarify = false;
        route.clarify_question.clear();
        append_route_reason(route, reason_tag);
    }
    Some(DirectAnswerPreflight::PlannerExecute(ctx.clone()))
}

fn direct_answer_gate_promotion_depends_only_on_background_context(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    promoted_contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    has_structural_session_alias_target: bool,
) -> bool {
    if has_structural_session_alias_target {
        return false;
    }
    let Some(candidate) = normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent)
    else {
        return false;
    };
    if route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(&route.output_contract)
        || !route.output_contract.locator_hint.trim().is_empty()
        || !output_contract_requires_planner_execution(promoted_contract)
        || text_mentions_artifact_locator(&candidate)
    {
        return false;
    }
    if (direct_answer_gate_contract_allows_locatorless_execution(
        state,
        current_user_request,
        promoted_contract,
    ) || (package_manager_skill_available_for_plan(state)
        && route_has_package_manager_install_preview_candidate(route)))
        && !direct_answer_gate_reference_requires_clarify(reference_resolution)
    {
        return false;
    }

    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !direct_answer_gate_reference_is_present(reference_resolution)
        && !current_request_mentions_resolvable_gate_locator(
            state,
            current_user_request,
            promoted_contract,
        )
        && !surface.has_explicit_path_or_url()
        && surface.locator_target_pair.is_none()
        && surface.field_selector_count == 0
        && surface.dotted_field_selector.is_none()
        && !surface.has_delivery_token_reference()
        && surface
            .filename_candidates_excluding_field_selectors()
            .is_empty()
}

fn direct_answer_gate_promotion_needs_unbound_deictic_clarify(
    state: &AppState,
    current_user_request: &str,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
    has_structural_session_alias_target: bool,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> bool {
    if !output_contract_requires_planner_execution(contract) {
        return false;
    }
    let reference_requires_clarify =
        direct_answer_gate_reference_requires_clarify(reference_resolution);
    if !(matches!(
        contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::Url
    ) || (contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && reference_requires_clarify))
    {
        return false;
    }
    if current_request_has_direct_answer_gate_locator_surface(state, current_user_request, contract)
    {
        return false;
    }
    if has_authoritative_deictic_anchor || has_structural_session_alias_target {
        return false;
    }
    if auto_locator_path.is_some_and(|path| !path.trim().is_empty()) {
        return false;
    }
    true
}

fn direct_answer_gate_untrusted_locator_hint_requires_clarify(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
    has_structural_session_alias_target: bool,
) -> bool {
    if !contract.requires_content_evidence
        || contract.locator_hint.trim().is_empty()
        || !matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
                | crate::OutputLocatorKind::CurrentWorkspace
        )
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            contract,
        )
        || has_authoritative_deictic_anchor
        || has_structural_session_alias_target
        || auto_locator_path.is_some_and(|path| !path.trim().is_empty())
    {
        return false;
    }
    matches!(
        direct_answer_gate_reference_target(reference_resolution),
        "" | "none"
            | "current_turn_locator"
            | "unresolved_prior_object"
            | "missing_locator"
            | "ambiguous_locator"
    )
}

fn current_request_has_direct_answer_gate_locator_surface(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.has_concrete_locator_hint()
        || surface.has_structured_target_refinement()
        || surface.has_delivery_token_reference()
        || (contract.requires_content_evidence
            && matches!(
                contract.locator_kind,
                crate::OutputLocatorKind::Path
                    | crate::OutputLocatorKind::Filename
                    | crate::OutputLocatorKind::CurrentWorkspace
            )
            && current_request_mentions_resolvable_gate_locator(
                state,
                current_user_request,
                contract,
            ))
}

fn current_request_mentions_resolvable_gate_locator(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> bool {
    contract.requires_content_evidence
        && matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
        && locator_hint_mentions_current_request(&contract.locator_hint, current_user_request)
        && resolve_gate_locator_from_hint_or_request(state, current_user_request, contract)
            .is_some()
}

fn resolve_gate_locator_from_hint_or_request(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> Option<String> {
    let locator_kind = if contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace {
        crate::OutputLocatorKind::Path
    } else {
        contract.locator_kind
    };
    crate::worker::try_resolve_implicit_locator_path(
        state,
        current_user_request,
        contract.locator_hint.trim(),
        locator_kind,
        None,
    )
    .and_then(|resolution| match resolution {
        crate::worker::LocatorAutoResolution::Direct(path) => Some(path),
        crate::worker::LocatorAutoResolution::Fuzzy(_) => None,
    })
    .or_else(|| {
        crate::worker::try_resolve_workspace_child_locator_from_text(
            &state.skill_rt.workspace_root,
            &state.skill_rt.default_locator_search_dir,
            current_user_request,
        )
    })
}

fn locator_hint_mentions_current_request(locator_hint: &str, current_user_request: &str) -> bool {
    let request_lower = current_user_request.to_ascii_lowercase();
    locator_hint
        .split(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    ',' | ';'
                        | ':'
                        | '|'
                        | '/'
                        | '\\'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '，'
                        | '、'
                        | '；'
                        | '：'
                )
        })
        .map(|token| token.trim_matches(|ch: char| ch == '"' || ch == '\'' || ch == '`'))
        .filter(|token| token.len() >= 3)
        .any(|token| request_lower.contains(&token.to_ascii_lowercase()))
}

fn direct_answer_route_introduces_unmentioned_distinctive_context_target(
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> bool {
    distinctive_context_tokens(&direct_answer_gate_context_target_text(route, gate))
        .into_iter()
        .any(|token| !distinctive_token_present_in_request(current_user_request, &token))
}

fn direct_answer_gate_context_target_text(
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> String {
    let mut text = String::new();
    let (resolved_intent, _) = strip_embedded_answer_candidate_from_intent(&route.resolved_intent);
    text.push_str(&resolved_intent);
    text.push('\n');
    text.push_str(&route.route_reason);
    text.push('\n');
    text.push_str(&gate.resolved_user_intent);
    text.push('\n');
    text.push_str(&gate.reason);
    text
}

fn direct_answer_route_introduces_unmentioned_locatorlike_context_target(
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> bool {
    let text = direct_answer_gate_context_target_text(route, gate);
    if answer_candidate_introduces_unmentioned_pathlike_target(current_user_request, &text) {
        return true;
    }
    crate::delivery_utils::extract_filename_candidates(&text)
        .into_iter()
        .filter(|candidate| {
            !crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(candidate)
        })
        .any(|candidate| !distinctive_token_present_in_request(current_user_request, &candidate))
}

fn distinctive_context_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| {
        !(ch.is_ascii_alphanumeric()
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || matches!(ch, '_' | '-' | '/' | '.' | ':'))
    })
    .map(|token| token.trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '.' | ':')))
    .filter(|token| distinctive_context_token(token))
    .map(ToOwned::to_owned)
    .collect()
}

fn distinctive_context_token(token: &str) -> bool {
    let signal_chars = token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    let has_identifier_separator = token.contains(['_', '/', '.', ':']);
    let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
    (signal_chars >= 4 && has_identifier_separator)
        || (signal_chars >= 8 && has_digit)
        || signal_chars >= 16
}

fn distinctive_token_present_in_request(request: &str, token: &str) -> bool {
    let request = request.to_ascii_lowercase();
    let token = token.to_ascii_lowercase();
    if request.contains(&token) {
        return true;
    }
    token
        .split(['_', '-', '/', '.', ':'])
        .filter(|part| part.len() >= 3)
        .any(|part| request.contains(part))
}

fn answer_candidate_pathlike_tokens(candidate: &str) -> Vec<String> {
    candidate
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                ch.is_ascii_punctuation() && !matches!(ch, '/' | '\\' | '.' | '_' | '-' | '~' | ':')
            })
        })
        .filter(|token| token_looks_like_pathlike_locator(token))
        .filter(|token| distinctive_context_token(token))
        .map(ToOwned::to_owned)
        .collect()
}

fn token_looks_like_pathlike_locator(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return false;
    }
    if token.contains("://")
        || token.contains('\\')
        || token.starts_with("~/")
        || token.starts_with("./")
        || token.starts_with("../")
        || (token.starts_with('/') && token.len() > 1)
    {
        return true;
    }
    let bytes = token.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
    {
        return true;
    }
    if !token.contains('/') {
        return false;
    }
    let parts = token.split('/').collect::<Vec<_>>();
    parts.len() >= 2
        && parts
            .iter()
            .all(|part| token_path_component_looks_structural(part))
}

fn token_path_component_looks_structural(part: &str) -> bool {
    let part = part.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'));
    !part.is_empty()
        && part.chars().any(|ch| ch.is_ascii_alphanumeric())
        && part
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn answer_candidate_introduces_unmentioned_pathlike_target(
    current_user_request: &str,
    candidate: &str,
) -> bool {
    let request = current_user_request.to_ascii_lowercase();
    answer_candidate_pathlike_tokens(candidate)
        .into_iter()
        .map(|token| token.to_ascii_lowercase())
        .any(|token| {
            if request.contains(&token) {
                return false;
            }
            let basename = token
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(token.as_str())
                .trim();
            basename.is_empty() || !request.contains(basename)
        })
}

fn direct_answer_gate_contract_is_pure_chat(contract: &crate::IntentOutputContract) -> bool {
    !output_contract_requires_planner_execution(contract)
        && !matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        && contract.locator_hint.trim().is_empty()
        && !contract.self_extension.execute_now
        && matches!(contract.self_extension.mode, crate::SelfExtensionMode::None)
        && matches!(
            contract.self_extension.trigger,
            crate::SelfExtensionTrigger::None
        )
}

fn direct_answer_gate_self_contained_inline_json_chat(current_user_request: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.inline_json_shape.is_some()
        && !crate::intent::surface_signals::inline_json_transform_request(current_user_request)
        && !surface.has_explicit_path_or_url()
        && !surface.has_delivery_token_reference()
        && surface.locator_target_pair.is_none()
}

fn direct_answer_gate_contextual_inline_structured_payload_execute(
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.inline_json_shape.is_some()
        && !crate::intent::surface_signals::inline_json_transform_request(current_user_request)
        && !surface.has_explicit_path_or_url()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && surface.locator_target_pair.is_none()
        && contract.requires_content_evidence
        && !contract.delivery_required
        && matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
        && matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        && contract.locator_hint.trim().is_empty()
}

fn direct_answer_gate_embedded_inline_json_payload_surface(current_user_request: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    matches!(
        surface.inline_json_shape,
        Some(crate::intent::surface_signals::InlineJsonShape::EmbeddedPayload)
    ) && !surface.has_explicit_path_or_url()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && surface.locator_target_pair.is_none()
}

fn direct_answer_gate_structured_transform_candidate(answer_candidate: &str) -> bool {
    let trimmed = answer_candidate.trim();
    if trimmed.is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .is_some_and(|value| {
            matches!(
                value,
                serde_json::Value::Array(_) | serde_json::Value::Object(_)
            )
        })
        || direct_answer_gate_answer_candidate_is_markdown_table(trimmed)
}

fn direct_answer_gate_answer_candidate_is_markdown_table(candidate: &str) -> bool {
    let lines = candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines.len() >= 2
        && lines
            .first()
            .is_some_and(|line| line.starts_with('|') && line.ends_with('|'))
        && lines
            .get(1)
            .is_some_and(|line| line.chars().all(|ch| matches!(ch, '|' | '-' | ':' | ' ')))
}

fn direct_answer_gate_has_recent_observed_result(
    ctx: &crate::agent_engine::AgentRunContext,
) -> bool {
    let Some(context) = ctx
        .cross_turn_recent_execution_context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<none>")
    else {
        return false;
    };
    context.contains("latest_result=") || context.contains(" result=")
}

fn direct_answer_gate_existing_observed_result_should_stay_chat(
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
    ctx: &crate::agent_engine::AgentRunContext,
) -> bool {
    if route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || !route.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ContentPresenceCheck
        )
        || !direct_answer_gate_has_recent_observed_result(ctx)
    {
        return false;
    }
    let reference_target = gate.reference_resolution.target.trim();
    let normalizer_answer_candidate_present =
        normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent).is_some();
    let reference_binds_observed_result = matches!(
        reference_target,
        "current_action_result" | "comparison_result"
    ) || (normalizer_answer_candidate_present
        && matches!(
            reference_target,
            "unresolved_prior_object" | "missing_locator" | "ambiguous_locator"
        ));
    if !reference_binds_observed_result {
        return false;
    }
    let gate_locator_kind = gate.output_contract.locator_kind.trim();
    let gate_delivery_intent = gate.output_contract.delivery_intent.trim();
    let gate_locator_hint = gate.output_contract.locator_hint.trim();
    let gate_semantic_kind = gate.output_contract.semantic_kind.trim();
    if gate.output_contract.delivery_required
        || !gate_locator_kind.is_empty() && gate_locator_kind != "none"
        || !gate_delivery_intent.is_empty() && gate_delivery_intent != "none"
        || !gate_locator_hint.is_empty()
        || !matches!(
            gate_semantic_kind,
            "" | "none"
                | "content_excerpt_summary"
                | "raw_command_output"
                | "command_output_summary"
                | "content_presence_check"
        )
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !surface.has_concrete_locator_hint()
        && !surface.has_explicit_path_or_url()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && !surface.has_structured_target_refinement()
        && surface.locator_target_pair.is_none()
        && !crate::intent::surface_signals::inline_json_transform_request(current_user_request)
}

fn direct_answer_gate_allows_contextual_chat_reference(
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> bool {
    if parse_direct_answer_gate_decision(&gate.decision) != DirectAnswerGateDecision::DirectAnswer
        || route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || direct_answer_gate_reference_requires_clarify(&gate.reference_resolution)
        || !gate.clarify_question.trim().is_empty()
        || direct_answer_route_introduces_unmentioned_locatorlike_context_target(
            current_user_request,
            route,
            gate,
        )
    {
        return false;
    }
    let gate_contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &route.output_contract,
    );
    if !direct_answer_gate_contract_is_pure_chat(&route.output_contract)
        || !direct_answer_gate_contract_is_pure_chat(&gate_contract)
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if direct_answer_gate_self_contained_inline_json_chat(current_user_request) {
        return true;
    }
    !surface.has_concrete_locator_hint()
        && !surface.is_structural_locator_only_reply()
        && !surface.has_structured_target_refinement()
        && !surface.has_delivery_token_reference()
        && !surface.has_filename_candidates()
        && surface.locator_target_pair.is_none()
}

fn direct_answer_gate_candidate_needs_unbound_context_clarify(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
    has_structural_session_alias_target: bool,
    normalizer_candidate_matches_bound_context: bool,
) -> bool {
    let candidate = normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent);
    if route.needs_clarify
        || route.is_execute_gate()
        || has_authoritative_deictic_anchor
        || has_structural_session_alias_target
        || auto_locator_path.is_some_and(|path| !path.trim().is_empty())
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            &route.output_contract,
        )
    {
        return false;
    }
    let Some(candidate) = candidate else {
        let gate_contract = output_contract_from_direct_answer_gate(
            gate.output_contract.clone(),
            &route.output_contract,
        );
        if direct_answer_gate_self_contained_inline_json_chat(current_user_request)
            && parse_direct_answer_gate_decision(&gate.decision)
                == DirectAnswerGateDecision::DirectAnswer
            && gate.clarify_question.trim().is_empty()
            && !direct_answer_gate_reference_requires_clarify(&gate.reference_resolution)
            && direct_answer_gate_contract_is_pure_chat(&route.output_contract)
            && direct_answer_gate_contract_is_pure_chat(&gate_contract)
        {
            return false;
        }
        if direct_answer_gate_allows_contextual_chat_reference(current_user_request, route, gate) {
            return false;
        }
        let reference_requires_clarify =
            direct_answer_gate_reference_requires_clarify(&gate.reference_resolution);
        if !reference_requires_clarify
            && !current_request_has_context_binding_surface(current_user_request)
        {
            return false;
        }
        return direct_answer_route_introduces_unmentioned_distinctive_context_target(
            current_user_request,
            route,
            gate,
        );
    };
    if normalizer_candidate_matches_bound_context
        || normalizer_answer_candidate_matches_runtime_fact(state, &candidate)
    {
        return false;
    }
    if direct_answer_gate_allows_contextual_chat_reference(current_user_request, route, gate)
        && !answer_candidate_introduces_unmentioned_pathlike_target(
            current_user_request,
            &candidate,
        )
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if !surface.has_concrete_locator_hint()
        && !surface.has_structured_target_refinement()
        && !surface.has_delivery_token_reference()
        && !surface.has_filename_candidates()
        && !surface.has_deictic_reference()
    {
        return false;
    }
    direct_answer_route_introduces_unmentioned_distinctive_context_target(
        current_user_request,
        route,
        gate,
    ) || answer_candidate_introduces_unmentioned_pathlike_target(current_user_request, &candidate)
}

fn direct_answer_gate_contract_allows_locatorless_execution(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> bool {
    if crate::intent::surface_signals::inline_json_transform_request(current_user_request) {
        return true;
    }
    match contract.semantic_kind {
        crate::OutputSemanticKind::PackageManagerDetection => {
            package_manager_skill_supports_detection(state)
        }
        crate::OutputSemanticKind::None
            if matches!(contract.response_shape, crate::OutputResponseShape::Scalar) =>
        {
            true
        }
        crate::OutputSemanticKind::RawCommandOutput => {
            crate::agent_engine::explicit_command_segment_for_policy(
                &state.policy.command_intent,
                current_user_request.trim(),
            )
            .is_some()
        }
        crate::OutputSemanticKind::ServiceStatus
        | crate::OutputSemanticKind::WorkspaceProjectSummary
        | crate::OutputSemanticKind::GitCommitSubject
        | crate::OutputSemanticKind::GitRepositoryState
        | crate::OutputSemanticKind::DockerPs
        | crate::OutputSemanticKind::DockerImages
        | crate::OutputSemanticKind::DockerLogs
        | crate::OutputSemanticKind::DockerContainerLifecycle
        | crate::OutputSemanticKind::WeatherQuery
        | crate::OutputSemanticKind::MarketQuote
        | crate::OutputSemanticKind::ImageUnderstanding
        | crate::OutputSemanticKind::PublishingPreview => true,
        _ => false,
    }
}

fn direct_answer_gate_planner_needs_unbound_locator_clarify(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
) -> bool {
    if !contract.requires_content_evidence
        || contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !contract.locator_hint.trim().is_empty()
        || !direct_answer_gate_reference_is_present(reference_resolution)
        || (direct_answer_gate_reference_is_present(reference_resolution)
            && !direct_answer_gate_reference_requires_clarify(reference_resolution))
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            contract,
        )
        || has_authoritative_deictic_anchor
        || auto_locator_path.is_some_and(|path| !path.trim().is_empty())
    {
        return false;
    }
    !direct_answer_gate_contract_allows_locatorless_execution(state, current_user_request, contract)
}

fn direct_answer_gate_delivery_needs_unbound_existing_file_clarify(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
    has_structural_session_alias_target: bool,
) -> bool {
    let requires_file_delivery = contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || matches!(
            contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        );
    if !requires_file_delivery
        || matches!(
            contract.semantic_kind,
            crate::OutputSemanticKind::GeneratedFileDelivery
        )
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            contract,
        )
        || has_authoritative_deictic_anchor
        || has_structural_session_alias_target
        || auto_locator_path.is_some_and(|path| !path.trim().is_empty())
    {
        return false;
    }
    true
}

fn direct_answer_gate_reference_target(
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> &str {
    reference_resolution.target.trim()
}

fn direct_answer_gate_reference_is_present(
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> bool {
    !matches!(
        direct_answer_gate_reference_target(reference_resolution),
        "" | "none"
    )
}

fn direct_answer_gate_reference_requires_clarify(
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> bool {
    matches!(
        direct_answer_gate_reference_target(reference_resolution),
        "unresolved_prior_object" | "missing_locator" | "ambiguous_locator"
    )
}

fn planner_finalize_style_for_output_contract(
    contract: &crate::IntentOutputContract,
) -> ActFinalizeStyle {
    if let Some(style) =
        crate::post_route_policy::content_evidence_execution_finalize_style(contract, false)
    {
        return style;
    }
    if matches!(
        contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        ActFinalizeStyle::Plain
    } else {
        ActFinalizeStyle::ChatWrapped
    }
}

fn promote_direct_answer_gate_to_planner(
    ctx: &mut crate::agent_engine::AgentRunContext,
    gate: &DirectAnswerGateOut,
    mut contract: crate::IntentOutputContract,
    reason_tag: &str,
) -> DirectAnswerPreflight {
    let Some(route) = ctx.route_result.as_mut() else {
        return DirectAnswerPreflight::DirectAnswer;
    };
    let package_install_preview_candidate = normalizer_answer_candidate_from_resolved_prompt(
        &route.resolved_intent,
    )
    .filter(|candidate| {
        crate::package_commands::package_install_packages_from_commandish_text(candidate).is_some()
    });
    contract.requires_content_evidence = true;
    let finalize_style = planner_finalize_style_for_output_contract(&contract);
    route.output_contract = contract;
    route.set_planner_execute_finalize(finalize_style);
    route.needs_clarify = false;
    route.clarify_question.clear();
    if !gate.resolved_user_intent.trim().is_empty() {
        route.resolved_intent = gate.resolved_user_intent.trim().to_string();
        if let Some(candidate) = package_install_preview_candidate {
            route.resolved_intent.push_str("\nanswer_candidate: ");
            route.resolved_intent.push_str(candidate.trim());
        }
    }
    append_route_reason(route, &format!("{reason_tag}:{}", gate.reason.trim()));
    DirectAnswerPreflight::PlannerExecute(ctx.clone())
}

fn promote_inline_json_transform_context_to_planner(
    ctx: &mut crate::agent_engine::AgentRunContext,
    current_user_request: &str,
) -> bool {
    let Some(route) = ctx.route_result.as_mut() else {
        return false;
    };
    let answer_candidate = normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent);
    let mut contract = route.output_contract.clone();
    contract.requires_content_evidence = true;
    contract.delivery_required = false;
    contract.locator_kind = crate::OutputLocatorKind::None;
    contract.locator_hint.clear();
    contract.delivery_intent = crate::OutputDeliveryIntent::None;
    contract.semantic_kind = crate::OutputSemanticKind::None;
    if matches!(
        contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        contract.response_shape = crate::OutputResponseShape::Strict;
    }
    let finalize_style = planner_finalize_style_for_output_contract(&contract);
    route.output_contract = contract;
    route.set_planner_execute_finalize(finalize_style);
    route.needs_clarify = false;
    route.clarify_question.clear();
    route.resolved_intent = current_user_request.trim().to_string();
    if let Some(candidate) = answer_candidate {
        route.resolved_intent.push_str("\nanswer_candidate: ");
        route.resolved_intent.push_str(candidate.trim());
    }
    append_route_reason(route, "inline_json_transform_structured_execute");
    true
}

fn resolve_direct_answer_gate_contract_locator(
    state: &AppState,
    current_user_request: &str,
    gate: &DirectAnswerGateOut,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> Option<String> {
    if !matches!(
        contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
    ) {
        return None;
    }
    let hint = contract.locator_hint.trim();
    if contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace && hint.is_empty() {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if direct_answer_gate_reference_requires_clarify(reference_resolution)
        && !surface.has_concrete_locator_hint()
        && !surface.has_structured_target_refinement()
        && !surface.has_delivery_token_reference()
    {
        return None;
    }
    let resolved = if hint.is_empty() {
        gate.resolved_user_intent.trim()
    } else {
        hint
    };
    if resolved.is_empty() {
        return None;
    }
    let locator_kind = if contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace {
        crate::OutputLocatorKind::Path
    } else {
        contract.locator_kind
    };
    let direct_resolution = crate::worker::try_resolve_implicit_locator_path(
        state,
        current_user_request,
        resolved,
        locator_kind,
        None,
    )
    .and_then(|resolution| match resolution {
        crate::worker::LocatorAutoResolution::Direct(path) => Some(path),
        crate::worker::LocatorAutoResolution::Fuzzy(_) => None,
    });
    direct_resolution.or_else(|| {
        crate::worker::try_resolve_workspace_child_locator_from_text(
            &state.skill_rt.workspace_root,
            &state.skill_rt.default_locator_search_dir,
            current_user_request,
        )
    })
}

fn bind_direct_answer_gate_contract_locator(
    state: &AppState,
    current_user_request: &str,
    gate: &DirectAnswerGateOut,
    contract: &mut crate::IntentOutputContract,
) -> Option<String> {
    let path = resolve_direct_answer_gate_contract_locator(
        state,
        current_user_request,
        gate,
        contract,
        &gate.reference_resolution,
    )?;
    contract.locator_kind = crate::OutputLocatorKind::Path;
    contract.locator_hint = path.clone();
    Some(path)
}

trait OutputContractFallbackShape {
    fn with_fallback_shape(self, fallback: &crate::IntentOutputContract) -> Self;
}

impl OutputContractFallbackShape for crate::IntentOutputContract {
    fn with_fallback_shape(mut self, fallback: &crate::IntentOutputContract) -> Self {
        if matches!(self.response_shape, crate::OutputResponseShape::Free)
            && !matches!(fallback.response_shape, crate::OutputResponseShape::Free)
        {
            self.response_shape = fallback.response_shape;
            self.exact_sentence_count = fallback.exact_sentence_count;
        }
        if self.locator_hint.is_empty()
            && matches!(
                self.locator_kind,
                crate::OutputLocatorKind::Path
                    | crate::OutputLocatorKind::Filename
                    | crate::OutputLocatorKind::Url
            )
        {
            self.locator_hint = fallback.locator_hint.clone();
        }
        self
    }
}

fn append_route_reason(route: &mut crate::RouteResult, addition: &str) {
    let addition = addition.trim();
    if addition.is_empty() || route.route_reason.contains(addition) {
        return;
    }
    if route.route_reason.trim().is_empty() {
        route.route_reason = addition.to_string();
    } else {
        route.route_reason.push_str("; ");
        route.route_reason.push_str(addition);
    }
}

fn turn_analysis_has_alias_only_state_patch(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(crate::conversation_state::state_patch_is_alias_bindings_only)
}

fn route_is_memory_update_ack_contract(
    route: &crate::RouteResult,
    has_alias_only_state_patch: bool,
) -> bool {
    (route.should_refresh_long_term_memory || has_alias_only_state_patch)
        && route_allows_memory_ack_shape(route)
}

fn route_allows_memory_ack_shape(route: &crate::RouteResult) -> bool {
    !route.needs_clarify
        && !route.wants_file_delivery
        && route.is_chat_gate()
        && !route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        && matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        )
}

fn route_has_executionless_direct_downgrade(route: &crate::RouteResult) -> bool {
    route
        .route_reason
        .contains("executionless_route_downgraded_to_direct_answer")
}

fn current_request_has_structural_execution_target(current_user_request: &str) -> bool {
    if crate::intent::surface_signals::inline_json_transform_request(current_user_request) {
        return true;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.has_explicit_path_or_url()
        || surface.locator_target_pair.is_some()
        || surface.field_selector_count > 0
        || surface.dotted_field_selector.is_some()
        || surface.has_delivery_token_reference()
        || surface.has_filename_candidates()
}

fn current_request_has_context_binding_surface(current_user_request: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.has_concrete_locator_hint()
        || surface.has_structured_target_refinement()
        || surface.has_delivery_token_reference()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_deictic_reference()
}

fn current_request_has_workspace_child_locator_surface(current_user_request: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.has_concrete_locator_hint()
        || surface.has_structured_target_refinement()
        || surface.has_delivery_token_reference()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_explicit_path_or_url()
}

fn current_request_resolves_workspace_child_locator(
    state: &AppState,
    current_user_request: &str,
) -> Option<String> {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_deictic_reference() && !surface.has_explicit_path_or_url() {
        return None;
    }
    crate::worker::try_resolve_workspace_child_locator_from_text(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        current_user_request,
    )
}

fn current_request_resolves_workspace_child_locator_surface(
    state: &AppState,
    current_user_request: &str,
) -> Option<String> {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_explicit_path_or_url() {
        let resolved = crate::worker::try_resolve_implicit_locator_path(
            state,
            current_user_request,
            "",
            crate::OutputLocatorKind::Path,
            None,
        )
        .and_then(|resolution| match resolution {
            crate::worker::LocatorAutoResolution::Direct(path) => Some(path),
            crate::worker::LocatorAutoResolution::Fuzzy(_) => None,
        })?;
        let resolved_path = Path::new(&resolved);
        return (!paths_refer_to_same_existing_location(
            resolved_path,
            &state.skill_rt.workspace_root,
        ))
        .then_some(resolved);
    }
    let resolved = current_request_resolves_workspace_child_locator(state, current_user_request)?;
    if current_request_has_workspace_child_locator_surface(current_user_request) {
        return Some(resolved);
    }
    Path::new(&resolved).is_dir().then_some(resolved)
}

fn current_request_resolves_structural_workspace_child_locator_surface(
    state: &AppState,
    current_user_request: &str,
) -> Option<String> {
    current_request_has_workspace_child_locator_surface(current_user_request)
        .then(|| {
            current_request_resolves_workspace_child_locator_surface(state, current_user_request)
        })
        .flatten()
}

fn direct_answer_gate_chat_promotion_lacks_structured_target(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    has_structural_session_alias_target: bool,
) -> bool {
    if !route.is_chat_gate()
        || route.needs_clarify
        || has_structural_session_alias_target
        || (package_manager_skill_available_for_plan(state)
            && route_has_package_manager_install_preview_candidate(route))
        || direct_answer_gate_reference_requires_clarify(reference_resolution)
        || direct_answer_gate_contract_allows_locatorless_execution(
            state,
            current_user_request,
            contract,
        )
        || current_request_mentions_resolvable_gate_locator(state, current_user_request, contract)
        || matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || current_request_has_structural_execution_target(current_user_request)
        || crate::intent::surface_signals::analyze_prompt_surface(current_user_request)
            .has_deictic_reference()
        || current_request_resolves_structural_workspace_child_locator_surface(
            state,
            current_user_request,
        )
        .is_some()
        || matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    true
}

fn direct_answer_gate_preference_or_memory_promotion_should_stay_chat(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    turn_type: Option<crate::intent_router::TurnType>,
    has_structural_session_alias_target: bool,
) -> bool {
    if turn_type != Some(crate::intent_router::TurnType::PreferenceOrMemory)
        || !route.is_chat_gate()
        || route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || has_structural_session_alias_target
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || !output_contract_requires_planner_execution(contract)
        || direct_answer_gate_reference_requires_clarify(reference_resolution)
        || current_request_has_structural_execution_target(current_user_request)
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            contract,
        )
        || current_request_mentions_resolvable_gate_locator(state, current_user_request, contract)
        || current_request_resolves_structural_workspace_child_locator_surface(
            state,
            current_user_request,
        )
        .is_some()
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !surface.has_explicit_path_or_url()
        && !surface.has_concrete_locator_hint()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && surface.locator_target_pair.is_none()
        && surface.field_selector_count == 0
        && surface.dotted_field_selector.is_none()
}

fn direct_answer_gate_promotes_workspace_child_context(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    contract: &mut crate::IntentOutputContract,
) -> bool {
    if route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || route.should_refresh_long_term_memory
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(contract)
        || !matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || !contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let Some(path) = current_request_resolves_structural_workspace_child_locator_surface(
        state,
        current_user_request,
    ) else {
        return false;
    };
    contract.requires_content_evidence = true;
    contract.locator_kind = crate::OutputLocatorKind::Path;
    contract.locator_hint = path;
    true
}

fn structural_session_alias_locator_for_target(
    target: &str,
) -> Option<crate::intent::locator_extractor::ExtractedLocator> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    crate::intent::locator_extractor::extract_explicit_locator_for_fallback(target)
}

fn current_request_structural_session_alias_locator(
    ctx: &crate::agent_engine::AgentRunContext,
    current_user_request: &str,
) -> Option<crate::intent::locator_extractor::ExtractedLocator> {
    let binding = crate::conversation_state::single_alias_binding_mentioned_in_prompt(
        &ctx.session_alias_bindings,
        current_user_request,
    )?;
    structural_session_alias_locator_for_target(&binding.target)
}

fn bind_session_alias_locator_to_contract(
    locator: Option<&crate::intent::locator_extractor::ExtractedLocator>,
    contract: &mut crate::IntentOutputContract,
) {
    let Some(locator) = locator else {
        return;
    };
    contract.requires_content_evidence = true;
    contract.locator_kind = locator.locator_kind;
    contract.locator_hint = locator.locator_hint.clone();
}

fn normalized_schema_tokens(raw: &str) -> Vec<String> {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn resolved_intent_declares_structured_scalar_extraction(resolved_intent: &str) -> bool {
    let (stripped, _) = strip_embedded_answer_candidate_from_intent(resolved_intent);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.chars().any(char::is_whitespace) {
            return false;
        }
        let tokens = normalized_schema_tokens(line);
        tokens.iter().any(|token| {
            matches!(
                token.as_str(),
                "scalar" | "title" | "heading" | "subject" | "value"
            )
        }) || tokens.windows(2).any(|pair| {
            matches!(
                (&pair[0][..], &pair[1][..]),
                ("extract", "title") | ("extract", "scalar") | ("first", "heading")
            )
        })
    })
}

fn preserve_structured_scalar_extraction_contract(
    contract: &mut crate::IntentOutputContract,
    structured_scalar_extraction: bool,
) {
    if !structured_scalar_extraction || contract.delivery_required {
        return;
    }
    contract.requires_content_evidence = true;
    contract.response_shape = crate::OutputResponseShape::Scalar;
    if matches!(
        contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    ) {
        contract.semantic_kind = crate::OutputSemanticKind::None;
    }
}

fn apply_direct_answer_gate_outcome(
    state: &AppState,
    ctx: &mut crate::agent_engine::AgentRunContext,
    current_user_request: &str,
    gate: DirectAnswerGateOut,
) -> DirectAnswerPreflight {
    let mut gate = gate;
    if let Some(preflight) = contract_test_hint_forced_planner_preflight(
        ctx,
        current_user_request,
        "direct_answer_gate_contract_hint_forced_planner",
    ) {
        return preflight;
    }
    let decision = parse_direct_answer_gate_decision(&gate.decision);
    if gate.confidence < 0.60 {
        return DirectAnswerPreflight::DirectAnswer;
    }
    let recent_request_file_target_count =
        collect_recent_execution_request_file_targets(state, Some(ctx)).len();
    let has_alias_only_state_patch =
        turn_analysis_has_alias_only_state_patch(ctx.turn_analysis.as_ref());
    let structural_session_alias_locator =
        current_request_structural_session_alias_locator(ctx, current_user_request);
    let has_structural_session_alias_target = structural_session_alias_locator.is_some();
    let normalizer_candidate_matches_bound_context = ctx
        .route_result
        .as_ref()
        .and_then(|route| normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent))
        .is_some_and(|candidate| {
            normalizer_answer_candidate_matches_bound_runtime_context(state, &candidate, Some(ctx))
        });
    let existing_observed_result_should_stay_chat =
        ctx.route_result.as_ref().is_some_and(|route| {
            direct_answer_gate_existing_observed_result_should_stay_chat(
                current_user_request,
                route,
                &gate,
                ctx,
            )
        });
    let preserve_active_task_text_mutation =
        direct_answer_gate_can_skip_for_active_task_text_mutation(current_user_request, Some(ctx));
    let turn_type = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.turn_type);
    let Some(route) = ctx.route_result.as_mut() else {
        return DirectAnswerPreflight::DirectAnswer;
    };
    let structured_scalar_extraction =
        resolved_intent_declares_structured_scalar_extraction(&route.resolved_intent);
    let auto_locator_path = ctx.auto_locator_path.as_deref();
    let has_authoritative_deictic_anchor = ctx.has_authoritative_deictic_anchor;
    let force_inline_transform_execution = transform_skill_available_for_plan(state)
        && (crate::intent::surface_signals::inline_json_transform_request(current_user_request)
            || (direct_answer_gate_embedded_inline_json_payload_surface(current_user_request)
                && (decision == DirectAnswerGateDecision::PlannerExecute
                    || normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent)
                        .as_deref()
                        .is_some_and(direct_answer_gate_structured_transform_candidate))));
    let force_package_manager_detect_execution = package_manager_skill_supports_detection(state)
        && output_contract_requests_package_manager_detection(&route.output_contract);
    let force_package_manager_install_preview_execution =
        package_manager_skill_available_for_plan(state)
            && route_has_package_manager_install_preview_candidate(route);
    if route_is_memory_update_ack_contract(route, has_alias_only_state_patch) {
        append_route_reason(route, "direct_answer_gate_memory_update_ignored");
        return DirectAnswerPreflight::DirectAnswer;
    }
    if preserve_active_task_text_mutation {
        append_route_reason(
            route,
            "direct_answer_gate_active_task_text_mutation_ignored",
        );
        return DirectAnswerPreflight::DirectAnswer;
    }
    if route_has_executionless_direct_downgrade(route)
        && decision == DirectAnswerGateDecision::PlannerExecute
        && !current_request_has_structural_execution_target(current_user_request)
        && current_request_resolves_structural_workspace_child_locator_surface(
            state,
            current_user_request,
        )
        .is_none()
        && !has_structural_session_alias_target
        && !force_package_manager_install_preview_execution
    {
        append_route_reason(route, "direct_answer_gate_executionless_promotion_blocked");
        return DirectAnswerPreflight::Clarify(String::new());
    }
    if existing_observed_result_should_stay_chat {
        append_route_reason(route, "direct_answer_gate_existing_observed_result_ignored");
        return DirectAnswerPreflight::DirectAnswer;
    }
    if decision != DirectAnswerGateDecision::PlannerExecute
        && direct_answer_gate_candidate_needs_unbound_context_clarify(
            state,
            current_user_request,
            route,
            &gate,
            auto_locator_path,
            has_authoritative_deictic_anchor,
            has_structural_session_alias_target,
            normalizer_candidate_matches_bound_context,
        )
    {
        return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
    }
    match decision {
        DirectAnswerGateDecision::DirectAnswer => {
            let fallback_contract = route.output_contract.clone();
            let resolved_prompt = route.resolved_intent.clone();
            let mut contract = output_contract_from_direct_answer_gate(
                gate.output_contract.clone(),
                &fallback_contract,
            );
            preserve_structured_scalar_extraction_contract(
                &mut contract,
                structured_scalar_extraction,
            );
            if force_inline_transform_execution {
                contract.requires_content_evidence = true;
                contract.locator_kind = crate::OutputLocatorKind::None;
                contract.locator_hint.clear();
                contract.semantic_kind = crate::OutputSemanticKind::None;
                if matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
                ) {
                    contract.response_shape = crate::OutputResponseShape::Strict;
                }
                return promote_direct_answer_gate_to_planner(
                    ctx,
                    &gate,
                    contract,
                    "direct_answer_gate_inline_transform_execute",
                );
            }
            if force_package_manager_detect_execution {
                contract.requires_content_evidence = true;
                contract.locator_kind = crate::OutputLocatorKind::None;
                contract.locator_hint.clear();
                contract.semantic_kind = crate::OutputSemanticKind::PackageManagerDetection;
                if matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
                ) {
                    contract.response_shape = crate::OutputResponseShape::Strict;
                }
                return promote_direct_answer_gate_to_planner(
                    ctx,
                    &gate,
                    contract,
                    "direct_answer_gate_package_manager_detect_execute",
                );
            }
            if normalizer_answer_candidate_from_resolved_prompt(&resolved_prompt).is_some()
                && !output_contract_requires_planner_execution(&contract)
                && bound_direct_answer_candidate_satisfies_output_contract(&contract)
                && matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::OneSentence
                )
                && contract.exact_sentence_count == Some(1)
            {
                append_route_reason(
                    route,
                    "direct_answer_gate_exact_candidate_ignored_execution",
                );
                return DirectAnswerPreflight::DirectAnswer;
            }
            let promoted_workspace_child_context =
                direct_answer_gate_promotes_workspace_child_context(
                    state,
                    current_user_request,
                    route,
                    &mut contract,
                );
            let promoted_artifact_listing =
                promote_artifact_listing_candidate_contract(&resolved_prompt, &mut contract);
            let promoted_recent_file_context =
                direct_answer_gate_should_force_recent_file_context_execution(
                    current_user_request,
                    &resolved_prompt,
                    &contract,
                    recent_request_file_target_count,
                );
            if promoted_workspace_child_context {
                gate.resolved_user_intent = current_user_request.trim().to_string();
            }
            if promoted_recent_file_context {
                contract.requires_content_evidence = true;
                if matches!(contract.locator_kind, crate::OutputLocatorKind::None) {
                    contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
                }
                if matches!(contract.semantic_kind, crate::OutputSemanticKind::None) {
                    contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
                }
            }
            if normalizer_candidate_matches_bound_context
                && bound_direct_answer_candidate_satisfies_output_contract(&contract)
            {
                append_route_reason(route, "direct_answer_gate_bound_candidate_evidence");
                return DirectAnswerPreflight::DirectAnswer;
            }
            if output_contract_requires_planner_execution(&contract) {
                if existing_observed_result_should_stay_chat {
                    append_route_reason(
                        route,
                        "direct_answer_gate_existing_observed_result_ignored",
                    );
                    return DirectAnswerPreflight::DirectAnswer;
                }
                if direct_answer_gate_untrusted_locator_hint_requires_clarify(
                    state,
                    current_user_request,
                    &contract,
                    &gate.reference_resolution,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                    has_structural_session_alias_target,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                if direct_answer_gate_chat_promotion_lacks_structured_target(
                    state,
                    current_user_request,
                    route,
                    &contract,
                    &gate.reference_resolution,
                    has_structural_session_alias_target,
                ) {
                    append_route_reason(
                        route,
                        "direct_answer_gate_chat_promotion_without_structured_target_ignored",
                    );
                    return DirectAnswerPreflight::DirectAnswer;
                }
                if direct_answer_gate_preference_or_memory_promotion_should_stay_chat(
                    state,
                    current_user_request,
                    route,
                    &contract,
                    &gate.reference_resolution,
                    turn_type,
                    has_structural_session_alias_target,
                ) {
                    append_route_reason(
                        route,
                        "direct_answer_gate_preference_memory_context_ignored",
                    );
                    return DirectAnswerPreflight::DirectAnswer;
                }
                if direct_answer_gate_promotion_depends_only_on_background_context(
                    state,
                    current_user_request,
                    route,
                    &contract,
                    &gate.reference_resolution,
                    has_structural_session_alias_target,
                ) {
                    append_route_reason(route, "direct_answer_gate_background_only_ignored");
                    return DirectAnswerPreflight::DirectAnswer;
                }
                bind_direct_answer_gate_contract_locator(
                    state,
                    current_user_request,
                    &gate,
                    &mut contract,
                );
                bind_session_alias_locator_to_contract(
                    structural_session_alias_locator.as_ref(),
                    &mut contract,
                );
                if direct_answer_gate_untrusted_locator_hint_requires_clarify(
                    state,
                    current_user_request,
                    &contract,
                    &gate.reference_resolution,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                    has_structural_session_alias_target,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                if direct_answer_gate_delivery_needs_unbound_existing_file_clarify(
                    state,
                    current_user_request,
                    &contract,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                    has_structural_session_alias_target,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                if direct_answer_gate_planner_needs_unbound_locator_clarify(
                    state,
                    current_user_request,
                    &contract,
                    &gate.reference_resolution,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                if direct_answer_gate_promotion_needs_unbound_deictic_clarify(
                    state,
                    current_user_request,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                    has_structural_session_alias_target,
                    &contract,
                    &gate.reference_resolution,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                let reason_tag = if promoted_recent_file_context {
                    "direct_answer_gate_recent_file_context_execute"
                } else if promoted_artifact_listing {
                    "direct_answer_gate_artifact_listing_execute"
                } else if promoted_workspace_child_context {
                    "direct_answer_gate_workspace_child_context_execute"
                } else if direct_answer_gate_contextual_inline_structured_payload_execute(
                    current_user_request,
                    &contract,
                ) {
                    "inline_structured_payload_context_execute"
                } else {
                    "direct_answer_gate_contract_execute"
                };
                promote_direct_answer_gate_to_planner(ctx, &gate, contract, reason_tag)
            } else {
                DirectAnswerPreflight::DirectAnswer
            }
        }
        DirectAnswerGateDecision::Clarify => {
            let question = gate.clarify_question.trim();
            if question.is_empty() {
                DirectAnswerPreflight::DirectAnswer
            } else {
                route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
                route.needs_clarify = true;
                route.clarify_question = question.to_string();
                append_route_reason(
                    route,
                    &format!("direct_answer_gate_clarify:{}", gate.reason.trim()),
                );
                DirectAnswerPreflight::Clarify(question.to_string())
            }
        }
        DirectAnswerGateDecision::PlannerExecute => {
            let fallback_contract = route.output_contract.clone();
            let mut contract = output_contract_from_direct_answer_gate(
                gate.output_contract.clone(),
                &fallback_contract,
            );
            preserve_structured_scalar_extraction_contract(
                &mut contract,
                structured_scalar_extraction,
            );
            let promoted_workspace_child_context =
                direct_answer_gate_promotes_workspace_child_context(
                    state,
                    current_user_request,
                    route,
                    &mut contract,
                );
            if promoted_workspace_child_context {
                gate.resolved_user_intent = current_user_request.trim().to_string();
            }
            if force_inline_transform_execution {
                contract.requires_content_evidence = true;
                contract.locator_kind = crate::OutputLocatorKind::None;
                contract.locator_hint.clear();
                contract.semantic_kind = crate::OutputSemanticKind::None;
                if matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
                ) {
                    contract.response_shape = crate::OutputResponseShape::Strict;
                }
                return promote_direct_answer_gate_to_planner(
                    ctx,
                    &gate,
                    contract,
                    "direct_answer_gate_inline_transform_execute",
                );
            }
            if existing_observed_result_should_stay_chat {
                append_route_reason(route, "direct_answer_gate_existing_observed_result_ignored");
                return DirectAnswerPreflight::DirectAnswer;
            }
            if direct_answer_gate_untrusted_locator_hint_requires_clarify(
                state,
                current_user_request,
                &contract,
                &gate.reference_resolution,
                auto_locator_path,
                has_authoritative_deictic_anchor,
                has_structural_session_alias_target,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            if direct_answer_gate_chat_promotion_lacks_structured_target(
                state,
                current_user_request,
                route,
                &contract,
                &gate.reference_resolution,
                has_structural_session_alias_target,
            ) {
                append_route_reason(
                    route,
                    "direct_answer_gate_chat_promotion_without_structured_target_ignored",
                );
                return DirectAnswerPreflight::DirectAnswer;
            }
            if direct_answer_gate_preference_or_memory_promotion_should_stay_chat(
                state,
                current_user_request,
                route,
                &contract,
                &gate.reference_resolution,
                turn_type,
                has_structural_session_alias_target,
            ) {
                append_route_reason(
                    route,
                    "direct_answer_gate_preference_memory_context_ignored",
                );
                return DirectAnswerPreflight::DirectAnswer;
            }
            if normalizer_candidate_matches_bound_context
                && bound_direct_answer_candidate_satisfies_output_contract(&contract)
            {
                append_route_reason(route, "direct_answer_gate_bound_candidate_evidence");
                return DirectAnswerPreflight::DirectAnswer;
            }
            if direct_answer_gate_promotion_depends_only_on_background_context(
                state,
                current_user_request,
                route,
                &contract,
                &gate.reference_resolution,
                has_structural_session_alias_target,
            ) {
                append_route_reason(route, "direct_answer_gate_background_only_ignored");
                return DirectAnswerPreflight::DirectAnswer;
            }
            bind_direct_answer_gate_contract_locator(
                state,
                current_user_request,
                &gate,
                &mut contract,
            );
            bind_session_alias_locator_to_contract(
                structural_session_alias_locator.as_ref(),
                &mut contract,
            );
            if direct_answer_gate_untrusted_locator_hint_requires_clarify(
                state,
                current_user_request,
                &contract,
                &gate.reference_resolution,
                auto_locator_path,
                has_authoritative_deictic_anchor,
                has_structural_session_alias_target,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            if direct_answer_gate_delivery_needs_unbound_existing_file_clarify(
                state,
                current_user_request,
                &contract,
                auto_locator_path,
                has_authoritative_deictic_anchor,
                has_structural_session_alias_target,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            if direct_answer_gate_planner_needs_unbound_locator_clarify(
                state,
                current_user_request,
                &contract,
                &gate.reference_resolution,
                auto_locator_path,
                has_authoritative_deictic_anchor,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            if direct_answer_gate_promotion_needs_unbound_deictic_clarify(
                state,
                current_user_request,
                auto_locator_path,
                has_authoritative_deictic_anchor,
                has_structural_session_alias_target,
                &contract,
                &gate.reference_resolution,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            let reason_tag = if promoted_workspace_child_context {
                "direct_answer_gate_workspace_child_context_execute"
            } else if direct_answer_gate_contextual_inline_structured_payload_execute(
                current_user_request,
                &contract,
            ) {
                "inline_structured_payload_context_execute"
            } else {
                "direct_answer_gate_execute"
            };
            promote_direct_answer_gate_to_planner(ctx, &gate, contract, reason_tag)
        }
    }
}

fn apply_direct_answer_gate_unbound_deictic_clarify(
    route: &mut crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> DirectAnswerPreflight {
    let mut preserved_contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &route.output_contract,
    );
    preserved_contract.locator_kind = crate::OutputLocatorKind::None;
    preserved_contract.locator_hint.clear();

    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.needs_clarify = true;
    route.clarify_question.clear();
    route.wants_file_delivery = preserved_contract.delivery_required
        || matches!(
            preserved_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
    route.output_contract = preserved_contract;
    append_route_reason(route, "direct_answer_gate_unbound_deictic_clarify");
    DirectAnswerPreflight::Clarify(route.clarify_question.clone())
}

fn direct_answer_gate_route_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> String {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return "<none>".to_string();
    };
    let mut lines = Vec::new();
    let (resolved_intent, removed_answer_candidate) =
        strip_embedded_answer_candidate_from_intent(route.resolved_intent.trim());
    if !resolved_intent.is_empty() {
        lines.push(format!("resolved_user_intent: {resolved_intent}"));
    }
    if removed_answer_candidate {
        lines.push("normalizer_answer_candidate_present: true (not runtime evidence)".to_string());
    }
    let locator_hint = route.output_contract.locator_hint.trim();
    if !locator_hint.is_empty() {
        lines.push(format!("locator_hint: {locator_hint}"));
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
    if !route_reason.is_empty() {
        lines.push(format!("prior_route_reason: {route_reason}"));
    }
    format!(
        "### PRIOR_ROUTE_CONTEXT\nReview this prior route context, but do not treat it as observed evidence. The current request and runtime-evidence rules win over prior answer candidates or prior route reasons.\n{}\n",
        lines.join("\n")
    )
}

fn direct_answer_gate_recent_execution_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> String {
    let Some(context) = agent_run_context
        .and_then(|ctx| ctx.cross_turn_recent_execution_context.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<none>")
    else {
        return "<none>".to_string();
    };
    let context = crate::providers::utf8_safe_prefix(context, 6000);
    format!(
        "### RECENT_EXECUTION_CONTEXT\nUse this only for current-turn follow-up reference binding. Previous executed targets are authoritative for relative/ordinal file or action references. Paths mentioned inside a prior file excerpt are content, not the executed file target unless the current request explicitly asks about the excerpt content.\n{context}"
    )
}

fn direct_answer_gate_runtime_context(state: &AppState) -> String {
    let current_process_cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    format!(
        "workspace_root: {}\ncurrent_process_cwd: {}\nruntime_has_tools: true",
        state.skill_rt.workspace_root.display(),
        current_process_cwd
    )
}

async fn run_direct_answer_gate(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<DirectAnswerGateOut> {
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        DIRECT_ANSWER_GATE_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            tracing::info!(
                "{} direct_answer_gate prompt_missing task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    let route_context = direct_answer_gate_route_context(agent_run_context);
    let recent_execution_context = direct_answer_gate_recent_execution_context(agent_run_context);
    let runtime_context = direct_answer_gate_runtime_context(state);
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            ("__REQUEST__", user_request.trim()),
            ("__ROUTE_CONTEXT__", &route_context),
            ("__RECENT_EXECUTION_CONTEXT__", &recent_execution_context),
            ("__RUNTIME_CONTEXT__", &runtime_context),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "direct_answer_gate_prompt",
        &resolved.source,
        resolved.version.as_deref(),
        None,
    );
    let prompt_source = resolved.source;
    let llm_out = match crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::info!(
                "{} direct_answer_gate llm_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    match crate::prompt_utils::validate_against_schema::<DirectAnswerGateOut>(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::DirectAnswerGate,
    ) {
        Ok(validated) => Some(validated.value),
        Err(err) => {
            tracing::info!(
                "{} direct_answer_gate schema_validation_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            None
        }
    }
}

fn ask_reply_with_chat_process(text: String, _language_hint: &str) -> AskReply {
    let answer = text.trim().to_string();
    if answer.is_empty() || crate::finalize::is_execution_summary_message(&answer) {
        AskReply::llm(text)
    } else {
        AskReply::llm(answer)
    }
}

fn ask_reply_with_clarify_process(
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

fn schema_value_requests_filename_only_output(value: &Value) -> bool {
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

fn request_uses_filename_only_schema_token(prompt: &str) -> bool {
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

fn route_contract_requests_filename_only_output(route: Option<&crate::RouteResult>) -> bool {
    route.is_some_and(|route| {
        matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
        )
    })
}

fn turn_analysis_requests_filename_only_output(
    analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(schema_value_requests_filename_only_output)
}

fn session_alias_target_direct_answer_candidate(
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
    let turn_analysis_declares_filename_only =
        turn_analysis_requests_filename_only_output(ctx.turn_analysis.as_ref());
    let route_contract_declares_filename_only = route_contract_requests_filename_only_output(route);
    let wants_filename_only = current_request_declares_filename_only
        || turn_analysis_declares_filename_only
        || route_contract_declares_filename_only;
    if !wants_filename_only {
        return None;
    }
    if route.is_some_and(|route| route.output_contract.requires_content_evidence)
        && !current_request_declares_filename_only
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

fn session_alias_rebind_ack(
    state: &AppState,
    task: &ClaimedTask,
    current_user_request: &str,
    language_hint: &str,
) -> Option<AskReply> {
    let prior_state = crate::conversation_state::load_active_conversation_state(state, task);
    let _binding = crate::conversation_state::structural_alias_rebind_from_prompt(
        prior_state.as_ref(),
        current_user_request,
    )?;
    let answer = if language_hint == "en" {
        "Updated.".to_string()
    } else {
        "已更新。".to_string()
    };
    Some(ask_reply_with_chat_process(answer, language_hint))
}

fn state_patch_alias_bindings_ack(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    language_hint: &str,
) -> Option<AskReply> {
    let ctx = agent_run_context?;
    let analysis = ctx.turn_analysis.as_ref()?;
    let state_patch = analysis.state_patch.as_ref()?;
    if !crate::conversation_state::state_patch_is_alias_bindings_only(state_patch) {
        return None;
    }
    if let Some(route) = ctx.route_result.as_ref() {
        if !route_is_memory_update_ack_contract(route, true) {
            return None;
        }
    }
    let bindings =
        crate::conversation_state::session_alias_bindings_from_state_patch(Some(state_patch));
    let pairs = bindings
        .iter()
        .filter_map(|binding| {
            let alias = binding.alias.trim();
            let target = binding.target.trim();
            if alias.is_empty() || target.is_empty() {
                None
            } else {
                Some((alias, target))
            }
        })
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        return None;
    }
    let allow_misclassified_alias_update =
        analysis.turn_type == Some(crate::intent_router::TurnType::TaskRequest);
    if analysis.turn_type.is_some()
        && analysis.turn_type != Some(crate::intent_router::TurnType::PreferenceOrMemory)
        && !allow_misclassified_alias_update
    {
        return None;
    }
    let answer = if allow_misclassified_alias_update {
        if language_hint == "en" {
            "Updated.".to_string()
        } else {
            "已更新。".to_string()
        }
    } else if language_hint == "en" {
        if pairs.len() == 1 {
            format!("Remembered: `{}` -> `{}`.", pairs[0].0, pairs[0].1)
        } else {
            let lines = pairs
                .iter()
                .map(|(alias, target)| format!("- `{alias}` -> `{target}`"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("Remembered:\n{lines}")
        }
    } else if pairs.len() == 1 {
        format!("已记住：`{}` -> `{}`。", pairs[0].0, pairs[0].1)
    } else {
        let lines = pairs
            .iter()
            .map(|(alias, target)| format!("- `{alias}` -> `{target}`"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("已记住：\n{lines}")
    };
    Some(ask_reply_with_chat_process(answer, language_hint))
}

fn structural_alias_binding_ack(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    prompt: &str,
    resolved_prompt_for_execution: &str,
    language_hint: &str,
) -> Option<AskReply> {
    let ctx = agent_run_context?;
    let analysis = ctx.turn_analysis.as_ref()?;
    if analysis.turn_type != Some(crate::intent_router::TurnType::PreferenceOrMemory) {
        return None;
    }
    if analysis
        .state_patch
        .as_ref()
        .and_then(|patch| patch.get("alias_bindings"))
        .is_some()
    {
        return None;
    }
    let route_result = ctx.route_result.as_ref()?;
    if !route_allows_memory_ack_shape(route_result) {
        return None;
    }
    let binding = crate::conversation_state::structural_alias_binding_from_memory_prompt(
        prompt,
        route_result,
        resolved_prompt_for_execution,
    )?;
    let answer = normalizer_memory_ack_answer_candidate(route_result).unwrap_or_else(|| {
        if language_hint == "en" {
            format!("Remembered: `{}` -> `{}`.", binding.alias, binding.target)
        } else {
            format!("已记住：`{}` -> `{}`。", binding.alias, binding.target)
        }
    });
    Some(ask_reply_with_chat_process(answer, language_hint))
}

fn normalizer_memory_ack_answer_candidate(route_result: &crate::RouteResult) -> Option<String> {
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

fn build_resume_followup_discussion_prompt_from_parts(
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

fn chat_wrapped_execution_goal_from_prompt(prompt_with_memory: &str) -> String {
    format!(
        "{}\n\nFinalize hint: complete required actions first, then return a concise user-facing reply that confirms results naturally.",
        prompt_with_memory
    )
}

fn fuzzy_locator_clarify_context(candidates: &[String]) -> Option<String> {
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

fn preferred_route_clarify_question(
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

fn route_structured_clarify_context(
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
    let clarify_case = if route.output_contract.delivery_required {
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
    }?;
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

fn chat_route_resolution_context(
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
    if !route_reason.is_empty() {
        lines.push(format!("route_reason: {route_reason}"));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "### ROUTE_RESOLUTION\nTreat the following route resolution as authoritative for this turn. It is resolved context, not missing context. If older memory or unrelated assistant history conflicts with it, prefer this resolution unless the user explicitly asks about older history.\n{}\n",
        lines.join("\n")
    ))
}

fn active_task_text_mutation_context(ctx: &crate::agent_engine::AgentRunContext) -> bool {
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

fn active_task_semantic_answer_candidate_draft(
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

fn route_draft_is_compact_scalar_shape(draft: &str) -> bool {
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

fn active_task_required_visible_literals(
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
    trusted_required_visible_literals_from_state_patch(state_patch)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTaskReplacementPair {
    from: String,
    to: String,
}

#[cfg(test)]
fn required_visible_literals_from_state_patch(state_patch: &serde_json::Value) -> Vec<String> {
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

fn trusted_required_visible_literals_from_state_patch(
    state_patch: &serde_json::Value,
) -> Vec<String> {
    let mut literals = Vec::new();
    for key in [
        "required_content_literals",
        "active_task_required_content_literals",
        "content_literals",
    ] {
        collect_required_visible_literals(state_patch.get(key), &mut literals);
    }
    if let Some(constraints) = state_patch.get("visible_constraints") {
        collect_required_visible_literals(
            constraints.get("required_content_literals"),
            &mut literals,
        );
        collect_required_visible_literals(constraints.get("content_literals"), &mut literals);
    }
    for key in [
        "required_visible_literals",
        "active_task_required_visible_literals",
        "visible_literals",
    ] {
        collect_typed_content_visible_literals(state_patch.get(key), &mut literals);
    }
    if let Some(constraints) = state_patch.get("visible_constraints") {
        collect_typed_content_visible_literals(
            constraints.get("required_visible_literals"),
            &mut literals,
        );
        collect_typed_content_visible_literals(constraints.get("literals"), &mut literals);
    }
    for pair in replacement_pairs_from_state_patch(state_patch) {
        push_required_visible_literal(&pair.to, &mut literals);
    }
    literals
}

fn active_task_forbidden_visible_literals(
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

fn forbidden_visible_literals_from_state_patch(state_patch: &serde_json::Value) -> Vec<String> {
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

fn active_task_replacement_pairs(
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

fn replacement_pairs_from_state_patch(
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

fn collect_replacement_pairs(
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

fn collect_required_visible_literals(value: Option<&serde_json::Value>, out: &mut Vec<String>) {
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

fn collect_typed_content_visible_literals(
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

fn push_required_visible_literal(value: &str, out: &mut Vec<String>) {
    let value = value
        .trim()
        .trim_matches(|c: char| c.is_ascii_whitespace() || matches!(c, '"' | '\'' | '`'))
        .trim();
    if value.is_empty() || value.contains('\n') || value.chars().count() > 80 {
        return;
    }
    if out.iter().any(|existing| existing == value) {
        return;
    }
    out.push(value.to_string());
}

fn answer_contains_required_visible_literal(answer: &str, literal: &str) -> bool {
    if answer.contains(literal) {
        return true;
    }
    literal.is_ascii()
        && answer
            .to_ascii_lowercase()
            .contains(&literal.to_ascii_lowercase())
}

fn ensure_active_task_required_visible_literals(
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

fn strip_embedded_answer_candidate_from_intent(resolved_intent: &str) -> (String, bool) {
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

fn chat_prompt_context_with_route_resolution(
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

fn active_task_text_rewrite_context(
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

fn chat_recent_execution_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
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

fn chat_route_should_include_recent_execution_context(route: &crate::RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        || route_reason_has_exact_marker(route, "semantic_contract_requires_evidence")
        || route_reason_has_exact_marker(route, "active_text_followup_route_repair")
}

fn route_reason_has_exact_marker(route: &crate::RouteResult, marker: &str) -> bool {
    route
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| part == marker)
        || route.route_reason.trim() == marker
}

fn chat_user_request<'a>(resolved_prompt: &'a str, execution_user_request: &'a str) -> &'a str {
    if execution_user_request.trim() != resolved_prompt.trim() {
        execution_user_request
    } else {
        resolved_prompt
    }
}

fn direct_answer_chat_user_request(
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

fn chat_request_for_prompt(original_user_request: &str, semantic_request: &str) -> String {
    let original = original_user_request.trim();
    let semantic = semantic_request.trim();
    if original.is_empty() || original == semantic {
        return semantic.to_string();
    }
    format!(
        "Original user request:\n{original}\n\nResolved semantic intent / answer candidate:\n{semantic}\n\nUse the resolved semantic intent to answer the original request. If the original request asks for only a value, ID, path, name, or one short answer, output only the resolved value with no preamble."
    )
}

fn direct_chat_answer_needs_repair(answer: &str) -> bool {
    let trimmed = answer.trim();
    trimmed.is_empty()
        || crate::finalize::looks_like_planner_artifact(trimmed)
        || crate::finalize::looks_like_internal_trace_artifact(trimmed)
        || direct_chat_answer_has_unclosed_code_fence(trimmed)
}

fn direct_chat_answer_has_unclosed_code_fence(answer: &str) -> bool {
    let fence_count = answer
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("```"))
        .count();
    fence_count % 2 == 1
}

fn direct_chat_answer_repair_prompt(chat_prompt: &str, rejected_answer: &str) -> String {
    format!(
        "{chat_prompt}\n\n### Previous Draft Rejected\nThe previous draft is malformed or incomplete and cannot be shown to the user:\n{rejected_answer}\n\nReturn only a complete final answer for the same user request. Do not use a code fence unless the user explicitly requested code."
    )
}

fn active_task_rewrite_conservation_prompt(chat_prompt: &str, draft_answer: &str) -> String {
    format!(
        "{chat_prompt}\n\n### Active Task Rewrite Conservation\nThe previous draft may have added facts, instructions, examples, use cases, paths, docs/guides, setup steps, credential/setup detail categories, or operational claims that were not present in the most recent generated output:\n{draft_answer}\n\nRewrite the final answer for the same current user request using only the factual content already present in the most recent generated output plus the current style/length/audience constraint. Preserve any statement that concrete details were not observed. Return only the corrected final answer."
    )
}

fn active_task_recent_generated_output(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let text = agent_run_context?
        .route_result
        .as_ref()?
        .resolved_intent
        .as_str();
    const MARKER: &str = "Most recent generated output:\n";
    let (_, tail) = text.split_once(MARKER)?;
    let stop_idx = [
        "\n\nContinuity rules:",
        "\n\nStructured task updates:",
        "\n\nNew user instruction:",
        "\n\n### SESSION_ALIAS_BINDINGS",
    ]
    .iter()
    .filter_map(|marker| tail.find(marker))
    .min()
    .unwrap_or(tail.len());
    let output = tail[..stop_idx].trim();
    (!output.is_empty() && output != "<none>").then(|| output.to_string())
}

fn conservative_active_task_rewrite_passthrough(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    if !active_task_text_rewrite_context(agent_run_context) {
        return None;
    }
    let output = active_task_recent_generated_output(agent_run_context)?;
    let lower = output.to_ascii_lowercase();
    if lower.contains("does not include concrete per-channel setup") {
        return Some(output);
    }
    None
}

fn task_payload_text(task: &ClaimedTask) -> Option<String> {
    crate::task_payload_value(task)?
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn execute_via_planner_loop(
    state: &AppState,
    task: &ClaimedTask,
    prompt_with_memory: &str,
    execution_user_request: &str,
    ask_mode: &crate::AskMode,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<AskReply, String> {
    let planner_goal = if ask_mode.finalize_chat_wrapped() {
        chat_wrapped_execution_goal_from_prompt(prompt_with_memory)
    } else {
        prompt_with_memory.to_string()
    };
    crate::agent_engine::run_agent_with_tools(
        state,
        task,
        &planner_goal,
        execution_user_request,
        agent_run_context,
    )
    .await
}

pub(crate) async fn execute_ask_routed(
    state: &AppState,
    task: &ClaimedTask,
    chat_prompt_context: &str,
    prompt_with_memory: &str,
    resolved_prompt: &str,
    execution_user_request: &str,
    agent_mode: bool,
    resume_force_chat: bool,
    route_ask_mode: Option<crate::AskMode>,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<AskReply, String> {
    // Callers pass the first-layer AskMode directly. If it is missing, choose a
    // conservative local fallback instead of starting another routing LLM round.
    let route_ask_mode_for_log = route_ask_mode.clone();
    let (ask_mode, override_reason) = if resume_force_chat {
        (crate::AskMode::direct_answer(), Some("resume_force_chat"))
    } else if let Some(mode) = route_ask_mode {
        (mode, None)
    } else if agent_mode {
        (
            crate::AskMode::clarify(),
            Some("route_ask_mode=None and agent_mode=true"),
        )
    } else {
        (
            crate::AskMode::direct_answer(),
            Some("route_ask_mode=None and agent_mode=false"),
        )
    };
    let route_label = ask_mode.route_label();
    tracing::info!(
        "{} worker_once: ask task_id={} first_layer_decision={} ask_mode={} derived_route_label={} agent_mode={} override={}",
        crate::highlight_tag("routing"),
        task.task_id,
        ask_mode.first_layer_decision().as_str(),
        route_ask_mode_for_log
            .as_ref()
            .map(crate::AskMode::as_str)
            .unwrap_or("none"),
        route_label,
        agent_mode,
        override_reason.unwrap_or("")
    );
    if let Some(reply) = crate::self_extension::maybe_handle_ask_self_extension(
        state,
        task,
        resolved_prompt,
        execution_user_request,
        agent_run_context.as_ref(),
    )
    .await?
    {
        return Ok(reply);
    }
    let current_turn_user_request_for_process =
        task_payload_text(task).unwrap_or_else(|| execution_user_request.to_string());
    let process_language_hint = crate::language_policy::task_response_language_hint(
        state,
        task,
        &current_turn_user_request_for_process,
    );
    if let Some(candidate) = active_ordered_entries_count_direct_answer_candidate(
        &current_turn_user_request_for_process,
        agent_run_context.as_ref(),
    ) {
        tracing::info!(
            "{} worker_once: ask active_ordered_entries_count_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&candidate)
        );
        return Ok(ask_reply_with_chat_process(
            candidate,
            &process_language_hint,
        ));
    }
    if let Some(candidate) = recent_count_comparison_direct_answer(
        state,
        task,
        &current_turn_user_request_for_process,
        agent_run_context.as_ref(),
    ) {
        tracing::info!(
            "{} worker_once: ask recent_count_comparison_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&candidate)
        );
        return Ok(ask_reply_with_chat_process(
            candidate,
            &process_language_hint,
        ));
    }
    if let Some(candidate) = runtime_approval_wait_status_direct_answer_candidate(
        agent_run_context.as_ref(),
        &process_language_hint,
    ) {
        tracing::info!(
            "{} worker_once: ask runtime_approval_wait_status_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&candidate)
        );
        return Ok(ask_reply_with_chat_process(
            candidate,
            &process_language_hint,
        ));
    }
    if let Some(candidate) = session_alias_target_direct_answer_candidate(
        state,
        task,
        &current_turn_user_request_for_process,
        agent_run_context.as_ref(),
    ) {
        tracing::info!(
            "{} worker_once: ask session_alias_target_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&candidate)
        );
        return Ok(ask_reply_with_chat_process(
            candidate,
            &process_language_hint,
        ));
    }
    if let Some(reply) = session_alias_rebind_ack(
        state,
        task,
        &current_turn_user_request_for_process,
        &process_language_hint,
    ) {
        tracing::info!(
            "{} worker_once: ask session_alias_rebind_ack task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
        return Ok(reply);
    }
    if let Some(candidate) = normalizer_runtime_fact_direct_answer_candidate(
        state,
        resolved_prompt,
        agent_run_context.as_ref(),
    ) {
        tracing::info!(
            "{} worker_once: ask normalizer_runtime_fact_direct_answer task_id={} len={}",
            crate::highlight_tag("routing"),
            task.task_id,
            candidate.len()
        );
        return Ok(ask_reply_with_chat_process(
            candidate,
            &process_language_hint,
        ));
    }
    if let Some(candidate) =
        active_file_basename_direct_answer_candidate(state, agent_run_context.as_ref())
    {
        tracing::info!(
            "{} worker_once: ask active_file_basename_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            candidate
        );
        return Ok(ask_reply_with_chat_process(
            candidate,
            &process_language_hint,
        ));
    }
    if let Some(candidate) =
        runtime_scalar_path_direct_answer_candidate(state, agent_run_context.as_ref())
    {
        tracing::info!(
            "{} worker_once: ask runtime_scalar_path_direct_answer task_id={} len={}",
            crate::highlight_tag("routing"),
            task.task_id,
            candidate.len()
        );
        return Ok(ask_reply_with_chat_process(
            candidate,
            &process_language_hint,
        ));
    }
    if let Some(reply) =
        state_patch_alias_bindings_ack(agent_run_context.as_ref(), &process_language_hint)
    {
        tracing::info!(
            "{} worker_once: ask state_patch_alias_bindings_ack task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
        return Ok(reply);
    }
    match ask_mode.first_layer_decision() {
        crate::FirstLayerDecision::DirectAnswer => {
            if let Some(candidate) = normalizer_chat_direct_answer_candidate(
                state,
                resolved_prompt,
                agent_run_context.as_ref(),
            ) {
                tracing::info!(
                    "{} worker_once: ask normalizer_verified_runtime_candidate task_id={} len={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    candidate.len()
                );
                return Ok(ask_reply_with_chat_process(
                    candidate,
                    &process_language_hint,
                ));
            }
            let chat_prompt_context = chat_prompt_context_with_route_resolution(
                chat_prompt_context,
                agent_run_context.as_ref(),
            );
            if let Some(candidate) = normalizer_chat_direct_answer_candidate_with_context_summary(
                state,
                resolved_prompt,
                agent_run_context.as_ref(),
                Some(&chat_prompt_context),
            ) {
                tracing::info!(
                    "{} worker_once: ask normalizer_verified_context_candidate task_id={} len={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    candidate.len()
                );
                return Ok(ask_reply_with_chat_process(
                    candidate,
                    &process_language_hint,
                ));
            }
            let resolved_chat_prompt =
                crate::bootstrap::load_required_prompt_template_for_state_with_meta(
                    state,
                    crate::CHAT_RESPONSE_PROMPT_LOGICAL_PATH,
                )
                .map_err(|e| e.to_string())?;
            let chat_prompt_template = resolved_chat_prompt.template;
            let chat_prompt_source = resolved_chat_prompt.source;
            let chat_prompt_version = resolved_chat_prompt.version;
            crate::log_prompt_render_with_version(
                state,
                &task.task_id,
                "chat_response_prompt",
                &chat_prompt_source,
                chat_prompt_version.as_deref(),
                None,
            );
            let task_persona_prompt = state.task_persona_prompt(task);
            let chat_user_request = chat_user_request(resolved_prompt, execution_user_request);
            let current_turn_user_request =
                task_payload_text(task).unwrap_or_else(|| chat_user_request.to_string());
            let request_language_hint = crate::language_policy::task_response_language_hint(
                state,
                task,
                &current_turn_user_request,
            );
            if transform_skill_available_for_plan(state)
                && crate::intent::surface_signals::inline_json_transform_request(
                    &current_turn_user_request,
                )
            {
                if let Some(mut promoted_ctx) = agent_run_context.clone() {
                    if promote_inline_json_transform_context_to_planner(
                        &mut promoted_ctx,
                        &current_turn_user_request,
                    ) {
                        tracing::info!(
                            "{} worker_once: ask inline_json_transform_promoted_to_planner task_id={}",
                            crate::highlight_tag("routing"),
                            task.task_id
                        );
                        let promoted_prompt_with_memory = format!(
                            "{}\n\nStructured inline transform request:\n{}",
                            prompt_with_memory.trim(),
                            current_turn_user_request.trim()
                        );
                        return execute_via_planner_loop(
                            state,
                            task,
                            &promoted_prompt_with_memory,
                            execution_user_request,
                            &crate::AskMode::planner_execute_chat_wrapped(),
                            Some(promoted_ctx),
                        )
                        .await;
                    }
                }
            }
            if let Some(reply) =
                state_patch_alias_bindings_ack(agent_run_context.as_ref(), &request_language_hint)
            {
                tracing::info!(
                    "{} worker_once: ask state_patch_alias_bindings_ack task_id={}",
                    crate::highlight_tag("routing"),
                    task.task_id
                );
                return Ok(reply);
            }
            if let Some(reply) = structural_alias_binding_ack(
                agent_run_context.as_ref(),
                &current_turn_user_request,
                execution_user_request,
                &request_language_hint,
            ) {
                tracing::info!(
                    "{} worker_once: ask structural_alias_binding_ack task_id={}",
                    crate::highlight_tag("routing"),
                    task.task_id
                );
                return Ok(reply);
            }
            if contract_test_hint_should_enter_planner_loop(
                &current_turn_user_request,
                agent_run_context.as_ref(),
            ) {
                tracing::info!(
                    "{} worker_once: ask contract_test_hint_promoted_to_planner task_id={}",
                    crate::highlight_tag("routing"),
                    task.task_id
                );
                return execute_via_planner_loop(
                    state,
                    task,
                    prompt_with_memory,
                    execution_user_request,
                    &crate::AskMode::planner_execute_chat_wrapped(),
                    agent_run_context.clone(),
                )
                .await;
            }
            let mut direct_answer_gate_approved = false;
            let skip_direct_answer_gate =
                direct_answer_gate_can_skip_for_self_contained_payload(
                    &current_turn_user_request,
                    agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref()),
                ) || direct_answer_gate_can_skip_for_pure_chat_draft(
                    state,
                    &current_turn_user_request,
                    agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref()),
                ) || direct_answer_gate_can_skip_for_active_task_text_mutation(
                    &current_turn_user_request,
                    agent_run_context.as_ref(),
                ) || direct_answer_gate_can_skip_for_active_observed_output_chat_repair(
                    agent_run_context.as_ref(),
                ) || direct_answer_gate_can_skip_for_recent_count_context(
                    state,
                    task,
                    agent_run_context.as_ref(),
                );
            if skip_direct_answer_gate {
                tracing::info!(
                    "{} worker_once: ask direct_answer_gate_skipped task_id={}",
                    crate::highlight_tag("routing"),
                    task.task_id
                );
            } else if let Some(mut gate_ctx) = agent_run_context.clone() {
                if let Some(gate) =
                    run_direct_answer_gate(state, task, &current_turn_user_request, Some(&gate_ctx))
                        .await
                {
                    match apply_direct_answer_gate_outcome(
                        state,
                        &mut gate_ctx,
                        &current_turn_user_request,
                        gate,
                    ) {
                        DirectAnswerPreflight::DirectAnswer => {
                            direct_answer_gate_approved = true;
                        }
                        DirectAnswerPreflight::Clarify(question) => {
                            tracing::info!(
                                "{} worker_once: ask direct_answer_gate_clarify task_id={}",
                                crate::highlight_tag("routing"),
                                task.task_id
                            );
                            let question = if question.trim().is_empty() {
                                let clarify_reason = gate_ctx
                                    .route_result
                                    .as_ref()
                                    .map(|route| route.route_reason.as_str())
                                    .unwrap_or("direct_answer_gate_requires_clarify");
                                let structured_clarify_context =
                                    route_structured_clarify_context(Some(&gate_ctx));
                                crate::intent_router::generate_or_reuse_clarify_question(
                                    state,
                                    task,
                                    &current_turn_user_request,
                                    clarify_reason,
                                    structured_clarify_context.as_deref(),
                                    None,
                                    crate::intent_router::ClarifyQuestionPolicy::SafeFallback,
                                    crate::fallback::ClarifyFallbackSource::IntentUnresolved,
                                )
                                .await
                            } else {
                                question
                            };
                            return Ok(ask_reply_with_clarify_process(
                                task,
                                &current_turn_user_request,
                                question,
                                gate_ctx.route_result.as_ref(),
                            ));
                        }
                        DirectAnswerPreflight::PlannerExecute(promoted_ctx) => {
                            tracing::info!(
                                "{} worker_once: ask direct_answer_gate_promoted_to_planner task_id={}",
                                crate::highlight_tag("routing"),
                                task.task_id
                            );
                            let promoted_prompt_with_memory = promoted_ctx
                                .route_result
                                .as_ref()
                                .map(|route| route.resolved_intent.trim())
                                .filter(|intent| {
                                    !intent.is_empty() && *intent != prompt_with_memory.trim()
                                })
                                .map(|intent| {
                                    format!(
                                        "{}\n\nDirect answer gate resolved execution intent:\n{}",
                                        prompt_with_memory.trim(),
                                        intent
                                    )
                                })
                                .unwrap_or_else(|| prompt_with_memory.to_string());
                            return execute_via_planner_loop(
                                state,
                                task,
                                &promoted_prompt_with_memory,
                                execution_user_request,
                                &crate::AskMode::planner_execute_chat_wrapped(),
                                Some(promoted_ctx),
                            )
                            .await;
                        }
                    }
                }
            }
            let chat_user_request = direct_answer_chat_user_request(
                chat_user_request,
                &current_turn_user_request,
                direct_answer_gate_approved,
            );
            let request_for_chat_prompt =
                chat_request_for_prompt(&current_turn_user_request, &chat_user_request);
            let chat_prompt = crate::render_prompt_template(
                &chat_prompt_template,
                &[
                    ("__PERSONA_PROMPT__", &task_persona_prompt),
                    ("__CONTEXT__", &chat_prompt_context),
                    (
                        "__CONFIG_RESPONSE_LANGUAGE__",
                        &state.policy.command_intent.default_locale,
                    ),
                    ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
                    ("__REQUEST__", &request_for_chat_prompt),
                ],
            );
            if let Some(answer) =
                conservative_active_task_rewrite_passthrough(agent_run_context.as_ref())
            {
                tracing::info!(
                    "{} worker_once: ask active_task_rewrite_passthrough task_id={} len={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    answer.len()
                );
                return Ok(ask_reply_with_chat_process(answer, &request_language_hint));
            }
            let raw_answer = crate::llm_gateway::run_with_fallback_with_prompt_source(
                state,
                task,
                &chat_prompt,
                &chat_prompt_source,
            )
            .await
            .map_err(|e| e.to_string())?;
            let mut answer = ensure_active_task_required_visible_literals(
                raw_answer,
                agent_run_context.as_ref(),
            );
            if active_task_text_rewrite_context(agent_run_context.as_ref()) {
                let repair_prompt = active_task_rewrite_conservation_prompt(&chat_prompt, &answer);
                let repaired_answer = crate::llm_gateway::run_with_fallback_with_prompt_source(
                    state,
                    task,
                    &repair_prompt,
                    &chat_prompt_source,
                )
                .await
                .map_err(|e| e.to_string())?;
                answer = ensure_active_task_required_visible_literals(
                    repaired_answer,
                    agent_run_context.as_ref(),
                );
            }
            if direct_chat_answer_needs_repair(&answer) {
                tracing::warn!(
                    "{} worker_once: ask direct_chat_answer_repair task_id={} rejected={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(&answer)
                );
                let repair_prompt = direct_chat_answer_repair_prompt(&chat_prompt, &answer);
                let repaired_answer = crate::llm_gateway::run_with_fallback_with_prompt_source(
                    state,
                    task,
                    &repair_prompt,
                    &chat_prompt_source,
                )
                .await
                .map_err(|e| e.to_string())?;
                let repaired_answer = ensure_active_task_required_visible_literals(
                    repaired_answer,
                    agent_run_context.as_ref(),
                );
                if direct_chat_answer_needs_repair(&repaired_answer) {
                    return Err(format!(
                        "direct chat answer remained malformed after repair: {}",
                        crate::truncate_for_log(&repaired_answer)
                    ));
                }
                answer = repaired_answer;
            }
            Ok(ask_reply_with_chat_process(answer, &request_language_hint))
        }
        crate::FirstLayerDecision::PlannerExecute => {
            execute_via_planner_loop(
                state,
                task,
                prompt_with_memory,
                execution_user_request,
                &ask_mode,
                agent_run_context.clone(),
            )
            .await
        }
        crate::FirstLayerDecision::Clarify => {
            let clarify_reason = agent_run_context
                .as_ref()
                .and_then(|ctx| ctx.route_result.as_ref())
                .map(|route| route.route_reason.as_str())
                .unwrap_or("router_selected_clarify");
            let preferred_clarify = preferred_route_clarify_question(agent_run_context.as_ref());
            let structured_clarify_context =
                route_structured_clarify_context(agent_run_context.as_ref());
            let clarify_policy = if structured_clarify_context.is_some()
                || (preferred_clarify.is_none()
                    && agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref())
                        .is_some_and(|route| route.clarify_question.trim().is_empty()))
            {
                crate::intent_router::ClarifyQuestionPolicy::SafeFallback
            } else {
                crate::intent_router::ClarifyQuestionPolicy::AllowModel
            };
            let clarify = crate::intent_router::generate_or_reuse_clarify_question(
                state,
                task,
                resolved_prompt,
                clarify_reason,
                structured_clarify_context.as_deref(),
                preferred_clarify.as_deref(),
                clarify_policy,
                // §7.2: ask_flow 路由到 AskClarify 但 route_result 也没给 clarify_question
                // → IntentUnresolved（与 ask_pipeline 同语义）。
                crate::fallback::ClarifyFallbackSource::IntentUnresolved,
            )
            .await;
            Ok(ask_reply_with_chat_process(clarify, &process_language_hint))
        }
    }
}

pub(crate) async fn analyze_attached_images_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    resolved_prompt: &str,
) -> anyhow::Result<Option<String>> {
    let Some(images) = payload.get("images").and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    if images.is_empty() {
        return Ok(None);
    }
    let mut args = json!({
        "action": "describe",
        "images": images,
    });
    let instruction = resolved_prompt.trim();
    if let Some(obj) = args.as_object_mut() {
        if !instruction.is_empty() {
            obj.insert(
                "instruction".to_string(),
                Value::String(instruction.to_string()),
            );
        }
        if let Some(language) = payload
            .get("response_language")
            .or_else(|| payload.get("language"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            obj.insert(
                "response_language".to_string(),
                Value::String(language.to_string()),
            );
        }
    }
    crate::skills::run_skill_with_runner(state, task, "image_vision", args)
        .await
        .map_err(anyhow::Error::msg)
        .map(Some)
}

#[cfg(test)]
#[path = "ask_flow_tests.rs"]
mod tests;

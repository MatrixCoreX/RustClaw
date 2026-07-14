use serde_json::Value;
use std::path::{Component, Path, PathBuf};

use crate::memory;
use crate::memory::service::PromptMemoryContext;
use crate::{AppState, ClaimedTask, RouteResult};

#[path = "task_context_builder/summary.rs"]
mod summary;

#[derive(Debug, Clone, Default)]
pub(crate) struct TaskContextRawSources {
    pub(crate) resume_context: String,
    pub(crate) binding_context: String,
    pub(crate) now_iso: String,
    pub(crate) timezone: String,
    pub(crate) schedule_rules: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PlannerContextView {
    pub(crate) visible_skills: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RouteContextView {
    pub(crate) budget_tier: RouteContextBudgetTier,
    pub(crate) active_task_context: String,
    pub(crate) active_execution_anchor_context: String,
    pub(crate) session_alias_context: String,
    pub(crate) request_surface_hints: String,
    pub(crate) recent_execution_context: String,
    pub(crate) capability_map: String,
    pub(crate) recent_assistant_replies: String,
    pub(crate) recent_turns_full: String,
    pub(crate) memory_context: String,
    pub(crate) memory_trace: Option<Value>,
    pub(crate) last_turn_full: String,
}

pub(crate) struct ExecutionContextView {
    pub(crate) budget_tier: ExecutionContextBudgetTier,
    pub(crate) memory_ctx: PromptMemoryContext,
    pub(crate) runtime_context: String,
    pub(crate) active_task_context: String,
    pub(crate) active_execution_anchor_context: String,
    pub(crate) session_alias_context: String,
    pub(crate) recent_turns_full: String,
    pub(crate) last_turn_full: String,
    pub(crate) recent_execution_anchor: String,
    pub(crate) recent_execution_context: String,
    pub(crate) image_context: Option<String>,
}

pub(crate) struct TaskContextBundle {
    pub(crate) raw_sources: TaskContextRawSources,
    pub(crate) planner_view: PlannerContextView,
    pub(crate) route_view: Option<RouteContextView>,
    pub(crate) execution_view: Option<ExecutionContextView>,
}

impl TaskContextBundle {
    pub(crate) fn summary(&self) -> String {
        summary::task_context_bundle_summary(self)
    }

    pub(crate) fn memory_trace(&self) -> Option<Value> {
        self.route_view
            .as_ref()
            .and_then(|view| view.memory_trace.clone())
            .or_else(|| {
                self.execution_view
                    .as_ref()
                    .and_then(|view| view.memory_ctx.memory_trace.clone())
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RouteContextBudgetTier {
    #[default]
    Full,
    AnchorOnly,
    None,
}

impl RouteContextBudgetTier {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::AnchorOnly => "anchor_only",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionContextBudgetTier {
    Full,
    Light,
}

impl ExecutionContextBudgetTier {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Light => "light",
        }
    }
}

fn serialize_context_value(value: Option<&Value>) -> String {
    value
        .map(|v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
        .filter(|s| !s.is_empty() && s != "{}")
        .unwrap_or_else(|| "<none>".to_string())
}

fn canonicalize_for_context(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn build_runtime_context(state: &AppState) -> String {
    let workspace_root = canonicalize_for_context(&state.skill_rt.workspace_root);
    let current_process_cwd = std::env::current_dir()
        .map(|path| canonicalize_for_context(&path))
        .unwrap_or_else(|_| workspace_root.clone());
    format!(
        "### RUNTIME_CONTEXT\n\
current_process_cwd: {}\n\
workspace_root: {}\n\
Use these as current-turn runtime facts. For local filesystem operations, workspace_root is the default workspace boundary; current_process_cwd is the clawd process working directory.",
        current_process_cwd.display(),
        workspace_root.display()
    )
}

fn truncate_context_snippet(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push_str("...(truncated)");
    out
}

fn build_active_task_context(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> String {
    let Some(conversation_state) = session_snapshot.conversation_state.as_ref() else {
        return "<none>".to_string();
    };
    let last_prompt = conversation_state
        .last_primary_task_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let last_output = conversation_state
        .last_primary_task_output
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if last_prompt.is_none() && last_output.is_none() {
        return "<none>".to_string();
    }

    let mut lines = vec![
        "### ACTIVE_TASK_CONTEXT".to_string(),
        "Use this as authoritative semantic context for short follow-ups, corrections, scope updates, and output-shape refinements on the current active task. It is not a filesystem locator or execution target by itself.".to_string(),
    ];
    if let Some(prompt) = last_prompt {
        lines.push("last_primary_task_prompt:".to_string());
        lines.push(truncate_context_snippet(prompt, 700));
    }
    if let Some(output) = last_output {
        lines.push("last_primary_task_output:".to_string());
        lines.push(truncate_context_snippet(output, 1000));
    }
    lines.join("\n")
}

fn ordered_entries_context_line(entries: &[String]) -> Option<String> {
    let mut rendered = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (idx, entry) in entries.iter().enumerate() {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_ascii_lowercase();
        if !seen.insert(normalized) {
            continue;
        }
        rendered.push(format!(
            "{}:{}",
            idx + 1,
            truncate_context_snippet(trimmed, 120)
        ));
        if rendered.len() >= crate::followup_frame::MAX_ORDERED_ENTRIES {
            break;
        }
    }
    (!rendered.is_empty()).then(|| rendered.join(" | "))
}

fn build_active_execution_anchor_context(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> String {
    let mut lines = vec![
        "### ACTIVE_EXECUTION_ANCHOR".to_string(),
        "Latest structured execution state for immediate/proximal follow-ups only. Prefer this over older active-task text for references to the current/latest result, but do not use it when the current request structurally selects an older assistant or execution turn by relative offset; use the matching recent-turn or recent-execution context for that older offset.".to_string(),
    ];
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        let source_request = frame.source_request.trim();
        if !source_request.is_empty() {
            lines.push(format!(
                "followup_source_request: {}",
                truncate_context_snippet(source_request, 180)
            ));
        }
        lines.push(format!("followup_op_kind: {:?}", frame.op_kind));
        if followup_frame_allows_execution_anchor_target(frame) {
            if let Some(target) = frame
                .bound_target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty())
            {
                lines.push(format!(
                    "followup_bound_target: {}",
                    truncate_context_snippet(target, 220)
                ));
            }
            if let Some(entries) = ordered_entries_context_line(&frame.ordered_entries) {
                lines.push(["followup_ordered_entries:", entries.as_str()].join(" "));
            }
        }
    }
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts
            .bound_target
            .as_deref()
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            lines.push(format!(
                "observed_bound_target: {}",
                truncate_context_snippet(target, 220)
            ));
        }
        if let Some(entries) = ordered_entries_context_line(&facts.ordered_entries) {
            lines.push(["observed_ordered_entries:", entries.as_str()].join(" "));
        }
    }
    if lines.len() <= 2 {
        "<none>".to_string()
    } else {
        lines.join("\n")
    }
}

fn build_session_alias_context(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> String {
    let Some(conversation_state) = session_snapshot.conversation_state.as_ref() else {
        return "<none>".to_string();
    };
    if conversation_state.alias_bindings.is_empty() {
        return "<none>".to_string();
    }

    let mut lines = vec![
        "### SESSION_ALIAS_BINDINGS".to_string(),
        "Temporary user-defined references for this session. Use them only when the current message explicitly mentions one of these aliases or updates a mapping. They are not durable memory and not execution evidence by themselves.".to_string(),
    ];
    for binding in conversation_state.alias_bindings.iter().rev().take(8).rev() {
        lines.push(format!(
            "- alias: {}\n  target: {}",
            truncate_context_snippet(&binding.alias, 80),
            truncate_context_snippet(&binding.target, 180)
        ));
    }
    lines.join("\n")
}

fn build_request_surface_hints(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> String {
    let mut lines = Vec::new();
    if let Some((left, right)) = surface.locator_target_pair.as_ref() {
        lines.push(format!("locator_target_pair: {left} | {right}"));
    }
    if lines.is_empty() {
        "<none>".to_string()
    } else {
        let mut block = vec![
            "### REQUEST_SURFACE_HINTS".to_string(),
            "Current-turn structural hints extracted from the user message. These are locator and parameter hints only; semantic planning and response meaning must come from the planner contract, not local phrase classifiers.".to_string(),
        ];
        block.extend(lines);
        block.join("\n")
    }
}

fn request_can_fill_active_clarify_target(
    user_request: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
) -> bool {
    crate::intent::continuation_resolver::surface_can_fill_active_clarify_target(
        user_request,
        active_clarify_state,
        surface,
    )
}

fn observed_facts_provide_immediate_anchor(
    active_observed_facts: Option<&crate::observed_facts::ObservedFacts>,
) -> bool {
    active_observed_facts.is_some_and(|facts| {
        facts.bound_target.is_some()
            || !facts.ordered_entries.is_empty()
            || !facts.delivery_targets.is_empty()
    })
}

fn followup_frame_provides_immediate_anchor(
    active_followup_frame: Option<&crate::followup_frame::FollowupFrame>,
) -> bool {
    active_followup_frame.is_some_and(followup_frame_allows_execution_anchor_target)
}

fn followup_frame_allows_execution_anchor_target(
    frame: &crate::followup_frame::FollowupFrame,
) -> bool {
    matches!(
        frame.op_kind,
        crate::followup_frame::FollowupOpKind::Read
            | crate::followup_frame::FollowupOpKind::List
            | crate::followup_frame::FollowupOpKind::CodeWorkspace
            | crate::followup_frame::FollowupOpKind::Delivery
            | crate::followup_frame::FollowupOpKind::ClarifyPending
    ) && (frame
        .bound_target
        .as_deref()
        .is_some_and(|target| !target.trim().is_empty())
        || !frame.ordered_entries.is_empty())
}

fn needs_text_anchor_probe_for_route(
    user_request: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let _ = (user_request, surface, session_snapshot);
    // Planner-first: do not run a local deictic/pronoun detector before the normalizer.
    // Immediate anchors are still passed through structured session state; semantic
    // reference resolution belongs to the LLM normalizer/planner.
    false
}

fn session_snapshot_provides_execution_state_anchor(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    session_snapshot.active_clarify_state.is_some()
        || followup_frame_provides_immediate_anchor(session_snapshot.active_followup_frame.as_ref())
        || observed_facts_provide_immediate_anchor(session_snapshot.active_observed_facts.as_ref())
}

fn session_snapshot_has_primary_task_context(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    session_snapshot
        .conversation_state
        .as_ref()
        .is_some_and(|state| {
            state
                .last_primary_task_prompt
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                || state
                    .last_primary_task_output
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
        })
}

fn request_qualifies_for_anchor_only_route_context(
    user_request: &str,
    signals: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    let trimmed = user_request.trim();
    if trimmed.is_empty() {
        return false;
    }
    let has_structured_local_read_signal = signals.has_structured_target_refinement();
    signals.has_explicit_path_or_url()
        || signals.is_structural_locator_only_reply()
        || (!signals.has_explicit_path_or_url() && signals.has_single_filename_candidate())
        || signals.locator_target_pair.is_some()
        || has_structured_local_read_signal
}

fn classify_route_context_budget(
    user_request: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    _last_turn_full: &str,
    _recent_assistant_replies: &str,
    active_followup_frame: Option<&crate::followup_frame::FollowupFrame>,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
    active_observed_facts: Option<&crate::observed_facts::ObservedFacts>,
) -> RouteContextBudgetTier {
    let _ = (active_followup_frame, active_observed_facts);
    if request_can_fill_active_clarify_target(user_request, surface, active_clarify_state) {
        return RouteContextBudgetTier::None;
    }
    if request_qualifies_for_anchor_only_route_context(user_request, surface) {
        RouteContextBudgetTier::AnchorOnly
    } else {
        RouteContextBudgetTier::Full
    }
}

struct RouteMemoryContext {
    block: String,
    trace: Option<Value>,
}

fn build_route_memory_context(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    route_budget: RouteContextBudgetTier,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> RouteMemoryContext {
    let decision = memory::use_policy::decide_route_memory_use_policy(
        state,
        route_budget,
        surface,
        session_snapshot,
    );
    if matches!(
        decision.profile,
        memory::use_policy::MemoryUseProfile::Disabled
    ) {
        return RouteMemoryContext {
            block: "<none>".to_string(),
            trace: Some(memory::service::memory_trace_for_structured_context(
                "route",
                &decision,
                &memory::retrieval::StructuredMemoryContext::default(),
                "<none>",
            )),
        };
    }

    let recent_limit = if decision.needs_recent_recall() {
        match route_budget {
            RouteContextBudgetTier::Full => state.policy.memory.prompt_recall_limit.max(1),
            RouteContextBudgetTier::AnchorOnly => 1,
            RouteContextBudgetTier::None => 0,
        }
    } else {
        0
    };

    let structured = memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        user_request,
        recent_limit,
        decision.include_long_term_summary,
        decision.include_preferences,
    );
    let structured = memory::use_policy::filter_structured_memory_context(structured, &decision);
    let block = memory::service::structured_memory_context_block(
        &structured,
        decision.mode,
        decision.max_chars,
    );
    let rendered = if block == "<none>" {
        block
    } else {
        format!("{}\n\n{}", decision.prompt_header(), block)
    };
    RouteMemoryContext {
        trace: Some(memory::service::memory_trace_for_structured_context(
            "route",
            &decision,
            &structured,
            &rendered,
        )),
        block: rendered,
    }
}

fn route_uses_structured_bound_scalar_read(route_result: &RouteResult) -> bool {
    route_result.ask_mode.is_plain_act()
        && route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::CurrentWorkspace
        )
        && !route_result.output_contract.locator_hint.trim().is_empty()
}

fn route_uses_structured_content_read(
    route_result: &RouteResult,
    intent_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    let has_concrete_locator = match route_result.output_contract.locator_kind {
        crate::OutputLocatorKind::Filename
        | crate::OutputLocatorKind::Path
        | crate::OutputLocatorKind::Url => true,
        crate::OutputLocatorKind::CurrentWorkspace => false,
        crate::OutputLocatorKind::None => intent_surface.has_any_locator_reference(),
    };

    route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && has_concrete_locator
}

fn route_uses_bounded_observation_summary_light_budget(route_result: &RouteResult) -> bool {
    if route_result.risk_ceiling != crate::RiskCeiling::Low
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
    {
        return false;
    }
    if !matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
    ) {
        return false;
    }
    route_result.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::CommandOutputSummary,
        crate::OutputSemanticKind::RawCommandOutput,
        crate::OutputSemanticKind::ServiceStatus,
    ]) || route_has_capability_ref_machine_token(route_result)
}

fn route_uses_bounded_scalar_boundary_light_budget(route_result: &RouteResult) -> bool {
    route_result.ask_mode.is_plain_act()
        && !matches!(route_result.risk_ceiling, crate::RiskCeiling::High)
        && route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.delivery_intent == crate::OutputDeliveryIntent::None
        && route_result.schedule_kind == crate::ScheduleKind::None
        && !route_result.wants_file_delivery
        && !route_result.should_refresh_long_term_memory
        && !route_has_any_output_contract_marker(route_result)
}

fn route_uses_bounded_local_machine_boundary_light_budget(route_result: &RouteResult) -> bool {
    if route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(route_result.schedule_kind, crate::ScheduleKind::None)
        || !(route_result.ask_mode.is_plain_act() || route_result.ask_mode.finalize_chat_wrapped())
    {
        return false;
    }
    if route_has_output_contract_marker(route_result, "workspace_project_summary")
        || route_result
            .output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary)
    {
        return false;
    }
    let bounded_response = matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Free
            | crate::OutputResponseShape::Scalar
            | crate::OutputResponseShape::OneSentence
            | crate::OutputResponseShape::Strict
    );
    let bounded_locator = matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
    );
    if !bounded_response || !bounded_locator {
        return false;
    }
    if route_result.has_route_reason_machine_marker("inline_structured_payload_context_execute") {
        return route_result.risk_ceiling == crate::RiskCeiling::Low
            && route_result.output_contract.requires_content_evidence;
    }
    if route_result.has_route_reason_machine_marker("executionless_finalize_trace_plain")
        && route_result.risk_ceiling != crate::RiskCeiling::High
        && !route_result.output_contract.requires_content_evidence
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
    {
        return true;
    }
    if route_result
        .has_route_reason_machine_marker("auto_locator_suppressed_multiple_explicit_paths")
        && matches!(
            route_result.risk_ceiling,
            crate::RiskCeiling::Low | crate::RiskCeiling::Medium
        )
    {
        return true;
    }
    false
}

fn route_uses_structured_clarify_boundary_light_budget(route_result: &RouteResult) -> bool {
    route_result.needs_clarify
        || route_result.has_route_reason_machine_marker("standalone_freeform_clarify_loop_context")
        || route_result.has_route_reason_machine_marker("alias_state_patch_ack")
}

fn route_has_capability_ref_machine_token(route_result: &RouteResult) -> bool {
    [&route_result.route_reason, &route_result.resolved_intent]
        .iter()
        .any(|surface| machine_context_has_capability_ref_token(surface))
}

fn machine_context_has_capability_ref_token(machine_context: &str) -> bool {
    machine_context
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | '(' | ')' | '[' | ']'))
        .map(|part| part.trim().to_ascii_lowercase())
        .any(|part| {
            let Some(capability) = part.strip_prefix("capability_ref=") else {
                return false;
            };
            !capability.is_empty()
                && capability.bytes().any(|byte| byte == b'.')
                && capability.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-' | b'.')
                })
        })
}

pub(crate) fn uses_light_execution_context_budget(
    route_result: &RouteResult,
    resolved_prompt: &str,
) -> bool {
    if uses_local_workspace_execution_context_budget(route_result) {
        return true;
    }
    if route_needs_recent_execution_history(route_result) {
        return false;
    }
    let intent_surface =
        crate::intent::surface_signals::analyze_prompt_surface(route_result.resolved_intent.trim());
    if route_uses_explicit_locator_surface_light_budget(route_result, resolved_prompt) {
        return true;
    }
    if route_uses_structured_clarify_boundary_light_budget(route_result) {
        return true;
    }
    if route_uses_structured_chat_wrapped_light_budget(route_result, &intent_surface) {
        return true;
    }
    if route_uses_bounded_observation_summary_light_budget(route_result) {
        return true;
    }
    if route_uses_bounded_scalar_boundary_light_budget(route_result) {
        return true;
    }
    if route_uses_bounded_local_machine_boundary_light_budget(route_result) {
        return true;
    }
    if !route_result.ask_mode.is_plain_act() {
        return false;
    }
    route_uses_structured_bound_scalar_read(route_result)
        || route_has_output_contract_marker(route_result, "scalar_path_only")
        || route_has_output_contract_marker(route_result, "existence_with_path")
        || route_uses_structured_listing(route_result)
        || route_uses_structured_content_read(route_result, &intent_surface)
}

fn route_uses_explicit_locator_surface_light_budget(
    route_result: &RouteResult,
    resolved_prompt: &str,
) -> bool {
    if route_result.needs_clarify
        || route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::None
        || route_result.schedule_kind != crate::ScheduleKind::None
        || route_result.should_refresh_long_term_memory
        || matches!(route_result.risk_ceiling, crate::RiskCeiling::High)
        || !(route_result.ask_mode.is_plain_act() || route_result.ask_mode.finalize_chat_wrapped())
        || !route_result
            .has_route_reason_machine_marker("executable_contract_preserved_for_agent_loop")
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(resolved_prompt);
    surface.has_explicit_path_or_url() || surface.has_concrete_locator_hint()
}

pub(crate) fn uses_local_workspace_execution_context_budget(route_result: &RouteResult) -> bool {
    if route_result.needs_clarify
        || !(route_result.ask_mode.is_plain_act() || route_result.ask_mode.finalize_chat_wrapped())
        || !matches!(route_result.schedule_kind, crate::ScheduleKind::None)
        || !route_result
            .has_route_reason_machine_marker("executable_contract_preserved_for_agent_loop")
    {
        return false;
    }
    let contract = route_result.effective_output_contract();
    let shape_allowed = matches!(
        contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
            | crate::OutputLocatorKind::None
    ) && matches!(
        contract.response_shape,
        crate::OutputResponseShape::Free
            | crate::OutputResponseShape::Strict
            | crate::OutputResponseShape::OneSentence
            | crate::OutputResponseShape::FileToken
    );
    if !shape_allowed {
        return false;
    }

    workspace_scoped_execution_locator_allowed(
        contract.locator_kind,
        &contract.locator_hint,
        route_result.risk_ceiling,
    )
}

fn workspace_scoped_execution_locator_allowed(
    locator_kind: crate::OutputLocatorKind,
    locator_hint: &str,
    risk_ceiling: crate::RiskCeiling,
) -> bool {
    match locator_kind {
        crate::OutputLocatorKind::Filename => {
            locator_hint_is_relative_workspace_child(locator_hint)
        }
        crate::OutputLocatorKind::Path => locator_hint_is_relative_workspace_child(locator_hint),
        crate::OutputLocatorKind::CurrentWorkspace => true,
        crate::OutputLocatorKind::None => !matches!(risk_ceiling, crate::RiskCeiling::High),
        _ => false,
    }
}

fn locator_hint_is_relative_workspace_child(locator_hint: &str) -> bool {
    let trimmed = locator_hint.trim();
    if trimmed.is_empty() {
        return false;
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return false;
    }
    let mut has_normal = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_normal = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    has_normal
}

const ROUTE_OUTPUT_CONTRACT_MARKERS: &[&str] = &[
    "content_excerpt_summary",
    "content_excerpt_with_summary",
    "content_presence_check",
    "excerpt_kind_judgment",
    "file_basename",
    "file_names",
    "directory_names",
    "directory_entry_groups",
    "file_paths",
    "scalar_path_only",
    "existence_with_path",
    "quantity_comparison",
    "recent_artifacts_judgment",
    "workspace_project_summary",
    "sqlite_table_listing",
    "sqlite_table_names_only",
];

fn route_has_output_contract_marker(route_result: &RouteResult, marker: &str) -> bool {
    route_result.has_route_reason_machine_marker(marker)
}

fn route_has_any_output_contract_marker(route_result: &RouteResult) -> bool {
    ROUTE_OUTPUT_CONTRACT_MARKERS
        .iter()
        .any(|marker| route_has_output_contract_marker(route_result, marker))
}

fn route_uses_structured_listing(route_result: &RouteResult) -> bool {
    route_has_output_contract_marker(route_result, "file_names")
        || route_has_output_contract_marker(route_result, "directory_names")
        || route_has_output_contract_marker(route_result, "directory_entry_groups")
        || route_has_output_contract_marker(route_result, "file_paths")
        || route_has_output_contract_marker(route_result, "sqlite_table_listing")
        || route_has_output_contract_marker(route_result, "sqlite_table_names_only")
        || matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryLookup
                | crate::OutputDeliveryIntent::DirectoryBatchFiles
        )
}

fn route_uses_structured_chat_wrapped_light_budget(
    route_result: &RouteResult,
    intent_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    if !route_result.ask_mode.finalize_chat_wrapped()
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
    {
        return false;
    }
    if route_has_output_contract_marker(route_result, "workspace_project_summary") {
        return false;
    }
    route_uses_structured_content_read(route_result, intent_surface)
}

fn should_prefer_light_execution_memory_from_session(
    route_result: &RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !session_snapshot_provides_execution_state_anchor(session_snapshot)
    {
        return false;
    }
    let stateful_delivery_clarify_act = route_result.ask_mode.is_plain_act()
        && route_result.output_contract.delivery_required
        && session_snapshot.active_clarify_state.is_some();
    let chat_wrapped_content_read = route_result.ask_mode.finalize_chat_wrapped()
        && route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required;
    chat_wrapped_content_read || stateful_delivery_clarify_act
}

fn should_suppress_execution_anchor_context(
    route_result: &RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    budget_tier: ExecutionContextBudgetTier,
) -> bool {
    if route_needs_recent_execution_history(route_result) {
        return false;
    }
    session_snapshot_provides_execution_state_anchor(session_snapshot)
        && (matches!(budget_tier, ExecutionContextBudgetTier::Light)
            || should_prefer_light_execution_memory_from_session(route_result, session_snapshot))
}

fn route_needs_recent_execution_history(route_result: &RouteResult) -> bool {
    if route_has_concrete_locator_hint(route_result)
        && (route_has_output_contract_marker(route_result, "content_excerpt_summary")
            || route_has_output_contract_marker(route_result, "content_excerpt_with_summary")
            || route_has_output_contract_marker(route_result, "content_presence_check")
            || route_has_output_contract_marker(route_result, "excerpt_kind_judgment")
            || route_has_output_contract_marker(route_result, "file_basename"))
    {
        return false;
    }
    route_has_output_contract_marker(route_result, "quantity_comparison")
        || route_has_output_contract_marker(route_result, "content_excerpt_summary")
        || route_has_output_contract_marker(route_result, "content_excerpt_with_summary")
        || route_has_output_contract_marker(route_result, "content_presence_check")
        || route_has_output_contract_marker(route_result, "excerpt_kind_judgment")
        || route_has_output_contract_marker(route_result, "recent_artifacts_judgment")
        || route_has_output_contract_marker(route_result, "file_basename")
}

fn route_has_concrete_locator_hint(route_result: &RouteResult) -> bool {
    matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Url
    ) && !route_result.output_contract.locator_hint.trim().is_empty()
}

pub(crate) fn build_route_task_context_bundle(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    resume_context: Option<&Value>,
    binding_context: Option<&Value>,
    now_iso: &str,
    timezone: &str,
    schedule_rules: &str,
) -> TaskContextBundle {
    let planner_view = PlannerContextView {
        visible_skills: state.planner_available_skills_for_task(task),
    };
    let capability_map = crate::capability_map::build_capability_map_for_task(state, task);
    let owned_session_snapshot;
    let session_snapshot = if let Some(snapshot) = session_snapshot {
        snapshot
    } else {
        owned_session_snapshot =
            crate::conversation_state::load_active_session_snapshot(state, task);
        &owned_session_snapshot
    };
    let user_request_surface = crate::intent::surface_signals::analyze_prompt_surface(user_request);
    let anchor_probe_required =
        needs_text_anchor_probe_for_route(user_request, &user_request_surface, session_snapshot);
    let anchor_probe_recent_assistant_replies = if anchor_probe_required {
        memory::build_recent_assistant_replies_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            3,
            220,
        )
    } else {
        "<none>".to_string()
    };
    let anchor_probe_last_turn_full = if anchor_probe_required {
        memory::build_last_turn_full_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            1200,
            2400,
        )
    } else {
        "<none>".to_string()
    };
    let route_budget = classify_route_context_budget(
        user_request,
        &user_request_surface,
        &anchor_probe_last_turn_full,
        &anchor_probe_recent_assistant_replies,
        session_snapshot.active_followup_frame.as_ref(),
        session_snapshot.active_clarify_state.as_ref(),
        session_snapshot.active_observed_facts.as_ref(),
    );
    let route_memory_context = build_route_memory_context(
        state,
        task,
        user_request,
        route_budget,
        &user_request_surface,
        session_snapshot,
    );
    let route_view = RouteContextView {
        budget_tier: route_budget,
        active_task_context: build_active_task_context(session_snapshot),
        active_execution_anchor_context: build_active_execution_anchor_context(session_snapshot),
        session_alias_context: build_session_alias_context(session_snapshot),
        request_surface_hints: build_request_surface_hints(&user_request_surface),
        recent_execution_context: match route_budget {
            RouteContextBudgetTier::Full => {
                crate::routing_context::build_recent_execution_context(state, task, 8)
            }
            RouteContextBudgetTier::AnchorOnly => {
                crate::routing_context::build_recent_execution_anchor_context(state, task)
            }
            RouteContextBudgetTier::None => "<none>".to_string(),
        },
        capability_map,
        recent_assistant_replies: match route_budget {
            RouteContextBudgetTier::Full => memory::build_recent_assistant_replies_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                3,
                220,
            ),
            RouteContextBudgetTier::AnchorOnly => memory::build_recent_assistant_replies_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                2,
                160,
            ),
            RouteContextBudgetTier::None => "<none>".to_string(),
        },
        recent_turns_full: match route_budget {
            RouteContextBudgetTier::Full => memory::build_recent_turns_full_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                5,
                560,
                6400,
            ),
            RouteContextBudgetTier::AnchorOnly => memory::build_recent_turns_full_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                4,
                220,
                1400,
            ),
            RouteContextBudgetTier::None => "<none>".to_string(),
        },
        memory_context: route_memory_context.block,
        memory_trace: route_memory_context.trace,
        last_turn_full: match route_budget {
            RouteContextBudgetTier::Full => memory::build_last_turn_full_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                1200,
                2400,
            ),
            RouteContextBudgetTier::AnchorOnly => memory::build_last_turn_full_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                800,
                1200,
            ),
            RouteContextBudgetTier::None => "<none>".to_string(),
        },
    };
    TaskContextBundle {
        raw_sources: TaskContextRawSources {
            resume_context: serialize_context_value(resume_context),
            binding_context: serialize_context_value(binding_context),
            now_iso: now_iso.to_string(),
            timezone: timezone.to_string(),
            schedule_rules: schedule_rules.to_string(),
        },
        planner_view,
        route_view: Some(route_view),
        execution_view: None,
    }
}

pub(crate) fn build_execution_task_context_bundle(
    state: &AppState,
    task: &ClaimedTask,
    route_result: &RouteResult,
    resolved_prompt: &str,
    chat_memory_budget_chars: usize,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> TaskContextBundle {
    let planner_view = PlannerContextView {
        visible_skills: state.planner_available_skills_for_task(task),
    };
    let session_snapshot = crate::conversation_state::load_active_session_snapshot(state, task);
    let budget_tier = if uses_light_execution_context_budget(route_result, resolved_prompt) {
        ExecutionContextBudgetTier::Light
    } else {
        ExecutionContextBudgetTier::Full
    };
    let needs_recent_execution_history = route_needs_recent_execution_history(route_result);
    let has_active_session_state =
        session_snapshot_provides_execution_state_anchor(&session_snapshot);
    let has_active_primary_task_state =
        session_snapshot_has_primary_task_context(&session_snapshot);
    let has_active_task_context = has_active_session_state || has_active_primary_task_state;
    let suppress_preference_context =
        should_suppress_active_task_context(route_result, turn_analysis);
    let suppress_execution_text_context = suppress_preference_context
        || route_result.is_execute_gate()
            && matches!(budget_tier, ExecutionContextBudgetTier::Full)
            && has_active_session_state
            && !needs_recent_execution_history;
    let suppress_boundary_only_execution_anchor =
        should_suppress_boundary_only_execution_anchor_context(route_result, turn_analysis);
    let suppress_execution_anchor_context = suppress_preference_context
        || suppress_boundary_only_execution_anchor
        || should_suppress_execution_anchor_context(route_result, &session_snapshot, budget_tier);
    let planner_memory_decision = memory::use_policy::decide_planner_memory_use_policy(
        state,
        budget_tier,
        &route_result.ask_mode,
        planner_memory_context_hint(route_result, turn_analysis, has_active_session_state),
    );
    let chat_memory_decision = memory::use_policy::decide_chat_memory_use_policy(
        state,
        budget_tier,
        &route_result.ask_mode,
        &route_result.route_reason,
        has_active_session_state,
        chat_memory_budget_chars,
        chat_memory_context_hint(route_result, turn_analysis, has_active_task_context),
    );
    let memory_ctx = memory::service::prepare_prompt_with_memory_for_policy(
        state,
        task,
        resolved_prompt,
        &planner_memory_decision,
        &chat_memory_decision,
    );
    let mut execution_view = ExecutionContextView {
        budget_tier,
        memory_ctx,
        runtime_context: build_runtime_context(state),
        active_task_context: if suppress_preference_context {
            "<none>".to_string()
        } else {
            build_active_task_context(&session_snapshot)
        },
        active_execution_anchor_context: if suppress_preference_context
            || suppress_boundary_only_execution_anchor
        {
            "<none>".to_string()
        } else {
            build_active_execution_anchor_context(&session_snapshot)
        },
        session_alias_context: if suppress_preference_context {
            "<none>".to_string()
        } else {
            build_session_alias_context(&session_snapshot)
        },
        recent_turns_full: if matches!(budget_tier, ExecutionContextBudgetTier::Full)
            && !suppress_execution_text_context
        {
            memory::build_recent_turns_full_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                5,
                560,
                6400,
            )
        } else {
            "<none>".to_string()
        },
        last_turn_full: if matches!(budget_tier, ExecutionContextBudgetTier::Full)
            && !suppress_execution_text_context
        {
            memory::build_last_turn_full_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                1200,
                2400,
            )
        } else {
            "<none>".to_string()
        },
        recent_execution_anchor: if matches!(budget_tier, ExecutionContextBudgetTier::Full)
            && !suppress_execution_text_context
            && !suppress_execution_anchor_context
        {
            crate::routing_context::build_recent_execution_anchor_context(state, task)
        } else {
            "<none>".to_string()
        },
        recent_execution_context: if suppress_execution_anchor_context {
            "<none>".to_string()
        } else {
            crate::routing_context::build_recent_execution_context(state, task, 8)
        },
        image_context: None,
    };
    if matches!(budget_tier, ExecutionContextBudgetTier::Light)
        && !suppress_execution_anchor_context
    {
        execution_view.recent_execution_context =
            crate::routing_context::build_recent_execution_anchor_context(state, task);
    }
    TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view,
        route_view: None,
        execution_view: Some(execution_view),
    }
}

fn chat_memory_context_hint(
    route_result: &RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    has_active_session_state: bool,
) -> memory::use_policy::ChatMemoryContextHint {
    if active_task_context_only_memory_context(
        route_result,
        turn_analysis,
        has_active_session_state,
    ) {
        return memory::use_policy::ChatMemoryContextHint::ActiveTaskContextOnly;
    }
    if current_request_only_memory_context(route_result, turn_analysis, has_active_session_state) {
        memory::use_policy::ChatMemoryContextHint::CurrentRequestOnly
    } else {
        memory::use_policy::ChatMemoryContextHint::Default
    }
}

fn planner_memory_context_hint(
    route_result: &RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    has_active_session_state: bool,
) -> memory::use_policy::PlannerMemoryContextHint {
    if current_request_only_memory_context(route_result, turn_analysis, has_active_session_state) {
        memory::use_policy::PlannerMemoryContextHint::StableFactsDisabled
    } else {
        memory::use_policy::PlannerMemoryContextHint::Default
    }
}

fn active_task_context_only_memory_context(
    route_result: &RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    has_active_session_state: bool,
) -> bool {
    if !has_active_session_state
        || route_result.needs_clarify
        || !route_result.is_resume_discussion_mode()
        || route_result.wants_file_delivery
        || route_result.schedule_kind != crate::ScheduleKind::None
        || route_result.should_refresh_long_term_memory
    {
        return false;
    }
    let Some(analysis) = turn_analysis else {
        return false;
    };
    if analysis.attachment_processing_required || analysis.should_interrupt_active_run {
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
    let contract = &route_result.output_contract;
    !contract.requires_content_evidence
        && !contract.delivery_required
        && contract.locator_kind == crate::OutputLocatorKind::None
        && contract.delivery_intent == crate::OutputDeliveryIntent::None
        && !route_has_any_output_contract_marker(route_result)
}

fn current_request_only_memory_context(
    route_result: &RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    has_active_session_state: bool,
) -> bool {
    if has_active_session_state || !route_result.is_resume_discussion_mode() {
        return false;
    }
    if let Some(analysis) = turn_analysis {
        if analysis.turn_type != Some(crate::intent_router::TurnType::TaskRequest)
            || !matches!(
                analysis.target_task_policy,
                None | Some(crate::intent_router::TargetTaskPolicy::Standalone)
            )
        {
            return false;
        }
    }
    let contract = &route_result.output_contract;
    route_result.schedule_kind == crate::ScheduleKind::None
        && !route_result.wants_file_delivery
        && !route_result.should_refresh_long_term_memory
        && !contract.requires_content_evidence
        && !contract.delivery_required
        && contract.locator_kind == crate::OutputLocatorKind::None
        && contract.delivery_intent == crate::OutputDeliveryIntent::None
        && !route_has_any_output_contract_marker(route_result)
}

fn should_suppress_active_task_context(
    route_result: &RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(analysis) = turn_analysis else {
        return false;
    };
    if analysis.turn_type != Some(crate::intent_router::TurnType::PreferenceOrMemory)
        || !matches!(
            analysis.target_task_policy,
            None | Some(crate::intent_router::TargetTaskPolicy::Standalone)
        )
        || analysis.attachment_processing_required
        || analysis.should_interrupt_active_run
        || route_result.needs_clarify
        || route_result.wants_file_delivery
        || route_result.schedule_kind != crate::ScheduleKind::None
        || route_result.should_refresh_long_term_memory
    {
        return false;
    }
    let contract = &route_result.output_contract;
    !contract.requires_content_evidence
        && !contract.delivery_required
        && contract.locator_kind == crate::OutputLocatorKind::None
        && contract.delivery_intent == crate::OutputDeliveryIntent::None
        && !route_has_any_output_contract_marker(route_result)
}

fn should_suppress_boundary_only_execution_anchor_context(
    route_result: &RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if route_result.needs_clarify
        || route_result.wants_file_delivery
        || route_result.schedule_kind != crate::ScheduleKind::None
        || route_result.should_refresh_long_term_memory
    {
        return false;
    }
    if let Some(analysis) = turn_analysis {
        if analysis.attachment_processing_required
            || analysis.should_interrupt_active_run
            || matches!(
                analysis.turn_type,
                Some(
                    crate::intent_router::TurnType::TaskAppend
                        | crate::intent_router::TurnType::TaskCorrect
                        | crate::intent_router::TurnType::TaskReplace
                        | crate::intent_router::TurnType::TaskScopeUpdate
                        | crate::intent_router::TurnType::RunControl
                )
            )
            || matches!(
                analysis.target_task_policy,
                Some(
                    crate::intent_router::TargetTaskPolicy::ReuseActive
                        | crate::intent_router::TargetTaskPolicy::ReplaceActive
                        | crate::intent_router::TargetTaskPolicy::PauseAndQueue
                )
            )
            || analysis
                .state_patch
                .as_ref()
                .is_some_and(task_context_state_patch_is_meaningful)
        {
            return false;
        }
    }
    let contract = &route_result.output_contract;
    let is_boundary_only_finalize = route_result.is_resume_discussion_mode()
        || route_result.has_route_reason_machine_marker("executionless_finalize_trace_plain");
    is_boundary_only_finalize
        && !contract.requires_content_evidence
        && !contract.delivery_required
        && contract.locator_kind == crate::OutputLocatorKind::None
        && contract.delivery_intent == crate::OutputDeliveryIntent::None
        && !route_has_any_output_contract_marker(route_result)
}

fn task_context_state_patch_is_meaningful(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
        Value::Array(items) => items.iter().any(task_context_state_patch_is_meaningful),
        Value::Object(map) => map.values().any(task_context_state_patch_is_meaningful),
    }
}

pub(crate) fn set_execution_image_context(
    bundle: &mut TaskContextBundle,
    image_context: Option<String>,
) {
    if let Some(execution_view) = bundle.execution_view.as_mut() {
        execution_view.image_context = image_context;
    }
}

pub(crate) fn apply_execution_context_to_prompts(
    bundle: &TaskContextBundle,
    chat_prompt_context: &mut String,
    resolved_prompt_for_execution: &mut String,
    prompt_with_memory_for_execution: &mut String,
) {
    let Some(execution_view) = bundle.execution_view.as_ref() else {
        return;
    };
    if execution_view.runtime_context != "<none>" {
        chat_prompt_context.push_str("\n\n");
        chat_prompt_context.push_str(&execution_view.runtime_context);
        prompt_with_memory_for_execution.push_str("\n\n");
        prompt_with_memory_for_execution.push_str(&execution_view.runtime_context);
    }
    if execution_view.session_alias_context != "<none>" {
        let alias_context_block = format!(
            "\n\n{}\nAlias execution rule: when the current goal or request mentions more than one alias, treat each alias target as an independent authoritative concrete target. Do not rebuild a file alias under another directory alias unless that exact alias target says it is inside that directory.",
            execution_view.session_alias_context
        );
        resolved_prompt_for_execution.push_str(&alias_context_block);
        prompt_with_memory_for_execution.push_str(&alias_context_block);
    }
    if execution_view.active_task_context != "<none>" {
        let active_task_context_block = format!("\n\n{}", execution_view.active_task_context);
        resolved_prompt_for_execution.push_str(&active_task_context_block);
        prompt_with_memory_for_execution.push_str(&active_task_context_block);
    }
    if execution_view.active_execution_anchor_context != "<none>" {
        let anchor_context_block = format!(
            "\n\n{}\nActive ordered-entry rule: when the current request semantically selects an item by position or relative position from this active ordered list and the reference is to the current/latest result, use that exact listed entry under the bound target. Do not re-list, sort, or reinterpret the parent directory to choose a different item. If the request structurally selects an older turn/result, do not apply this active ordered list; bind the selected item from the matching recent-turn or recent-execution context instead.",
            execution_view.active_execution_anchor_context
        );
        resolved_prompt_for_execution.push_str(&anchor_context_block);
        prompt_with_memory_for_execution.push_str(&anchor_context_block);
    }
    if execution_view.recent_turns_full != "<none>" {
        chat_prompt_context.push_str("\n\n");
        chat_prompt_context.push_str(&execution_view.recent_turns_full);
    } else if execution_view.last_turn_full != "<none>" {
        chat_prompt_context.push_str("\n\n");
        chat_prompt_context.push_str(&execution_view.last_turn_full);
    }
    let prompt_execution_context = if execution_view.recent_execution_anchor != "<none>" {
        execution_view.recent_execution_anchor.as_str()
    } else if execution_view
        .recent_execution_context
        .trim_start()
        .starts_with("###")
    {
        execution_view.recent_execution_context.as_str()
    } else {
        "<none>"
    };
    if prompt_execution_context != "<none>" {
        prompt_with_memory_for_execution.push_str(
            "\n\n### RECENT_EXECUTION_CONTEXT\n\
Use this block only as supporting evidence for genuinely short follow-up requests. Reuse a previous target only when the current request or recent context already binds exactly one concrete target of the correct type. Do not let this block override a needed clarification, and do not treat an artifact-type noun alone as a concrete target.\n",
        );
        prompt_with_memory_for_execution.push_str(prompt_execution_context);
    }
    if let Some(image_context) = execution_view
        .image_context
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        let image_context_block =
            format!("\n\nAttached image analysis context:\n{}", image_context);
        resolved_prompt_for_execution.push_str(&image_context_block);
        prompt_with_memory_for_execution.push_str(&image_context_block);
    }
}

#[cfg(test)]
#[path = "task_context_builder_tests.rs"]
mod tests;

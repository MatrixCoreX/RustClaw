use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::memory;
use crate::memory::service::PromptMemoryContext;
use crate::{AppState, ClaimedTask, RouteResult};

#[derive(Debug, Clone, Default)]
pub(crate) struct TaskContextRawSources {
    pub(crate) resume_context: String,
    pub(crate) binding_context: String,
    pub(crate) now_iso: String,
    pub(crate) timezone: String,
    pub(crate) schedule_rules: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct PlannerContextView {
    pub(crate) visible_skills: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RouteContextView {
    pub(crate) budget_tier: RouteContextBudgetTier,
    pub(crate) active_task_context: String,
    pub(crate) request_surface_hints: String,
    pub(crate) recent_execution_context: String,
    pub(crate) capability_map: String,
    pub(crate) recent_assistant_replies: String,
    pub(crate) recent_turns_full: String,
    pub(crate) memory_context: String,
    pub(crate) last_turn_full: String,
}

pub(crate) struct ExecutionContextView {
    pub(crate) budget_tier: ExecutionContextBudgetTier,
    pub(crate) memory_ctx: PromptMemoryContext,
    pub(crate) runtime_context: String,
    pub(crate) recent_turns_full: String,
    pub(crate) last_turn_full: String,
    pub(crate) recent_execution_anchor: String,
    pub(crate) recent_execution_context: String,
    pub(crate) image_context: Option<String>,
}

#[allow(dead_code)]
pub(crate) struct TaskContextBundle {
    pub(crate) raw_sources: TaskContextRawSources,
    pub(crate) planner_view: PlannerContextView,
    pub(crate) route_view: Option<RouteContextView>,
    pub(crate) execution_view: Option<ExecutionContextView>,
}

impl TaskContextBundle {
    pub(crate) fn summary(&self) -> String {
        let route_attached = self.route_view.is_some();
        let route_budget = self
            .route_view
            .as_ref()
            .map(|view| view.budget_tier.as_str())
            .unwrap_or("n/a");
        let execution_attached = self.execution_view.is_some();
        let execution_budget = self
            .execution_view
            .as_ref()
            .map(|view| view.budget_tier.as_str())
            .unwrap_or("n/a");
        let visible_skills = self.planner_view.visible_skills.len();
        let has_resume_context = self.raw_sources.resume_context != "<none>";
        let has_binding_context = self.raw_sources.binding_context != "<none>";
        format!(
            "route_view={} route_budget={} execution_view={} execution_budget={} visible_skills={} resume_context={} binding_context={}",
            route_attached,
            route_budget,
            execution_attached,
            execution_budget,
            visible_skills,
            has_resume_context,
            has_binding_context
        )
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

fn build_request_surface_hints(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> String {
    let mut lines = Vec::new();
    if let Some(count) = surface.requested_sentence_count {
        lines.push(format!("requested_sentence_count: {count}"));
    }
    if let Some(range) = surface.requested_read_range {
        lines.push(format!(
            "requested_read_range: {}",
            render_requested_read_range(range)
        ));
    }
    if let Some((left, right)) = surface.compare_target_pair.as_ref() {
        lines.push(format!("compare_target_pair: {left} | {right}"));
    }
    if let Some(dir_hint) = surface.workspace_child_directory_hint.as_deref() {
        lines.push(format!("workspace_child_directory_hint: {dir_hint}"));
    }
    if let Some(limit) = surface.requested_listing_limit {
        lines.push(format!("requested_listing_limit: {limit}"));
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

fn render_requested_read_range(range: crate::read_range_request::RequestedReadRange) -> String {
    match range {
        crate::read_range_request::RequestedReadRange::Head { n } => format!("head:{n}"),
        crate::read_range_request::RequestedReadRange::Tail { n } => format!("tail:{n}"),
        crate::read_range_request::RequestedReadRange::Range {
            start_line,
            end_line,
        } => format!("range:{start_line}-{end_line}"),
    }
}

fn request_looks_like_fresh_deictic_reference(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.token_count > 0 && surface.has_fresh_or_object_deictic_reference()
}

fn should_suppress_route_memory_context(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    last_turn_full: &str,
    recent_assistant_replies: &str,
    active_followup_frame: Option<&crate::followup_frame::FollowupFrame>,
    active_observed_facts: Option<&crate::observed_facts::ObservedFacts>,
) -> bool {
    request_looks_like_fresh_deictic_reference(surface)
        && !crate::intent::continuation_resolver::context_contains_immediate_locator_anchor(
            last_turn_full,
        )
        && !crate::intent::continuation_resolver::context_contains_immediate_locator_anchor(
            recent_assistant_replies,
        )
        && !followup_frame_provides_immediate_anchor(active_followup_frame)
        && !observed_facts_provide_immediate_anchor(active_observed_facts)
}

fn request_looks_like_active_clarify_reply(
    user_request: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
) -> bool {
    crate::intent::continuation_resolver::prompt_looks_like_active_clarify_reply_with_surface(
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
    active_followup_frame.is_some_and(|frame| {
        frame
            .bound_target
            .as_deref()
            .is_some_and(|target| !target.trim().is_empty())
            || !frame.ordered_entries.is_empty()
    })
}

fn needs_text_anchor_probe_for_route(
    user_request: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    request_looks_like_fresh_deictic_reference(surface)
        && crate::conversation_state::session_alias_target_for_prompt(
            user_request,
            Some(session_snapshot),
        )
        .is_none()
        && !request_looks_like_active_clarify_reply(
            user_request,
            surface,
            session_snapshot.active_clarify_state.as_ref(),
        )
        && !crate::intent::continuation_resolver::session_contains_immediate_locator_anchor(Some(
            session_snapshot,
        ))
}

fn session_snapshot_provides_execution_state_anchor(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    session_snapshot.active_clarify_state.is_some()
        || followup_frame_provides_immediate_anchor(session_snapshot.active_followup_frame.as_ref())
        || observed_facts_provide_immediate_anchor(session_snapshot.active_observed_facts.as_ref())
}

fn request_qualifies_for_anchor_only_route_context(
    user_request: &str,
    signals: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    let trimmed = user_request.trim();
    if trimmed.is_empty() || request_looks_like_fresh_deictic_reference(signals) {
        return false;
    }
    let has_structured_local_read_signal =
        signals.has_structured_target_refinement() || signals.has_workspace_single_token_hint();
    signals.has_explicit_path_or_url()
        || signals.looks_like_locator_only_reply()
        || (!signals.has_explicit_path_or_url() && signals.has_single_filename_candidate())
        || signals.compare_target_pair.is_some()
        || has_structured_local_read_signal
}

fn classify_route_context_budget(
    user_request: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    last_turn_full: &str,
    recent_assistant_replies: &str,
    active_followup_frame: Option<&crate::followup_frame::FollowupFrame>,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
    active_observed_facts: Option<&crate::observed_facts::ObservedFacts>,
) -> RouteContextBudgetTier {
    if request_looks_like_active_clarify_reply(user_request, surface, active_clarify_state) {
        return RouteContextBudgetTier::None;
    }
    if request_looks_like_fresh_deictic_reference(surface) {
        if should_suppress_route_memory_context(
            surface,
            last_turn_full,
            recent_assistant_replies,
            active_followup_frame,
            active_observed_facts,
        ) {
            RouteContextBudgetTier::None
        } else {
            RouteContextBudgetTier::AnchorOnly
        }
    } else if request_qualifies_for_anchor_only_route_context(user_request, surface) {
        RouteContextBudgetTier::AnchorOnly
    } else {
        RouteContextBudgetTier::Full
    }
}

fn build_route_memory_context(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    route_budget: RouteContextBudgetTier,
) -> String {
    if !state.policy.memory.route_memory_enabled
        || matches!(route_budget, RouteContextBudgetTier::None)
    {
        return "<none>".to_string();
    }

    let (recent_limit, include_long_term, include_preferences, max_chars) = match route_budget {
        RouteContextBudgetTier::Full => (
            state.policy.memory.prompt_recall_limit.max(1),
            true,
            true,
            state
                .policy
                .memory
                .route_trigger_budget_chars
                .max(384)
                .min(state.policy.memory.route_memory_max_chars.max(384)),
        ),
        RouteContextBudgetTier::AnchorOnly => (
            1,
            true,
            false,
            state
                .policy
                .memory
                .route_trigger_budget_chars
                .max(384)
                .min(640)
                .min(state.policy.memory.route_memory_max_chars.max(384)),
        ),
        RouteContextBudgetTier::None => unreachable!(),
    };

    let structured = memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        user_request,
        recent_limit,
        include_long_term,
        include_preferences,
    );
    memory::service::structured_memory_context_block(
        &structured,
        memory::retrieval::MemoryContextMode::Route,
        max_chars,
    )
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

pub(crate) fn uses_light_execution_context_budget(
    route_result: &RouteResult,
    _resolved_prompt: &str,
) -> bool {
    if route_result.needs_clarify {
        return false;
    }
    let intent = route_result.resolved_intent.trim();
    let intent_surface = crate::intent::surface_signals::analyze_prompt_surface(intent);
    if route_uses_structured_chat_wrapped_light_budget(route_result, &intent_surface) {
        return true;
    }
    if !route_result.ask_mode.is_plain_act() {
        return false;
    }
    route_uses_structured_bound_scalar_read(route_result)
        || route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarPathOnly
        || route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::ExistenceWithPath
        || route_uses_structured_listing(route_result)
        || route_uses_structured_content_read(route_result, &intent_surface)
}

fn route_uses_structured_listing(route_result: &RouteResult) -> bool {
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::SqliteTableListing
            | crate::OutputSemanticKind::SqliteTableNamesOnly
    ) || matches!(
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
    if route_result.output_contract.semantic_kind
        == crate::OutputSemanticKind::WorkspaceProjectSummary
    {
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
    session_snapshot_provides_execution_state_anchor(session_snapshot)
        && (matches!(budget_tier, ExecutionContextBudgetTier::Light)
            || should_prefer_light_execution_memory_from_session(route_result, session_snapshot))
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
        visible_skills: state.planner_visible_skills_for_task(task),
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
    let route_view = RouteContextView {
        budget_tier: route_budget,
        active_task_context: build_active_task_context(session_snapshot),
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
        memory_context: build_route_memory_context(state, task, user_request, route_budget),
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
) -> TaskContextBundle {
    let planner_view = PlannerContextView {
        visible_skills: state.planner_visible_skills_for_task(task),
    };
    let session_snapshot = crate::conversation_state::load_active_session_snapshot(state, task);
    let budget_tier = if uses_light_execution_context_budget(route_result, resolved_prompt) {
        ExecutionContextBudgetTier::Light
    } else {
        ExecutionContextBudgetTier::Full
    };
    let suppress_execution_text_context = matches!(budget_tier, ExecutionContextBudgetTier::Full)
        && session_snapshot_provides_execution_state_anchor(&session_snapshot);
    let memory_budget_mode = if matches!(budget_tier, ExecutionContextBudgetTier::Light)
        || should_prefer_light_execution_memory_from_session(route_result, &session_snapshot)
    {
        crate::memory::service::PromptMemoryBudgetMode::Light
    } else {
        crate::memory::service::PromptMemoryBudgetMode::Full
    };
    let suppress_execution_anchor_context =
        should_suppress_execution_anchor_context(route_result, &session_snapshot, budget_tier);
    let memory_ctx = memory::service::prepare_prompt_with_memory_for_mode(
        state,
        task,
        resolved_prompt,
        chat_memory_budget_chars,
        memory_budget_mode,
    );
    let mut execution_view = ExecutionContextView {
        budget_tier,
        memory_ctx,
        runtime_context: build_runtime_context(state),
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
    if execution_view.recent_turns_full != "<none>" {
        chat_prompt_context.push_str("\n\n");
        chat_prompt_context.push_str(&execution_view.recent_turns_full);
    } else if execution_view.last_turn_full != "<none>" {
        chat_prompt_context.push_str("\n\n");
        chat_prompt_context.push_str(&execution_view.last_turn_full);
    }
    if execution_view.recent_execution_anchor != "<none>" {
        prompt_with_memory_for_execution.push_str(
            "\n\n### RECENT_EXECUTION_CONTEXT\n\
Use this block only as supporting evidence for genuinely short follow-up requests. Reuse a previous target only when the current request or recent context already binds exactly one concrete target of the correct type. Do not let this block override a needed clarification, and do not treat an artifact type word alone (for example README / config / log) as a concrete target.\n",
        );
        prompt_with_memory_for_execution.push_str(&execution_view.recent_execution_anchor);
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
mod tests {
    use super::{
        apply_execution_context_to_prompts, build_active_task_context, build_request_surface_hints,
        classify_route_context_budget, needs_text_anchor_probe_for_route,
        observed_facts_provide_immediate_anchor, request_looks_like_active_clarify_reply,
        request_looks_like_fresh_deictic_reference,
        request_qualifies_for_anchor_only_route_context,
        session_snapshot_provides_execution_state_anchor,
        should_prefer_light_execution_memory_from_session,
        should_suppress_execution_anchor_context, should_suppress_route_memory_context,
        uses_light_execution_context_budget, ExecutionContextView, PlannerContextView,
        RouteContextBudgetTier, TaskContextBundle, TaskContextRawSources,
    };

    #[test]
    fn active_task_context_is_empty_without_primary_task_state() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert_eq!(build_active_task_context(&snapshot), "<none>");
    }

    #[test]
    fn active_task_context_includes_primary_prompt_and_output() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "Write one deployment note that mentions Python 3.10".to_string(),
                ),
                last_primary_task_output: Some(
                    "Ensure the target environment has Python 3.10 installed.".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let context = build_active_task_context(&snapshot);
        assert!(context.contains("### ACTIVE_TASK_CONTEXT"));
        assert!(context.contains("last_primary_task_prompt:"));
        assert!(context.contains("Write one deployment note"));
        assert!(context.contains("last_primary_task_output:"));
        assert!(context.contains("Python 3.10 installed"));
        assert!(context.contains("not a filesystem locator"));
    }

    #[test]
    fn active_task_context_truncates_long_primary_output() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Write a compact test plan".to_string()),
                last_primary_task_output: Some("x".repeat(1100)),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let context = build_active_task_context(&snapshot);
        assert!(context.contains("...(truncated)"));
        assert!(context.len() < 1500);
    }

    #[test]
    fn request_surface_hints_include_workspace_child_directory_hint() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface("看看 docs 目录");
        let rendered = build_request_surface_hints(&surface);
        assert!(rendered.contains("### REQUEST_SURFACE_HINTS"));
        assert!(rendered.contains("workspace_child_directory_hint: docs"));
    }

    #[test]
    fn request_surface_hints_do_not_export_semantic_phrase_shapes() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "看看 data/db-basic-contract.sqlite 里有哪些表，并简短告诉我结果",
        );
        let rendered = build_request_surface_hints(&surface);
        assert_eq!(rendered, "<none>");

        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "用一句话说当前机器的包管理器是什么",
        );
        let rendered = build_request_surface_hints(&surface);
        assert!(rendered.contains("requested_sentence_count: 1"));
        assert!(!rendered.contains("workspace_root_request_shape"));
        assert!(!rendered.contains("semantic_request_shape"));
        assert!(!rendered.contains("table_request_shape"));
        assert!(!rendered.contains("output_request_shape"));
        assert!(!rendered.contains("output_compression_shape"));
    }

    #[test]
    fn request_surface_hints_include_requested_sentence_count() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "读一下 README.md，然后用三句话总结",
        );
        let rendered = build_request_surface_hints(&surface);
        assert!(rendered.contains("### REQUEST_SURFACE_HINTS"));
        assert!(rendered.contains("requested_sentence_count: 3"));
    }

    #[test]
    fn request_surface_hints_include_requested_read_range() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "先读一下 README.md 前 4 行，再用一句话说重点",
        );
        let rendered = build_request_surface_hints(&surface);
        assert!(rendered.contains("### REQUEST_SURFACE_HINTS"));
        assert!(rendered.contains("requested_read_range: head:4"));
    }

    #[test]
    fn request_surface_hints_include_compare_target_pair() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "比较 README.md 和 AGENTS.md 哪个更大",
        );
        let rendered = build_request_surface_hints(&surface);
        assert!(rendered.contains("### REQUEST_SURFACE_HINTS"));
        assert!(rendered.contains("compare_target_pair:"));
        assert!(rendered.contains("README.md"));
        assert!(rendered.contains("AGENTS.md"));
    }

    #[test]
    fn detects_fresh_deictic_reference_without_explicit_path() {
        assert!(request_looks_like_fresh_deictic_reference(
            &crate::intent::surface_signals::analyze_prompt_surface(
                "看一下那个 model io log 最后 4 行，再一句话说有什么现象",
            ),
        ));
        assert!(request_looks_like_fresh_deictic_reference(
            &crate::intent::surface_signals::analyze_prompt_surface("把那个文件发给我"),
        ));
        assert!(request_looks_like_fresh_deictic_reference(
            &crate::intent::surface_signals::analyze_prompt_surface(
                "Use THIS log and summarize briefly",
            ),
        ));
        assert!(!request_looks_like_fresh_deictic_reference(
            &crate::intent::surface_signals::analyze_prompt_surface(
                "/home/guagua/rustclaw/README.md",
            ),
        ));
        assert!(!request_looks_like_fresh_deictic_reference(
            &crate::intent::surface_signals::analyze_prompt_surface(
                "thisness should not count as a deictic reference",
            ),
        ));
    }

    #[test]
    fn suppresses_route_memory_for_fresh_deictic_filename_wrapper_without_anchor() {
        assert!(should_suppress_route_memory_context(
            &crate::intent::surface_signals::analyze_prompt_surface(
                "看一下那个 model io log 最后 4 行，再一句话说有什么现象",
            ),
            "<none>",
            "<none>",
            None,
            None,
        ));
    }

    #[test]
    fn keeps_route_memory_for_deictic_filename_wrapper_with_immediate_anchor() {
        assert!(!should_suppress_route_memory_context(
            &crate::intent::surface_signals::analyze_prompt_surface("读一下那个 README 开头"),
            "### LAST_TURN_FULL\n[TURN -1]\nAssistant: FILE:/home/guagua/rustclaw/README.md\n[/TURN]",
            "<none>",
            None,
            None,
        ));
    }

    #[test]
    fn keeps_route_memory_for_explicit_path_request() {
        assert!(!should_suppress_route_memory_context(
            &crate::intent::surface_signals::analyze_prompt_surface(
                "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/model_io.log",
            ),
            "<none>",
            "<none>",
            None,
            None,
        ));
    }

    #[test]
    fn route_budget_uses_anchor_only_for_explicit_local_file_reads() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo，只输出值",
        );
        assert!(request_qualifies_for_anchor_only_route_context(
            "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo，只输出值",
            &surface,
        ));
        assert_eq!(
            classify_route_context_budget(
                "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo，只输出值",
                &surface,
                "<none>",
                "<none>",
                None,
                None,
                None,
            ),
            RouteContextBudgetTier::AnchorOnly
        );
    }

    #[test]
    fn route_budget_uses_anchor_only_for_explicit_compare_targets() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "比较 README.md 和 AGENTS.md 哪个更大，再用一句通俗话解释原因",
        );
        assert!(request_qualifies_for_anchor_only_route_context(
            "比较 README.md 和 AGENTS.md 哪个更大，再用一句通俗话解释原因",
            &surface,
        ));
        assert_eq!(
            classify_route_context_budget(
                "比较 README.md 和 AGENTS.md 哪个更大，再用一句通俗话解释原因",
                &surface,
                "<none>",
                "<none>",
                None,
                None,
                None,
            ),
            RouteContextBudgetTier::AnchorOnly
        );
    }

    #[test]
    fn route_budget_uses_anchor_only_for_filename_only_excerpt_reads() {
        let surface =
            crate::intent::surface_signals::analyze_prompt_surface("先读一下 README.md 前 4 行");
        assert!(request_qualifies_for_anchor_only_route_context(
            "先读一下 README.md 前 4 行",
            &surface,
        ));
        assert_eq!(
            classify_route_context_budget(
                "先读一下 README.md 前 4 行",
                &surface,
                "<none>",
                "<none>",
                None,
                None,
                None
            ),
            RouteContextBudgetTier::AnchorOnly
        );
    }

    #[test]
    fn route_budget_uses_none_for_fresh_deictic_without_immediate_anchor() {
        assert_eq!(
            classify_route_context_budget(
                "看一下那个日志最后 4 行",
                &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
                "<none>",
                "<none>",
                None,
                None,
                None
            ),
            RouteContextBudgetTier::None
        );
    }

    #[test]
    fn route_budget_uses_anchor_only_for_fresh_deictic_with_immediate_anchor() {
        let last_turn_full = "### LAST_TURN_FULL\n[TURN -1]\nAssistant: FILE:/home/guagua/rustclaw/logs/model_io.log\n[/TURN]";
        assert_eq!(
            classify_route_context_budget(
                "看一下那个日志最后 4 行",
                &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
                last_turn_full,
                "<none>",
                None,
                None,
                None
            ),
            RouteContextBudgetTier::AnchorOnly
        );
    }

    #[test]
    fn route_budget_uses_none_for_active_locator_clarify_reply() {
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供具体要读取的文件名或路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: None,
            semantic_kind: None,
            source_request: "看一下那个日志最后 5 行".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        assert!(request_looks_like_active_clarify_reply(
            "/tmp/device_local/logs/model_io.log",
            &crate::intent::surface_signals::analyze_prompt_surface(
                "/tmp/device_local/logs/model_io.log",
            ),
            Some(&clarify_state)
        ));
        assert_eq!(
            classify_route_context_budget(
                "/tmp/device_local/logs/model_io.log",
                &crate::intent::surface_signals::analyze_prompt_surface(
                    "/tmp/device_local/logs/model_io.log",
                ),
                "<none>",
                "<none>",
                None,
                Some(&clarify_state),
                None
            ),
            RouteContextBudgetTier::None
        );
    }

    #[test]
    fn route_budget_leaves_active_clarify_candidate_selection_to_normalizer() {
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供具体的文件名或路径。".to_string(),
            candidate_targets: vec!["act_plan.log".to_string(), "clawd.log".to_string()],
            delivery_required: true,
            output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
            semantic_kind: None,
            source_request: "把那个文件发给我".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        assert!(!request_looks_like_active_clarify_reply(
            "第二个",
            &crate::intent::surface_signals::analyze_prompt_surface("第二个"),
            Some(&clarify_state)
        ));
        assert_eq!(
            classify_route_context_budget(
                "第二个",
                &crate::intent::surface_signals::analyze_prompt_surface("第二个"),
                "<none>",
                "<none>",
                None,
                Some(&clarify_state),
                None
            ),
            RouteContextBudgetTier::Full
        );
    }

    #[test]
    fn observed_facts_anchor_promotes_fresh_deictic_to_anchor_only() {
        let facts = crate::observed_facts::ObservedFacts {
            bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
            ..crate::observed_facts::ObservedFacts::default()
        };
        assert!(observed_facts_provide_immediate_anchor(Some(&facts)));
        assert!(!should_suppress_route_memory_context(
            &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
            "<none>",
            "<none>",
            None,
            Some(&facts),
        ));
        assert_eq!(
            classify_route_context_budget(
                "看一下那个日志最后 4 行",
                &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
                "<none>",
                "<none>",
                None,
                None,
                Some(&facts)
            ),
            RouteContextBudgetTier::AnchorOnly
        );
    }

    #[test]
    fn followup_frame_anchor_promotes_fresh_deictic_to_anchor_only() {
        let frame = crate::followup_frame::FollowupFrame {
            bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        };
        assert!(!should_suppress_route_memory_context(
            &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
            "<none>",
            "<none>",
            Some(&frame),
            None,
        ));
        assert_eq!(
            classify_route_context_budget(
                "看一下那个日志最后 4 行",
                &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
                "<none>",
                "<none>",
                Some(&frame),
                None,
                None
            ),
            RouteContextBudgetTier::AnchorOnly
        );
    }

    #[test]
    fn text_anchor_probe_is_skipped_when_session_already_has_locator_anchor() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!needs_text_anchor_probe_for_route(
            "看一下那个日志最后 4 行",
            &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
            &snapshot
        ));
    }

    #[test]
    fn text_anchor_probe_is_used_for_fresh_deictic_without_session_anchor() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(needs_text_anchor_probe_for_route(
            "看一下那个日志最后 4 行",
            &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
            &snapshot
        ));
    }

    #[test]
    fn text_anchor_probe_is_skipped_when_session_alias_binding_matches_prompt() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                    alias: "那个 README".to_string(),
                    target: "/tmp/device_local/README.md".to_string(),
                    updated_at_ts: 1,
                }],
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!needs_text_anchor_probe_for_route(
            "读一下那个 README 开头，然后一句话总结",
            &crate::intent::surface_signals::analyze_prompt_surface(
                "读一下那个 README 开头，然后一句话总结",
            ),
            &snapshot
        ));
    }

    #[test]
    fn execution_text_context_is_suppressed_when_session_has_clarify_state() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "请提供路径".to_string(),
                candidate_targets: vec![],
                delivery_required: false,
                output_shape: None,
                semantic_kind: None,
                source_request: "读一下那个 README".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };
        assert!(session_snapshot_provides_execution_state_anchor(&snapshot));
    }

    #[test]
    fn execution_text_context_is_suppressed_when_session_has_followup_or_observed_anchor() {
        let followup_snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(session_snapshot_provides_execution_state_anchor(
            &followup_snapshot
        ));

        let observed_snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: Some(crate::observed_facts::ObservedFacts {
                bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
                ..crate::observed_facts::ObservedFacts::default()
            }),
        };
        assert!(session_snapshot_provides_execution_state_anchor(
            &observed_snapshot
        ));
    }

    #[test]
    fn execution_text_context_is_not_suppressed_without_session_anchor() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!session_snapshot_provides_execution_state_anchor(&snapshot));
    }

    #[test]
    fn session_anchored_chat_wrapped_content_reads_prefer_light_memory_budget() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                bound_target: Some("/tmp/device_local/README.md".to_string()),
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(should_prefer_light_execution_memory_from_session(
            &route, &snapshot
        ));
    }

    #[test]
    fn open_or_unanchored_routes_do_not_force_light_memory_budget() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        route.output_contract.requires_content_evidence = true;
        let unanchored_snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!should_prefer_light_execution_memory_from_session(
            &route,
            &unanchored_snapshot
        ));

        let mut planning_like = route.clone();
        planning_like.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::Act);
        let anchored_snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                bound_target: Some("/tmp/device_local/README.md".to_string()),
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!should_prefer_light_execution_memory_from_session(
            &planning_like,
            &anchored_snapshot
        ));
    }

    #[test]
    fn session_anchored_normalizer_chat_keeps_full_memory_budget_for_recall() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::Chat;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::Chat);
        let anchored_snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "请提供具体的文件名或路径。".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: true,
                output_shape: Some("file_token".to_string()),
                semantic_kind: None,
                source_request: "把那个文件发给我".to_string(),
                source_task_id: "task-clarify".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 60,
            }),
            active_observed_facts: None,
        };
        assert!(!should_prefer_light_execution_memory_from_session(
            &route,
            &anchored_snapshot
        ));
    }

    #[test]
    fn session_anchored_clarify_delivery_act_prefers_light_memory_budget() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::Act;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::Act);
        route.output_contract.delivery_required = true;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        let anchored_snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "请提供具体的文件名或路径。".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: true,
                output_shape: Some("file_token".to_string()),
                semantic_kind: None,
                source_request: "把那个文件发给我".to_string(),
                source_task_id: "task-clarify".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 60,
            }),
            active_observed_facts: None,
        };
        assert!(should_prefer_light_execution_memory_from_session(
            &route,
            &anchored_snapshot
        ));
    }

    #[test]
    fn ordinary_normalizer_chat_without_state_anchor_keeps_full_memory_budget() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::Chat;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::Chat);
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!should_prefer_light_execution_memory_from_session(
            &route, &snapshot
        ));
    }

    #[test]
    fn stateful_light_routes_suppress_execution_anchor_context() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "README.md".to_string();
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                bound_target: Some("/tmp/device_local/README.md".to_string()),
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(should_suppress_execution_anchor_context(
            &route,
            &snapshot,
            crate::task_context_builder::ExecutionContextBudgetTier::Light,
        ));
    }

    #[test]
    fn unanchored_light_routes_keep_execution_anchor_context() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "README.md".to_string();
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!should_suppress_execution_anchor_context(
            &route,
            &snapshot,
            crate::task_context_builder::ExecutionContextBudgetTier::Light,
        ));
    }

    fn base_route_result() -> crate::RouteResult {
        crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    #[test]
    fn light_execution_budget_detects_scalar_manifest_reads() {
        let mut route = base_route_result();
        route.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "package.json".to_string();
        assert!(uses_light_execution_context_budget(
            &route,
            &route.resolved_intent
        ));
    }

    #[test]
    fn execution_context_adds_recent_turns_to_chat_prompt_before_last_turn_fallback() {
        let bundle = TaskContextBundle {
            raw_sources: TaskContextRawSources::default(),
            planner_view: PlannerContextView::default(),
            route_view: None,
            execution_view: Some(ExecutionContextView {
                budget_tier: crate::task_context_builder::ExecutionContextBudgetTier::Full,
                memory_ctx: crate::memory::service::PromptMemoryContext {
                    prompt_with_memory: String::new(),
                    chat_prompt_context: String::new(),
                    long_term_summary: None,
                    preferences: Vec::new(),
                    recalled: Vec::new(),
                    similar_triggers: Vec::new(),
                    relevant_facts: Vec::new(),
                    recent_related_events: Vec::new(),
                },
                runtime_context: "<none>".to_string(),
                recent_turns_full: "### RECENT_TURNS_FULL\n[TURN -2]\nUser: 请记住测试编号 client-like-continuous-1\nAssistant: 已记住\n[/TURN]".to_string(),
                last_turn_full: "### LAST_TURN_FULL\n[TURN -1]\nUser: other\nAssistant: other\n[/TURN]".to_string(),
                recent_execution_anchor: "<none>".to_string(),
                recent_execution_context: "<none>".to_string(),
                image_context: None,
            }),
        };
        let mut chat_context = "### MEMORY_CONTEXT\n<none>".to_string();
        let mut resolved = "刚才编号是什么".to_string();
        let mut execution = "刚才编号是什么".to_string();
        apply_execution_context_to_prompts(
            &bundle,
            &mut chat_context,
            &mut resolved,
            &mut execution,
        );
        assert!(chat_context.contains("### RECENT_TURNS_FULL"));
        assert!(chat_context.contains("client-like-continuous-1"));
        assert!(!chat_context.contains("### LAST_TURN_FULL"));
    }

    #[test]
    fn execution_context_adds_runtime_context_to_chat_and_planner_prompts() {
        let bundle = TaskContextBundle {
            raw_sources: TaskContextRawSources::default(),
            planner_view: PlannerContextView::default(),
            route_view: None,
            execution_view: Some(ExecutionContextView {
                budget_tier: crate::task_context_builder::ExecutionContextBudgetTier::Full,
                memory_ctx: crate::memory::service::PromptMemoryContext {
                    prompt_with_memory: String::new(),
                    chat_prompt_context: String::new(),
                    long_term_summary: None,
                    preferences: Vec::new(),
                    recalled: Vec::new(),
                    similar_triggers: Vec::new(),
                    relevant_facts: Vec::new(),
                    recent_related_events: Vec::new(),
                },
                runtime_context: "### RUNTIME_CONTEXT\ncurrent_process_cwd: /tmp/workspace\nworkspace_root: /tmp/workspace".to_string(),
                recent_turns_full: "<none>".to_string(),
                last_turn_full: "<none>".to_string(),
                recent_execution_anchor: "<none>".to_string(),
                recent_execution_context: "<none>".to_string(),
                image_context: None,
            }),
        };
        let mut chat_context = "### MEMORY_CONTEXT\n<none>".to_string();
        let mut resolved = "当前工作目录是哪个".to_string();
        let mut execution = "当前工作目录是哪个".to_string();
        apply_execution_context_to_prompts(
            &bundle,
            &mut chat_context,
            &mut resolved,
            &mut execution,
        );
        assert!(chat_context.contains("### RUNTIME_CONTEXT"));
        assert!(chat_context.contains("current_process_cwd: /tmp/workspace"));
        assert!(execution.contains("### RUNTIME_CONTEXT"));
        assert!(execution.contains("workspace_root: /tmp/workspace"));
    }

    #[test]
    fn light_execution_budget_detects_generic_explicit_scalar_path_reads() {
        let mut route = base_route_result();
        route.resolved_intent =
            "读取 /home/guagua/rustclaw/configs/config.toml 中的 tools.allow_sudo 配置项的值，并仅输出该值"
                .to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint =
            "/home/guagua/rustclaw/configs/config.toml".to_string();
        assert!(uses_light_execution_context_budget(
            &route,
            &route.resolved_intent
        ));
    }

    #[test]
    fn light_execution_budget_detects_explicit_tail_reads() {
        let mut route = base_route_result();
        route.resolved_intent = "看一下 /tmp/model_io.log 最后 5 行".to_string();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/model_io.log".to_string();
        assert!(uses_light_execution_context_budget(
            &route,
            &route.resolved_intent
        ));
    }

    #[test]
    fn light_execution_budget_detects_bounded_listing_and_existence() {
        let mut listing = base_route_result();
        listing.resolved_intent = "列出 logs 目录下前 5 个文件名".to_string();
        listing.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        listing.output_contract.requires_content_evidence = true;
        listing.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        listing.output_contract.locator_hint = "logs".to_string();
        assert!(uses_light_execution_context_budget(
            &listing,
            &listing.resolved_intent
        ));

        let mut existence = base_route_result();
        existence.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        existence.resolved_intent = "看看 /tmp/rustclaw.service 在不在".to_string();
        assert!(uses_light_execution_context_budget(
            &existence,
            &existence.resolved_intent
        ));
    }

    #[test]
    fn light_execution_budget_detects_scalar_path_only_pwd_route() {
        let mut route = base_route_result();
        route.resolved_intent = "只输出当前工作目录的绝对路径，不要解释".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        assert!(uses_light_execution_context_budget(
            &route,
            &route.resolved_intent
        ));
    }

    #[test]
    fn light_execution_budget_detects_structured_chat_wrapped_content_reads() {
        let mut read_range = base_route_result();
        read_range.routed_mode = crate::RoutedMode::ChatAct;
        read_range.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        read_range.resolved_intent = "先读一下 README.md 前 4 行".to_string();
        read_range.output_contract.response_shape = crate::OutputResponseShape::Free;
        read_range.output_contract.requires_content_evidence = true;
        read_range.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
        read_range.output_contract.locator_hint = "README.md".to_string();
        assert!(uses_light_execution_context_budget(
            &read_range,
            &read_range.resolved_intent
        ));

        let mut single_read = base_route_result();
        single_read.routed_mode = crate::RoutedMode::ChatAct;
        single_read.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        single_read.resolved_intent =
            "看一下 /home/guagua/rustclaw/configs/config.toml，然后一句话说它主要配了什么"
                .to_string();
        single_read.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        single_read.output_contract.requires_content_evidence = true;
        single_read.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        single_read.output_contract.locator_hint =
            "/home/guagua/rustclaw/configs/config.toml".to_string();
        assert!(uses_light_execution_context_budget(
            &single_read,
            &single_read.resolved_intent
        ));
    }

    #[test]
    fn light_execution_budget_skips_workspace_project_summary() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        route.resolved_intent =
            "Summarize the current repository, focusing only on the UI components".to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
        assert!(!uses_light_execution_context_budget(
            &route,
            &route.resolved_intent
        ));
    }

    #[test]
    fn light_execution_budget_detects_clarify_rewrite_bound_reads() {
        let mut route = base_route_result();
        route.resolved_intent = "Continue the previous request that was waiting for clarification: 读一下那个文件里的名字字段，只输出值\nUser now provides the missing target/content: package.json".to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "package.json".to_string();
        assert!(uses_light_execution_context_budget(
            &route,
            &route.resolved_intent
        ));
    }

    #[test]
    fn light_execution_budget_skips_non_structured_chat_wrapped_or_clarify_routes() {
        let mut chat_act = base_route_result();
        chat_act.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        chat_act.resolved_intent = "比较这两个文件大小，然后一句话总结".to_string();
        assert!(!uses_light_execution_context_budget(
            &chat_act,
            &chat_act.resolved_intent
        ));

        let mut delivery = base_route_result();
        delivery.routed_mode = crate::RoutedMode::ChatAct;
        delivery.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        delivery.resolved_intent = "把 README.md 发给我".to_string();
        delivery.output_contract.requires_content_evidence = true;
        delivery.output_contract.delivery_required = true;
        assert!(!uses_light_execution_context_budget(
            &delivery,
            &delivery.resolved_intent
        ));

        let mut clarify = base_route_result();
        clarify.needs_clarify = true;
        clarify.resolved_intent = "看一下那个日志".to_string();
        assert!(!uses_light_execution_context_budget(
            &clarify,
            &clarify.resolved_intent
        ));
    }

    #[test]
    fn light_execution_budget_skips_unscoped_current_workspace_drafting_evidence() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        route.resolved_intent =
            "Write a short setup note grounded in the current workspace docs".to_string();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.locator_hint.clear();

        assert!(!uses_light_execution_context_budget(
            &route,
            &route.resolved_intent
        ));
    }

    #[test]
    fn light_execution_budget_skips_generic_current_workspace_hint_drafting_evidence() {
        let mut route = base_route_result();
        route.routed_mode = crate::RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct);
        route.resolved_intent =
            "Write a short RustClaw setup note for the current workspace project".to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.locator_hint = "rustclaw workspace".to_string();

        assert!(!uses_light_execution_context_budget(
            &route,
            &route.resolved_intent
        ));
    }
}

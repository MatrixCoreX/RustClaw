use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::memory;
use crate::memory::service::PromptMemoryContext;
use crate::{AppState, ClaimedTask};

#[path = "task_context_builder/summary.rs"]
mod summary;

#[derive(Debug, Clone, Default)]
pub(crate) struct TaskContextRawSources {
    pub(crate) resume_context: String,
    pub(crate) binding_context: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PlannerContextView {
    pub(crate) visible_skills: Vec<String>,
}

pub(crate) struct ExecutionContextView {
    pub(crate) budget_tier: ExecutionContextBudgetTier,
    pub(crate) memory_ctx: PromptMemoryContext,
    pub(crate) runtime_context: String,
    pub(crate) goal_context: String,
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
    pub(crate) execution_view: Option<ExecutionContextView>,
}

impl TaskContextBundle {
    pub(crate) fn summary(&self) -> String {
        summary::task_context_bundle_summary(self)
    }

    pub(crate) fn memory_trace(&self) -> Option<Value> {
        self.execution_view
            .as_ref()
            .and_then(|view| view.memory_ctx.memory_trace.clone())
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

fn context_slot_present(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed != "<none>"
}

const LONG_SESSION_CONTEXT_CHARS: usize = 4096;

fn context_budget_slots(view: &ExecutionContextView) -> [(&'static str, &str); 11] {
    [
        (
            "prompt_memory_context",
            view.memory_ctx.prompt_with_memory.as_str(),
        ),
        ("runtime_context", view.runtime_context.as_str()),
        ("goal_context", view.goal_context.as_str()),
        ("active_task_context", view.active_task_context.as_str()),
        (
            "active_execution_anchor_context",
            view.active_execution_anchor_context.as_str(),
        ),
        ("session_alias_context", view.session_alias_context.as_str()),
        ("recent_turns_full", view.recent_turns_full.as_str()),
        ("last_turn_full", view.last_turn_full.as_str()),
        (
            "recent_execution_anchor",
            view.recent_execution_anchor.as_str(),
        ),
        (
            "recent_execution_context",
            view.recent_execution_context.as_str(),
        ),
        (
            "image_context",
            view.image_context.as_deref().unwrap_or("<none>"),
        ),
    ]
}

fn context_input_inventory_item(
    input_kind: &'static str,
    source_refs: &[&'static str],
    slots: &[(&'static str, &str)],
) -> Value {
    let included_source_refs = source_refs
        .iter()
        .filter(|source_ref| {
            slots
                .iter()
                .any(|(slot, value)| slot == *source_ref && context_slot_present(value))
        })
        .map(|source_ref| Value::String((*source_ref).to_string()))
        .collect::<Vec<_>>();
    let excluded_source_refs = source_refs
        .iter()
        .filter(|source_ref| {
            !slots
                .iter()
                .any(|(slot, value)| slot == *source_ref && context_slot_present(value))
        })
        .map(|source_ref| Value::String((*source_ref).to_string()))
        .collect::<Vec<_>>();
    let status = if included_source_refs.is_empty() {
        "not_attached"
    } else if excluded_source_refs.is_empty() {
        "attached"
    } else {
        "partially_attached"
    };
    json!({
        "input_kind": input_kind,
        "status": status,
        "source_refs": source_refs,
        "included_source_refs": included_source_refs,
        "excluded_source_refs": excluded_source_refs,
    })
}

fn context_input_inventory_json(view: &ExecutionContextView) -> Value {
    let slots = context_budget_slots(view);
    let inputs = vec![
        context_input_inventory_item(
            "conversation_state",
            &[
                "active_task_context",
                "active_execution_anchor_context",
                "session_alias_context",
                "recent_turns_full",
                "last_turn_full",
            ],
            &slots,
        ),
        context_input_inventory_item("memory_recent_records", &["prompt_memory_context"], &slots),
        context_input_inventory_item("goal_fields", &["goal_context"], &slots),
        context_input_inventory_item(
            "task_journal",
            &["recent_execution_anchor", "recent_execution_context"],
            &slots,
        ),
        context_input_inventory_item("artifact_refs", &["image_context"], &slots),
        context_input_inventory_item(
            "previous_task_results",
            &["recent_execution_context"],
            &slots,
        ),
        context_input_inventory_item("llm_trace_debug_data", &[], &slots),
        context_input_inventory_item(
            "coding_evidence",
            &["recent_execution_anchor", "recent_execution_context"],
            &slots,
        ),
    ];
    let present_input_count = inputs
        .iter()
        .filter(|item| item.get("status").and_then(Value::as_str) != Some("not_attached"))
        .count();
    json!({
        "schema_version": 1,
        "input_count": inputs.len(),
        "present_input_count": present_input_count,
        "inputs": inputs,
    })
}

fn execution_context_compaction_triggers(view: &ExecutionContextView) -> Vec<&'static str> {
    let mut triggers = Vec::new();
    if matches!(view.budget_tier, ExecutionContextBudgetTier::Light) {
        triggers.push("over_budget");
    }
    let transcript_chars =
        view.recent_turns_full.chars().count() + view.last_turn_full.chars().count();
    if transcript_chars > LONG_SESSION_CONTEXT_CHARS {
        triggers.push("long_session");
    }
    triggers
}

pub(super) fn execution_context_budget_report_json(view: &ExecutionContextView) -> Value {
    let slots = context_budget_slots(view);
    let included_refs = slots
        .iter()
        .filter(|(_, value)| context_slot_present(value))
        .map(|(slot, value)| json!({"ref": slot, "char_count": value.chars().count()}))
        .collect::<Vec<_>>();
    let excluded_refs = slots
        .iter()
        .filter(|(_, value)| !context_slot_present(value))
        .map(|(slot, _)| json!({"ref": slot, "reason": "not_included"}))
        .collect::<Vec<_>>();
    let char_estimate = included_refs
        .iter()
        .filter_map(|item| item.get("char_count").and_then(Value::as_u64))
        .sum::<u64>();
    json!({
        "schema_version": 1,
        "budget_tier": view.budget_tier.as_str(),
        "included_ref_count": included_refs.len(),
        "included_refs": included_refs,
        "excluded_ref_count": excluded_refs.len(),
        "excluded_refs": excluded_refs,
        "char_estimate": char_estimate,
        "token_estimate": (char_estimate / 4).max(1),
        "truncation_reason": if matches!(view.budget_tier, ExecutionContextBudgetTier::Light) {
            "light_execution_budget"
        } else {
            "none"
        },
        "compaction_triggers": execution_context_compaction_triggers(view),
        "safety_reason": "context_budget_policy",
        "compaction_source": "deterministic_context_builder",
        "context_input_inventory": context_input_inventory_json(view),
    })
}

fn execution_context_budget_report_block(view: &ExecutionContextView) -> String {
    let report = execution_context_budget_report_json(view);
    let body = serde_json::to_string_pretty(&report).unwrap_or_else(|_| report.to_string());
    let mut block = String::from("### CONTEXT_BUDGET_REPORT");
    block.push('\n');
    block.push_str(&body);
    block
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

fn build_task_goal_context(task: &ClaimedTask) -> String {
    let Some(payload) = serde_json::from_str::<Value>(&task.payload_json).ok() else {
        return "<none>".to_string();
    };
    let Some(goal) = payload
        .get("goal")
        .or_else(|| payload.get("goal_spec"))
        .or_else(|| payload.get("task_goal"))
        .filter(|value| value.is_object())
    else {
        return "<none>".to_string();
    };
    let goal_context = json!({
        "schema_version": 1,
        "task_id": task.task_id,
        "source": "task_payload",
        "goal": goal,
    });
    format!(
        "### TASK_GOAL_CONTEXT\n{}",
        serde_json::to_string_pretty(&goal_context).unwrap_or_else(|_| goal_context.to_string())
    )
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

fn session_snapshot_provides_execution_state_anchor(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    session_snapshot.active_clarify_state.is_some()
        || followup_frame_provides_immediate_anchor(session_snapshot.active_followup_frame.as_ref())
        || observed_facts_provide_immediate_anchor(session_snapshot.active_observed_facts.as_ref())
}

pub(crate) fn build_agent_loop_task_context_bundle(
    state: &AppState,
    task: &ClaimedTask,
    planner_user_request: &str,
    chat_memory_budget_chars: usize,
) -> TaskContextBundle {
    let planner_view = PlannerContextView {
        visible_skills: state.planner_available_skills_for_task(task),
    };
    let session_snapshot = crate::conversation_state::load_active_session_snapshot(state, task);
    let budget_tier = ExecutionContextBudgetTier::Full;
    let has_active_session_state =
        session_snapshot_provides_execution_state_anchor(&session_snapshot);
    let planner_memory_decision = memory::use_policy::decide_planner_memory_use_policy(
        state,
        budget_tier,
        memory::use_policy::PlannerMemoryContextHint::Default,
    );
    let chat_memory_decision = memory::use_policy::decide_chat_memory_use_policy(
        state,
        budget_tier,
        "agent_loop_semantic_authority",
        has_active_session_state,
        chat_memory_budget_chars,
        memory::use_policy::ChatMemoryContextHint::Default,
    );
    let memory_ctx = memory::service::prepare_prompt_with_memory_for_policy(
        state,
        task,
        planner_user_request,
        &planner_memory_decision,
        &chat_memory_decision,
    );
    let execution_view = ExecutionContextView {
        budget_tier,
        memory_ctx,
        runtime_context: build_runtime_context(state),
        goal_context: build_task_goal_context(task),
        active_task_context: build_active_task_context(&session_snapshot),
        active_execution_anchor_context: build_active_execution_anchor_context(&session_snapshot),
        session_alias_context: build_session_alias_context(&session_snapshot),
        recent_turns_full: memory::build_recent_turns_full_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            5,
            560,
            6400,
        ),
        last_turn_full: memory::build_last_turn_full_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            1200,
            2400,
        ),
        recent_execution_anchor: crate::routing_context::build_recent_execution_anchor_context(
            state, task,
        ),
        recent_execution_context: crate::routing_context::build_recent_execution_context(
            state, task, 8,
        ),
        image_context: None,
    };
    TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view,
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
    if execution_view.session_alias_context != "<none>" {
        let alias_context_block = format!(
            "\n\n{}\nAlias execution rule: when the current goal or request mentions more than one alias, treat each alias target as an independent authoritative concrete target. Do not rebuild a file alias under another directory alias unless that exact alias target says it is inside that directory.",
            execution_view.session_alias_context
        );
        resolved_prompt_for_execution.push_str(&alias_context_block);
        prompt_with_memory_for_execution.push_str(&alias_context_block);
    }
    if execution_view.goal_context != "<none>" {
        let goal_context_block = format!("\n\n{}", execution_view.goal_context);
        resolved_prompt_for_execution.push_str(&goal_context_block);
        prompt_with_memory_for_execution.push_str(&goal_context_block);
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
    let budget_report_block = format!(
        "\n\n{}",
        execution_context_budget_report_block(execution_view)
    );
    resolved_prompt_for_execution.push_str(&budget_report_block);
    prompt_with_memory_for_execution.push_str(&budget_report_block);
}

#[cfg(test)]
#[path = "task_context_builder_summary_tests.rs"]
mod summary_tests;

#[cfg(test)]
#[path = "task_context_builder_tests.rs"]
mod tests;

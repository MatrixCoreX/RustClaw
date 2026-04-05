use serde_json::Value;

use crate::memory;
use crate::memory::service::PromptMemoryContext;
use crate::{AppState, ClaimedTask};

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
    pub(crate) recent_execution_context: String,
    pub(crate) capability_map: String,
    pub(crate) recent_assistant_replies: String,
    pub(crate) recent_turns_full: String,
    pub(crate) memory_context: String,
    pub(crate) last_turn_full: String,
}

pub(crate) struct ExecutionContextView {
    pub(crate) memory_ctx: PromptMemoryContext,
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
        let execution_attached = self.execution_view.is_some();
        let visible_skills = self.planner_view.visible_skills.len();
        let has_resume_context = self.raw_sources.resume_context != "<none>";
        let has_binding_context = self.raw_sources.binding_context != "<none>";
        format!(
            "route_view={} execution_view={} visible_skills={} resume_context={} binding_context={}",
            route_attached,
            execution_attached,
            visible_skills,
            has_resume_context,
            has_binding_context
        )
    }
}

fn serialize_context_value(value: Option<&Value>) -> String {
    value
        .map(|v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
        .filter(|s| !s.is_empty() && s != "{}")
        .unwrap_or_else(|| "<none>".to_string())
}

pub(crate) fn build_route_task_context_bundle(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resume_context: Option<&Value>,
    binding_context: Option<&Value>,
    now_iso: &str,
    timezone: &str,
    schedule_rules: &str,
) -> TaskContextBundle {
    let planner_view = PlannerContextView {
        visible_skills: state.planner_visible_skills_for_task(task),
    };
    let route_view = RouteContextView {
        recent_execution_context: crate::routing_context::build_recent_execution_context(
            state, task, 8,
        ),
        capability_map: crate::capability_map::build_capability_map_for_task(state, task),
        recent_assistant_replies: memory::build_recent_assistant_replies_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            3,
            220,
        ),
        recent_turns_full: memory::build_recent_turns_full_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            5,
            560,
            6400,
        ),
        memory_context: if state.memory.route_memory_enabled {
            let structured = memory::service::recall_structured_memory_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                user_request,
                state.memory.prompt_recall_limit.max(1),
                true,
                true,
            );
            memory::service::structured_memory_context_block(
                &structured,
                memory::retrieval::MemoryContextMode::Route,
                state
                    .memory
                    .route_trigger_budget_chars
                    .max(384)
                    .min(state.memory.route_memory_max_chars.max(384)),
            )
        } else {
            "<none>".to_string()
        },
        last_turn_full: memory::build_last_turn_full_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            1200,
            2400,
        ),
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
    resolved_prompt: &str,
    chat_memory_budget_chars: usize,
) -> TaskContextBundle {
    let planner_view = PlannerContextView {
        visible_skills: state.planner_visible_skills_for_task(task),
    };
    let memory_ctx = memory::service::prepare_prompt_with_memory(
        state,
        task,
        resolved_prompt,
        chat_memory_budget_chars,
    );
    let execution_view = ExecutionContextView {
        memory_ctx,
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
    if execution_view.last_turn_full != "<none>" {
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

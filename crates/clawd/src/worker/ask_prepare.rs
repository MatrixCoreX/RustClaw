use serde_json::{json, Value};
use tracing::info;

use crate::{schedule_service, AppState};

pub(super) struct PreparedAskExecutionContext {
    pub(super) context_bundle: crate::task_context_builder::TaskContextBundle,
    pub(super) chat_prompt_context: String,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) recent_execution_context: String,
}

pub(super) struct PreparedAskRouting {
    pub(super) route_result: crate::RouteResult,
    pub(super) resolved_prompt: String,
    pub(super) agent_mode: bool,
    pub(super) direct_resume_execution: bool,
    pub(super) direct_resume_discussion: bool,
    pub(super) classifier_direct_mode: bool,
}

pub(super) struct PreparedAskInput {
    pub(super) prompt: String,
    pub(super) source: String,
}

pub(super) struct PreparedRunSkillInput {
    pub(super) skill_name: String,
    pub(super) args: Value,
}

#[derive(Clone)]
enum ResumeContextSource {
    ExplicitContinue,
    RecentFailedCandidate,
}

#[derive(Clone)]
struct ResumeContextBinding {
    source: ResumeContextSource,
    resume_context: Value,
    failed_ts: Option<i64>,
    has_newer_successful_ask_after_failed_task: bool,
}

fn explicit_resume_context_binding(
    payload: &Value,
    is_resume_continue: bool,
) -> Option<ResumeContextBinding> {
    if !is_resume_continue {
        return None;
    }
    Some(ResumeContextBinding {
        source: ResumeContextSource::ExplicitContinue,
        resume_context: payload.get("resume_context").cloned()?,
        failed_ts: payload
            .get("failed_resume_context_ts")
            .and_then(|v| v.as_i64()),
        has_newer_successful_ask_after_failed_task: payload
            .get("has_newer_successful_ask_after_failed_task")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

fn recent_failed_resume_candidate(
    state: &AppState,
    task: &crate::ClaimedTask,
    explicit_binding_present: bool,
) -> Option<ResumeContextBinding> {
    if explicit_binding_present {
        return None;
    }
    let candidate = crate::find_recent_failed_resume_context(state, task.user_id, task.chat_id)?;
    Some(ResumeContextBinding {
        source: ResumeContextSource::RecentFailedCandidate,
        resume_context: candidate.resume_context,
        failed_ts: Some(candidate.failed_ts),
        has_newer_successful_ask_after_failed_task: candidate
            .has_newer_successful_ask_after_failed_task,
    })
}

fn binding_context_json(
    source: &str,
    is_resume_continue: bool,
    resume_binding: Option<&ResumeContextBinding>,
) -> Value {
    let (
        resume_context_source,
        failed_resume_context_ts,
        has_newer_successful_ask_after_failed_task,
    ) = match resume_binding {
        Some(binding) => (
            match binding.source {
                ResumeContextSource::ExplicitContinue => "explicit_continue_source",
                ResumeContextSource::RecentFailedCandidate => "recent_failed_resume_candidate",
            },
            binding.failed_ts.map(Value::from).unwrap_or(Value::Null),
            binding.has_newer_successful_ask_after_failed_task,
        ),
        None => ("none", Value::Null, false),
    };
    json!({
        "source": source.trim(),
        "is_resume_continue_source": is_resume_continue,
        "resume_context_source": resume_context_source,
        "failed_resume_context_ts": failed_resume_context_ts,
        "has_newer_successful_ask_after_failed_task": has_newer_successful_ask_after_failed_task,
    })
}

fn select_resume_runtime_binding<'a>(
    route_result: &crate::RouteResult,
    resume_binding: Option<&'a ResumeContextBinding>,
) -> Option<&'a ResumeContextBinding> {
    (!matches!(route_result.resume_behavior, crate::ResumeBehavior::None))
        .then_some(resume_binding)
        .flatten()
}

fn log_ask_memory_snapshot(
    task: &crate::ClaimedTask,
    long_term_log: &str,
    preferences_log: &str,
    trigger_log: &str,
    fact_log: &str,
    related_log: &str,
    recalled_count: usize,
    recalled_log: &str,
) {
    info!(
        "worker_once: ask memory task_id={} memory.long_term_summary={} memory.preferences={} memory.similar_triggers={} memory.relevant_facts={} memory.related_events={} memory.recalled_recent_count={} memory.recalled_recent={}",
        task.task_id,
        long_term_log,
        preferences_log,
        trigger_log,
        fact_log,
        related_log,
        recalled_count,
        recalled_log,
    );
}

pub(super) async fn prepare_ask_execution_context(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    resolved_prompt: &str,
) -> anyhow::Result<PreparedAskExecutionContext> {
    let chat_memory_budget_chars =
        crate::dynamic_chat_memory_budget_chars(state, task, resolved_prompt);
    let mut context_bundle = crate::task_context_builder::build_execution_task_context_bundle(
        state,
        task,
        resolved_prompt,
        chat_memory_budget_chars,
    );
    let execution_view = context_bundle
        .execution_view
        .as_ref()
        .expect("execution context bundle should include execution_view");
    let long_term_summary = execution_view.memory_ctx.long_term_summary.clone();
    let preferences = execution_view.memory_ctx.preferences.clone();
    let recalled = execution_view.memory_ctx.recalled.clone();
    let similar_triggers = execution_view.memory_ctx.similar_triggers.clone();
    let relevant_facts = execution_view.memory_ctx.relevant_facts.clone();
    let recent_related_events = execution_view.memory_ctx.recent_related_events.clone();
    let prompt_with_memory = execution_view.memory_ctx.prompt_with_memory.clone();
    let mut chat_prompt_context = execution_view.memory_ctx.chat_prompt_context.clone();
    let mut resolved_prompt_for_execution = resolved_prompt.to_string();
    let mut prompt_with_memory_for_execution = prompt_with_memory.clone();
    let recent_execution_context = execution_view.recent_execution_context.clone();
    if let Some(image_context) =
        crate::analyze_attached_images_for_ask(state, task, payload, resolved_prompt).await?
    {
        crate::task_context_builder::set_execution_image_context(
            &mut context_bundle,
            Some(image_context),
        );
    }
    crate::task_context_builder::apply_execution_context_to_prompts(
        &context_bundle,
        &mut chat_prompt_context,
        &mut resolved_prompt_for_execution,
        &mut prompt_with_memory_for_execution,
    );
    let long_term_log = long_term_summary
        .as_deref()
        .map(crate::truncate_for_log)
        .unwrap_or_else(|| "<none>".to_string());
    let recalled_log = if recalled.is_empty() {
        "<none>".to_string()
    } else {
        let merged = recalled
            .iter()
            .map(|(role, content)| format!("{role}:{content}"))
            .collect::<Vec<_>>()
            .join(" | ");
        crate::truncate_for_log(&merged)
    };
    let preferences_log = if preferences.is_empty() {
        "<none>".to_string()
    } else {
        let merged = preferences
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" | ");
        crate::truncate_for_log(&merged)
    };
    let trigger_log = if similar_triggers.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &similar_triggers
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    let fact_log = if relevant_facts.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &relevant_facts
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    let related_log = if recent_related_events.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &recent_related_events
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    log_ask_memory_snapshot(
        task,
        &long_term_log,
        &preferences_log,
        &trigger_log,
        &fact_log,
        &related_log,
        recalled.len(),
        &recalled_log,
    );
    Ok(PreparedAskExecutionContext {
        context_bundle,
        chat_prompt_context,
        resolved_prompt_for_execution,
        prompt_with_memory_for_execution,
        recent_execution_context,
    })
}

pub(super) async fn prepare_ask_input(
    _state: &AppState,
    _task: &crate::ClaimedTask,
    payload: &mut Value,
) -> PreparedAskInput {
    let prompt = payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let source = payload
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    PreparedAskInput { prompt, source }
}

pub(super) fn prepare_run_skill_input(payload: &Value) -> PreparedRunSkillInput {
    let skill_name = payload
        .get("skill_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let args = payload.get("args").cloned().unwrap_or_else(|| json!(""));
    PreparedRunSkillInput { skill_name, args }
}

pub(super) async fn maybe_finalize_schedule_direct_text_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
) -> anyhow::Result<bool> {
    let is_schedule_triggered = payload
        .get("schedule_triggered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let schedule_task_mode = payload
        .get("schedule_task_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let schedule_force_agent = payload
        .get("schedule_force_agent")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let schedule_direct_text_mode = is_schedule_triggered
        && !schedule_force_agent
        && (schedule_task_mode.is_empty() || schedule_task_mode == "direct_text");
    if !schedule_direct_text_mode {
        return Ok(false);
    }
    let direct_text = prompt.trim();
    if direct_text.is_empty() {
        return Ok(false);
    }
    let answer_text = crate::intercept_response_text_for_delivery(direct_text);
    super::finalize_ask_direct_success(
        state,
        task,
        payload,
        prompt,
        &answer_text,
        "schedule_direct_text",
    )
    .await?;
    Ok(true)
}

pub(super) async fn prepare_ask_routing(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    source: &str,
) -> PreparedAskRouting {
    let main_rules = crate::main_flow_rules(state);
    let is_resume_continue = super::is_resume_continue_source(main_rules, source);
    let (now_iso, timezone_str, schedule_rules) =
        schedule_service::schedule_context_for_normalizer(state);
    let explicit_resume_binding = explicit_resume_context_binding(payload, is_resume_continue);
    let recent_failed_resume_binding =
        recent_failed_resume_candidate(state, task, explicit_resume_binding.is_some());
    let resume_binding = explicit_resume_binding
        .clone()
        .or_else(|| recent_failed_resume_binding.clone());
    let binding_context_value =
        binding_context_json(source, is_resume_continue, resume_binding.as_ref());
    let normalizer_out = crate::intent_router::run_intent_normalizer(
        state,
        task,
        prompt,
        resume_binding
            .as_ref()
            .map(|binding| &binding.resume_context),
        Some(&binding_context_value),
        &now_iso,
        &timezone_str,
        &schedule_rules,
    )
    .await;
    let route_result =
        crate::intent_router::route_result_from_normalizer(state, task, &normalizer_out);
    let resume_runtime_binding =
        select_resume_runtime_binding(&route_result, resume_binding.as_ref());
    let resume_should_apply_context = resume_runtime_binding.is_some()
        && route_result.resume_behavior == crate::ResumeBehavior::ResumeExecute;
    let resume_should_discuss_context = resume_runtime_binding.is_some()
        && route_result.resume_behavior == crate::ResumeBehavior::ResumeDiscuss;
    info!(
        "worker_once: ask raw_message task_id={} user_id={} chat_id={} text={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(prompt)
    );
    let runtime_prompt = if resume_should_apply_context {
        match resume_runtime_binding {
            Some(ResumeContextBinding {
                source: ResumeContextSource::ExplicitContinue,
                ..
            }) => crate::build_resume_continue_execute_prompt(state, payload, prompt),
            Some(binding) => crate::ask_flow::build_resume_continue_execute_prompt_from_context(
                state,
                prompt,
                &binding.resume_context,
            ),
            None => route_result.resolved_intent.clone(),
        }
    } else if resume_should_discuss_context {
        match resume_runtime_binding {
            Some(ResumeContextBinding {
                source: ResumeContextSource::ExplicitContinue,
                ..
            }) => crate::build_resume_followup_discussion_prompt(state, payload, prompt),
            Some(binding) => crate::ask_flow::build_resume_followup_discussion_prompt_from_context(
                state,
                prompt,
                &binding.resume_context,
            ),
            None => route_result.resolved_intent.clone(),
        }
    } else {
        route_result.resolved_intent.clone()
    };
    info!(
        "worker_once: ask received_message task_id={} user_id={} chat_id={} text={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(&runtime_prompt)
    );
    let context_resolution = crate::intent_router::ContextResolution {
        resolved_user_intent: runtime_prompt.clone(),
        needs_clarify: route_result.needs_clarify,
        confidence: route_result.route_confidence,
        reason: route_result.route_reason.clone(),
    };
    let resolved_prompt = context_resolution.resolved_user_intent.clone();
    info!(
        "{} worker_once: ask resolved_message task_id={} needs_clarify={} confidence={} reason={} resolved_text={}",
        crate::highlight_tag("routing"),
        task.task_id,
        context_resolution.needs_clarify,
        context_resolution.confidence.unwrap_or(-1.0),
        crate::truncate_for_log(&context_resolution.reason),
        crate::truncate_for_log(&resolved_prompt)
    );
    let agent_mode = payload
        .get("agent_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let classifier_direct_mode = crate::main_flow_rules(state)
        .classifier_direct_sources
        .iter()
        .any(|s| s == &source.to_ascii_lowercase());
    PreparedAskRouting {
        route_result,
        resolved_prompt,
        agent_mode,
        direct_resume_execution: resume_should_apply_context,
        direct_resume_discussion: resume_should_discuss_context,
        classifier_direct_mode,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        binding_context_json, select_resume_runtime_binding, ResumeContextBinding,
        ResumeContextSource,
    };

    #[test]
    fn binding_context_marks_recent_failed_candidate_without_mutating_source() {
        let binding = ResumeContextBinding {
            source: ResumeContextSource::RecentFailedCandidate,
            resume_context: json!({"resume_context_id":"ctx-1"}),
            failed_ts: Some(42),
            has_newer_successful_ask_after_failed_task: true,
        };
        let value = binding_context_json("manual", false, Some(&binding));
        assert_eq!(
            value.get("resume_context_source").and_then(|v| v.as_str()),
            Some("recent_failed_resume_candidate")
        );
        assert_eq!(
            value
                .get("is_resume_continue_source")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .get("has_newer_successful_ask_after_failed_task")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn runtime_resume_binding_is_disabled_when_normalizer_rejects_resume() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            resolved_intent: "list current workspace".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
            output_contract: crate::IntentOutputContract::default(),
        };
        let binding = ResumeContextBinding {
            source: ResumeContextSource::RecentFailedCandidate,
            resume_context: json!({"resume_context_id":"ctx-2"}),
            failed_ts: Some(7),
            has_newer_successful_ask_after_failed_task: false,
        };
        assert!(select_resume_runtime_binding(&route, Some(&binding)).is_none());
    }
}

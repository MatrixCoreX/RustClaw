use serde_json::{json, Value};
use tracing::warn;

use crate::{turn_context::TargetTaskPolicy, AppState, ClaimedTask, RouteResult};

#[derive(Clone)]
pub(crate) enum ResumeContextSource {
    ExplicitContinue,
    ActiveCheckpointCandidate,
    RecentFailedCandidate,
}

#[derive(Clone)]
pub(crate) struct ResumeContextBinding {
    pub(crate) source: ResumeContextSource,
    pub(crate) resume_context: Value,
    pub(crate) failed_ts: Option<i64>,
    pub(crate) has_newer_successful_ask_after_failed_task: bool,
}

pub(crate) struct ResumeRuntimePromptResolution {
    pub(crate) runtime_prompt: String,
    pub(crate) should_apply_context: bool,
    pub(crate) should_discuss_context: bool,
}

pub(crate) fn explicit_resume_context_binding(
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

pub(crate) fn recent_failed_resume_candidate(
    state: &AppState,
    task: &ClaimedTask,
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

pub(crate) fn active_checkpoint_resume_candidate(
    state: &AppState,
    task: &ClaimedTask,
    explicit_binding_present: bool,
) -> Option<ResumeContextBinding> {
    if explicit_binding_present {
        return None;
    }
    let candidate =
        crate::repo::find_active_checkpoint_resume_context(state, task.user_id, task.chat_id)?;
    Some(ResumeContextBinding {
        source: ResumeContextSource::ActiveCheckpointCandidate,
        resume_context: candidate.resume_context,
        failed_ts: None,
        has_newer_successful_ask_after_failed_task: false,
    })
}

pub(crate) fn binding_context_json(
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
                ResumeContextSource::ActiveCheckpointCandidate => "active_checkpoint_resume",
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

pub(crate) fn select_resume_runtime_binding<'a>(
    route_result: &RouteResult,
    resume_binding: Option<&'a ResumeContextBinding>,
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
) -> Option<&'a ResumeContextBinding> {
    if matches!(route_result.resume_behavior, crate::ResumeBehavior::None) {
        return None;
    }
    let binding = resume_binding?;
    if ambient_resume_binding_blocked_by_turn_policy(binding, turn_analysis) {
        return None;
    }
    Some(binding)
}

fn ambient_resume_binding_blocked_by_turn_policy(
    binding: &ResumeContextBinding,
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
) -> bool {
    if matches!(binding.source, ResumeContextSource::ExplicitContinue) {
        return false;
    }
    matches!(
        turn_analysis.and_then(|analysis| analysis.target_task_policy),
        Some(TargetTaskPolicy::Standalone | TargetTaskPolicy::PauseAndQueue)
    )
}

pub(crate) fn resolve_resume_runtime_prompt(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    prompt: &str,
    route_result: &RouteResult,
    resume_binding: Option<&ResumeContextBinding>,
) -> ResumeRuntimePromptResolution {
    let should_apply_context = resume_binding.is_some()
        && route_result.resume_behavior == crate::ResumeBehavior::ResumeExecute;
    let should_discuss_context = resume_binding.is_some()
        && route_result.resume_behavior == crate::ResumeBehavior::ResumeDiscuss;
    let resume_prompt_or_fallback =
        |result: Result<String, crate::bootstrap::RequiredPromptLoadError>| match result {
            Ok(prompt) => prompt,
            Err(err) => {
                warn!("resume_runtime_prompt prompt_missing: {err}");
                route_result.resolved_intent.clone()
            }
        };
    let runtime_prompt = if should_apply_context {
        match resume_binding {
            Some(ResumeContextBinding {
                source: ResumeContextSource::ExplicitContinue,
                ..
            }) => resume_prompt_or_fallback(crate::build_resume_continue_execute_prompt(
                state, task, payload, prompt,
            )),
            Some(binding) => resume_prompt_or_fallback(
                crate::ask_flow::build_resume_continue_execute_prompt_from_context(
                    state,
                    task,
                    prompt,
                    &binding.resume_context,
                ),
            ),
            None => route_result.resolved_intent.clone(),
        }
    } else if should_discuss_context {
        match resume_binding {
            Some(ResumeContextBinding {
                source: ResumeContextSource::ExplicitContinue,
                ..
            }) => resume_prompt_or_fallback(crate::build_resume_followup_discussion_prompt(
                state, task, payload, prompt,
            )),
            Some(binding) => resume_prompt_or_fallback(
                crate::ask_flow::build_resume_followup_discussion_prompt_from_context(
                    state,
                    task,
                    prompt,
                    &binding.resume_context,
                ),
            ),
            None => route_result.resolved_intent.clone(),
        }
    } else {
        route_result.resolved_intent.clone()
    };
    ResumeRuntimePromptResolution {
        runtime_prompt,
        should_apply_context,
        should_discuss_context,
    }
}

#[cfg(test)]
#[path = "resume_policy_tests.rs"]
mod tests;

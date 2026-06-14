use serde_json::Value;

use super::LoopState;
use crate::{AgentAction, RouteResult};

pub(super) fn has_discussion_followup_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| match action {
        AgentAction::Respond { .. } => true,
        AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Think { .. } => false,
    })
}

pub(super) fn is_discussion_followup_action(action: &AgentAction) -> bool {
    match action {
        AgentAction::Respond { .. } => true,
        AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Think { .. } => false,
    }
}

fn synthesize_answer_requires_runtime_execution(evidence_refs: &[String]) -> bool {
    evidence_refs.len() > 1
        || evidence_refs
            .iter()
            .any(|reference| reference.trim() != "last_output")
}

pub(super) fn should_preserve_terminal_followup_for_observed_finalize(
    action: &AgentAction,
) -> bool {
    match action {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            synthesize_answer_requires_runtime_execution(evidence_refs)
        }
        _ => false,
    }
}

pub(super) fn has_authoritative_delivery(loop_state: &LoopState) -> bool {
    !loop_state.delivery_messages.is_empty()
        || loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
        || loop_state
            .last_publishable_synthesis_output
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
}

pub(super) fn is_plain_respond_only_plan(actions: &[AgentAction]) -> Option<&str> {
    match actions {
        [AgentAction::Respond { content }] => Some(content.as_str()),
        _ => None,
    }
}

pub(super) fn is_delivery_failure_terminal_reply(actions: &[AgentAction]) -> bool {
    let Some(content) = is_plain_respond_only_plan(actions) else {
        return false;
    };
    let trimmed = content.trim();
    !trimmed.is_empty() && crate::finalize::parse_delivery_token(trimmed).is_none()
}

fn missing_target_path_from_step_error(
    step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    let err = step.error.as_deref()?.trim();
    if !crate::skills::is_missing_target_skill_error(&step.skill, err) {
        return None;
    }
    if let Some(path) = err.strip_prefix("__RC_READ_FILE_NOT_FOUND__:") {
        let path = path.trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    if let Some(structured) = crate::skills::parse_structured_skill_error(err) {
        if let Some(extra) = structured.extra.as_ref() {
            for key in ["path", "target_path", "resolved_path"] {
                if let Some(path) = extra.get(key).and_then(Value::as_str).map(str::trim) {
                    if !path.is_empty() {
                        return Some(path.to_string());
                    }
                }
            }
        }
        let text = structured.error_text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    None
}

pub(super) fn terminal_reply_mentions_observed_missing_target(
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(content) = is_plain_respond_only_plan(actions) else {
        return false;
    };
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .filter_map(missing_target_path_from_step_error)
        .any(|path| trimmed.contains(path.trim()))
}

pub(super) fn route_expects_terminal_user_answer(route_result: &RouteResult) -> bool {
    if route_result.output_contract.delivery_required {
        return false;
    }
    !matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    )
}

pub(super) fn last_executable_action(actions: &[AgentAction]) -> Option<&AgentAction> {
    actions.iter().rev().find(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    })
}

pub(super) fn route_explicitly_requests_raw_command_output(
    route_result: Option<&RouteResult>,
) -> bool {
    route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
    })
}

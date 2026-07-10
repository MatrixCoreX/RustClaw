use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tracing::{info, warn};

use super::support::publish_agent_loop_checkpoint_progress;
use super::{
    append_progress_hint, attempt_ledger, encode_progress_i18n, ensure_task_running,
    execute_actions_once, load_agent_loop_guard_policy, prepare_round_actions, push_round_trace,
    AgentLoopGuardPolicy, AgentRunContext, LoopState, RoundOutcome,
};
use crate::{AgentAction, AppState, AskReply, ClaimedTask, RouteResult};

#[path = "loop_control_answer_recovery.rs"]
mod loop_control_answer_recovery;
#[path = "loop_control_answer_recovery_parse.rs"]
mod loop_control_answer_recovery_parse;
#[path = "loop_control_answer_recovery_text.rs"]
mod loop_control_answer_recovery_text;
#[path = "loop_control_filesystem_mutation_recovery.rs"]
mod loop_control_filesystem_mutation_recovery;
#[path = "loop_control_local_health_recovery.rs"]
mod loop_control_local_health_recovery;
#[path = "loop_control_recent_artifacts_recovery.rs"]
mod loop_control_recent_artifacts_recovery;

use loop_control_answer_recovery::*;
use loop_control_answer_recovery_parse::*;
use loop_control_answer_recovery_text::*;
use loop_control_filesystem_mutation_recovery::*;
use loop_control_local_health_recovery::*;
use loop_control_recent_artifacts_recovery::*;

fn has_authoritative_delivery(loop_state: &LoopState) -> bool {
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

fn answer_verifier_output_format_machine_payload_gap(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
    reply_text: &str,
) -> bool {
    if !verifier.high_confidence_retry_gap()
        || !verifier
            .missing_evidence_fields
            .iter()
            .any(|field| field == "output_format")
    {
        return false;
    }
    if verifier.answer_incomplete_reason == "machine_status_token_visible" {
        return true;
    }
    if visible_answer_is_machine_field_projection(reply_text) {
        return true;
    }
    serde_json::from_str::<Value>(reply_text.trim())
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|object| {
            object.contains_key("message_key")
                || object.contains_key("reason_code")
                || object.contains_key("candidates")
                || object.contains_key("risks")
                || object.contains_key("contract_marker")
                || object
                    .get("output_format")
                    .and_then(Value::as_str)
                    .is_some_and(|format| format == "machine_json")
                || (object.contains_key("status") && object.contains_key("steps"))
        })
}

fn visible_answer_is_machine_field_projection(reply_text: &str) -> bool {
    let mut field_count = 0usize;
    for token in reply_text.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if machine_projection_field_key(key) && !value.is_empty() {
            field_count += 1;
            if field_count >= 2 {
                return true;
            }
        }
    }
    false
}

fn machine_projection_field_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '.' | '-')
        })
        && key.chars().any(|ch| ch.is_ascii_lowercase())
}

fn visible_answer_is_observed_machine_status_token(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    reply_text: &str,
) -> bool {
    if matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        return false;
    }
    if !route_requires_direct_candidate_for_observed_stop(route_result) {
        return false;
    }
    let token = reply_text.trim();
    if !single_machine_token_answer(token) {
        return false;
    }
    journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .filter_map(|output| serde_json::from_str::<Value>(output.trim()).ok())
        .any(|value| {
            observed_machine_status_tokens(&value)
                .iter()
                .any(|candidate| candidate == token)
        })
}

fn single_machine_token_answer(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 80
        && !token.starts_with('{')
        && !token.starts_with('[')
        && !token.chars().any(char::is_whitespace)
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '/'))
}

fn observed_machine_status_tokens(value: &Value) -> Vec<String> {
    let mut tokens = Vec::new();
    collect_observed_machine_status_tokens(value, &mut tokens);
    tokens
}

fn collect_observed_machine_status_tokens(value: &Value, tokens: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if matches!(key.as_str(), "text" | "error_text") {
                    continue;
                }
                if machine_status_field_key(key) {
                    if let Some(token) = child
                        .as_str()
                        .map(str::trim)
                        .filter(|token| !token.is_empty() && single_machine_token_answer(token))
                    {
                        tokens.push(token.to_string());
                    }
                }
                collect_observed_machine_status_tokens(child, tokens);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_observed_machine_status_tokens(item, tokens);
            }
        }
        _ => {}
    }
}

fn machine_status_field_key(key: &str) -> bool {
    matches!(
        key,
        "status" | "status_code" | "state" | "reason_code" | "message_key"
    )
}

fn machine_status_visible_output_format_gap(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    reply_text: &str,
) -> Option<crate::answer_verifier::AnswerVerifierOut> {
    visible_answer_is_observed_machine_status_token(route_result, journal, reply_text).then(|| {
        crate::answer_verifier::AnswerVerifierOut {
            pass: false,
            missing_evidence_fields: vec!["output_format".to_string()],
            answer_incomplete_reason: "machine_status_token_visible".to_string(),
            should_retry: true,
            retry_instruction: "render_observed_machine_status_as_user_visible_answer".to_string(),
            confidence: 0.9,
        }
        .normalized()
    })
}

fn answer_verifier_summary_to_out(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> crate::answer_verifier::AnswerVerifierOut {
    crate::answer_verifier::AnswerVerifierOut {
        pass: verifier.pass,
        missing_evidence_fields: verifier.missing_evidence_fields.clone(),
        answer_incomplete_reason: verifier.answer_incomplete_reason.clone(),
        should_retry: verifier.should_retry,
        retry_instruction: verifier.retry_instruction.clone(),
        confidence: verifier.confidence,
    }
    .normalized()
}

fn retry_verifier_accepts_rewritten_answer(
    verifier: &crate::answer_verifier::AnswerVerifierOut,
) -> bool {
    verifier.pass && !verifier.high_confidence_gap()
}

fn commit_answer_verifier_retry_answer(reply: &mut AskReply, retried_answer: String) {
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(retried_answer.clone());
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&retried_answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        journal.record_final_stop_signal(
            crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL,
        );
    }
    reply.text = retried_answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = true;
}

async fn try_rewrite_exhausted_answer_verifier_gap_with_observed_evidence(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    route_result: Option<&RouteResult>,
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract.delivery_required
        || route.wants_file_delivery
        || !verifier.high_confidence_retry_gap()
        || !verifier.should_retry
    {
        return false;
    }
    let Some(journal_snapshot) = reply.task_journal.clone() else {
        return false;
    };
    if !crate::task_journal::evidence_coverage_for_route(route, &journal_snapshot).is_complete() {
        return false;
    }
    let verifier_out = answer_verifier_summary_to_out(verifier);
    let rejected_answer = reply.text.clone();
    let Some(retried_answer) = crate::finalize::retry_loop_answer_after_verifier(
        state,
        task,
        user_text,
        &journal_snapshot,
        &rejected_answer,
        &verifier_out,
    )
    .await
    else {
        return false;
    };
    if let Some(retry_verifier) = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        user_text,
        route,
        &journal_snapshot,
        &retried_answer,
    )
    .await
    {
        if retry_verifier_accepts_rewritten_answer(&retry_verifier) {
            commit_answer_verifier_retry_answer(reply, retried_answer);
            info!("answer_verifier_retry_exhausted_rewritten_with_observed_evidence");
            return true;
        }
        if let Some(journal) = reply.task_journal.as_mut() {
            journal.record_answer_verifier_summary(retry_verifier);
        }
        return false;
    }
    commit_answer_verifier_retry_answer(reply, retried_answer);
    info!("answer_verifier_retry_exhausted_rewritten_with_observed_evidence");
    true
}

fn record_session_start_hooks(task: &ClaimedTask, user_text: &str, loop_state: &mut LoopState) {
    let mut session_start =
        crate::agent_hooks::session_start_outcome().to_machine_json("agent_loop");
    if let Some(obj) = session_start.as_object_mut() {
        obj.insert("task_id".to_string(), json!(task.task_id));
        obj.insert("task_kind".to_string(), json!(task.kind));
        obj.insert("task_channel".to_string(), json!(task.channel));
    }
    loop_state.task_observations.push(session_start);

    let mut prompt_submit =
        crate::agent_hooks::user_prompt_submit_outcome().to_machine_json("agent_loop");
    if let Some(obj) = prompt_submit.as_object_mut() {
        obj.insert("task_id".to_string(), json!(task.task_id));
        obj.insert(
            "input_char_count".to_string(),
            json!(user_text.chars().count()),
        );
        obj.insert("input_byte_count".to_string(), json!(user_text.len()));
    }
    loop_state.task_observations.push(prompt_submit);
}

fn terminal_user_answer_stop_signal(loop_state: &LoopState) -> Option<&'static str> {
    has_authoritative_delivery(loop_state).then_some("terminal_user_answer_ready")
}

fn reply_final_status_is_clarify(reply: &AskReply) -> bool {
    reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.final_status)
        .is_some_and(|status| {
            matches!(status, crate::task_journal::TaskJournalFinalStatus::Clarify)
        })
}

fn route_expects_terminal_user_answer(route_result: &RouteResult) -> bool {
    if route_result.output_contract.delivery_required {
        return false;
    }
    !matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    )
}

fn route_requires_direct_candidate_for_observed_stop(route_result: &RouteResult) -> bool {
    route_result.output_contract_marker_is(crate::OutputSemanticKind::ServiceStatus)
        && crate::evidence_policy::final_answer_shape_for_route(route_result)
            .is_some_and(|shape| shape.allows_model_language())
}

fn has_discussion_followup_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| match action {
        AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::Think { .. } => false,
        AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. }
        | AgentAction::CallCapability { .. } => false,
    })
}

fn has_executable_observation_or_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. }
                | AgentAction::CallTool { .. }
                | AgentAction::SynthesizeAnswer { .. }
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuredRespondTerminalIntent {
    terminal_intent: String,
    content: Option<String>,
    clarify_reason_code: Option<String>,
    missing_slot: Option<String>,
    message_key: Option<String>,
    field_path: Option<String>,
    locator_kind: Option<String>,
}

fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn raw_plan_steps(raw_plan_text: &str) -> Vec<Value> {
    let Some(value) =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw_plan_text)
    else {
        return Vec::new();
    };
    if let Some(steps) = value.get("steps").and_then(Value::as_array) {
        return steps.clone();
    }
    if let Some(actions) = value.get("actions").and_then(Value::as_array) {
        return actions.clone();
    }
    value.as_array().cloned().unwrap_or_default()
}

fn structured_respond_terminal_intent_from_object(
    value: &Value,
) -> Option<StructuredRespondTerminalIntent> {
    let terminal_intent = string_field(value, &["terminal_intent"])?.to_ascii_lowercase();
    if !matches!(
        terminal_intent.as_str(),
        "answer" | "clarify" | "cannot_proceed" | "needs_confirmation" | "continue"
    ) {
        return None;
    }
    Some(StructuredRespondTerminalIntent {
        terminal_intent,
        content: string_field(value, &["content"]).map(str::to_string),
        clarify_reason_code: string_field(value, &["clarify_reason_code"]).map(str::to_string),
        missing_slot: string_field(value, &["missing_slot"]).map(str::to_string),
        message_key: string_field(value, &["message_key"]).map(str::to_string),
        field_path: string_field(value, &["field_path"]).map(str::to_string),
        locator_kind: string_field(value, &["locator_kind"]).map(str::to_string),
    })
}

fn structured_respond_terminal_intent_from_plan_step(
    step: &crate::PlanStep,
) -> Option<StructuredRespondTerminalIntent> {
    (step.action_type == "respond")
        .then(|| structured_respond_terminal_intent_from_object(&step.args))?
}

fn structured_respond_terminal_intent_from_raw_step(
    step: &Value,
) -> Option<StructuredRespondTerminalIntent> {
    let raw_type = string_field(step, &["type", "action_type", "action"])?.to_ascii_lowercase();
    (raw_type == "respond").then(|| structured_respond_terminal_intent_from_object(step))?
}

fn structured_respond_terminal_intent_from_plan(
    plan: &crate::PlanResult,
) -> Option<StructuredRespondTerminalIntent> {
    plan.steps
        .iter()
        .find_map(structured_respond_terminal_intent_from_plan_step)
        .or_else(|| {
            raw_plan_steps(&plan.raw_plan_text)
                .iter()
                .find_map(structured_respond_terminal_intent_from_raw_step)
        })
}

fn structured_respond_terminal_intent_from_route_owned_clarify(
    route: Option<&RouteResult>,
    actions: &[AgentAction],
) -> Option<StructuredRespondTerminalIntent> {
    let route = route?;
    if !route.needs_clarify || !actions_allow_structured_respond_terminal_intent(actions) {
        return None;
    }
    let content = actions.iter().find_map(|action| match action {
        AgentAction::Respond { content } => Some(content.trim()),
        _ => None,
    })?;
    if content.is_empty() {
        return None;
    }
    Some(StructuredRespondTerminalIntent {
        terminal_intent: "clarify".to_string(),
        content: Some(content.to_string()),
        clarify_reason_code: None,
        missing_slot: None,
        message_key: None,
        field_path: None,
        locator_kind: Some(route.output_contract.locator_kind.as_str().to_string()),
    })
}

fn actions_allow_structured_respond_terminal_intent(actions: &[AgentAction]) -> bool {
    actions.iter().all(|action| {
        matches!(
            action,
            AgentAction::Respond { .. } | AgentAction::Think { .. }
        )
    })
}

fn apply_structured_respond_clarify_to_loop_state(
    loop_state: &mut LoopState,
    intent: &StructuredRespondTerminalIntent,
) -> RoundOutcome {
    loop_state.pending_user_input_required = true;
    loop_state.output_vars.insert(
        "agent_loop.terminal_intent".to_string(),
        "clarify".to_string(),
    );
    if let Some(content) = intent
        .content
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        loop_state.delivery_messages.push(content.to_string());
        loop_state.last_user_visible_respond = Some(content.to_string());
    }
    record_structured_clarify_machine_fields(loop_state, intent);
    loop_state.history_compact.push(format!(
        "round={} structured_respond_terminal_intent=clarify missing_slot={}",
        loop_state.round_no,
        intent.missing_slot.as_deref().unwrap_or("")
    ));
    RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("structured_respond_clarify".to_string()),
        next_goal_hint: None,
        no_progress: false,
    }
}

fn machine_slot_is_nonblocking_freeform_slot(slot: Option<&str>) -> bool {
    let Some(slot) = slot.map(str::trim).filter(|slot| !slot.is_empty()) else {
        return false;
    };
    slot.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .any(|part| {
            matches!(
                part,
                "topic"
                    | "scope"
                    | "audience"
                    | "style"
                    | "format"
                    | "detail"
                    | "details"
                    | "section"
                    | "sections"
                    | "context"
                    | "goal"
                    | "goals"
            )
        })
}

fn route_allows_low_risk_freeform_clarify_replan(route: &RouteResult) -> bool {
    if route.needs_clarify || route.wants_file_delivery {
        return false;
    }
    if route.risk_ceiling != crate::RiskCeiling::Low {
        return false;
    }
    let contract = &route.output_contract;
    !contract.delivery_required
        && contract.delivery_intent == crate::OutputDeliveryIntent::None
        && contract.locator_kind == crate::OutputLocatorKind::None
        && !contract.requires_content_evidence
        && contract.semantic_kind == crate::OutputSemanticKind::None
        && matches!(
            contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::OneSentence
        )
}

fn try_replan_avoidable_low_risk_freeform_clarify(
    loop_state: &mut LoopState,
    route: Option<&RouteResult>,
    intent: &StructuredRespondTerminalIntent,
) -> Option<RoundOutcome> {
    let route = route?;
    if loop_state
        .output_vars
        .contains_key("agent_loop.avoidable_clarify_replan_used")
    {
        return None;
    }
    if !route_allows_low_risk_freeform_clarify_replan(route) {
        return None;
    }
    if !machine_slot_is_nonblocking_freeform_slot(intent.missing_slot.as_deref()) {
        return None;
    }

    loop_state.has_recoverable_failure_context = true;
    loop_state.output_vars.insert(
        "agent_loop.avoidable_clarify_replan_used".to_string(),
        "true".to_string(),
    );
    loop_state.history_compact.push(format!(
        "round={} recoverable_clarify=low_risk_freeform missing_slot={}",
        loop_state.round_no,
        intent.missing_slot.as_deref().unwrap_or("")
    ));
    attempt_ledger::record_attempt_with_retry_instruction(
        loop_state,
        "planner_clarify",
        &format!(
            "terminal_intent=clarify missing_slot={} route_risk={} locator_kind={} delivery_required={}",
            intent.missing_slot.as_deref().unwrap_or(""),
            route.risk_ceiling.as_str(),
            route.output_contract.locator_kind.as_str(),
            route.output_contract.delivery_required
        ),
        crate::executor::StepExecutionStatus::Error,
        intent.clarify_reason_code.as_deref().unwrap_or(""),
        Some("avoidable_clarify"),
        "recoverable_clarify_low_risk_freeform",
        Some(
            "The previous planner step asked for optional drafting details, but the route is low-risk chat-only freeform with no required evidence, locator, delivery, credential, or confirmation boundary. Replan with a useful best-effort draft/outline using neutral assumptions unless a real boundary slot is missing.",
        ),
    );
    info!(
        "low_risk_freeform_clarify_replan round={} missing_slot={}",
        loop_state.round_no,
        intent.missing_slot.as_deref().unwrap_or("")
    );
    Some(RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: None,
        no_progress: false,
    })
}

fn planner_locator_clarify_has_no_route_boundary(
    route: &RouteResult,
    intent: &StructuredRespondTerminalIntent,
) -> bool {
    if route.needs_clarify || route.wants_file_delivery {
        return false;
    }
    let contract = &route.output_contract;
    if contract.delivery_required
        || contract.delivery_intent != crate::OutputDeliveryIntent::None
        || contract.locator_kind != crate::OutputLocatorKind::None
        || contract.requires_content_evidence
    {
        return false;
    }

    let mentions_locator_boundary = intent
        .locator_kind
        .as_deref()
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .is_some_and(|kind| kind != "none")
        || intent
            .missing_slot
            .as_deref()
            .map(str::trim)
            .is_some_and(|slot| {
                slot.split(|ch: char| !ch.is_ascii_alphanumeric())
                    .filter(|part| !part.is_empty())
                    .any(|part| matches!(part, "locator" | "path" | "file" | "directory"))
            });
    mentions_locator_boundary
}

fn apply_nonblocking_structured_clarify_as_answer(
    loop_state: &mut LoopState,
    intent: &StructuredRespondTerminalIntent,
) -> RoundOutcome {
    loop_state.output_vars.insert(
        "agent_loop.terminal_intent".to_string(),
        "answer".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.recovered_terminal_intent".to_string(),
        "clarify".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.nonblocking_clarify_answer".to_string(),
        "true".to_string(),
    );
    record_structured_clarify_machine_fields(loop_state, intent);
    if let Some(content) = intent
        .content
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let content = append_structured_clarify_machine_line(content, intent);
        loop_state.delivery_messages.push(content.clone());
        loop_state.last_user_visible_respond = Some(content);
    }
    loop_state.history_compact.push(format!(
        "round={} structured_respond_terminal_intent=clarify treated_as=answer missing_slot={}",
        loop_state.round_no,
        intent.missing_slot.as_deref().unwrap_or("")
    ));
    RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("structured_respond_nonblocking_clarify_answer".to_string()),
        next_goal_hint: None,
        no_progress: false,
    }
}

fn record_structured_clarify_machine_fields(
    loop_state: &mut LoopState,
    intent: &StructuredRespondTerminalIntent,
) {
    for (key, value) in [
        (
            "agent_loop.clarify_reason_code",
            intent.clarify_reason_code.as_deref(),
        ),
        ("agent_loop.missing_slot", intent.missing_slot.as_deref()),
        ("agent_loop.message_key", intent.message_key.as_deref()),
        ("agent_loop.field_path", intent.field_path.as_deref()),
        ("agent_loop.locator_kind", intent.locator_kind.as_deref()),
    ] {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            loop_state
                .output_vars
                .insert(key.to_string(), value.to_string());
        }
    }
}

fn append_structured_clarify_machine_line(
    content: &str,
    intent: &StructuredRespondTerminalIntent,
) -> String {
    let Some(machine_line) = structured_clarify_machine_line(intent) else {
        return content.to_string();
    };
    if content.contains(machine_line.as_str()) {
        return content.to_string();
    }
    format!("{content}\n{machine_line}")
}

fn structured_clarify_machine_line(intent: &StructuredRespondTerminalIntent) -> Option<String> {
    let mut parts = vec!["terminal_intent=clarify".to_string()];
    for (key, value) in [
        ("clarify_reason_code", intent.clarify_reason_code.as_deref()),
        ("missing_slot", intent.missing_slot.as_deref()),
        ("message_key", intent.message_key.as_deref()),
        ("field_path", intent.field_path.as_deref()),
        ("locator_kind", intent.locator_kind.as_deref()),
    ] {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            parts.push(format!("{key}={value}"));
        }
    }
    (parts.len() > 1).then(|| parts.join(" "))
}

fn try_recover_inconsistent_boundary_clarify(
    loop_state: &mut LoopState,
    route: Option<&RouteResult>,
    intent: &StructuredRespondTerminalIntent,
) -> Option<RoundOutcome> {
    let route = route?;
    if !planner_locator_clarify_has_no_route_boundary(route, intent) {
        return None;
    }
    if loop_state
        .output_vars
        .contains_key("agent_loop.inconsistent_boundary_clarify_replan_used")
    {
        return Some(apply_nonblocking_structured_clarify_as_answer(
            loop_state, intent,
        ));
    }

    loop_state.has_recoverable_failure_context = true;
    loop_state.output_vars.insert(
        "agent_loop.inconsistent_boundary_clarify_replan_used".to_string(),
        "true".to_string(),
    );
    loop_state.history_compact.push(format!(
        "round={} recoverable_clarify=inconsistent_boundary missing_slot={} locator_kind={}",
        loop_state.round_no,
        intent.missing_slot.as_deref().unwrap_or(""),
        intent.locator_kind.as_deref().unwrap_or("")
    ));
    attempt_ledger::record_attempt_with_retry_instruction(
        loop_state,
        "planner_clarify",
        &format!(
            "terminal_intent=clarify missing_slot={} locator_kind={} route_locator_kind={} delivery_required={} content_evidence={}",
            intent.missing_slot.as_deref().unwrap_or(""),
            intent.locator_kind.as_deref().unwrap_or(""),
            route.output_contract.locator_kind.as_str(),
            route.output_contract.delivery_required,
            route.output_contract.requires_content_evidence
        ),
        crate::executor::StepExecutionStatus::Error,
        intent.clarify_reason_code.as_deref().unwrap_or(""),
        Some("inconsistent_boundary_clarify"),
        "recoverable_clarify_inconsistent_boundary",
        Some(
            "The previous planner step asked for a locator boundary, but the active route/output contract has no locator, delivery, content-evidence, or route-owned clarify boundary. Replan as a normal answer or recovery-guidance response unless the next plan can cite a concrete required machine boundary.",
        ),
    );
    info!(
        "inconsistent_boundary_clarify_replan round={} missing_slot={} locator_kind={}",
        loop_state.round_no,
        intent.missing_slot.as_deref().unwrap_or(""),
        intent.locator_kind.as_deref().unwrap_or("")
    );
    Some(RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: None,
        no_progress: false,
    })
}

fn record_agent_loop_decision_envelope_output_vars(
    loop_state: &mut LoopState,
    route: Option<&RouteResult>,
    plan: &crate::PlanResult,
) {
    let Some(route) = route else {
        return;
    };
    let envelope =
        crate::task_journal::agent_loop_round_plan_decision_envelope_for_runtime(route, plan);
    loop_state.output_vars.insert(
        "agent_loop.decision_envelope".to_string(),
        envelope.to_string(),
    );
    loop_state
        .output_vars
        .entry("agent_loop.first_decision_envelope".to_string())
        .or_insert_with(|| envelope.to_string());
    if envelope
        .get("control_intent")
        .and_then(Value::as_str)
        .is_some_and(|intent| intent == "act")
    {
        loop_state
            .output_vars
            .entry("agent_loop.first_act_decision_envelope".to_string())
            .or_insert_with(|| envelope.to_string());
    }
    for field in [
        "decision",
        "terminal_intent",
        "control_intent",
        "control_reason_code",
        "reason_code",
        "validation_status",
        "validation_reason_code",
        "capability_ref",
        "semantic_authority",
        "answer_shape",
        "risk_level",
    ] {
        if let Some(value) = envelope.get(field).and_then(Value::as_str) {
            loop_state
                .output_vars
                .insert(format!("agent_loop.{field}"), value.to_string());
            loop_state.output_vars.insert(
                format!("agent_loop.decision_envelope.{field}"),
                value.to_string(),
            );
        }
    }
}

fn last_executable_action(actions: &[AgentAction]) -> Option<&AgentAction> {
    actions.iter().rev().find(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    })
}

fn action_reads_text_content(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        AgentAction::CallCapability { .. } => return false,
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => return false,
    };
    let normalized_skill = skill.trim().replace('-', "_").to_ascii_lowercase();
    if matches!(normalized_skill.as_str(), "read_file" | "doc_parse") {
        return true;
    }
    normalized_skill == "system_basic"
        && args
            .get("action")
            .and_then(|value| value.as_str())
            .map(|action| action.trim().eq_ignore_ascii_case("read_range"))
            .unwrap_or(false)
}

fn route_needs_workspace_text_evidence_before_observed_finalize(route: &RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route.output_contract.response_shape == crate::OutputResponseShape::Free
        && route.output_contract_is_unclassified()
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.output_contract.locator_hint.trim().is_empty()
}

fn structured_scalar_equality_observation_can_finalize(
    route_result: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    route_result.output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
        && !route_result.output_contract.delivery_required
        && has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && super::observed_output::structured_scalar_equality_direct_answer(
            None,
            route_result,
            loop_state,
            None,
        )
        .is_some()
}

fn latest_path_batch_facts_all_missing(loop_state: &LoopState) -> bool {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || step.skill != "system_basic" {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts") {
            continue;
        }
        let Some(facts) = value.get("facts").and_then(|value| value.as_array()) else {
            return false;
        };
        if facts.is_empty() {
            return false;
        }
        return facts
            .iter()
            .all(|fact| fact.get("exists").and_then(|value| value.as_bool()) == Some(false));
    }
    false
}

pub(crate) fn requested_success_marker(
    _agent_run_context: Option<&AgentRunContext>,
) -> Option<&'static str> {
    None
}

fn observed_answer_contains_required_success_marker(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    marker: &str,
) -> bool {
    super::observed_output::extract_answer_from_observed_output(loop_state, agent_run_context)
        .is_some_and(|answer| text_has_exact_marker_line(&answer, marker))
        || super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some_and(|answer| text_has_exact_marker_line(&answer, marker))
}

fn text_has_exact_marker_line(text: &str, marker: &str) -> bool {
    let marker = marker.trim();
    !marker.is_empty() && text.lines().map(str::trim).any(|line| line == marker)
}

fn should_stop_for_observed_finalize(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if loop_state.execution_recipe.is_active()
        && !matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Done
        )
    {
        return false;
    }
    if loop_state.execution_recipe.needs_validation() {
        return false;
    }
    let route_result = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let Some(route_result) = route_result else {
        return false;
    };
    if route_result.needs_clarify
        || !loop_state.has_tool_or_skill_output
        || has_authoritative_delivery(loop_state)
    {
        return false;
    }
    if route_needs_workspace_text_evidence_before_observed_finalize(route_result)
        && !has_discussion_followup_action(actions)
        && !last_executable_action(actions).is_some_and(action_reads_text_content)
    {
        return false;
    }
    let required_success_marker = requested_success_marker(agent_run_context);
    let has_direct_observed_answer =
        super::observed_output::extract_answer_from_observed_output(loop_state, agent_run_context)
            .is_some();
    if structured_scalar_equality_observation_can_finalize(route_result, loop_state, actions) {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.output_contract_marker_is(crate::OutputSemanticKind::ExistenceWithPath)
        && has_direct_observed_answer
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && recent_artifacts_inventory_observation_can_finalize(route_result, loop_state)
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.output_contract_marker_is(crate::OutputSemanticKind::RecentArtifactsJudgment)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if quantity_comparison_one_sentence_needs_model_language_before_stop(route_result)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if super::observed_output::route_disallows_direct_observation_passthrough(route_result)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        && loop_state.round_no < loop_state.max_rounds
        && latest_path_batch_facts_all_missing(loop_state)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if has_direct_observed_answer
        && route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar {
        if super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some()
        {
            return required_success_marker.is_none_or(|marker| {
                observed_answer_contains_required_success_marker(
                    agent_run_context,
                    loop_state,
                    marker,
                )
            });
        }
        if super::observed_output::scalar_route_prefers_structured_observed_answer(
            route_result,
            loop_state,
        ) && super::observed_output::extract_answer_from_observed_output(
            loop_state,
            agent_run_context,
        )
        .is_some()
        {
            return required_success_marker.is_none_or(|marker| {
                observed_answer_contains_required_success_marker(
                    agent_run_context,
                    loop_state,
                    marker,
                )
            });
        }
    }
    let can_stop = has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && route_expects_terminal_user_answer(route_result)
        && if route_requires_direct_candidate_for_observed_stop(route_result) {
            has_direct_observed_answer
        } else {
            super::observed_output::has_observed_answer_candidates(loop_state)
        };
    can_stop
        && required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        })
}

fn quantity_comparison_one_sentence_needs_model_language_before_stop(
    route_result: &RouteResult,
) -> bool {
    route_result.output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison)
        && route_result.output_contract.response_shape == crate::OutputResponseShape::OneSentence
        && crate::evidence_policy::final_answer_shape_for_route(route_result)
            .is_some_and(|shape| shape.allows_model_language())
}

fn evaluate_round_outcome(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    outcome: &RoundOutcome,
) -> bool {
    if outcome.had_error {
        info!(
            "loop_round_stop task_id={} round={} reason=had_error",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if let Some(reason) = &outcome.stop_signal {
        if reason == "recoverable_failure_continue_round" {
            if !policy.multi_round_enabled {
                info!(
                    "loop_round_stop task_id={} round={} reason=recoverable_failure_multi_round_disabled",
                    task.task_id, loop_state.round_no
                );
                return true;
            }
            if loop_state.round_no >= loop_state.max_rounds {
                if loop_state.recoverable_failure_extra_rounds_used
                    >= policy.recoverable_failure_extra_rounds
                {
                    info!(
                        "loop_round_stop task_id={} round={} reason=recoverable_failure_extra_rounds_exhausted used={} limit={}",
                        task.task_id,
                        loop_state.round_no,
                        loop_state.recoverable_failure_extra_rounds_used,
                        policy.recoverable_failure_extra_rounds
                    );
                    return true;
                }
                loop_state.recoverable_failure_extra_rounds_used += 1;
                loop_state.max_rounds += 1;
                info!(
                    "loop_round_extend task_id={} round={} reason={} new_max_rounds={} used_extra={}",
                    task.task_id,
                    loop_state.round_no,
                    reason,
                    loop_state.max_rounds,
                    loop_state.recoverable_failure_extra_rounds_used
                );
            }
            loop_state.consecutive_no_progress = 0;
            info!(
                "loop_round_continue task_id={} round={} reason={}",
                task.task_id, loop_state.round_no, reason
            );
            return false;
        }
        info!(
            "loop_round_stop task_id={} round={} reason={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            reason,
            crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
        );
        return true;
    }
    if outcome.executed_actions == 0 {
        info!(
            "loop_round_stop task_id={} round={} reason=no_actions",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if outcome.no_progress {
        loop_state.consecutive_no_progress += 1;
    } else {
        loop_state.consecutive_no_progress = 0;
    }
    if loop_state.consecutive_no_progress > policy.no_progress_limit {
        info!(
            "loop_round_stop task_id={} round={} reason=no_progress limit={} count={}",
            task.task_id,
            loop_state.round_no,
            policy.no_progress_limit,
            loop_state.consecutive_no_progress
        );
        return true;
    }
    if !policy.multi_round_enabled {
        info!(
            "loop_round_stop task_id={} round={} reason=multi_round_disabled",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if loop_state.round_no >= loop_state.max_rounds {
        info!(
            "loop_round_stop task_id={} round={} reason=max_rounds reached={}",
            task.task_id, loop_state.round_no, loop_state.max_rounds
        );
        return true;
    }
    false
}

fn soft_budget_checkpoint_resume_reason(
    loop_state: &LoopState,
    policy: &AgentLoopGuardPolicy,
    outcome: &RoundOutcome,
) -> Option<&'static str> {
    if outcome.had_error {
        return None;
    }
    if outcome
        .stop_signal
        .as_deref()
        .is_some_and(|signal| signal == "recoverable_failure_continue_round")
    {
        if let Some(reason) = recoverable_provider_blocker_resume_reason(loop_state) {
            return Some(reason);
        }
        return None;
    }
    if outcome.stop_signal.is_some() || outcome.executed_actions == 0 {
        return None;
    }
    if outcome.no_progress && loop_state.consecutive_no_progress > policy.no_progress_limit {
        return Some("agent_loop_no_progress_limit");
    }
    if policy.multi_round_enabled && loop_state.round_no >= loop_state.max_rounds {
        return Some("agent_loop_max_rounds");
    }
    None
}

fn recoverable_provider_blocker_resume_reason(loop_state: &LoopState) -> Option<&'static str> {
    let latest = loop_state
        .attempt_ledger_entries
        .iter()
        .rev()
        .find(|entry| {
            entry.provider_status.is_some()
                && entry.recovery_action.trim() == "wait_background"
                && entry.status.trim() != crate::executor::StepExecutionStatus::Ok.as_str()
        })?;
    latest.provider_status.as_ref()?;
    Some("provider_blocker_wait_background")
}

fn worker_soft_checkpoint_after_seconds(worker_timeout_secs: u64) -> Option<u64> {
    let timeout = worker_timeout_secs.max(1);
    if timeout <= 2 {
        return None;
    }
    let reserve = (timeout / 10).clamp(1, 30);
    let soft_after = timeout.saturating_sub(reserve);
    (soft_after > 0 && soft_after < timeout).then_some(soft_after)
}

fn worker_soft_checkpoint_after(worker_timeout_secs: u64) -> Option<Duration> {
    worker_soft_checkpoint_after_seconds(worker_timeout_secs).map(Duration::from_secs)
}

fn worker_budget_near_exhaustion(
    started_at: Instant,
    soft_checkpoint_after: Option<Duration>,
) -> bool {
    soft_checkpoint_after.is_some_and(|duration| started_at.elapsed() >= duration)
}

fn loop_state_has_recoverable_checkpoint_state(loop_state: &LoopState) -> bool {
    loop_state.task_checkpoint.is_some()
        || !loop_state.executed_step_results.is_empty()
        || !loop_state.successful_action_fingerprints.is_empty()
        || loop_state.has_tool_or_skill_output
}

async fn run_agent_round(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<RoundOutcome, String> {
    info!(
        "loop_round_start task_id={} round={} max_rounds={} total_steps={} tool_calls_total={}",
        task.task_id,
        loop_state.round_no,
        loop_state.max_rounds,
        loop_state.total_steps_executed,
        loop_state.tool_calls_total
    );
    let prepared_round = prepare_round_actions(
        state,
        task,
        goal,
        user_text,
        policy,
        loop_state,
        agent_run_context,
    )
    .await?;
    push_round_trace(loop_state, goal, &prepared_round);
    let route_result = prepared_round
        .effective_route_result
        .as_ref()
        .or_else(|| agent_run_context.and_then(|ctx| ctx.route_result.as_ref()));
    record_agent_loop_decision_envelope_output_vars(
        loop_state,
        route_result,
        &prepared_round.plan_result,
    );
    if let Some(route) = route_result {
        loop_state.route_policy_context = Some(route.clone());
    }
    if let Some(output_contract) = prepared_round.effective_output_contract.as_ref() {
        loop_state.output_contract = Some(output_contract.clone());
        if let Some(final_answer_shape) =
            crate::evidence_policy::final_answer_shape_for_output_contract(output_contract)
        {
            loop_state.output_vars.insert(
                "agent_loop.final_answer_shape".to_string(),
                final_answer_shape.as_str().to_string(),
            );
            loop_state.output_vars.insert(
                "agent_loop.final_answer_shape_class".to_string(),
                final_answer_shape.class().as_str().to_string(),
            );
        }
    }
    let _budget_profile =
        AgentLoopGuardPolicy::budget_profile_for_context(loop_state.execution_recipe, route_result);
    if let Some(intent) = structured_respond_terminal_intent_from_plan(&prepared_round.plan_result)
        .filter(|intent| intent.terminal_intent == "clarify")
        .filter(|_| actions_allow_structured_respond_terminal_intent(&prepared_round.actions))
        .or_else(|| {
            structured_respond_terminal_intent_from_route_owned_clarify(
                route_result,
                &prepared_round.actions,
            )
        })
    {
        if let Some(outcome) =
            try_replan_avoidable_low_risk_freeform_clarify(loop_state, route_result, &intent)
        {
            info!(
                "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
                task.task_id,
                loop_state.round_no,
                outcome.executed_actions,
                outcome.no_progress,
                outcome.stop_signal.as_deref().unwrap_or(""),
                crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
            );
            return Ok(outcome);
        }
        if let Some(outcome) =
            try_recover_inconsistent_boundary_clarify(loop_state, route_result, &intent)
        {
            info!(
                "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
                task.task_id,
                loop_state.round_no,
                outcome.executed_actions,
                outcome.no_progress,
                outcome.stop_signal.as_deref().unwrap_or(""),
                crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
            );
            return Ok(outcome);
        }
        let outcome = apply_structured_respond_clarify_to_loop_state(loop_state, &intent);
        info!(
            "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            outcome.executed_actions,
            outcome.no_progress,
            outcome.stop_signal.as_deref().unwrap_or(""),
            crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
        );
        return Ok(outcome);
    }
    let actions = prepared_round.actions;
    let mut outcome = execute_actions_once(
        state,
        task,
        goal,
        user_text,
        &actions,
        loop_state,
        policy,
        agent_run_context,
    )
    .await?;
    if outcome.stop_signal.is_none() {
        if let Some(stop_signal) = terminal_user_answer_stop_signal(loop_state) {
            outcome.stop_signal = Some(stop_signal.to_string());
        }
    }
    if outcome.stop_signal.is_none()
        && should_stop_for_observed_finalize(agent_run_context, loop_state, &actions)
    {
        outcome.stop_signal = Some("observed_output_ready".to_string());
    }
    info!(
        "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
        task.task_id,
        loop_state.round_no,
        outcome.executed_actions,
        outcome.no_progress,
        outcome.stop_signal.as_deref().unwrap_or(""),
        crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
    );
    Ok(outcome)
}

fn initial_execution_recipe_spec(
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> crate::execution_recipe::ExecutionRecipeSpec {
    if let Some(spec) = agent_run_context.and_then(|ctx| ctx.execution_recipe_hint) {
        return spec;
    }
    let _ = (goal, user_text);
    warn!(
        "execution_recipe_no_hint_bypass_local_detector route_available={} user_request_available={}",
        agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .is_some(),
        agent_run_context
            .and_then(|ctx| ctx.user_request.as_deref())
            .is_some_and(|text| !text.trim().is_empty())
    );
    crate::execution_recipe::ExecutionRecipeSpec::default()
}

pub(super) async fn run_agent_with_loop(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<AskReply, String> {
    run_agent_with_loop_seeded(state, task, goal, user_text, agent_run_context, None).await
}

pub(super) async fn run_agent_with_loop_seeded(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    resume_checkpoint: Option<&crate::task_lifecycle::TaskCheckpoint>,
) -> Result<AskReply, String> {
    let base_policy = load_agent_loop_guard_policy(state);
    let mut loop_state = LoopState::new(base_policy.max_rounds.max(1));
    super::seed_loop_state_for_agent_run(&mut loop_state, agent_run_context, resume_checkpoint);
    record_session_start_hooks(task, user_text, &mut loop_state);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        initial_execution_recipe_spec(goal, user_text, agent_run_context),
    );
    let route_result = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let budget_profile =
        AgentLoopGuardPolicy::budget_profile_for_context(loop_state.execution_recipe, route_result);
    let policy = base_policy.adjusted_for_context(loop_state.execution_recipe, route_result);
    loop_state.max_rounds = policy.max_rounds.max(1);
    base_policy.apply_recipe_runtime_overrides(&mut loop_state.execution_recipe);
    let enabled_rollout_switches = policy.enabled_rollout_switches();
    if !enabled_rollout_switches.is_empty() {
        loop_state.output_vars.insert(
            "rollout_switches_enabled".to_string(),
            enabled_rollout_switches.join(","),
        );
    }
    info!(
        "loop_budget_profile task_id={} profile={} max_rounds={} max_steps={} max_tool_calls={} no_progress_limit={} repeat_action_limit={}",
        task.task_id,
        budget_profile.as_str(),
        policy.max_rounds,
        policy.max_steps,
        policy.max_tool_calls,
        policy.no_progress_limit,
        policy.repeat_action_limit
    );
    let mut round = 1usize;
    let mut answer_verifier_retry_count = 0usize;
    let loop_started_at = Instant::now();
    let worker_soft_checkpoint_after =
        worker_soft_checkpoint_after(state.worker.worker_task_timeout_seconds);
    loop {
        while round <= loop_state.max_rounds {
            ensure_task_running(state, task)?;
            loop_state.round_no = round;
            if worker_budget_near_exhaustion(loop_started_at, worker_soft_checkpoint_after)
                && loop_state_has_recoverable_checkpoint_state(&loop_state)
            {
                loop_state.last_stop_signal = Some("budget_near_exhaustion".to_string());
                publish_agent_loop_checkpoint_progress(
                    state,
                    task,
                    &mut loop_state,
                    "budget_near_exhaustion",
                );
                break;
            }
            super::maybe_publish_execution_recipe_phase_hint(state, task, &mut loop_state);
            let outcome = run_agent_round(
                state,
                task,
                goal,
                user_text,
                &policy,
                &mut loop_state,
                agent_run_context,
            )
            .await?;
            loop_state.last_stop_signal = outcome.stop_signal.clone();
            if worker_budget_near_exhaustion(loop_started_at, worker_soft_checkpoint_after)
                && !outcome.had_error
                && outcome.executed_actions > 0
                && loop_state_has_recoverable_checkpoint_state(&loop_state)
            {
                loop_state.last_stop_signal = Some("budget_near_exhaustion".to_string());
                publish_agent_loop_checkpoint_progress(
                    state,
                    task,
                    &mut loop_state,
                    "budget_near_exhaustion",
                );
                break;
            }
            if evaluate_round_outcome(task, &mut loop_state, &policy, &outcome) {
                if let Some(resume_reason) =
                    soft_budget_checkpoint_resume_reason(&loop_state, &policy, &outcome)
                {
                    publish_agent_loop_checkpoint_progress(
                        state,
                        task,
                        &mut loop_state,
                        resume_reason,
                    );
                }
                break;
            }
            round += 1;
        }
        let pre_finalize_loop_state = loop_state.clone();
        let mut reply = crate::finalize::finalize_loop_reply(
            state,
            task,
            user_text,
            loop_state,
            agent_run_context,
        )
        .await?;
        let answer_contract_route_result =
            answer_contract_route_result_for_reply(agent_run_context, &reply);
        prefer_terminal_model_answer_for_verifier_candidate(
            &mut reply,
            answer_contract_route_result.as_ref(),
        );
        attach_answer_verifier_if_missing(
            state,
            task,
            user_text,
            &policy,
            answer_contract_route_result.as_ref(),
            &mut reply,
        )
        .await;
        let route_result = answer_contract_route_result.as_ref();
        suppress_answer_verifier_retry_if_structurally_satisfied(&mut reply, route_result);
        suppress_answer_verifier_retry_if_user_locator_disambiguation(&mut reply, route_result);
        suppress_answer_verifier_retry_if_confirmed_missing_file_delivery(&mut reply, route_result);
        if try_preserve_rss_source_hosts_from_structured_evidence(route_result, &mut reply) {
            return Ok(reply);
        }
        if try_recover_document_heading_answer_verifier_gap(route_result, &mut reply) {
            return Ok(reply);
        }
        if let Some(verifier) = answer_verifier_retry_summary(&reply, route_result).cloned() {
            if answer_verifier_output_format_machine_payload_gap(&verifier, &reply.text) {
                if let (Some(route), Some(journal_snapshot)) =
                    (route_result, reply.task_journal.clone())
                {
                    let verifier_out = answer_verifier_summary_to_out(&verifier);
                    if let Some(retried_answer) = crate::finalize::retry_loop_answer_after_verifier(
                        state,
                        task,
                        user_text,
                        &journal_snapshot,
                        &reply.text,
                        &verifier_out,
                    )
                    .await
                    {
                        let retry_verifier = crate::answer_verifier::verify_answer_observe_only(
                            state,
                            task,
                            user_text,
                            route,
                            &journal_snapshot,
                            &retried_answer,
                        )
                        .await;
                        if let Some(retry_verifier) = retry_verifier {
                            if retry_verifier_accepts_rewritten_answer(&retry_verifier) {
                                commit_answer_verifier_retry_answer(&mut reply, retried_answer);
                                info!(
                                    "answer_verifier_machine_payload_rewritten_to_visible_answer"
                                );
                                return Ok(reply);
                            }
                            if let Some(journal) = reply.task_journal.as_mut() {
                                journal.record_answer_verifier_summary(retry_verifier);
                            }
                        } else {
                            commit_answer_verifier_retry_answer(&mut reply, retried_answer);
                            info!("answer_verifier_machine_payload_rewritten_to_visible_answer");
                            return Ok(reply);
                        }
                    }
                }
            }
            if try_recover_structured_listing_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if answer_verifier_retry_budget_available(&policy, answer_verifier_retry_count) {
                loop_state = pre_finalize_loop_state;
                answer_verifier_retry_count += 1;
                loop_state.has_recoverable_failure_context = true;
                loop_state.delivery_messages.clear();
                loop_state.last_user_visible_respond = None;
                loop_state.last_publishable_synthesis_output = None;
                loop_state.last_stop_signal = Some("answer_verifier_retry".to_string());
                attempt_ledger::record_attempt_with_retry_instruction(
                    &mut loop_state,
                    "answer_verifier",
                    &format!(
                        "missing_evidence_fields={}",
                        verifier.missing_evidence_fields.join(",")
                    ),
                    crate::executor::StepExecutionStatus::Error,
                    &reply.text,
                    Some("answer_incomplete"),
                    &verifier.answer_incomplete_reason,
                    Some(&verifier.retry_instruction),
                );
                append_progress_hint(
                    state,
                    task,
                    &mut loop_state.progress_messages,
                    encode_progress_i18n("telegram.progress.answer_incomplete_retry", &[]),
                );
                if loop_state.round_no >= loop_state.max_rounds {
                    loop_state.max_rounds += 1;
                }
                round = loop_state.round_no + 1;
                info!(
                    "loop_round_extend task_id={} round={} reason=answer_verifier_retry new_max_rounds={}",
                    task.task_id, loop_state.round_no, loop_state.max_rounds
                );
                continue;
            }
            warn!(
                "answer_verifier_retry_exhausted task_id={} retry_count={} limit={} reason={}",
                task.task_id,
                answer_verifier_retry_count,
                policy.answer_verifier_retry_limit,
                crate::truncate_for_log(&verifier.answer_incomplete_reason)
            );
            if try_recover_structured_listing_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_latest_synthesis_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_log_analyze_answer_verifier_gap(user_text, &mut reply) {
                return Ok(reply);
            }
            if try_recover_structured_count_answer_verifier_gap(route_result, user_text, &mut reply)
            {
                return Ok(reply);
            }
            if try_recover_structured_search_answer_verifier_gap(
                route_result,
                user_text,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_rss_news_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_document_heading_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_structured_scalar_output_format_answer_verifier_gap(
                route_result,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_machine_kv_summary_output_format_answer_verifier_gap(
                route_result,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_rewrite_exhausted_answer_verifier_gap_with_observed_evidence(
                state,
                task,
                user_text,
                route_result,
                &verifier,
                &mut reply,
            )
            .await
            {
                return Ok(reply);
            }
            if try_recover_structured_evidence_table_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_http_health_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_local_health_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_recent_artifacts_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_generic_path_content_read_range_answer_verifier_gap(
                route_result,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_content_excerpt_summary_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_filesystem_mutation_success_answer_verifier_gap(route_result, &mut reply)
            {
                return Ok(reply);
            }
            if try_accept_language_only_output_format_answer_verifier_gap(route_result, &mut reply)
            {
                return Ok(reply);
            }
            mark_reply_failed_after_answer_verifier_exhausted(user_text, &mut reply, &verifier);
        }
        return Ok(reply);
    }
}

fn answer_verifier_retry_budget_available(
    policy: &AgentLoopGuardPolicy,
    answer_verifier_retry_count: usize,
) -> bool {
    answer_verifier_retry_count < policy.answer_verifier_retry_limit
}

async fn attach_answer_verifier_if_missing(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    route_result: Option<&RouteResult>,
    reply: &mut AskReply,
) {
    if reply.should_fail_task || reply_final_status_is_clarify(reply) {
        return;
    }
    let Some(route_result) = route_result else {
        return;
    };
    let Some(journal) = reply.task_journal.as_mut() else {
        return;
    };
    if journal.answer_verifier_summary.is_some() {
        return;
    }
    if let Some((selected_class, answer_verifier)) =
        selected_contract_structured_evidence_gap(policy, route_result, journal)
    {
        journal.record_answer_verifier_summary(answer_verifier);
        let summary = journal.answer_verifier_summary.as_ref();
        journal.rollout_attribution.push(
            crate::task_journal::TaskJournalRolloutAttribution::selected_contract_structured_evidence_block(
                summary,
                selected_class,
            ),
        );
        return;
    }
    if let Some(answer_verifier) =
        machine_status_visible_output_format_gap(route_result, journal, &reply.text)
    {
        journal.record_answer_verifier_summary(answer_verifier);
        return;
    }
    if let Some(answer_verifier) = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        user_text,
        route_result,
        journal,
        &reply.text,
    )
    .await
    {
        journal.record_answer_verifier_summary(answer_verifier);
    }
}

fn answer_contract_route_result_for_reply(
    agent_run_context: Option<&AgentRunContext>,
    reply: &AskReply,
) -> Option<RouteResult> {
    reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.route_result.clone())
        .or_else(|| agent_run_context.and_then(|ctx| ctx.route_result.clone()))
}

fn selected_contract_structured_evidence_gap(
    policy: &AgentLoopGuardPolicy,
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> Option<(&'static str, crate::answer_verifier::AnswerVerifierOut)> {
    if !policy.structured_evidence_required_for_selected_contracts {
        return None;
    }
    if route_result.risk_ceiling == crate::RiskCeiling::High
        || route_result.schedule_kind != crate::ScheduleKind::None
    {
        return None;
    }
    let selected_class =
        super::migration_class::agent_decides_eligible_migration_class(route_result);
    if selected_class == "none" {
        return None;
    }
    crate::answer_verifier::local_missing_evidence_verifier_gap(route_result, journal)
        .map(|gap| (selected_class, gap))
}

#[cfg(test)]
#[path = "loop_control_tests.rs"]
mod tests;

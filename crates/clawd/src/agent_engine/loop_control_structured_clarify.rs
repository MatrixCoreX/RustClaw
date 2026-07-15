use serde_json::Value;
use tracing::info;

use super::{attempt_ledger, LoopState, RoundOutcome};
use crate::{AgentAction, RouteResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StructuredRespondTerminalIntent {
    pub(super) terminal_intent: String,
    pub(super) content: Option<String>,
    pub(super) clarify_reason_code: Option<String>,
    pub(super) missing_slot: Option<String>,
    pub(super) message_key: Option<String>,
    pub(super) field_path: Option<String>,
    pub(super) locator_kind: Option<String>,
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

pub(super) fn structured_respond_terminal_intent_from_plan_step(
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

pub(super) fn structured_respond_terminal_intent_from_plan(
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

pub(super) fn structured_respond_terminal_intent_from_route_owned_clarify(
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

pub(super) fn structured_respond_terminal_intent_from_boundary_observation_clarify(
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> Option<StructuredRespondTerminalIntent> {
    if !loop_state.boundary_observation_needs_clarify
        || !actions_allow_structured_respond_terminal_intent(actions)
    {
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
        clarify_reason_code: Some("boundary_observation_needs_clarify".to_string()),
        missing_slot: None,
        message_key: None,
        field_path: None,
        locator_kind: None,
    })
}

pub(super) fn forced_boundary_observation_clarify_intent(
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> Option<StructuredRespondTerminalIntent> {
    if !loop_state.boundary_observation_needs_clarify
        || actions_allow_structured_respond_terminal_intent(actions)
        || actions_have_concrete_target_binding(actions)
    {
        return None;
    }
    Some(StructuredRespondTerminalIntent {
        terminal_intent: "clarify".to_string(),
        content: None,
        clarify_reason_code: Some("boundary_observation_needs_clarify".to_string()),
        missing_slot: Some("referent".to_string()),
        message_key: Some("clawd.clarify.missing_referent".to_string()),
        field_path: Some("agent_loop.boundary_observations.missing_referent".to_string()),
        locator_kind: Some("none".to_string()),
    })
}

fn actions_have_concrete_target_binding(actions: &[AgentAction]) -> bool {
    actions.iter().any(action_has_concrete_target_binding)
}

fn action_has_concrete_target_binding(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallTool { args, .. }
        | AgentAction::CallSkill { args, .. }
        | AgentAction::CallCapability { args, .. } => args_have_concrete_target_binding(args),
        AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Think { .. } => false,
    }
}

fn args_have_concrete_target_binding(args: &Value) -> bool {
    let Some(object) = args.as_object() else {
        return false;
    };
    [
        "path",
        "paths",
        "root",
        "target",
        "target_path",
        "source_path",
        "left_path",
        "right_path",
        "db_path",
        "url",
        "task_id",
        "job_id",
    ]
    .iter()
    .any(|key| json_value_has_concrete_target(object.get(*key)))
}

fn json_value_has_concrete_target(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| json_value_has_concrete_target(Some(item))),
        Some(Value::Object(object)) => object
            .values()
            .any(|item| json_value_has_concrete_target(Some(item))),
        _ => false,
    }
}

pub(super) fn actions_allow_structured_respond_terminal_intent(actions: &[AgentAction]) -> bool {
    actions.iter().all(|action| {
        matches!(
            action,
            AgentAction::Respond { .. } | AgentAction::Think { .. }
        )
    })
}

pub(super) fn apply_structured_respond_clarify_to_loop_state(
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
        let content = content.to_string();
        loop_state.delivery_messages.push(content.clone());
        loop_state.last_user_visible_respond = Some(content);
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
    if slot == "user_input" {
        return true;
    }
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
                    | "content"
                    | "context"
                    | "goal"
                    | "goals"
            )
        })
}

fn route_allows_side_effect_free_freeform_clarify_replan(
    loop_state: &LoopState,
    route: &RouteResult,
) -> bool {
    if route.wants_file_delivery {
        return false;
    }
    if route.schedule_kind != crate::ScheduleKind::None || route.should_refresh_long_term_memory {
        return false;
    }
    if route.needs_clarify && !loop_state.pending_user_boundary_present {
        return false;
    }
    if route.risk_ceiling == crate::RiskCeiling::High {
        return false;
    }
    if !matches!(
        route.risk_ceiling,
        crate::RiskCeiling::Low | crate::RiskCeiling::Medium
    ) {
        return false;
    }
    let contract = &route.output_contract;
    if contract.delivery_required
        || contract.delivery_intent != crate::OutputDeliveryIntent::None
        || contract.semantic_kind != crate::OutputSemanticKind::None
        || !matches!(
            contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::OneSentence
        )
    {
        return false;
    }
    if contract.locator_kind == crate::OutputLocatorKind::None {
        return !contract.requires_content_evidence || loop_state.pending_user_boundary_present;
    }
    loop_state.pending_user_boundary_present
        && route.has_route_reason_machine_marker("structured_observation_clarify_repair")
}

pub(super) fn try_replan_avoidable_side_effect_free_freeform_clarify(
    loop_state: &mut LoopState,
    route: Option<&RouteResult>,
    intent: &StructuredRespondTerminalIntent,
) -> Option<RoundOutcome> {
    let route = route?;
    if !route_allows_side_effect_free_freeform_clarify_replan(loop_state, route) {
        return None;
    }
    if !machine_slot_is_nonblocking_freeform_slot(intent.missing_slot.as_deref()) {
        return None;
    }
    if loop_state
        .output_vars
        .contains_key("agent_loop.avoidable_clarify_replan_used")
    {
        return Some(apply_nonblocking_structured_clarify_as_answer(
            loop_state, intent,
        ));
    }

    loop_state.has_recoverable_failure_context = true;
    loop_state.output_vars.insert(
        "agent_loop.avoidable_clarify_replan_used".to_string(),
        "true".to_string(),
    );
    loop_state.history_compact.push(format!(
        "round={} recoverable_clarify=side_effect_free_freeform missing_slot={}",
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
        "recoverable_clarify_side_effect_free_freeform",
        Some(
            "The previous planner step asked for optional drafting details, but the current plan is respond-only and the output contract has no required evidence, locator, delivery, credential, confirmation, or side-effect boundary. Replan with a useful best-effort draft/outline using neutral assumptions unless a real boundary slot is missing.",
        ),
    );
    info!(
        "side_effect_free_freeform_clarify_replan round={} missing_slot={}",
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
    if route.has_route_reason_machine_marker("standalone_freeform_clarify_loop_context") {
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

pub(super) fn apply_nonblocking_structured_clarify_as_answer(
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
        let content = content.to_string();
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

pub(super) fn record_structured_clarify_machine_fields(
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

pub(super) fn try_recover_inconsistent_boundary_clarify(
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

pub(super) fn record_agent_loop_decision_envelope_output_vars(
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
    for field in [
        "clarify_reason_code",
        "missing_slot",
        "message_key",
        "field_path",
        "locator_kind",
    ] {
        let mut loop_key = String::from("agent_loop.");
        loop_key.push_str(field);
        loop_state.output_vars.remove(&loop_key);
        let mut envelope_key = String::from("agent_loop.decision_envelope.");
        envelope_key.push_str(field);
        loop_state.output_vars.remove(&envelope_key);
    }
    if envelope
        .get("terminal_intent")
        .and_then(Value::as_str)
        .is_some_and(|intent| intent != "clarify")
    {
        loop_state.pending_user_input_required = false;
        loop_state
            .output_vars
            .remove("agent_loop.recovered_terminal_intent");
        loop_state
            .output_vars
            .remove("agent_loop.nonblocking_clarify_answer");
    }
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
        "clarify_reason_code",
        "missing_slot",
        "message_key",
        "field_path",
        "locator_kind",
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

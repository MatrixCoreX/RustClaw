use serde_json::Value;

use crate::agent_engine::{AgentRunContext, LoopState};

pub(super) fn current_delivery_contains_full_structured_contract(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    delivery_messages
        .iter()
        .chain(loop_state.delivery_messages.iter())
        .chain(loop_state.last_user_visible_respond.iter())
        .any(|message| structured_contract_json_should_remain_full(message))
}

fn structured_contract_json_should_remain_full(text: &str) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    if structured_config_guard_json_should_remain_full(&object) {
        return true;
    }
    object.contains_key("contract_marker")
        && (object.contains_key("async_timeout_policy")
            || object.contains_key("adapter_result")
            || object.contains_key("pending_async_job_contract")
            || object.contains_key("task_lifecycle")
            || object.contains_key("execution_policy"))
}

fn structured_config_guard_json_should_remain_full(
    object: &serde_json::Map<String, Value>,
) -> bool {
    let Some(message_key) = object.get("message_key").and_then(Value::as_str) else {
        return false;
    };
    if !matches!(
        message_key,
        "clawd.msg.config_edit.guard" | "clawd.msg.config_risk.summary"
    ) {
        return false;
    }
    object.contains_key("path")
        && (object.contains_key("risk_count")
            || object.contains_key("count")
            || object.contains_key("risks")
            || object.contains_key("candidates"))
}

pub(super) fn should_restore_config_guard_payload(
    agent_run_context: Option<&AgentRunContext>,
    requested_summary: &str,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if route.delivery_required
        || !route.semantic_kind_is_any(&[
            crate::OutputSemanticKind::ContentExcerptSummary,
            crate::OutputSemanticKind::ConfigValidation,
        ])
    {
        return false;
    }
    if config_guard_payload_from_text(requested_summary).is_some() {
        return false;
    }
    super::machine_kv_units(requested_summary).len() <= 2
}

pub(super) fn latest_config_guard_machine_payload(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> Option<String> {
    delivery_messages
        .iter()
        .rev()
        .chain(loop_state.delivery_messages.iter().rev())
        .chain(loop_state.last_user_visible_respond.iter())
        .chain(loop_state.last_publishable_synthesis_output.iter())
        .find_map(|message| config_guard_payload_from_text(message))
        .or_else(|| {
            loop_state
                .executed_step_results
                .iter()
                .rev()
                .filter(|step| step.is_ok())
                .filter_map(|step| step.output.as_deref())
                .find_map(config_guard_payload_from_text)
        })
}

fn config_guard_payload_from_text(text: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(text.trim()).ok()?;
    config_guard_payload_from_json(&value)
}

fn config_guard_payload_from_json(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    if structured_config_guard_json_should_remain_full(object) {
        return Some(Value::Object(object.clone()).to_string());
    }
    for key in ["text", "output"] {
        if let Some(nested) = object
            .get(key)
            .and_then(Value::as_str)
            .and_then(config_guard_payload_from_text)
        {
            return Some(nested);
        }
    }
    None
}

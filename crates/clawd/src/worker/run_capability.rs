use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::{agent_engine, AppState, ClaimedTask};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DirectCapabilityRequest {
    pub(super) capability: String,
    pub(super) args: Value,
}

pub(crate) fn is_direct_capability_payload(payload: &Value) -> bool {
    payload.get("entrypoint").and_then(Value::as_str) == Some("run_capability")
}

pub(crate) fn parse_direct_capability_request(payload: &Value) -> Result<DirectCapabilityRequest> {
    if !is_direct_capability_payload(payload) {
        anyhow::bail!("run_capability_entrypoint_invalid");
    }
    let capability = payload
        .get("capability")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| valid_capability_ref(value))
        .context("run_capability_ref_invalid")?;
    let args = payload.get("args").cloned().unwrap_or_else(|| json!({}));
    if !args.is_object() {
        anyhow::bail!("run_capability_args_invalid");
    }
    Ok(DirectCapabilityRequest {
        capability: capability.to_string(),
        args,
    })
}

pub(super) async fn process_run_capability_task(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
) -> Result<()> {
    let request = parse_direct_capability_request(payload)?;
    let request_envelope = json!({
        "request_kind": "direct_capability",
        "capability": request.capability,
        "args_keys": request
            .args
            .as_object()
            .map(|args| args.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
    })
    .to_string();
    let plan =
        agent_engine::direct_capability_plan(state, &request.capability, request.args.clone());
    let mut agent_context = agent_engine::AgentRunContext::default();
    agent_context.original_user_request = Some(request_envelope.clone());
    agent_context.user_request = Some(request_envelope.clone());
    agent_context.context_bundle_summary = Some("source=direct_capability".to_string());

    crate::log_ask_transition(
        state,
        &task.task_id,
        None,
        crate::AskState::Executing,
        "direct_capability_agent_loop_entry",
        None,
    );
    let result = agent_engine::run_agent_with_tools_direct_plan(
        state,
        task,
        &request_envelope,
        Some(agent_context),
        &plan,
    )
    .await;

    crate::finalize::finalize_ask_result(
        state,
        task,
        payload,
        &request_envelope,
        "source=direct_capability",
        None,
        &request_envelope,
        None,
        &[],
        None,
        result,
    )
    .await
}

fn valid_capability_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.')
        })
}

#[cfg(test)]
#[path = "run_capability_tests.rs"]
mod tests;

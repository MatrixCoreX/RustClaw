use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use claw_core::model_turn::{ModelTurnEvent, ModelTurnRequest, ModelTurnResponse};
use serde_json::json;
use tracing::{info, warn};

use super::{
    llm_cost_policy_allows, record_llm_cost, start_llm_task_lease_heartbeat,
    stop_llm_task_lease_heartbeat, touch_llm_task_lease, NO_ELIGIBLE_LLM_PROVIDER_ERR,
    TASK_LLM_COST_POLICY_BLOCKED_ERR,
};
use crate::providers::client::ProviderErrorKind;
use crate::runtime::TaskProviderBlocker;
use crate::{AppState, ClaimedTask};

pub(crate) async fn run_native_model_turn_with_fallback(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    prompt_source: &str,
    request: &ModelTurnRequest,
) -> Result<Option<ModelTurnResponse>, String> {
    let task_providers = state.task_llm_providers(task);
    if !task_providers
        .iter()
        .any(|provider| provider.model_capabilities().native_tools)
    {
        return Ok(None);
    }
    state.clear_task_provider_blocker(&task.task_id);
    state.clear_task_cost_blocker(&task.task_id);
    state.restore_task_llm_call_count_from_cost_ledger(&task.task_id);
    if !llm_cost_policy_allows(state, task, None, prompt_source) {
        return Err(TASK_LLM_COST_POLICY_BLOCKED_ERR.to_string());
    }
    if let Some(reason) = state.task_llm_budget_exceeded(&task.task_id) {
        return Err(reason);
    }

    let prompt_label = super::classify_prompt_source(prompt_source);
    state.note_task_llm_call_with_label_and_prompt_size(&task.task_id, prompt_label, prompt.len());
    let logical_call_index = state.task_llm_call_count(&task.task_id);
    let hints = crate::ChatRequestHints {
        requires_native_tools: true,
        timeout_seconds: request
            .metadata
            .get("provider_timeout_seconds")
            .and_then(serde_json::Value::as_u64),
        ..crate::ChatRequestHints::default()
    };
    let routing_plan = crate::providers::route_providers(task_providers, prompt, &hints);
    state.note_task_provider_routing_plan_with_label(
        &task.task_id,
        prompt_label,
        routing_plan.evaluations,
    );
    if routing_plan.providers.is_empty() {
        return Err(NO_ELIGIBLE_LLM_PROVIDER_ERR.to_string());
    }

    touch_llm_task_lease(
        state,
        &task.task_id,
        task.claim_attempt,
        prompt_label,
        "native_turn_start",
    );
    let mut heartbeat_stop = Some(start_llm_task_lease_heartbeat(
        state.clone(),
        task.task_id.clone(),
        task.claim_attempt,
        prompt_label,
    ));
    let call_started_at = std::time::Instant::now();
    let mut last_error = NO_ELIGIBLE_LLM_PROVIDER_ERR.to_string();
    let mut recoverable_blocker = None;
    let model_event_index = Arc::new(AtomicU64::new(0));

    for provider in routing_plan.providers {
        let provider_name = format!("{}:{}", provider.config.name, provider.config.model);
        match provider.breaker.before_attempt() {
            crate::providers::AttemptDecision::SkipCooldown { .. } => continue,
            crate::providers::AttemptDecision::Allow
            | crate::providers::AttemptDecision::AllowTrial => {}
        }
        if !llm_cost_policy_allows(
            state,
            task,
            Some(provider.config.name.as_str()),
            prompt_source,
        ) {
            stop_llm_task_lease_heartbeat(&mut heartbeat_stop);
            return Err(TASK_LLM_COST_POLICY_BLOCKED_ERR.to_string());
        }
        let provider_started_at = std::time::Instant::now();
        info!(
            "{} [MODEL_TURN] stage=request task_id={} provider={} prompt_source={} native_tools=true",
            crate::highlight_tag("llm"),
            task.task_id,
            provider_name,
            prompt_source
        );
        let event_state = state.clone();
        let event_task = task.clone();
        let event_provider = provider_name.clone();
        let event_index = model_event_index.clone();
        let event_sink: crate::providers::client::ModelTurnEventSink =
            Arc::new(move |event: ModelTurnEvent| {
                let index = event_index.fetch_add(1, Ordering::Relaxed) + 1;
                publish_model_turn_event(&event_state, &event_task, &event_provider, index, &event);
            });
        match crate::providers::call_model_turn_with_retry(
            provider.clone(),
            request,
            &hints,
            Some(event_sink),
        )
        .await
        {
            Ok(mut output) => {
                provider
                    .latency
                    .note_sample(provider_started_at.elapsed().as_millis() as u64);
                provider.breaker.note_success();
                state.note_task_provider_attempts_with_label(
                    &task.task_id,
                    prompt_label,
                    output.attempts,
                    output.retryable_error_count,
                    output.last_retry_error_kind,
                    None,
                );
                let vendor = crate::llm_vendor_name(&provider);
                let (cleaned_text, sanitized) =
                    crate::maybe_sanitize_llm_text_output(vendor, &output.turn.text);
                output.turn.text = cleaned_text;
                let clean_response = model_turn_log_response(&output.turn);
                crate::append_model_io_log(
                    state,
                    task,
                    &provider,
                    logical_call_index,
                    "ok",
                    prompt_source,
                    prompt,
                    &output.request_payload,
                    Some(&output.raw_response),
                    Some(&clean_response),
                    output.turn.usage.as_ref(),
                    sanitized,
                    None,
                );
                record_llm_cost(
                    state,
                    task,
                    crate::providers::build_cost_record(
                        logical_call_index,
                        prompt_label,
                        &provider.config.name,
                        &provider.config.model,
                        "ok",
                        output.attempts,
                        output.turn.usage.as_ref(),
                        provider.pricing.as_ref(),
                    ),
                    prompt_source,
                );
                state.note_task_llm_elapsed_with_label(
                    &task.task_id,
                    prompt_label,
                    call_started_at.elapsed().as_millis() as u64,
                );
                state.clear_task_provider_blocker(&task.task_id);
                stop_llm_task_lease_heartbeat(&mut heartbeat_stop);
                touch_llm_task_lease(
                    state,
                    &task.task_id,
                    task.claim_attempt,
                    prompt_label,
                    "native_turn_success",
                );
                return Ok(Some(output.turn));
            }
            Err(err) => {
                provider
                    .latency
                    .note_sample(provider_started_at.elapsed().as_millis() as u64);
                let error_kind = err.observability_kind();
                if err.should_trip_breaker() {
                    provider.breaker.note_failure();
                } else if err.should_reset_breaker() {
                    provider.breaker.note_success();
                }
                if let Some(retry_after_seconds) = err.background_wait_seconds() {
                    recoverable_blocker = Some(TaskProviderBlocker {
                        provider: provider.config.name.clone(),
                        status_code: error_kind.to_string(),
                        retry_after_seconds,
                        external_provider_blocked: true,
                        message_key: provider_message_key(err.kind).to_string(),
                    });
                }
                state.note_task_provider_attempts_with_label(
                    &task.task_id,
                    prompt_label,
                    err.attempts,
                    err.retryable_error_count,
                    None,
                    Some(error_kind),
                );
                crate::append_model_io_log(
                    state,
                    task,
                    &provider,
                    logical_call_index,
                    "failed",
                    prompt_source,
                    prompt,
                    &err.request_payload,
                    err.raw_response.as_deref(),
                    None,
                    err.usage.as_ref(),
                    false,
                    Some(&err.message),
                );
                record_llm_cost(
                    state,
                    task,
                    crate::providers::build_cost_record(
                        logical_call_index,
                        prompt_label,
                        &provider.config.name,
                        &provider.config.model,
                        "failed",
                        err.attempts,
                        err.usage.as_ref(),
                        provider.pricing.as_ref(),
                    ),
                    prompt_source,
                );
                last_error =
                    format!("provider={provider_name} error_kind={error_kind} failed: {err}");
                warn!("{last_error}");
            }
        }
    }

    if let Some(blocker) = recoverable_blocker {
        state.note_task_provider_blocker(&task.task_id, blocker);
    }
    state.note_task_llm_elapsed_with_label(
        &task.task_id,
        prompt_label,
        call_started_at.elapsed().as_millis() as u64,
    );
    stop_llm_task_lease_heartbeat(&mut heartbeat_stop);
    Err(last_error)
}

fn model_turn_log_response(turn: &ModelTurnResponse) -> String {
    if turn.tool_calls.is_empty() {
        return turn.text.clone();
    }
    json!({
        "text": turn.text,
        "tool_calls": turn.tool_calls,
        "finish_reason": turn.finish_reason,
    })
    .to_string()
}

fn provider_message_key(kind: ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::QuotaExhausted => "provider.quota_exhausted",
        ProviderErrorKind::RateLimited => "provider.rate_limited",
        _ => "provider.temporarily_unavailable",
    }
}

fn publish_model_turn_event(
    state: &AppState,
    task: &ClaimedTask,
    provider: &str,
    model_event_index: u64,
    event: &ModelTurnEvent,
) {
    let payload = model_turn_event_payload(provider, model_event_index, event);
    if let Err(error) =
        crate::task_event_transport::publish_claimed_event(state, task, "model_turn", payload)
    {
        warn!(
            "model_turn_event_publish_failed task_id={} error={}",
            task.task_id,
            crate::truncate_for_log(&error.to_string())
        );
    }
}

fn model_turn_event_payload(
    provider: &str,
    model_event_index: u64,
    event: &ModelTurnEvent,
) -> serde_json::Value {
    match event {
        ModelTurnEvent::Started { attempt } => {
            json!({"model_event_index": model_event_index, "provider": provider, "type": "started", "attempt": attempt})
        }
        ModelTurnEvent::TextDelta { text } => json!({
            "model_event_index": model_event_index,
            "provider": provider,
            "type": "text_delta",
            "text_delta_bytes": text.len()
        }),
        ModelTurnEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments_delta,
        } => json!({
            "model_event_index": model_event_index,
            "provider": provider,
            "type": "tool_call_delta",
            "tool_index": index,
            "tool_call_id": id,
            "tool_name": name,
            "arguments_delta_bytes": arguments_delta.len()
        }),
        ModelTurnEvent::ToolCall { call } => json!({
            "model_event_index": model_event_index,
            "provider": provider,
            "type": "tool_call",
            "tool_call_id": call.id,
            "tool_name": call.name
        }),
        ModelTurnEvent::Usage { usage } => json!({
            "model_event_index": model_event_index,
            "provider": provider,
            "type": "usage",
            "usage": usage
        }),
        ModelTurnEvent::Finished { reason } => json!({
            "model_event_index": model_event_index,
            "provider": provider,
            "type": "finished",
            "finish_reason": reason
        }),
        ModelTurnEvent::Interrupted { code, retryable } => json!({
            "model_event_index": model_event_index,
            "provider": provider,
            "type": "interrupted",
            "code": code,
            "retryable": retryable
        }),
    }
}

#[cfg(test)]
#[path = "llm_gateway_model_turn_tests.rs"]
mod tests;

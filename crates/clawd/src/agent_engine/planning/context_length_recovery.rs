use claw_core::model_turn::{ModelContentPart, ModelRole, ModelTurnRequest};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{AppState, ClaimedTask};

pub(super) fn context_length_recovery_request(
    state: &AppState,
    task: &ClaimedTask,
    request: &ModelTurnRequest,
    original_prompt: &str,
) -> Option<(String, ModelTurnRequest, Value)> {
    let policy = crate::task_context_builder::ContextWindowPolicy::for_task(state, task)?;
    Some(build_context_length_recovery_request(
        &policy,
        request,
        original_prompt,
    ))
}

fn build_context_length_recovery_request(
    policy: &crate::task_context_builder::ContextWindowPolicy,
    request: &ModelTurnRequest,
    original_prompt: &str,
) -> (String, ModelTurnRequest, Value) {
    let request_json = serde_json::to_string(request).unwrap_or_default();
    let original_tokens =
        crate::token_estimator::estimate_generic_tokens(&request_json).provider_tokens;
    let reserved_tokens = policy
        .output_reserve_tokens
        .saturating_add(policy.tool_observation_reserve_tokens)
        .saturating_add(policy.estimator_safety_margin_tokens);
    let input_budget_tokens = policy.context_window_tokens.saturating_sub(reserved_tokens);
    // A structured provider rejection means the estimate was optimistic. Keep
    // one complete reserve block of recovery headroom instead of another fixed
    // percentage trigger.
    let recovery_headroom = reserved_tokens.max(1_024);
    let target_tokens = input_budget_tokens
        .min(original_tokens)
        .saturating_sub(recovery_headroom)
        .max(1_024);
    let tools_tokens = crate::token_estimator::estimate_generic_tokens(
        &serde_json::to_string(&request.tools).unwrap_or_default(),
    )
    .provider_tokens;
    let system_tokens = request
        .messages
        .iter()
        .filter(|message| message.role == ModelRole::System)
        .map(|message| {
            crate::token_estimator::estimate_generic_tokens(
                &serde_json::to_string(message).unwrap_or_default(),
            )
            .provider_tokens
        })
        .sum::<usize>();
    let dynamic_budget = target_tokens
        .saturating_sub(tools_tokens)
        .saturating_sub(system_tokens)
        .max(512);
    let mut recovery_request = request.clone();
    let dynamic_message_count = recovery_request
        .messages
        .iter()
        .filter(|message| message.role != ModelRole::System)
        .count()
        .max(1);
    let per_message_budget = dynamic_budget
        .saturating_div(dynamic_message_count)
        .max(256);
    for message in &mut recovery_request.messages {
        if message.role == ModelRole::System {
            continue;
        }
        for part in &mut message.content {
            if let ModelContentPart::Text { text } = part {
                *text = compact_text_to_token_budget(text, per_message_budget);
            }
        }
    }
    recovery_request.metadata.insert(
        "context_length_recovery_attempt".to_string(),
        Value::Number(1_u64.into()),
    );
    let recovery_prompt = recovery_request
        .messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| match part {
            ModelContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let recovery_tokens = crate::token_estimator::estimate_generic_tokens(
        &serde_json::to_string(&recovery_request).unwrap_or_default(),
    )
    .provider_tokens;
    let observation = json!({
        "schema_version": 1,
        "observation_kind": "context_length_recovery_snapshot",
        "reason_code": "context_length_exceeded",
        "source": "provider_structured_error",
        "attempt": 1,
        "retry_limit": 1,
        "input_digest": format!("sha256:{:x}", Sha256::digest(original_prompt.as_bytes())),
        "output_digest": format!("sha256:{:x}", Sha256::digest(recovery_prompt.as_bytes())),
        "original_token_estimate": original_tokens,
        "recovery_token_estimate": recovery_tokens,
        "target_tokens": target_tokens,
        "reserved_tokens": reserved_tokens,
        "system_context_preserved": true,
        "tool_catalog_preserved": true,
        "completed_side_effect_replay": false,
        "canonical_task_events_preserved": true,
    });
    (recovery_prompt, recovery_request, observation)
}

pub(super) fn compact_text_to_token_budget(text: &str, token_budget: usize) -> String {
    let estimate = crate::token_estimator::estimate_generic_tokens(text).provider_tokens;
    if estimate <= token_budget {
        return text.to_string();
    }
    let target_chars = text
        .chars()
        .count()
        .saturating_mul(token_budget)
        .saturating_div(estimate.max(1))
        .max(256);
    let prefix_chars = target_chars.saturating_div(3);
    let suffix_chars = target_chars.saturating_sub(prefix_chars);
    let prefix = text.chars().take(prefix_chars).collect::<String>();
    let suffix = text
        .chars()
        .rev()
        .take(suffix_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!(
        "{prefix}\n<context_recovery_gap data_only=\"true\" canonical_events_preserved=\"true\" />\n{suffix}"
    )
}

#[cfg(test)]
#[path = "context_length_recovery_tests.rs"]
mod tests;

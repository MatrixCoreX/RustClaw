use claw_core::capability_result::{
    CapabilityDeliveryIntent, CapabilityResultEnvelope, CapabilityResultStatus,
};
use serde::Deserialize;
use serde_json::{json, Map as JsonMap, Value};

use super::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

const PROMPT_LOGICAL_PATH: &str = "prompts/capability_result_synthesis_prompt.md";
const MAX_RESULTS: usize = 8;
const MAX_RESULT_JSON_CHARS: usize = 64 * 1024;
const MAX_RESULT_PREVIEW_CHARS: usize = 24 * 1024;

#[derive(Debug, Deserialize)]
struct CapabilitySynthesisOutput {
    #[serde(default)]
    answer: String,
    #[serde(default)]
    qualified: bool,
    #[serde(default)]
    needs_clarify: bool,
    #[serde(default)]
    is_meta_instruction: bool,
    #[serde(default)]
    publishable: bool,
    #[serde(default)]
    confidence: f64,
    #[serde(default, rename = "reason")]
    _reason: String,
}

pub(super) struct CapabilitySynthesis {
    pub(super) answer: String,
    pub(super) confidence: f64,
    pub(super) evidence_count: usize,
}

pub(super) fn eligible_for_capability_result_synthesis(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    if loop_state.capability_results.is_empty()
        || loop_state.capability_results.iter().any(|result| {
            result.delivery.intent != CapabilityDeliveryIntent::ModelSynthesis
                || !matches!(
                    result.status,
                    CapabilityResultStatus::Ok | CapabilityResultStatus::Error
                )
                || result.continuation.is_some()
        })
    {
        return false;
    }
    agent_run_context
        .and_then(AgentRunContext::output_contract)
        .is_none_or(|contract| {
            !contract.delivery_required
                && matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
                )
        })
}

pub(super) async fn synthesize_from_capability_results(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<Option<CapabilitySynthesis>, String> {
    if !eligible_for_capability_result_synthesis(loop_state, agent_run_context) {
        return Ok(None);
    }
    let results = synthesis_result_projection(&loop_state.capability_results);
    let result_json = serde_json::to_string(&results)
        .map_err(|_| "capability_result_synthesis_input_serialize_failed".to_string())?;
    let constraints = delivery_constraints(agent_run_context);
    let constraints_json = constraints.to_string();
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request = crate::language_policy::task_original_user_text(task)
        .unwrap_or_else(|| user_text.trim().to_string());
    let (template, source) =
        crate::bootstrap::load_required_prompt_template_for_state(state, PROMPT_LOGICAL_PATH)
            .map_err(|_| "capability_result_synthesis_prompt_unavailable".to_string())?;
    let prompt = crate::render_prompt_template(
        &template,
        &[
            ("__USER_REQUEST__", &user_request),
            ("__DELIVERY_CONSTRAINTS__", &constraints_json),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            ("__CAPABILITY_RESULTS__", &result_json),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "capability_result_synthesis_prompt",
        &source,
        None,
    );
    let raw =
        crate::llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &source)
            .await
            .map_err(|_| "capability_result_synthesis_provider_unavailable".to_string())?;
    let parsed = crate::prompt_utils::validate_against_schema::<CapabilitySynthesisOutput>(
        raw.trim(),
        crate::prompt_utils::PromptSchemaId::FinalizerOut,
    )
    .map_err(|_| "capability_result_synthesis_schema_invalid".to_string())?
    .value;
    let answer = parsed.answer.trim().to_string();
    if answer.is_empty()
        || parsed.needs_clarify
        || parsed.is_meta_instruction
        || !parsed.qualified
        || !parsed.publishable
    {
        return Ok(None);
    }
    Ok(Some(CapabilitySynthesis {
        answer,
        confidence: parsed.confidence.clamp(0.0, 1.0),
        evidence_count: results.iter().map(|result| result.evidence.len()).sum(),
    }))
}

fn delivery_constraints(agent_run_context: Option<&AgentRunContext>) -> Value {
    let Some(contract) = agent_run_context.and_then(AgentRunContext::output_contract) else {
        return json!({
            "response_shape": "free",
            "delivery_required": false,
        });
    };
    json!({
        "response_shape": contract.response_shape.as_str(),
        "exact_sentence_count": contract.exact_sentence_count,
        "delivery_required": contract.delivery_required,
        "requires_content_evidence": contract.requires_content_evidence,
        "locator_kind": contract.locator_kind.as_str(),
        "selection": {
            "limit": contract.selection.list_selector.limit,
            "sort_by": contract.selection.list_selector.sort_by,
            "include_metadata": contract.selection.list_selector.include_metadata,
            "include_hidden": contract.selection.list_selector.include_hidden,
            "structured_field_selector": contract.selection.structured_field_selector,
        },
    })
}

fn synthesis_result_projection(
    results: &[CapabilityResultEnvelope],
) -> Vec<CapabilityResultEnvelope> {
    let start = results.len().saturating_sub(MAX_RESULTS);
    results[start..].iter().map(bounded_result).collect()
}

fn bounded_result(result: &CapabilityResultEnvelope) -> CapabilityResultEnvelope {
    let mut result = result.clone();
    result.data = bounded_json(&result.data, 0);
    for evidence in &mut result.evidence {
        evidence.metadata = bounded_json(&evidence.metadata, 0);
    }
    for artifact in &mut result.artifacts {
        artifact.metadata = bounded_json(&artifact.metadata, 0);
    }
    if let Some(error) = result.error.as_mut() {
        error.details = bounded_json(&error.details, 0);
    }
    if let Some(continuation) = result.continuation.as_mut() {
        if continuation.reference.is_some() {
            continuation.reference = Some("opaque_continuation".to_string());
        }
        continuation.state = bounded_json(&continuation.state, 0);
    }
    let serialized = serde_json::to_string(&result).unwrap_or_default();
    if serialized.chars().count() <= MAX_RESULT_JSON_CHARS {
        return result;
    }
    result.data = json!({
        "truncated": true,
        "original_chars": serialized.chars().count(),
        "preview": serialized.chars().take(MAX_RESULT_PREVIEW_CHARS).collect::<String>(),
    });
    result
}

fn bounded_json(value: &Value, depth: usize) -> Value {
    if depth >= 6 {
        return json!({"truncated": true, "reason": "depth_limit"});
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .take(48)
                .map(|(key, value)| (key.clone(), bounded_json(value, depth + 1)))
                .collect::<JsonMap<_, _>>(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(64)
                .map(|value| bounded_json(value, depth + 1))
                .collect(),
        ),
        Value::String(text) => Value::String(text.chars().take(8_000).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
#[path = "capability_result_synthesis_tests.rs"]
mod tests;

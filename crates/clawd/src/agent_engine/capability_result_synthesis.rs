use claw_core::capability_result::{
    CapabilityDeliveryIntent, CapabilityResultEnvelope, CapabilityResultStatus,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

const PROMPT_LOGICAL_PATH: &str = "prompts/capability_result_synthesis_prompt.md";
#[cfg(test)]
const MAX_RESULT_JSON_CHARS: usize = 64 * 1024;
#[cfg(test)]
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
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<Option<CapabilitySynthesis>, String> {
    if !eligible_for_capability_result_synthesis(loop_state, agent_run_context) {
        return Ok(None);
    }
    let results = synthesis_evidence_catalog(state, task, &loop_state.capability_results)?;
    loop_state.task_observations.push(json!({
        "schema_version": 1,
        "owner_layer": "canonical_evidence_store",
        "catalog": results.clone(),
    }));
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
        evidence_count: results["entries"]
            .as_array()
            .map_or(0, |entries| entries.len()),
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

fn synthesis_evidence_catalog(
    state: &AppState,
    task: &ClaimedTask,
    results: &[CapabilityResultEnvelope],
) -> Result<Value, String> {
    let model_budget_tokens = synthesis_model_view_budget_tokens(state);
    let per_result_tokens = model_budget_tokens
        .checked_div(results.len().max(1))
        .unwrap_or(model_budget_tokens)
        .max(1);
    let mut entries = Vec::with_capacity(results.len());
    let mut complete_model_view = true;
    for (index, result) in results.iter().enumerate() {
        let identity = result.canonical_evidence_identity();
        let serialized = serde_json::to_vec(result)
            .map_err(|_| "capability_result_synthesis_input_serialize_failed".to_string())?;
        let model_value = serde_json::to_value(result)
            .map_err(|_| "capability_result_synthesis_input_serialize_failed".to_string())?;
        let (model_value, model_view_redacted) =
            crate::skill_output_artifact::sensitivity_aware_json_model_view(&model_value);
        let token_estimate = crate::token_estimator::estimate_generic_tokens(
            std::str::from_utf8(&serialized).unwrap_or_default(),
        )
        .provider_tokens;
        let model_view = if token_estimate <= per_result_tokens {
            json!({
                "complete": true,
                "projection": "canonical_inline",
                "result": model_value,
                "sensitivity": if model_view_redacted { "restricted_redacted" } else { "task_owner" },
            })
        } else {
            complete_model_view = false;
            let published = crate::skill_output_artifact::publish_canonical_evidence_artifact(
                &state.skill_rt.workspace_root,
                &task.task_id,
                &identity.evidence_id,
                &serialized,
            )
            .map_err(|_| "canonical_evidence_artifact_write_failed".to_string())?;
            provider_fitted_scalar_page(&model_value, per_result_tokens, published.range_handle)
        };
        entries.push(json!({
            "ordinal": index + 1,
            "evidence_id": identity.evidence_id,
            "sha256": identity.sha256,
            "size_bytes": identity.size_bytes,
            "capability": result.capability,
            "action": result.action,
            "status": result.status,
            "canonical_completeness": result.completeness,
            "model_view_redacted": model_view_redacted,
            "model_view": model_view,
        }));
    }
    Ok(json!({
        "schema_version": 1,
        "catalog_kind": "canonical_capability_evidence",
        "canonical_complete": true,
        "model_view_complete": complete_model_view,
        "result_count": entries.len(),
        "provider_model_view_budget_tokens": model_budget_tokens,
        "entries": entries,
    }))
}

fn synthesis_model_view_budget_tokens(state: &AppState) -> usize {
    state
        .core
        .llm_providers
        .iter()
        .map(|provider| provider.model_descriptor())
        .filter_map(|descriptor| {
            descriptor.context_window_tokens.map(|window| {
                window
                    .saturating_sub(descriptor.output_reserve_tokens)
                    .saturating_mul(40)
                    .saturating_div(100)
            })
        })
        .min()
        .unwrap_or(32_768)
        .max(1)
}

fn provider_fitted_scalar_page(result: &Value, token_budget: usize, range_handle: Value) -> Value {
    let mut candidates = Vec::new();
    let data = result.get("data").unwrap_or(result);
    collect_scalar_candidates("", data, &mut candidates);
    let mut facts = Vec::new();
    let mut used_tokens = 0usize;
    for fact in candidates {
        let serialized = fact.to_string();
        let tokens = crate::token_estimator::estimate_generic_tokens(&serialized).provider_tokens;
        if used_tokens.saturating_add(tokens) > token_budget {
            break;
        }
        used_tokens = used_tokens.saturating_add(tokens);
        facts.push(fact);
    }
    json!({
        "complete": false,
        "projection": "provider_fitted_scalar_page",
        "returned_fact_count": facts.len(),
        "known_fact_count": candidates_len(data),
        "facts": facts,
        "partial_reason": "provider_context_window",
        "continuation": {
            "kind": "artifact_range",
            "range_handle": range_handle,
        },
    })
}

fn candidates_len(value: &Value) -> usize {
    let mut candidates = Vec::new();
    collect_scalar_candidates("", value, &mut candidates);
    candidates.len()
}

fn collect_scalar_candidates(path: &str, value: &Value, out: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                collect_scalar_candidates(&child_path, child, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                collect_scalar_candidates(&format!("{path}.{index}"), child, out);
            }
        }
        Value::String(text) => {
            let tokens = crate::token_estimator::estimate_generic_tokens(text).provider_tokens;
            if tokens <= 512 {
                out.push(json!({"path": path, "value": text}));
            } else {
                out.push(json!({
                    "path": path,
                    "value_kind": "large_string",
                    "char_count": text.chars().count(),
                    "sha256": format!("{:x}", Sha256::digest(text.as_bytes())),
                    "recovery": "artifact_range",
                }));
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {
            out.push(json!({"path": path, "value": value}));
        }
    }
}

#[cfg(test)]
fn bounded_result(result: &CapabilityResultEnvelope) -> CapabilityResultEnvelope {
    let mut result = result.clone();
    result.data = crate::capability_result::explicit_model_observation(&result.data)
        .map(|observation| {
            json!({
                "model_observation": bounded_json(observation, 0, 12),
            })
        })
        .unwrap_or_else(|| bounded_json(&result.data, 0, 6));
    for evidence in &mut result.evidence {
        evidence.metadata = bounded_json(&evidence.metadata, 0, 6);
    }
    for artifact in &mut result.artifacts {
        artifact.metadata = bounded_json(&artifact.metadata, 0, 6);
    }
    if let Some(error) = result.error.as_mut() {
        error.details = bounded_json(&error.details, 0, 6);
    }
    if let Some(continuation) = result.continuation.as_mut() {
        if continuation.reference.is_some() {
            continuation.reference = Some("opaque_continuation".to_string());
        }
        continuation.state = bounded_json(&continuation.state, 0, 6);
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

#[cfg(test)]
fn bounded_json(value: &Value, depth: usize, max_depth: usize) -> Value {
    use serde_json::Map as JsonMap;
    if depth >= max_depth {
        return json!({"truncated": true, "reason": "depth_limit"});
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .take(48)
                .map(|(key, value)| (key.clone(), bounded_json(value, depth + 1, max_depth)))
                .collect::<JsonMap<_, _>>(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(64)
                .map(|value| bounded_json(value, depth + 1, max_depth))
                .collect(),
        ),
        Value::String(text) => Value::String(text.chars().take(8_000).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
#[path = "capability_result_synthesis_tests.rs"]
mod tests;

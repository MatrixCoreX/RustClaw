use claw_core::capability_result::{
    ArtifactRef, ArtifactVisibility, CapabilityDeliveryIntent, CapabilityResultEnvelope,
    CapabilityResultStatus,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

const PROMPT_LOGICAL_PATH: &str = "prompts/capability_result_synthesis_prompt.md";
const TRANSCRIPT_REVISION_PROMPT_LOGICAL_PATH: &str = "prompts/transcript_revision_prompt.md";
const FALLBACK_TRANSCRIPT_REVISION_CHUNK_CHARS: usize = 4_000;
const DEFAULT_TRANSCRIPT_INLINE_MAX_CHARS: usize = 200;
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

#[derive(Debug, Deserialize)]
struct TranscriptRevisionOutput {
    #[serde(default)]
    reviewed_text: String,
    #[serde(default)]
    delivery_message: String,
    #[serde(default)]
    content_kind: TranscriptContentKind,
    #[serde(default)]
    qualified: bool,
    #[serde(default)]
    confidence: f64,
    #[serde(default, rename = "reason")]
    _reason: String,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TranscriptContentKind {
    Speech,
    NonSpeech,
    #[default]
    Unusable,
}

#[derive(Debug, Clone)]
struct TranscriptReviewContract {
    result_index: usize,
    raw_text: String,
    response_language: String,
    source: String,
    inline_max_chars: usize,
    long_text_filename: String,
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
    if !terminal_model_synthesis_results(&loop_state.capability_results) {
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

pub(super) fn pending_transcript_review(results: &[CapabilityResultEnvelope]) -> bool {
    transcript_review_contract(results).is_some()
}

fn terminal_model_synthesis_results(results: &[CapabilityResultEnvelope]) -> bool {
    !results.is_empty()
        && results.iter().all(|result| {
            result.delivery.intent == CapabilityDeliveryIntent::ModelSynthesis
                && matches!(
                    result.status,
                    CapabilityResultStatus::Ok | CapabilityResultStatus::Error
                )
                && result.continuation.is_none()
        })
}

pub(super) async fn synthesize_from_capability_results(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<Option<CapabilitySynthesis>, String> {
    let transcript_contract = transcript_review_contract(&loop_state.capability_results);
    if transcript_contract.is_none()
        && !eligible_for_capability_result_synthesis(loop_state, agent_run_context)
    {
        return Ok(None);
    }
    let results = synthesis_evidence_catalog(state, task, &loop_state.capability_results)?;
    loop_state.task_observations.push(json!({
        "schema_version": 1,
        "owner_layer": "canonical_evidence_store",
        "catalog": results.clone(),
    }));
    if let Some(contract) = transcript_contract {
        return synthesize_reviewed_transcript(
            state,
            task,
            user_text,
            loop_state,
            contract,
            results["entries"]
                .as_array()
                .map_or(0, |entries| entries.len()),
        )
        .await
        .map(Some);
    }
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

fn transcript_review_contract(
    results: &[CapabilityResultEnvelope],
) -> Option<TranscriptReviewContract> {
    results
        .iter()
        .enumerate()
        .rev()
        .find_map(|(result_index, result)| {
            if result.status != CapabilityResultStatus::Ok
                || result.delivery.intent != CapabilityDeliveryIntent::ModelSynthesis
                || result.continuation.is_some()
            {
                return None;
            }
            let contract = result
                .data
                .get("extra")
                .and_then(|extra| extra.get("transcription_review"))
                .and_then(Value::as_object)?;
            if contract.get("required").and_then(Value::as_bool) != Some(true) {
                return None;
            }
            let raw_text = contract
                .get("raw_text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())?
                .to_string();
            let delivery = contract.get("delivery").and_then(Value::as_object);
            let inline_max_chars = delivery
                .and_then(|delivery| delivery.get("inline_max_characters_exclusive"))
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| (1..=10_000).contains(value))
                .unwrap_or(DEFAULT_TRANSCRIPT_INLINE_MAX_CHARS);
            let long_text_filename = delivery
                .and_then(|delivery| delivery.get("long_text_filename"))
                .and_then(Value::as_str)
                .map(safe_transcript_filename)
                .filter(|filename| !filename.is_empty())
                .unwrap_or_else(|| "transcript.txt".to_string());
            Some(TranscriptReviewContract {
                result_index,
                raw_text,
                response_language: contract
                    .get("response_language")
                    .and_then(Value::as_str)
                    .unwrap_or("request-language")
                    .to_string(),
                source: contract
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("speech_to_text")
                    .to_string(),
                inline_max_chars,
                long_text_filename,
            })
        })
}

async fn synthesize_reviewed_transcript(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    contract: TranscriptReviewContract,
    evidence_count: usize,
) -> Result<CapabilitySynthesis, String> {
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let target_language =
        normalized_transcript_language(&contract.response_language, &request_language_hint);
    let chunks =
        split_transcript_chunks(&contract.raw_text, transcript_revision_chunk_chars(state));
    if chunks.is_empty() {
        return Err("transcript_revision_input_empty".to_string());
    }
    let (template, source) = crate::bootstrap::load_required_prompt_template_for_state(
        state,
        TRANSCRIPT_REVISION_PROMPT_LOGICAL_PATH,
    )
    .map_err(|_| "transcript_revision_prompt_unavailable".to_string())?;
    let mut reviewed_chunks = Vec::with_capacity(chunks.len());
    let mut delivery_message = String::new();
    let mut confidence = 1.0_f64;
    for (index, chunk) in chunks.iter().enumerate() {
        let chunk_index = (index + 1).to_string();
        let chunk_count = chunks.len().to_string();
        let prompt = crate::render_prompt_template(
            &template,
            &[
                ("__TARGET_LANGUAGE__", &target_language),
                ("__CHUNK_INDEX__", &chunk_index),
                ("__CHUNK_COUNT__", &chunk_count),
                ("__RAW_TRANSCRIPT__", chunk),
            ],
        );
        crate::log_prompt_render(
            state,
            &task.task_id,
            "transcript_revision_prompt",
            &source,
            None,
        );
        let raw =
            crate::llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &source)
                .await
                .map_err(|_| "transcript_revision_provider_unavailable".to_string())?;
        let parsed = crate::prompt_utils::validate_against_schema::<TranscriptRevisionOutput>(
            raw.trim(),
            crate::prompt_utils::PromptSchemaId::TranscriptRevision,
        )
        .map_err(|_| "transcript_revision_schema_invalid".to_string())?
        .value;
        let reviewed = parsed.reviewed_text.trim();
        let valid_non_speech = parsed.content_kind == TranscriptContentKind::NonSpeech;
        if (!parsed.qualified && !valid_non_speech)
            || parsed.content_kind == TranscriptContentKind::Unusable
            || reviewed.is_empty()
        {
            return Err("transcript_revision_unqualified".to_string());
        }
        if delivery_message.is_empty() {
            delivery_message = parsed.delivery_message.trim().to_string();
        }
        confidence = confidence.min(parsed.confidence.clamp(0.0, 1.0));
        reviewed_chunks.push(reviewed.to_string());
    }
    let reviewed_text = reviewed_chunks.join("\n\n").trim().to_string();
    if reviewed_text.is_empty() {
        return Err("transcript_revision_empty".to_string());
    }
    let character_count = reviewed_text.chars().count();
    let inline_delivery = transcript_delivery_is_inline(character_count, contract.inline_max_chars);
    let answer = if inline_delivery {
        reviewed_text.clone()
    } else {
        if delivery_message.is_empty() {
            return Err("transcript_revision_delivery_message_empty".to_string());
        }
        let published = crate::skill_output_artifact::publish_task_text_artifact(
            &state.skill_rt.workspace_root,
            &task.task_id,
            "transcript-review",
            &contract.long_text_filename,
            &(reviewed_text.clone() + "\n"),
            json!({
                "artifact_role": "transcript_text",
                "reviewed_by_model": true,
                "target_language": target_language,
                "source": contract.source,
                "character_count": character_count,
            }),
        )
        .map_err(|_| "transcript_revision_artifact_write_failed".to_string())?;
        let artifact = serde_json::from_value::<ArtifactRef>(published.artifact_ref)
            .map_err(|_| "transcript_revision_artifact_invalid".to_string())?;
        let result = loop_state
            .capability_results
            .get_mut(contract.result_index)
            .ok_or_else(|| "transcript_revision_result_missing".to_string())?;
        attach_reviewed_transcript_artifact(
            result,
            artifact,
            &contract.long_text_filename,
            &delivery_message,
        )?
    };
    if let Some(extra) = loop_state
        .capability_results
        .get_mut(contract.result_index)
        .and_then(|result| result.data.get_mut("extra"))
        .and_then(Value::as_object_mut)
    {
        extra.insert(
            "transcription_delivery".to_string(),
            json!({
                "mode": if inline_delivery { "inline" } else { "artifact" },
                "character_count": character_count,
                "reviewed_by_model": true,
                "target_language": target_language,
                "source": contract.source,
            }),
        );
        if let Some(transcription) = extra
            .get_mut("transcription")
            .and_then(Value::as_object_mut)
        {
            transcription.insert("reviewed_by_model".to_string(), Value::Bool(true));
            transcription.insert("review_required".to_string(), Value::Bool(false));
            transcription.insert("character_count".to_string(), json!(character_count));
        }
        if let Some(review) = extra
            .get_mut("transcription_review")
            .and_then(Value::as_object_mut)
        {
            review.insert("required".to_string(), Value::Bool(false));
            review.insert(
                "reviewed_character_count".to_string(),
                json!(character_count),
            );
        }
    }
    loop_state.task_observations.push(json!({
        "schema_version": 1,
        "owner_layer": "transcript_revision",
        "source": contract.source,
        "target_language": target_language,
        "raw_character_count": contract.raw_text.chars().count(),
        "reviewed_character_count": character_count,
        "chunk_count": chunks.len(),
        "delivery_mode": if inline_delivery { "inline" } else { "artifact" },
    }));
    Ok(CapabilitySynthesis {
        answer,
        confidence,
        evidence_count,
    })
}

fn attach_reviewed_transcript_artifact(
    result: &mut CapabilityResultEnvelope,
    mut artifact: ArtifactRef,
    filename: &str,
    delivery_message: &str,
) -> Result<String, String> {
    let path = artifact
        .path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "transcript_revision_artifact_path_missing".to_string())?
        .to_string();
    artifact.visibility = Some(ArtifactVisibility::UserDelivery);
    artifact.artifact_role = Some("transcript_text".to_string());
    artifact.filename = Some(filename.to_string());
    if !result
        .artifacts
        .iter()
        .any(|existing| existing == &artifact)
    {
        result.artifacts.push(artifact);
    }
    result.delivery.intent = CapabilityDeliveryIntent::Artifact;
    let extra = result
        .data
        .get_mut("extra")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "transcript_revision_result_extra_missing".to_string())?;
    let delivery = extra
        .entry("delivery")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "transcript_revision_delivery_contract_invalid".to_string())?;
    delivery.insert("deliver_to_user".to_string(), Value::Bool(true));
    delivery.insert("intent".to_string(), Value::String("artifact".to_string()));
    Ok(format!("{delivery_message}\nFILE:{path}"))
}

fn transcript_delivery_is_inline(character_count: usize, inline_max_chars: usize) -> bool {
    character_count < inline_max_chars
}

fn normalized_transcript_language(requested: &str, fallback: &str) -> String {
    let requested = requested.trim();
    let selected = if requested.is_empty()
        || matches!(
            requested.to_ascii_lowercase().as_str(),
            "request-language" | "preserve-source-language"
        ) {
        fallback.trim()
    } else {
        requested
    };
    let selected = selected
        .chars()
        .filter(|ch| !ch.is_control())
        .take(64)
        .collect::<String>();
    if selected.is_empty() {
        "request-language".to_string()
    } else {
        selected
    }
}

fn safe_transcript_filename(value: &str) -> String {
    let mut filename = value
        .trim()
        .chars()
        .take(96)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if !filename.to_ascii_lowercase().ends_with(".txt") {
        filename.push_str(".txt");
    }
    filename
}

fn transcript_revision_chunk_chars(state: &AppState) -> usize {
    state
        .core
        .llm_providers
        .iter()
        .map(|provider| provider.model_descriptor().output_reserve_tokens)
        .filter(|tokens| *tokens > 0)
        .min()
        .map(|tokens| tokens.saturating_mul(3).saturating_div(5))
        .unwrap_or(FALLBACK_TRANSCRIPT_REVISION_CHUNK_CHARS)
        .clamp(1_000, 12_000)
}

fn split_transcript_chunks(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let characters = text.trim().chars().collect::<Vec<_>>();
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < characters.len() {
        let mut end = (start + max_chars).min(characters.len());
        if end < characters.len() {
            let floor = start + max_chars / 2;
            if let Some(boundary) = (floor..end).rev().find(|index| {
                matches!(
                    characters[*index],
                    '\n' | '。' | '！' | '？' | '.' | '!' | '?'
                )
            }) {
                end = boundary + 1;
            }
        }
        let chunk = characters[start..end].iter().collect::<String>();
        if !chunk.trim().is_empty() {
            chunks.push(chunk);
        }
        start = end;
    }
    chunks
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

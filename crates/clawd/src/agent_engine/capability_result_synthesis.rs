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
    text_filename: String,
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

pub(super) fn transcript_bundle_delivery_is_complete(
    answer: &str,
    results: &[CapabilityResultEnvelope],
    task_id: &str,
) -> bool {
    let tokens = user_delivery_artifact_tokens(results, task_id);
    let Some(transcript_token) = transcript_delivery_token(results, task_id) else {
        return false;
    };
    answer_contains_delivery_token(answer, &transcript_token)
        && tokens
            .iter()
            .all(|token| answer_contains_delivery_token(answer, token))
}

fn transcript_delivery_token(
    results: &[CapabilityResultEnvelope],
    task_id: &str,
) -> Option<String> {
    results.iter().rev().find_map(|result| {
        result.artifacts.iter().rev().find_map(|artifact| {
            (artifact.visibility == Some(ArtifactVisibility::UserDelivery)
                && artifact.artifact_role.as_deref() == Some("transcript_text"))
            .then(|| delivery_token_for_artifact(artifact, task_id))
            .flatten()
        })
    })
}

fn answer_contains_delivery_token(answer: &str, token: &str) -> bool {
    let reference = token
        .split_once(':')
        .map(|(_, reference)| reference)
        .unwrap_or(token);
    answer.contains(token) || (!reference.is_empty() && answer.contains(reference))
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
    synthesize_from_capability_results_with_policy(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
        false,
        "ordinary_agent_loop",
    )
    .await
}

pub(super) async fn synthesize_scheduled_capability_result(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    result: CapabilityResultEnvelope,
) -> Result<Option<CapabilitySynthesis>, String> {
    let mut loop_state = LoopState::default();
    loop_state.capability_results.push(result);
    synthesize_from_capability_results_with_policy(
        state,
        task,
        user_text,
        &mut loop_state,
        None,
        true,
        "scheduled_job_triggered",
    )
    .await
}

async fn synthesize_from_capability_results_with_policy(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    allow_scheduled_terminal_step: bool,
    execution_context: &str,
) -> Result<Option<CapabilitySynthesis>, String> {
    let transcript_contract = transcript_review_contract(&loop_state.capability_results);
    if transcript_contract.is_none()
        && !(eligible_for_capability_result_synthesis(loop_state, agent_run_context)
            || (allow_scheduled_terminal_step
                && scheduled_terminal_step_is_synthesizable(&loop_state.capability_results)))
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
        let evidence_count = results["entries"]
            .as_array()
            .map_or(0, |entries| entries.len());
        let request_language_hint =
            crate::language_policy::task_response_language_hint(state, task, user_text);
        let target_language =
            normalized_transcript_language(&contract.response_language, &request_language_hint);
        let synthesis = synthesize_reviewed_transcript(
            state,
            task,
            user_text,
            loop_state,
            contract.clone(),
            evidence_count,
        )
        .await;
        let mut synthesis = match synthesis {
            Ok(synthesis) => synthesis,
            Err(error_code) => synthesize_unreviewed_transcript_fallback(
                state,
                task,
                loop_state,
                contract,
                evidence_count,
                &error_code,
                &target_language,
            ),
        };
        if has_companion_user_delivery_artifacts(&loop_state.capability_results) {
            if let Ok(Some(detailed)) = synthesize_model_capability_answer(
                state,
                task,
                user_text,
                loop_state,
                agent_run_context,
                execution_context,
            )
            .await
            {
                synthesis.answer = detailed.answer;
                synthesis.confidence = detailed.confidence;
                synthesis.evidence_count = detailed.evidence_count;
            }
        }
        synthesis.answer = append_user_delivery_artifact_tokens(
            synthesis.answer,
            &loop_state.capability_results,
            &task.task_id,
        );
        return Ok(Some(synthesis));
    }
    let mut synthesis = synthesize_model_capability_answer(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
        execution_context,
    )
    .await?;
    if let Some(synthesis) = synthesis.as_mut() {
        synthesis.answer = append_user_delivery_artifact_tokens(
            std::mem::take(&mut synthesis.answer),
            &loop_state.capability_results,
            &task.task_id,
        );
    }
    Ok(synthesis)
}

async fn synthesize_model_capability_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    execution_context: &str,
) -> Result<Option<CapabilitySynthesis>, String> {
    let results = synthesis_evidence_catalog(state, task, &loop_state.capability_results)?;
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
            ("__EXECUTION_CONTEXT__", execution_context),
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

fn scheduled_terminal_step_is_synthesizable(results: &[CapabilityResultEnvelope]) -> bool {
    !results.is_empty()
        && results.iter().all(|result| {
            result.delivery.intent == CapabilityDeliveryIntent::ModelSynthesis
                && matches!(
                    result.status,
                    CapabilityResultStatus::Ok | CapabilityResultStatus::Error
                )
        })
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
            let text_filename = delivery
                .and_then(|delivery| delivery.get("text_filename"))
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
                text_filename,
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
        confidence = confidence.min(parsed.confidence.clamp(0.0, 1.0));
        reviewed_chunks.push(reviewed.to_string());
    }
    let reviewed_text = normalize_transcript_script_for_language(
        reviewed_chunks.join("\n\n").trim(),
        &target_language,
    );
    if reviewed_text.is_empty() {
        return Err("transcript_revision_empty".to_string());
    }
    let character_count = reviewed_text.chars().count();
    let published = crate::skill_output_artifact::publish_task_text_artifact(
        &state.skill_rt.workspace_root,
        &task.task_id,
        "transcript-review",
        &contract.text_filename,
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
    let answer = attach_reviewed_transcript_artifact(
        result,
        artifact,
        &contract.text_filename,
        &reviewed_text,
        &target_language,
    )?;
    if let Some(extra) = loop_state
        .capability_results
        .get_mut(contract.result_index)
        .and_then(|result| result.data.get_mut("extra"))
        .and_then(Value::as_object_mut)
    {
        extra.insert(
            "transcription_delivery".to_string(),
            json!({
                "mode": "inline_and_artifact",
                "text_included": true,
                "artifact_included": true,
                "character_count": character_count,
                "reviewed_by_model": true,
                "target_language": target_language,
                "source": contract.source,
                "result_label_kind": "audio_transcript",
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
                "result_label_kind".to_string(),
                Value::String("audio_transcript".to_string()),
            );
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
        "delivery_mode": "inline_and_artifact",
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
    reviewed_text: &str,
    target_language: &str,
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
    Ok(format_labeled_audio_transcript(
        reviewed_text,
        target_language,
        Some(&path),
    ))
}

fn audio_transcript_label(language: &str) -> &'static str {
    let language = language.trim().replace('_', "-").to_ascii_lowercase();
    if matches!(language.as_str(), "zh" | "zh-cn" | "zh-sg" | "zh-hans")
        || language.starts_with("zh-hans-")
    {
        "音频转写"
    } else if matches!(language.as_str(), "zh-tw" | "zh-hk" | "zh-mo" | "zh-hant")
        || language.starts_with("zh-hant-")
    {
        "音訊轉寫"
    } else if language == "ja" || language.starts_with("ja-") {
        "音声文字起こし"
    } else if language == "ko" || language.starts_with("ko-") {
        "오디오 전사"
    } else {
        "Audio transcript"
    }
}

fn format_labeled_audio_transcript(
    text: &str,
    target_language: &str,
    artifact_path: Option<&str>,
) -> String {
    let label = audio_transcript_label(target_language);
    artifact_path.map_or_else(
        || format!("{label}:\n{text}"),
        |path| format!("{label}:\n{text}\nFILE:{path}"),
    )
}

fn has_companion_user_delivery_artifacts(results: &[CapabilityResultEnvelope]) -> bool {
    results.iter().any(|result| {
        result.artifacts.iter().any(|artifact| {
            artifact.visibility == Some(ArtifactVisibility::UserDelivery)
                && artifact.artifact_role.as_deref() != Some("transcript_text")
        }) || result
            .data
            .pointer("/extra/processing_inputs/video_audio/status")
            .and_then(Value::as_str)
            == Some("available")
    })
}

fn append_user_delivery_artifact_tokens(
    mut answer: String,
    results: &[CapabilityResultEnvelope],
    task_id: &str,
) -> String {
    for token in user_delivery_artifact_tokens(results, task_id) {
        let already_present = answer.lines().any(|line| {
            let Some(parsed) =
                claw_core::channel_delivery_tokens::parse_legacy_delivery_line_ref(line.trim())
            else {
                return false;
            };
            let reference = token
                .split_once(':')
                .map(|(_, reference)| reference)
                .unwrap_or(token.as_str());
            parsed.reference == reference || line.contains(token.as_str())
        });
        if already_present {
            continue;
        }
        if !answer.ends_with('\n') {
            answer.push('\n');
        }
        answer.push_str(&token);
    }
    answer
}

fn user_delivery_artifact_tokens(
    results: &[CapabilityResultEnvelope],
    task_id: &str,
) -> Vec<String> {
    let mut ranked = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for result in results {
        for artifact in &result.artifacts {
            if artifact.visibility != Some(ArtifactVisibility::UserDelivery) {
                continue;
            }
            if let Some(token) = delivery_token_for_artifact(artifact, task_id) {
                if seen.insert(token.clone()) {
                    ranked.push((
                        delivery_token_rank(artifact.artifact_role.as_deref()),
                        token,
                    ));
                }
            }
        }
        if let Some(token) = processing_audio_delivery_token(result, task_id) {
            if seen.insert(token.clone()) {
                ranked.push((1, token));
            }
        }
    }
    ranked.sort_by_key(|(rank, token)| (*rank, token.clone()));
    ranked.into_iter().map(|(_, token)| token).collect()
}

fn processing_audio_delivery_token(
    result: &CapabilityResultEnvelope,
    task_id: &str,
) -> Option<String> {
    let audio = result
        .data
        .pointer("/extra/processing_inputs/video_audio")?;
    if audio.get("status").and_then(Value::as_str) != Some("available") {
        return None;
    }
    if result
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_role.as_deref() == Some("extracted_audio"))
    {
        return None;
    }
    let artifact = ArtifactRef {
        artifact_ref: audio
            .get("artifact_ref")
            .and_then(Value::as_str)
            .map(str::to_string),
        id: audio
            .get("id")
            .or_else(|| audio.get("artifact_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        path: audio
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_string),
        uri: None,
        media_type: audio
            .get("mime_type")
            .or_else(|| audio.get("media_type"))
            .and_then(Value::as_str)
            .map(str::to_string),
        filename: audio
            .get("filename")
            .and_then(Value::as_str)
            .map(str::to_string),
        artifact_role: Some("extracted_audio".to_string()),
        size_bytes: audio.get("size_bytes").and_then(Value::as_u64),
        sha256: audio
            .get("sha256")
            .and_then(Value::as_str)
            .map(str::to_string),
        visibility: Some(ArtifactVisibility::UserDelivery),
        owner_task_id: Some(task_id.to_string()),
        producer: None,
        lease: None,
        metadata: json!({}),
    };
    delivery_token_for_artifact(&artifact, task_id)
}

fn delivery_token_for_artifact(artifact: &ArtifactRef, task_id: &str) -> Option<String> {
    let reference = artifact
        .artifact_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            artifact.id.as_deref().and_then(|id| {
                claw_core::task_delivery_artifacts::canonical_task_artifact_ref(task_id, id)
            })
        })
        .or_else(|| {
            artifact
                .path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })?;
    Some(format!(
        "{}{reference}",
        delivery_prefix_for_artifact(artifact)
    ))
}

fn delivery_prefix_for_artifact(artifact: &ArtifactRef) -> &'static str {
    let media_type = artifact.media_type.as_deref().unwrap_or_default();
    if media_type.starts_with("image/") {
        return "IMAGE_FILE:";
    }
    if media_type.starts_with("video/")
        || artifact.artifact_role.as_deref() == Some("original_video")
    {
        return "VIDEO_FILE:";
    }
    "FILE:"
}

fn delivery_token_rank(role: Option<&str>) -> u8 {
    match role {
        Some("original_video") => 0,
        Some("extracted_audio" | "background_audio") => 1,
        Some("transcript_text") => 2,
        _ => 3,
    }
}

fn synthesize_unreviewed_transcript_fallback(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    contract: TranscriptReviewContract,
    evidence_count: usize,
    error_code: &str,
    target_language: &str,
) -> CapabilitySynthesis {
    let normalized_text =
        normalize_transcript_script_for_language(&contract.raw_text, target_language);
    let character_count = normalized_text.chars().count();
    let artifact = crate::skill_output_artifact::publish_task_text_artifact(
        &state.skill_rt.workspace_root,
        &task.task_id,
        "transcript-fallback",
        &contract.text_filename,
        &(normalized_text.clone() + "\n"),
        json!({
            "artifact_role": "transcript_text",
            "reviewed_by_model": false,
            "review_status": "degraded",
            "review_error_code": error_code,
            "target_language": target_language,
            "source": contract.source,
            "character_count": character_count,
        }),
    )
    .ok()
    .and_then(|published| serde_json::from_value::<ArtifactRef>(published.artifact_ref).ok());
    let artifact_included = artifact
        .as_ref()
        .and_then(|artifact| artifact.path.as_deref())
        .is_some_and(|path| !path.trim().is_empty());
    let answer = loop_state
        .capability_results
        .get_mut(contract.result_index)
        .map(|result| {
            attach_unreviewed_transcript_fallback(
                result,
                artifact,
                &contract.text_filename,
                &normalized_text,
                target_language,
                &contract.source,
                error_code,
            )
        })
        .unwrap_or_else(|| normalized_text.clone());
    loop_state.task_observations.push(json!({
        "schema_version": 1,
        "owner_layer": "transcript_revision",
        "status": "degraded",
        "error_code": error_code,
        "source": contract.source,
        "target_language": target_language,
        "raw_character_count": character_count,
        "reviewed_by_model": false,
        "delivery_mode": if artifact_included {
            "inline_and_artifact"
        } else {
            "inline"
        },
    }));
    CapabilitySynthesis {
        answer,
        confidence: 0.0,
        evidence_count,
    }
}

fn attach_unreviewed_transcript_fallback(
    result: &mut CapabilityResultEnvelope,
    artifact: Option<ArtifactRef>,
    filename: &str,
    raw_text: &str,
    target_language: &str,
    source: &str,
    error_code: &str,
) -> String {
    let mut artifact_path = None;
    if let Some(mut artifact) = artifact {
        artifact_path = artifact
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_string);
        if artifact_path.is_some() {
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
        }
    }
    result.delivery.intent = if artifact_path.is_some() {
        CapabilityDeliveryIntent::Artifact
    } else {
        CapabilityDeliveryIntent::ExactMachine
    };
    if let Some(extra) = result.data.get_mut("extra").and_then(Value::as_object_mut) {
        let delivery = extra.entry("delivery").or_insert_with(|| json!({}));
        if !delivery.is_object() {
            *delivery = json!({});
        }
        if let Some(delivery) = delivery.as_object_mut() {
            delivery.insert("deliver_to_user".to_string(), Value::Bool(true));
            delivery.insert(
                "intent".to_string(),
                Value::String(
                    if artifact_path.is_some() {
                        "artifact"
                    } else {
                        "exact_machine"
                    }
                    .to_string(),
                ),
            );
        }
        extra.insert(
            "transcription_delivery".to_string(),
            json!({
                "mode": if artifact_path.is_some() { "inline_and_artifact" } else { "inline" },
                "text_included": true,
                "artifact_included": artifact_path.is_some(),
                "character_count": raw_text.chars().count(),
                "reviewed_by_model": false,
                "review_status": "degraded",
                "review_error_code": error_code,
                "target_language": target_language,
                "source": source,
                "result_label_kind": "audio_transcript",
            }),
        );
        if let Some(transcription) = extra
            .get_mut("transcription")
            .and_then(Value::as_object_mut)
        {
            transcription.insert("reviewed_by_model".to_string(), Value::Bool(false));
            transcription.insert("review_required".to_string(), Value::Bool(false));
            transcription.insert(
                "character_count".to_string(),
                json!(raw_text.chars().count()),
            );
        }
        if let Some(review) = extra
            .get_mut("transcription_review")
            .and_then(Value::as_object_mut)
        {
            review.insert("required".to_string(), Value::Bool(false));
            review.insert(
                "result_label_kind".to_string(),
                Value::String("audio_transcript".to_string()),
            );
            review.insert("status".to_string(), Value::String("degraded".to_string()));
            review.insert(
                "error_code".to_string(),
                Value::String(error_code.to_string()),
            );
        }
    }
    format_labeled_audio_transcript(raw_text, target_language, artifact_path.as_deref())
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

fn normalize_transcript_script_for_language(text: &str, language: &str) -> String {
    let language = language.trim().replace('_', "-").to_ascii_lowercase();
    let simplified_chinese = matches!(language.as_str(), "zh" | "zh-cn" | "zh-sg" | "zh-hans")
        || language.starts_with("zh-hans-");
    if simplified_chinese {
        // The skill has already applied regional phrase conversion before review.
        // Keep this final guard script-only so already-simplified model text remains stable.
        zhhz::Converter::new(zhhz::Config::T2s).convert(text)
    } else {
        text.to_string()
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

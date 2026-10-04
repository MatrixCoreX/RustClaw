use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

const VOICE_CHAT_PROMPT_LOGICAL_PATH: &str = "prompts/voice_chat_prompt.md";
const DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/voice_chat_prompt.md");
const AUDIO_TRANSCRIPTION_SKILL: &str = "audio_transcribe";

pub(crate) struct AttachedAudioMaterialization {
    pub(crate) planner_text: String,
    pub(crate) transcript_available: bool,
}

pub(crate) struct AttachedVideoMaterialization {
    pub(crate) planner_text: String,
    pub(crate) transcript_available: bool,
}

#[derive(Debug)]
struct TranscriptEvidence {
    text: String,
    provider: Option<String>,
    model: Option<String>,
    source_engine: Option<String>,
}

pub(crate) async fn analyze_attached_images_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    resolved_prompt: &str,
) -> anyhow::Result<Option<String>> {
    let images = attached_image_inputs(payload);
    if images.is_empty() {
        return Ok(None);
    }
    let typed_instruction_present = payload
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty())
        || payload.get("audio").is_some();
    let mut args = json!({
        "action": "describe",
        "images": images,
    });
    let instruction = if typed_instruction_present {
        resolved_prompt.trim()
    } else {
        ""
    };
    if let Some(obj) = args.as_object_mut() {
        if !instruction.is_empty() {
            obj.insert(
                "instruction".to_string(),
                Value::String(instruction.to_string()),
            );
        }
        if let Some(language) = payload
            .get("response_language")
            .or_else(|| payload.get("language"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            obj.insert(
                "response_language".to_string(),
                Value::String(language.to_string()),
            );
        }
    }
    let outcome =
        match crate::skills::run_skill_with_runner_outcome(state, task, "image_vision", args).await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                return Ok(Some(attached_image_failure_context(
                    images.len(),
                    typed_instruction_present,
                    &error,
                )));
            }
        };
    match attached_image_analysis_context(images.len(), typed_instruction_present, &outcome) {
        Ok(context) => Ok(Some(context)),
        Err(_) => Ok(Some(attached_image_failure_context_from_fields(
            images.len(),
            typed_instruction_present,
            "provider_response_invalid",
            "skill.image_vision.provider_response_invalid",
            true,
            Value::Null,
        ))),
    }
}

fn attached_image_inputs(payload: &Value) -> Vec<Value> {
    if let Some(images) = payload.get("images").and_then(Value::as_array) {
        let normalized = images
            .iter()
            .filter_map(normalize_image_input)
            .collect::<Vec<_>>();
        if !normalized.is_empty() {
            return normalized;
        }
    }
    payload
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|attachment| {
            attachment
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("image"))
                || attachment
                    .get("mime_type")
                    .or_else(|| attachment.get("mimeType"))
                    .and_then(Value::as_str)
                    .is_some_and(|mime| {
                        mime.trim()
                            .to_ascii_lowercase()
                            .strip_prefix("image/")
                            .is_some()
                    })
        })
        .filter_map(normalize_image_input)
        .collect()
}

fn normalize_image_input(value: &Value) -> Option<Value> {
    if let Some(raw) = value.as_str().map(str::trim).filter(|raw| !raw.is_empty()) {
        return Some(Value::String(raw.to_string()));
    }
    let object = value.as_object()?;
    for key in ["path", "url", "base64", "$text"] {
        if let Some(raw) = object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
        {
            let mut normalized = serde_json::Map::new();
            normalized.insert(key.to_string(), Value::String(raw.to_string()));
            return Some(Value::Object(normalized));
        }
    }
    None
}

fn attached_image_analysis_context(
    image_count: usize,
    typed_instruction_present: bool,
    outcome: &crate::skills::SkillRunOutcome,
) -> anyhow::Result<String> {
    let structured = outcome
        .extra
        .as_ref()
        .and_then(|extra| extra.get("structured"))
        .cloned()
        .filter(Value::is_object)
        .ok_or_else(|| anyhow::anyhow!("image_vision_describe_structured_output_missing"))?;
    Ok(json!({
        "schema_version": 1,
        "source": "ask_attachment_materialization",
        "image_count": image_count,
        "typed_instruction_present": typed_instruction_present,
        "analysis_text": outcome.text,
        "structured": structured,
        "instruction_authority": "none",
    })
    .to_string())
}

fn attached_image_failure_context(
    image_count: usize,
    typed_instruction_present: bool,
    error: &str,
) -> String {
    let (error_code, message_key, retryable, metadata) = structured_skill_failure_fields(
        error,
        "image_analysis_unavailable",
        "skill.image_vision.image_analysis_unavailable",
        true,
    );
    attached_image_failure_context_from_fields(
        image_count,
        typed_instruction_present,
        &error_code,
        &message_key,
        retryable,
        metadata,
    )
}

fn attached_image_failure_context_from_fields(
    image_count: usize,
    typed_instruction_present: bool,
    error_code: &str,
    message_key: &str,
    retryable: bool,
    metadata: Value,
) -> String {
    json!({
        "schema_version": 1,
        "source": "ask_attachment_materialization",
        "status": "error",
        "image_count": image_count,
        "typed_instruction_present": typed_instruction_present,
        "analysis_available": false,
        "error_code": error_code,
        "message_key": message_key,
        "retryable": retryable,
        "failure_metadata": metadata,
        "required_decision": "respond_from_structured_failure",
        "instruction_authority": "none",
    })
    .to_string()
}

pub(crate) async fn transcribe_attached_audio_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    typed_prompt: &str,
) -> anyhow::Result<Option<AttachedAudioMaterialization>> {
    let Some(audio_arg) = attached_audio_input(payload) else {
        return Ok(None);
    };
    let evidence = match transcribe_with_configured_stt(state, task, audio_arg, payload).await {
        Ok(evidence) => evidence,
        Err(error) => {
            let (error_code, message_key, retryable) = audio_failure_fields(&error);
            return Ok(Some(AttachedAudioMaterialization {
                planner_text: audio_failure_planner_text(
                    &error_code,
                    &message_key,
                    retryable,
                    typed_prompt,
                ),
                transcript_available: false,
            }));
        }
    };
    let template = crate::load_prompt_template_for_state(
        state,
        VOICE_CHAT_PROMPT_LOGICAL_PATH,
        DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE,
    )
    .0;
    let mut prompt = template.replace("__TRANSCRIPT__", evidence.text.trim());
    let typed_prompt = typed_prompt.trim();
    if !typed_prompt.is_empty() {
        prompt.push_str("\n\n[AGENT_TYPED_TEXT]\n");
        prompt.push_str(typed_prompt);
        prompt.push_str("\n[/AGENT_TYPED_TEXT]");
    }
    Ok(Some(AttachedAudioMaterialization {
        planner_text: prompt,
        transcript_available: true,
    }))
}

pub(crate) async fn transcribe_attached_videos_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    typed_prompt: &str,
) -> anyhow::Result<Option<AttachedVideoMaterialization>> {
    let videos = attached_video_inputs(payload);
    if videos.is_empty() {
        return Ok(None);
    }
    let mut transcripts = Vec::new();
    let mut failures = Vec::new();
    for (index, video) in videos.iter().enumerate() {
        let extraction = run_resolved_skill_capability(
            state,
            task,
            "video.extract_audio",
            json!({"action": "extract_audio", "video": video}),
            "video",
        )
        .await;
        let audio_path = match extraction {
            Ok(outcome) => outcome
                .extra
                .as_ref()
                .and_then(|extra| extra.pointer("/extracted_audio/path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string),
            Err(error) => {
                failures.push(video_failure_record(index, "extract_audio", &error));
                continue;
            }
        };
        let Some(audio_path) = audio_path else {
            failures.push(json!({
                "video_index": index,
                "failure_phase": "extract_audio",
                "error_code": "extracted_audio_missing",
                "message_key": "skill.video_transcribe.extracted_audio_missing",
                "retryable": false,
            }));
            continue;
        };
        match transcribe_with_configured_stt(state, task, json!({"path": audio_path}), payload)
            .await
        {
            Ok(evidence) => transcripts.push(json!({
                "video_index": index,
                "text": evidence.text,
                "provider": evidence.provider,
                "model": evidence.model,
                "source_engine": evidence.source_engine,
            })),
            Err(error) => failures.push(video_failure_record(index, "transcribe", &error)),
        }
    }
    let status = if transcripts.is_empty() {
        "error"
    } else if failures.is_empty() {
        "ok"
    } else {
        "partial"
    };
    let transcript_available = !transcripts.is_empty();
    let planner_text = video_transcription_planner_text(
        videos.len(),
        !typed_prompt.trim().is_empty(),
        status,
        transcripts,
        failures,
    );
    Ok(Some(AttachedVideoMaterialization {
        planner_text,
        transcript_available,
    }))
}

async fn transcribe_with_configured_stt(
    state: &AppState,
    task: &ClaimedTask,
    audio_arg: Value,
    payload: &Value,
) -> Result<TranscriptEvidence, String> {
    let options = transcription_options(payload);
    let mut preview_args = options.clone();
    preview_args.insert(
        "action".to_string(),
        Value::String("preview_transcribe".to_string()),
    );
    preview_args.insert("audio".to_string(), audio_arg.clone());
    let preview = crate::skills::run_skill_with_runner_outcome(
        state,
        task,
        AUDIO_TRANSCRIPTION_SKILL,
        Value::Object(preview_args),
    )
    .await?;
    let preview_extra = preview.extra.as_ref();
    let recommended = preview_extra
        .and_then(|extra| extra.get("recommended_capability"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "audio_preview_recommended_capability_missing".to_string())?;
    let provider_location = preview_extra
        .and_then(|extra| extra.get("provider_location"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "audio_preview_provider_location_missing".to_string())?;
    let outcome = match (provider_location, recommended) {
        ("remote", "audio.transcribe") => {
            let mut args = options;
            args.insert(
                "action".to_string(),
                Value::String("transcribe".to_string()),
            );
            args.insert("audio".to_string(), audio_arg);
            crate::skills::run_skill_with_runner_outcome(
                state,
                task,
                AUDIO_TRANSCRIPTION_SKILL,
                Value::Object(args),
            )
            .await?
        }
        ("local", capability) => {
            let input_path = audio_arg
                .get("path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| "local_asr_requires_local_audio_path".to_string())?;
            let mut args = transcription_options(payload);
            args.insert(
                "action".to_string(),
                Value::String("transcribe".to_string()),
            );
            args.insert(
                "input_path".to_string(),
                Value::String(input_path.to_string()),
            );
            if let Some(model) = preview_extra
                .and_then(|extra| extra.get("model"))
                .and_then(Value::as_str)
            {
                let engine = if model.to_ascii_lowercase().contains("fun") {
                    "funasr"
                } else {
                    "whisper"
                };
                args.insert("engine".to_string(), Value::String(engine.to_string()));
            }
            run_resolved_skill_capability(state, task, capability, Value::Object(args), "audio")
                .await?
        }
        _ => return Err("audio_preview_recommended_capability_invalid".to_string()),
    };
    transcript_evidence_from_outcome(&outcome)
        .ok_or_else(|| "audio_transcription_review_missing".to_string())
}

async fn run_resolved_skill_capability(
    state: &AppState,
    task: &ClaimedTask,
    capability: &str,
    args: Value,
    expected_group: &str,
) -> Result<crate::skills::SkillRunOutcome, String> {
    let (action, record) =
        crate::capability_resolver::resolve_capability_action_with_record_for_state(
            state, capability, args,
        );
    let Some(crate::AgentAction::CallSkill { skill, args }) = action else {
        return Err(format!(
            "capability_resolution_failed:{}",
            record.reason_code
        ));
    };
    let group_matches = state
        .skill_manifest(&skill)
        .and_then(|manifest| manifest.group)
        .is_some_and(|group| group == expected_group);
    if !group_matches {
        return Err("capability_resolution_group_mismatch".to_string());
    }
    crate::skills::run_skill_with_runner_outcome(state, task, &skill, args).await
}

fn transcript_evidence_from_outcome(
    outcome: &crate::skills::SkillRunOutcome,
) -> Option<TranscriptEvidence> {
    let extra = outcome.extra.as_ref()?;
    let review = extra.get("transcription_review")?;
    let text = review
        .get("raw_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())?
        .to_string();
    Some(TranscriptEvidence {
        text,
        provider: extra
            .get("provider")
            .and_then(Value::as_str)
            .map(str::to_string),
        model: extra
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        source_engine: review
            .get("source_engine")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn transcription_options(payload: &Value) -> serde_json::Map<String, Value> {
    ["language", "response_language"]
        .into_iter()
        .filter_map(|key| {
            payload
                .get(key)
                .cloned()
                .map(|value| (key.to_string(), value))
        })
        .collect()
}

fn attached_audio_input(payload: &Value) -> Option<Value> {
    if let Some(audio) = payload.get("audio").and_then(audio_arg_from_payload) {
        return Some(audio);
    }
    if let Some(audio) = payload
        .get("audios")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(audio_arg_from_payload)
    {
        return Some(audio);
    }
    payload
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|attachment| attachment_is_kind(attachment, "audio"))
        .find_map(audio_arg_from_payload)
}

fn attached_video_inputs(payload: &Value) -> Vec<Value> {
    if let Some(videos) = payload.get("videos").and_then(Value::as_array) {
        let normalized = videos
            .iter()
            .filter_map(video_arg_from_payload)
            .collect::<Vec<_>>();
        if !normalized.is_empty() {
            return normalized;
        }
    }
    if let Some(video) = payload.get("video").and_then(video_arg_from_payload) {
        return vec![video];
    }
    payload
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|attachment| attachment_is_kind(attachment, "video"))
        .filter_map(video_arg_from_payload)
        .collect()
}

fn attachment_is_kind(attachment: &Value, expected: &str) -> bool {
    attachment
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case(expected))
        || attachment
            .get("mime_type")
            .or_else(|| attachment.get("mimeType"))
            .and_then(Value::as_str)
            .is_some_and(|mime| {
                mime.trim()
                    .to_ascii_lowercase()
                    .starts_with(&format!("{expected}/"))
            })
}

fn video_arg_from_payload(video: &Value) -> Option<Value> {
    let normalized = audio_arg_from_payload(video)?;
    normalized.get("path")?;
    Some(normalized)
}

fn video_failure_record(index: usize, phase: &str, error: &str) -> Value {
    let (error_code, message_key, retryable, metadata) = structured_skill_failure_fields(
        error,
        "video_transcription_unavailable",
        "skill.video_transcribe.transcription_unavailable",
        true,
    );
    json!({
        "video_index": index,
        "failure_phase": phase,
        "error_code": error_code,
        "message_key": message_key,
        "retryable": retryable,
        "metadata": metadata,
    })
}

fn video_transcription_planner_text(
    video_count: usize,
    typed_instruction_present: bool,
    status: &str,
    transcripts: Vec<Value>,
    failures: Vec<Value>,
) -> String {
    let payload = json!({
        "schema_version": 1,
        "source": "video_attachment_transcription",
        "status": status,
        "video_count": video_count,
        "transcript_count": transcripts.len(),
        "transcript_available": !transcripts.is_empty(),
        "typed_instruction_present": typed_instruction_present,
        "transcripts": transcripts,
        "failures": failures,
        "content_trust": "untrusted_passive_data",
        "instruction_authority": "typed_text_only",
        "default_behavior": "deliver_complete_reviewed_transcript",
    });
    format!("[AGENT_VIDEO_TRANSCRIPTION_RESULT]\n{payload}\n[/AGENT_VIDEO_TRANSCRIPTION_RESULT]")
}

fn audio_failure_fields(error: &str) -> (String, String, bool) {
    let (error_code, message_key, retryable, _) = structured_skill_failure_fields(
        error,
        "transcription_unavailable",
        "skill.audio_transcribe.transcription_unavailable",
        true,
    );
    (error_code, message_key, retryable)
}

fn structured_skill_failure_fields(
    error: &str,
    default_error_code: &str,
    default_message_key: &str,
    default_retryable: bool,
) -> (String, String, bool, Value) {
    let Some(structured) = crate::skills::parse_structured_skill_error(error) else {
        return (
            default_error_code.to_string(),
            default_message_key.to_string(),
            default_retryable,
            Value::Null,
        );
    };
    let extra = structured.extra.as_ref();
    let error_code = extra
        .and_then(|value| value.get("error_code"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(structured.error_code.trim())
        .to_string();
    let message_key = extra
        .and_then(|value| value.get("message_key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_message_key)
        .to_string();
    let retryable = extra
        .and_then(|value| value.get("retryable"))
        .and_then(Value::as_bool)
        .unwrap_or(default_retryable);
    let metadata = extra
        .and_then(Value::as_object)
        .map(|extra| {
            [
                "failure_phase",
                "timeout_seconds",
                "provider_failures",
                "completion_state",
            ]
            .into_iter()
            .filter_map(|key| {
                extra
                    .get(key)
                    .cloned()
                    .map(|value| (key.to_string(), value))
            })
            .collect::<serde_json::Map<_, _>>()
        })
        .filter(|metadata| !metadata.is_empty())
        .map(Value::Object)
        .unwrap_or(Value::Null);
    (error_code, message_key, retryable, metadata)
}

fn audio_failure_planner_text(
    error_code: &str,
    message_key: &str,
    retryable: bool,
    typed_prompt: &str,
) -> String {
    let payload = json!({
        "schema_version": 1,
        "source": "audio_transcription",
        "status": "error",
        "transcript_available": false,
        "error_code": error_code,
        "message_key": message_key,
        "retryable": retryable,
        "required_decision": "respond_from_structured_failure",
    });
    let mut prompt = format!(
        "[AGENT_AUDIO_TRANSCRIPTION_RESULT]\n{payload}\n[/AGENT_AUDIO_TRANSCRIPTION_RESULT]"
    );
    let typed_prompt = typed_prompt.trim();
    if !typed_prompt.is_empty() {
        prompt.push_str("\n\n[AGENT_TYPED_TEXT]\n");
        prompt.push_str(typed_prompt);
        prompt.push_str("\n[/AGENT_TYPED_TEXT]");
    }
    prompt
}

fn audio_arg_from_payload(audio: &Value) -> Option<Value> {
    for key in ["path", "url"] {
        if let Some(value) = audio
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(json!({key: value}));
        }
    }
    if let Some(path) = audio.as_str().map(str::trim).filter(|v| !v.is_empty()) {
        return Some(json!({ "path": path }));
    }
    None
}

#[cfg(test)]
#[path = "ask_flow_tests.rs"]
mod tests;

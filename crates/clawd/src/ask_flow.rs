use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

const VOICE_CHAT_PROMPT_LOGICAL_PATH: &str = "prompts/voice_chat_prompt.md";
const DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/voice_chat_prompt.md");

pub(crate) struct AttachedAudioMaterialization {
    pub(crate) planner_text: String,
    pub(crate) transcript_available: bool,
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
    let outcome = crate::skills::run_skill_with_runner_outcome(state, task, "image_vision", args)
        .await
        .map_err(anyhow::Error::msg)?;
    let context =
        attached_image_analysis_context(images.len(), typed_instruction_present, &outcome)?;
    Ok(Some(context))
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

pub(crate) async fn transcribe_attached_audio_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    typed_prompt: &str,
) -> anyhow::Result<Option<AttachedAudioMaterialization>> {
    let Some(audio) = payload.get("audio") else {
        return Ok(None);
    };
    let Some(audio_arg) = audio_arg_from_payload(audio) else {
        return Ok(None);
    };
    let outcome = match crate::skills::run_skill_with_runner_outcome(
        state,
        task,
        "audio_transcribe",
        json!({ "audio": audio_arg }),
    )
    .await
    {
        Ok(outcome) => outcome,
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
    let transcript = outcome.text.trim();
    if transcript.is_empty() {
        return Ok(Some(AttachedAudioMaterialization {
            planner_text: audio_failure_planner_text(
                "empty_transcript",
                "skill.audio_transcribe.empty_transcript",
                true,
                typed_prompt,
            ),
            transcript_available: false,
        }));
    }
    let template = crate::load_prompt_template_for_state(
        state,
        VOICE_CHAT_PROMPT_LOGICAL_PATH,
        DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE,
    )
    .0;
    let mut prompt = template.replace("__TRANSCRIPT__", transcript);
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

fn audio_failure_fields(error: &str) -> (String, String, bool) {
    let Some(structured) = crate::skills::parse_structured_skill_error(error) else {
        return (
            "transcription_unavailable".to_string(),
            "skill.audio_transcribe.transcription_unavailable".to_string(),
            true,
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
        .unwrap_or("skill.audio_transcribe.transcription_unavailable")
        .to_string();
    let retryable = extra
        .and_then(|value| value.get("retryable"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    (error_code, message_key, retryable)
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
    if audio.get("path").and_then(Value::as_str).is_some()
        || audio.get("url").and_then(Value::as_str).is_some()
    {
        return Some(audio.clone());
    }
    if let Some(path) = audio.as_str().map(str::trim).filter(|v| !v.is_empty()) {
        return Some(json!({ "path": path }));
    }
    None
}

#[cfg(test)]
#[path = "ask_flow_tests.rs"]
mod tests;

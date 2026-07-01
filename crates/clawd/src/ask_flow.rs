use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

const VOICE_CHAT_PROMPT_LOGICAL_PATH: &str = "prompts/voice_chat_prompt.md";
const DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/voice_chat_prompt.md");

#[path = "ask_flow_resume.rs"]
mod resume;
pub(crate) use resume::{
    build_resume_continue_execute_prompt, build_resume_continue_execute_prompt_from_context,
    build_resume_followup_discussion_prompt, build_resume_followup_discussion_prompt_from_context,
};

pub(crate) async fn analyze_attached_images_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    resolved_prompt: &str,
) -> anyhow::Result<Option<String>> {
    let Some(images) = payload.get("images").and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    if images.is_empty() {
        return Ok(None);
    }
    let mut args = json!({
        "action": "describe",
        "images": images,
    });
    let instruction = resolved_prompt.trim();
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
    crate::skills::run_skill_with_runner(state, task, "image_vision", args)
        .await
        .map_err(anyhow::Error::msg)
        .map(Some)
}

pub(crate) async fn transcribe_attached_audio_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    typed_prompt: &str,
) -> anyhow::Result<Option<String>> {
    let Some(audio) = payload.get("audio") else {
        return Ok(None);
    };
    let Some(audio_arg) = audio_arg_from_payload(audio) else {
        return Ok(None);
    };
    let outcome = crate::skills::run_skill_with_runner_outcome(
        state,
        task,
        "audio_transcribe",
        json!({ "audio": audio_arg }),
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let transcript = outcome.text.trim();
    if transcript.is_empty() {
        return Err(anyhow::anyhow!("audio_transcript_empty"));
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
        prompt.push_str("\n\n[RUSTCLAW_TYPED_TEXT]\n");
        prompt.push_str(typed_prompt);
        prompt.push_str("\n[/RUSTCLAW_TYPED_TEXT]");
    }
    Ok(Some(prompt))
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

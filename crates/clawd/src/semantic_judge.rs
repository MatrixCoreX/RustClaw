use serde::Deserialize;

use crate::{llm_gateway, AppState, ClaimedTask};

const META_RESPOND_CLASSIFIER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/meta_respond_classifier_prompt.md");
const META_RESPOND_CLASSIFIER_PROMPT_LOGICAL_PATH: &str =
    "prompts/meta_respond_classifier_prompt.md";
const PUBLISHABLE_RAW_CLASSIFIER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/publishable_raw_classifier_prompt.md");
const PUBLISHABLE_RAW_CLASSIFIER_PROMPT_LOGICAL_PATH: &str =
    "prompts/publishable_raw_classifier_prompt.md";

#[derive(Debug, Deserialize)]
struct MetaRespondClassifierOut {
    #[serde(default)]
    is_meta_instruction: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
}

#[derive(Debug, Deserialize)]
struct PublishableRawClassifierOut {
    #[serde(default)]
    publishable: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
}

async fn classify_meta_respond_instruction_with_llm(
    state: &AppState,
    task: &ClaimedTask,
    text: &str,
) -> Option<(bool, String, f64)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some((false, "empty".to_string(), 1.0));
    }
    let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
        state,
        META_RESPOND_CLASSIFIER_PROMPT_LOGICAL_PATH,
        META_RESPOND_CLASSIFIER_PROMPT_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(&prompt_template, &[("__TEXT__", trimmed)]);
    crate::log_prompt_render(
        state,
        &task.task_id,
        "meta_respond_classifier_prompt",
        &prompt_source,
        None,
    );
    let llm_out =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await
            .ok()?;
    let trimmed_out = llm_out.trim();
    let parsed_raw = serde_json::from_str::<MetaRespondClassifierOut>(trimmed_out).ok();
    let parsed = parsed_raw.or_else(|| {
        crate::extract_first_json_object_any(&llm_out)
            .and_then(|json| serde_json::from_str::<MetaRespondClassifierOut>(&json).ok())
    })?;
    Some((
        parsed.is_meta_instruction,
        parsed.reason,
        parsed.confidence.clamp(0.0, 1.0),
    ))
}

pub(crate) async fn is_meta_respond_instruction(
    state: &AppState,
    task: &ClaimedTask,
    text: &str,
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.chars().count() > 600 {
        return false;
    }
    classify_meta_respond_instruction_with_llm(state, task, trimmed)
        .await
        .map(|(is_meta, _reason, confidence)| is_meta && confidence >= 0.55)
        .unwrap_or(false)
}

fn is_publishable_raw_deterministic_guard(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.len() <= 2 {
        return false;
    }
    if crate::finalizer::looks_like_planner_artifact(t) {
        return false;
    }
    if t.chars()
        .all(|c| c.is_ascii_digit() || c.is_ascii_punctuation() || c.is_whitespace())
    {
        return false;
    }
    true
}

async fn classify_publishable_raw_with_llm(
    state: &AppState,
    task: &ClaimedTask,
    text: &str,
) -> Option<(bool, String, f64)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some((false, "empty".to_string(), 1.0));
    }
    let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
        state,
        PUBLISHABLE_RAW_CLASSIFIER_PROMPT_LOGICAL_PATH,
        PUBLISHABLE_RAW_CLASSIFIER_PROMPT_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(&prompt_template, &[("__TEXT__", trimmed)]);
    crate::log_prompt_render(
        state,
        &task.task_id,
        "publishable_raw_classifier_prompt",
        &prompt_source,
        None,
    );
    let llm_out =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await
            .ok()?;
    let trimmed_out = llm_out.trim();
    let parsed_raw = serde_json::from_str::<PublishableRawClassifierOut>(trimmed_out).ok();
    let parsed = parsed_raw.or_else(|| {
        crate::extract_first_json_object_any(&llm_out)
            .and_then(|json| serde_json::from_str::<PublishableRawClassifierOut>(&json).ok())
    })?;
    Some((
        parsed.publishable,
        parsed.reason,
        parsed.confidence.clamp(0.0, 1.0),
    ))
}

pub(crate) async fn is_publishable_raw(state: &AppState, task: &ClaimedTask, s: &str) -> bool {
    if !is_publishable_raw_deterministic_guard(s) {
        return false;
    }
    let trimmed = s.trim();
    if trimmed.chars().count() > 180 {
        return true;
    }
    classify_publishable_raw_with_llm(state, task, trimmed)
        .await
        .map(|(publishable, _reason, confidence)| {
            if confidence >= 0.55 {
                publishable
            } else {
                true
            }
        })
        .unwrap_or(true)
}

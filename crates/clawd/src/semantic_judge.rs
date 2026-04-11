use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use crate::{llm_gateway, AppState, ClaimedTask};

const DELIVERY_TEXT_CLASSIFIER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/delivery_text_classifier_prompt.md");
const DELIVERY_TEXT_CLASSIFIER_PROMPT_LOGICAL_PATH: &str =
    "prompts/delivery_text_classifier_prompt.md";

#[derive(Debug, Clone, Deserialize)]
struct DeliveryTextClassifierOut {
    #[serde(default)]
    is_meta_instruction: bool,
    #[serde(default)]
    meta_reason: String,
    #[serde(default)]
    meta_confidence: f64,
    #[serde(default)]
    publishable: bool,
    #[serde(default)]
    publishable_reason: String,
    #[serde(default)]
    publishable_confidence: f64,
}

fn delivery_text_classifier_cache() -> &'static Mutex<HashMap<String, DeliveryTextClassifierOut>> {
    static CACHE: OnceLock<Mutex<HashMap<String, DeliveryTextClassifierOut>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn delivery_text_cache_key(task: &ClaimedTask, text: &str) -> String {
    format!("{}\n{}", task.task_id, text.trim())
}

async fn classify_delivery_text_with_llm(
    state: &AppState,
    task: &ClaimedTask,
    text: &str,
) -> Option<DeliveryTextClassifierOut> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(DeliveryTextClassifierOut {
            is_meta_instruction: false,
            meta_reason: "empty".to_string(),
            meta_confidence: 1.0,
            publishable: false,
            publishable_reason: "empty".to_string(),
            publishable_confidence: 1.0,
        });
    }
    let cache_key = delivery_text_cache_key(task, trimmed);
    if let Ok(cache) = delivery_text_classifier_cache().lock() {
        if let Some(cached) = cache.get(&cache_key) {
            return Some(cached.clone());
        }
    }
    let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
        state,
        DELIVERY_TEXT_CLASSIFIER_PROMPT_LOGICAL_PATH,
        DELIVERY_TEXT_CLASSIFIER_PROMPT_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(&prompt_template, &[("__TEXT__", trimmed)]);
    crate::log_prompt_render(
        state,
        &task.task_id,
        "delivery_text_classifier_prompt",
        &prompt_source,
        None,
    );
    let llm_out =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await
            .ok()?;
    let trimmed_out = llm_out.trim();
    let parsed_raw = serde_json::from_str::<DeliveryTextClassifierOut>(trimmed_out).ok();
    let parsed = parsed_raw.or_else(|| {
        crate::extract_first_json_object_any(&llm_out)
            .and_then(|json| serde_json::from_str::<DeliveryTextClassifierOut>(&json).ok())
    })?;
    let normalized = DeliveryTextClassifierOut {
        is_meta_instruction: parsed.is_meta_instruction,
        meta_reason: parsed.meta_reason,
        meta_confidence: parsed.meta_confidence.clamp(0.0, 1.0),
        publishable: parsed.publishable,
        publishable_reason: parsed.publishable_reason,
        publishable_confidence: parsed.publishable_confidence.clamp(0.0, 1.0),
    };
    if let Ok(mut cache) = delivery_text_classifier_cache().lock() {
        if cache.len() >= 1024 {
            cache.clear();
        }
        cache.insert(cache_key, normalized.clone());
    }
    Some(normalized)
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
    classify_delivery_text_with_llm(state, task, trimmed)
        .await
        .map(|out| out.is_meta_instruction && out.meta_confidence >= 0.55)
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

pub(crate) async fn is_publishable_raw(state: &AppState, task: &ClaimedTask, s: &str) -> bool {
    if !is_publishable_raw_deterministic_guard(s) {
        return false;
    }
    let trimmed = s.trim();
    if trimmed.chars().count() > 180 {
        return true;
    }
    classify_delivery_text_with_llm(state, task, trimmed)
        .await
        .map(|out| {
            if out.publishable_confidence >= 0.55 {
                out.publishable
            } else {
                true
            }
        })
        .unwrap_or(true)
}

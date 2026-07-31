use std::collections::HashMap;
use std::fs;

use serde::Deserialize;

use crate::evidence_policy::FinalAnswerShapeClass;
use crate::{AppState, ClaimedTask, IntentOutputContract, OutputResponseShape};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalUserResult {
    pub(crate) text: String,
    pub(crate) messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersonaStyleResult {
    pub(crate) text: String,
    pub(crate) messages: Vec<String>,
    pub(crate) applied: bool,
    pub(crate) reason: &'static str,
}

pub(crate) struct PersonaStyleRenderer;

impl PersonaStyleRenderer {
    pub(crate) fn render(
        state: &AppState,
        task: &ClaimedTask,
        prompt: &str,
        route: &IntentOutputContract,
        canonical: CanonicalUserResult,
    ) -> PersonaStyleResult {
        let agent = state.task_agent(task);
        let profile = agent.persona_profile.as_str();
        let fallback = |reason| PersonaStyleResult {
            text: canonical.text.clone(),
            messages: canonical.messages.clone(),
            applied: false,
            reason,
        };

        if matches!(profile, "inherit" | "executor") {
            return fallback("profile_noop");
        }
        if !persona_render_eligible(route, &canonical) {
            return fallback("exact_or_structured_bypass");
        }
        let language = crate::language_policy::task_response_language_hint(state, task, prompt);
        let prefix = persona_prefix(
            state,
            profile,
            language.to_ascii_lowercase().starts_with("en"),
        );
        if prefix.is_empty() {
            return fallback("empty_style_prefix");
        }
        let rendered_text = format!("{prefix}\n\n{}", canonical.text);
        let rendered_messages = if canonical.messages.len() == 1
            && canonical.messages[0].trim() == canonical.text.trim()
        {
            vec![rendered_text.clone()]
        } else {
            canonical.messages.clone()
        };
        if !persona_render_is_semantically_safe(&canonical.text, &rendered_text, &prefix) {
            return fallback("semantic_validation_fallback");
        }
        tracing::info!(
            agent_id = %agent.id,
            profile = %agent.persona_profile,
            persona_chars = agent.persona_fragment.chars().count(),
            persona_digest = %agent.persona_digest,
            result = "applied",
            "persona_style_render"
        );
        PersonaStyleResult {
            text: rendered_text,
            messages: rendered_messages,
            applied: true,
            reason: "applied",
        }
    }
}

fn persona_render_eligible(route: &IntentOutputContract, canonical: &CanonicalUserResult) -> bool {
    if route.delivery_required || route.response_shape != OutputResponseShape::Free {
        return false;
    }
    if crate::evidence_policy::final_answer_shape_for_output_contract(route)
        .is_some_and(|shape| shape.class() != FinalAnswerShapeClass::Freeform)
    {
        return false;
    }
    let text = canonical.text.trim();
    if text.is_empty() || canonical.messages.len() > 1 {
        return false;
    }
    if serde_json::from_str::<serde_json::Value>(text).is_ok()
        || text.contains("```")
        || text.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("diff --git ")
                || trimmed.starts_with("@@ ")
                || trimmed.starts_with("+++ ")
                || trimmed.starts_with("--- ")
        })
        || looks_like_table(text)
        || looks_like_exact_scalar(text)
        || !crate::extract_delivery_file_tokens(text).is_empty()
    {
        return false;
    }
    true
}

fn looks_like_table(text: &str) -> bool {
    let rows = text
        .lines()
        .filter(|line| line.matches('|').count() >= 2)
        .count();
    rows >= 2
}

fn looks_like_exact_scalar(text: &str) -> bool {
    if text.lines().count() != 1 || text.chars().count() > 128 {
        return false;
    }
    let has_sentence_punctuation = text.contains('。')
        || text.contains('！')
        || text.contains('？')
        || text.contains(". ")
        || text.contains("! ")
        || text.contains("? ");
    let token_like = !text.chars().any(char::is_whitespace)
        || text.starts_with('/')
        || text.starts_with("http://")
        || text.starts_with("https://")
        || text.parse::<f64>().is_ok();
    token_like && !has_sentence_punctuation
}

#[derive(Debug, Deserialize)]
struct PersonaStyleMessages {
    schema_version: u32,
    #[serde(default)]
    profiles: HashMap<String, PersonaStyleMessage>,
}

#[derive(Debug, Deserialize)]
struct PersonaStyleMessage {
    zh_cn: String,
    en: String,
}

fn persona_prefix(state: &AppState, profile: &str, english: bool) -> String {
    let path = state
        .skill_rt
        .workspace_root
        .join("configs/persona_style_messages.toml");
    let Some(messages) = fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<PersonaStyleMessages>(&raw).ok())
        .filter(|messages| messages.schema_version == 1)
    else {
        return String::new();
    };
    messages
        .profiles
        .get(profile)
        .map(|message| if english { &message.en } else { &message.zh_cn })
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

fn persona_render_is_semantically_safe(canonical: &str, rendered: &str, prefix: &str) -> bool {
    rendered
        .strip_prefix(prefix)
        .and_then(|tail| tail.strip_prefix("\n\n"))
        .is_some_and(|body| body == canonical)
        && protected_tokens(canonical) == protected_tokens(rendered)
}

fn protected_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|token| {
            token.chars().any(|character| character.is_ascii_digit())
                || token.starts_with("http://")
                || token.starts_with("https://")
                || token.starts_with('/')
                || token.contains("::")
                || token.contains('=')
                || token.starts_with('`')
        })
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
#[path = "persona_style_tests.rs"]
mod tests;

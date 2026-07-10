use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct ObservedAnswerFallbackOut {
    #[serde(default)]
    pub(super) answer: String,
    #[serde(default)]
    pub(super) qualified: bool,
    #[serde(default)]
    pub(super) needs_clarify: bool,
    #[serde(default)]
    pub(super) is_meta_instruction: bool,
    #[serde(default)]
    pub(super) publishable: bool,
    #[serde(default)]
    pub(super) confidence: f64,
    #[serde(default, rename = "reason")]
    pub(super) _reason: String,
}

pub(super) fn strip_bare_json_language_prefix(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(rest) = trimmed
        .strip_prefix("json")
        .or_else(|| trimmed.strip_prefix("JSON"))
    else {
        return trimmed;
    };
    let rest = rest.trim_start();
    if rest.starts_with('{') || rest.starts_with('[') {
        rest
    } else {
        trimmed
    }
}

pub(super) fn extract_answer_from_finalizer_envelope_text(raw: &str) -> Option<String> {
    let candidate = strip_bare_json_language_prefix(raw);
    crate::prompt_utils::validate_against_schema::<ObservedAnswerFallbackOut>(
        candidate,
        crate::prompt_utils::PromptSchemaId::FinalizerOut,
    )
    .ok()
    .map(|validated| validated.value.answer.trim().to_string())
    .filter(|answer| !answer.is_empty())
}

pub(super) fn non_code_markdown_text(raw: &str) -> Option<String> {
    let mut in_fence = false;
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

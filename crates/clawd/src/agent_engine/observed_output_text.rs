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

pub(super) fn freeform_observed_answer_fallback(raw: &str) -> Option<ObservedAnswerFallbackOut> {
    let trimmed_owned;
    let trimmed = if let Some(non_code_text) = non_code_markdown_text(raw) {
        trimmed_owned = non_code_text;
        trimmed_owned.trim()
    } else {
        raw.trim().trim_matches('`').trim()
    };
    if trimmed.is_empty() || trimmed.starts_with('{') || trimmed.starts_with('[') {
        return None;
    }
    Some(ObservedAnswerFallbackOut {
        answer: trimmed.to_string(),
        qualified: true,
        needs_clarify: false,
        is_meta_instruction: false,
        publishable: true,
        confidence: 0.7,
        _reason: String::from("freeform_text_fallback"),
    })
}

pub(super) fn non_code_markdown_text(raw: &str) -> Option<String> {
    let mut in_fence = false;
    let mut fence_lang = String::new();
    let mut fence_lines = Vec::new();
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") {
            if in_fence {
                if markdown_fence_body_is_publishable(&fence_lang, &fence_lines) {
                    lines.extend(
                        fence_lines
                            .iter()
                            .map(|line| line.trim())
                            .filter(|line| !line.is_empty())
                            .map(ToString::to_string),
                    );
                }
                fence_lang.clear();
                fence_lines.clear();
                in_fence = false;
            } else {
                fence_lang = trimmed_start
                    .trim_start_matches('`')
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            fence_lines.push(line.to_string());
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    if in_fence && markdown_fence_body_is_publishable(&fence_lang, &fence_lines) {
        lines.extend(
            fence_lines
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .map(ToString::to_string),
        );
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn markdown_fence_body_is_publishable(lang: &str, lines: &[String]) -> bool {
    matches!(lang, "markdown" | "md" | "gfm") && lines.iter().any(|line| !line.trim().is_empty())
}

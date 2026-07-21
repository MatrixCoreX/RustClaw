#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenEstimatorKind {
    MiniMaxM2,
    OpenAiCompatible,
    AnthropicCompatible,
    GenericUnicode,
}

impl TokenEstimatorKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::MiniMaxM2 => "minimax_m2_estimate_v1",
            Self::OpenAiCompatible => "openai_compatible_estimate_v1",
            Self::AnthropicCompatible => "anthropic_compatible_estimate_v1",
            Self::GenericUnicode => "generic_unicode_estimate_v1",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TokenEstimate {
    pub(crate) estimator: TokenEstimatorKind,
    pub(crate) provider_tokens: usize,
    pub(crate) safety_tokens: usize,
    pub(crate) byte_count: usize,
    pub(crate) char_count: usize,
}

pub(crate) fn estimate_provider_tokens(
    provider_name: &str,
    provider_type: &str,
    model: &str,
    text: &str,
) -> TokenEstimate {
    estimate_tokens(estimator_kind(provider_name, provider_type, model), text)
}

pub(crate) fn estimate_generic_tokens(text: &str) -> TokenEstimate {
    estimate_tokens(TokenEstimatorKind::GenericUnicode, text)
}

fn estimator_kind(provider_name: &str, provider_type: &str, model: &str) -> TokenEstimatorKind {
    let provider_name = provider_name.to_ascii_lowercase();
    let provider_type = provider_type.to_ascii_lowercase();
    let model = model.to_ascii_lowercase();
    if provider_name.contains("minimax") || model.contains("minimax") {
        TokenEstimatorKind::MiniMaxM2
    } else if provider_name.contains("anthropic")
        || provider_type.contains("anthropic")
        || model.contains("claude")
    {
        TokenEstimatorKind::AnthropicCompatible
    } else if provider_type.contains("openai")
        || provider_name.contains("openai")
        || model.starts_with("gpt-")
    {
        TokenEstimatorKind::OpenAiCompatible
    } else {
        TokenEstimatorKind::GenericUnicode
    }
}

fn estimate_tokens(estimator: TokenEstimatorKind, text: &str) -> TokenEstimate {
    let mut ascii_bytes = 0usize;
    let mut cjk_chars = 0usize;
    let mut other_unicode_chars = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii_bytes = ascii_bytes.saturating_add(1);
        } else if is_cjk_like(ch) {
            cjk_chars = cjk_chars.saturating_add(1);
        } else {
            other_unicode_chars = other_unicode_chars.saturating_add(1);
        }
    }

    let ascii_tokens = ceil_div(ascii_bytes, 4);
    let cjk_tokens = match estimator {
        // MiniMax's public pricing guide estimates about 1,000 tokens per
        // 1,600 Chinese characters. The runtime still uses the more
        // conservative safety estimate for admission.
        TokenEstimatorKind::MiniMaxM2 => ceil_div(cjk_chars.saturating_mul(5), 8),
        TokenEstimatorKind::OpenAiCompatible
        | TokenEstimatorKind::AnthropicCompatible
        | TokenEstimatorKind::GenericUnicode => cjk_chars,
    };
    let provider_tokens = ascii_tokens
        .saturating_add(cjk_tokens)
        .saturating_add(other_unicode_chars)
        .max(usize::from(!text.is_empty()));
    let byte_character_safety = ceil_div(ascii_bytes, 3)
        .saturating_add(cjk_chars)
        .saturating_add(other_unicode_chars);
    let safety_tokens = provider_tokens
        .max(byte_character_safety)
        .max(usize::from(!text.is_empty()));

    TokenEstimate {
        estimator,
        provider_tokens,
        safety_tokens,
        byte_count: text.len(),
        char_count: text.chars().count(),
    }
}

fn ceil_div(value: usize, divisor: usize) -> usize {
    value.saturating_add(divisor.saturating_sub(1)) / divisor
}

fn is_cjk_like(ch: char) -> bool {
    matches!(
        ch as u32,
        0x2E80..=0x2FFF
            | 0x3040..=0x30FF
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2FA1F
    )
}

#[cfg(test)]
#[path = "token_estimator_tests.rs"]
mod tests;

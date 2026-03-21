use super::client::LlmUsageSnapshot;
use serde_json::Value;

fn value_as_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
    })
}

fn sum_u64(parts: &[Option<u64>]) -> Option<u64> {
    let mut total = 0u64;
    let mut seen = false;
    for part in parts {
        if let Some(value) = part {
            total = total.saturating_add(*value);
            seen = true;
        }
    }
    seen.then_some(total)
}

pub(crate) fn openai_usage_snapshot(value: &Value) -> Option<LlmUsageSnapshot> {
    let usage = value.get("usage")?;
    let prompt_tokens = value_as_u64(usage.get("prompt_tokens"));
    let completion_tokens = value_as_u64(usage.get("completion_tokens"));
    let total_tokens = value_as_u64(usage.get("total_tokens"))
        .or_else(|| sum_u64(&[prompt_tokens, completion_tokens]));
    let reasoning_tokens = value_as_u64(
        usage
            .get("completion_tokens_details")
            .and_then(|details| details.get("reasoning_tokens")),
    );
    let cached_tokens = value_as_u64(
        usage
            .get("prompt_tokens_details")
            .and_then(|details| details.get("cached_tokens")),
    );
    if prompt_tokens.is_none()
        && completion_tokens.is_none()
        && total_tokens.is_none()
        && reasoning_tokens.is_none()
        && cached_tokens.is_none()
    {
        return None;
    }
    Some(LlmUsageSnapshot {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        input_tokens: None,
        output_tokens: None,
        reasoning_tokens,
        cached_tokens,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    })
}

pub(crate) fn gemini_usage_snapshot(value: &Value) -> Option<LlmUsageSnapshot> {
    let usage = value.get("usageMetadata")?;
    let prompt_tokens = value_as_u64(usage.get("promptTokenCount"));
    let completion_tokens = value_as_u64(usage.get("candidatesTokenCount"));
    let total_tokens = value_as_u64(usage.get("totalTokenCount"))
        .or_else(|| sum_u64(&[prompt_tokens, completion_tokens]));
    let reasoning_tokens = value_as_u64(usage.get("thoughtsTokenCount"));
    let cached_tokens = value_as_u64(usage.get("cachedContentTokenCount"));
    if prompt_tokens.is_none()
        && completion_tokens.is_none()
        && total_tokens.is_none()
        && reasoning_tokens.is_none()
        && cached_tokens.is_none()
    {
        return None;
    }
    Some(LlmUsageSnapshot {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        input_tokens: None,
        output_tokens: None,
        reasoning_tokens,
        cached_tokens,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    })
}

pub(crate) fn anthropic_usage_snapshot(value: &Value) -> Option<LlmUsageSnapshot> {
    let usage = value.get("usage")?;
    let input_tokens = value_as_u64(usage.get("input_tokens"));
    let output_tokens = value_as_u64(usage.get("output_tokens"));
    let cache_creation_input_tokens = value_as_u64(usage.get("cache_creation_input_tokens"));
    let cache_read_input_tokens = value_as_u64(usage.get("cache_read_input_tokens"));
    let total_tokens = sum_u64(&[input_tokens, output_tokens]);
    if input_tokens.is_none()
        && output_tokens.is_none()
        && cache_creation_input_tokens.is_none()
        && cache_read_input_tokens.is_none()
    {
        return None;
    }
    Some(LlmUsageSnapshot {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        total_tokens,
        input_tokens,
        output_tokens,
        reasoning_tokens: None,
        cached_tokens: None,
        cache_creation_input_tokens,
        cache_read_input_tokens,
    })
}

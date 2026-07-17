use std::collections::BTreeSet;

use claw_core::config::LlmModelPricingConfig;
use serde::{Deserialize, Serialize};

use super::client::LlmUsageSnapshot;

const TOKENS_PER_MILLION: u128 = 1_000_000;
const NANOS_PER_USD: f64 = 1_000_000_000.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct LlmCallCostRecord {
    pub(crate) logical_call_index: u64,
    pub(crate) prompt_label: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) provider_status: String,
    pub(crate) provider_attempts: u64,
    pub(crate) usage: Option<LlmUsageSnapshot>,
    pub(crate) cost_status: String,
    pub(crate) unknown_reason: Option<String>,
    pub(crate) estimated_cost_usd_nanos: Option<u64>,
    pub(crate) pricing_effective_from: Option<String>,
    pub(crate) pricing_source: Option<String>,
    pub(crate) pricing_currency: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LlmTaskCostSummary {
    pub(crate) status: String,
    pub(crate) logical_call_count: u64,
    pub(crate) covered_logical_call_count: u64,
    pub(crate) provider_record_count: u64,
    pub(crate) usage_record_count: u64,
    pub(crate) priced_record_count: u64,
    pub(crate) unknown_record_count: u64,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) cache_write_input_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) estimated_cost_usd_nanos: u64,
    pub(crate) unknown_reasons: Vec<String>,
}

pub(crate) fn resolve_model_pricing(
    catalog: &[LlmModelPricingConfig],
    provider_name: &str,
    provider_type: &str,
    model: &str,
) -> Option<LlmModelPricingConfig> {
    let provider_alias = provider_name
        .trim()
        .strip_prefix("vendor-")
        .unwrap_or(provider_name.trim());
    catalog
        .iter()
        .find(|entry| {
            entry.model.trim().eq_ignore_ascii_case(model.trim())
                && [provider_name.trim(), provider_alias, provider_type.trim()]
                    .iter()
                    .any(|candidate| entry.provider.trim().eq_ignore_ascii_case(candidate))
        })
        .cloned()
}

pub(crate) fn build_cost_record(
    logical_call_index: u64,
    prompt_label: &str,
    provider: &str,
    model: &str,
    provider_status: &str,
    provider_attempts: usize,
    usage: Option<&LlmUsageSnapshot>,
    pricing: Option<&LlmModelPricingConfig>,
) -> LlmCallCostRecord {
    let mut record = LlmCallCostRecord {
        logical_call_index,
        prompt_label: prompt_label.to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        provider_status: provider_status.to_string(),
        provider_attempts: provider_attempts.max(1) as u64,
        usage: usage.cloned(),
        cost_status: "unknown".to_string(),
        unknown_reason: None,
        estimated_cost_usd_nanos: None,
        pricing_effective_from: pricing.map(|value| value.effective_from.clone()),
        pricing_source: pricing.and_then(|value| value.source.clone()),
        pricing_currency: pricing.map(|value| value.currency.clone()),
    };
    let Some(usage) = usage else {
        record.unknown_reason = Some("usage_unavailable".to_string());
        return record;
    };
    let Some(pricing) = pricing else {
        record.unknown_reason = Some("pricing_not_configured".to_string());
        return record;
    };
    if !pricing.currency.trim().eq_ignore_ascii_case("USD") {
        record.unknown_reason = Some("unsupported_pricing_currency".to_string());
        return record;
    }
    match estimate_cost_usd_nanos(usage, pricing) {
        Ok(value) => {
            record.cost_status = "known".to_string();
            record.estimated_cost_usd_nanos = Some(value);
        }
        Err(reason) => record.unknown_reason = Some(reason.to_string()),
    }
    record
}

pub(crate) fn summarize_task_cost(
    logical_call_count: u64,
    records: &[LlmCallCostRecord],
) -> LlmTaskCostSummary {
    let mut summary = LlmTaskCostSummary {
        logical_call_count,
        provider_record_count: records.len() as u64,
        ..LlmTaskCostSummary::default()
    };
    let mut covered_calls = BTreeSet::new();
    let mut unknown_reasons = BTreeSet::new();
    for record in records {
        covered_calls.insert(record.logical_call_index);
        if let Some(usage) = record.usage.as_ref() {
            summary.usage_record_count = summary.usage_record_count.saturating_add(1);
            summary.input_tokens = summary
                .input_tokens
                .saturating_add(usage.input_tokens.or(usage.prompt_tokens).unwrap_or(0));
            summary.output_tokens = summary
                .output_tokens
                .saturating_add(usage.output_tokens.or(usage.completion_tokens).unwrap_or(0));
            summary.cached_input_tokens = summary.cached_input_tokens.saturating_add(
                usage
                    .cache_read_input_tokens
                    .or(usage.cached_tokens)
                    .unwrap_or(0),
            );
            summary.cache_write_input_tokens = summary
                .cache_write_input_tokens
                .saturating_add(usage.cache_creation_input_tokens.unwrap_or(0));
            summary.reasoning_tokens = summary
                .reasoning_tokens
                .saturating_add(usage.reasoning_tokens.unwrap_or(0));
        }
        if let Some(cost) = record.estimated_cost_usd_nanos {
            summary.priced_record_count = summary.priced_record_count.saturating_add(1);
            summary.estimated_cost_usd_nanos =
                summary.estimated_cost_usd_nanos.saturating_add(cost);
        } else {
            summary.unknown_record_count = summary.unknown_record_count.saturating_add(1);
            if let Some(reason) = record.unknown_reason.as_deref() {
                unknown_reasons.insert(reason.to_string());
            }
        }
    }
    summary.covered_logical_call_count = covered_calls.len() as u64;
    if logical_call_count == 0 {
        summary.status = "not_applicable".to_string();
    } else if summary.covered_logical_call_count < logical_call_count {
        summary.status = "unknown".to_string();
        unknown_reasons.insert("logical_call_not_observed".to_string());
    } else if summary.unknown_record_count > 0 {
        summary.status = "unknown".to_string();
    } else {
        summary.status = "known".to_string();
    }
    summary.unknown_reasons = unknown_reasons.into_iter().collect();
    summary
}

fn estimate_cost_usd_nanos(
    usage: &LlmUsageSnapshot,
    pricing: &LlmModelPricingConfig,
) -> Result<u64, &'static str> {
    let input_tokens = usage.input_tokens.or(usage.prompt_tokens).unwrap_or(0);
    let output_tokens = usage.output_tokens.or(usage.completion_tokens).unwrap_or(0);
    let cached_tokens = usage
        .cache_read_input_tokens
        .or(usage.cached_tokens)
        .unwrap_or(0);
    let cache_write_tokens = usage.cache_creation_input_tokens.unwrap_or(0);
    let reasoning_tokens = usage.reasoning_tokens.unwrap_or(0).min(output_tokens);
    let regular_input_tokens = if usage.cache_read_input_tokens.is_some() {
        input_tokens
    } else {
        input_tokens.saturating_sub(cached_tokens)
    };
    let total_context_tokens = regular_input_tokens
        .saturating_add(cached_tokens)
        .saturating_add(cache_write_tokens);
    let long_context = pricing
        .long_context_threshold_tokens
        .is_some_and(|threshold| total_context_tokens > threshold);
    let input_rate = if long_context {
        pricing
            .long_context_input_usd_per_million
            .or(pricing.input_usd_per_million)
    } else {
        pricing.input_usd_per_million
    };
    let output_rate = if long_context {
        pricing
            .long_context_output_usd_per_million
            .or(pricing.output_usd_per_million)
    } else {
        pricing.output_usd_per_million
    };
    let cache_read_rate = if long_context {
        pricing
            .long_context_cache_read_usd_per_million
            .or(pricing.cache_read_usd_per_million)
    } else {
        pricing.cache_read_usd_per_million
    };
    let mut total = component_cost(regular_input_tokens, input_rate)?;
    let separately_priced_reasoning = pricing.reasoning_usd_per_million.is_some();
    total = total.saturating_add(component_cost(
        if separately_priced_reasoning {
            output_tokens.saturating_sub(reasoning_tokens)
        } else {
            output_tokens
        },
        output_rate,
    )?);
    total = total.saturating_add(component_cost(cached_tokens, cache_read_rate)?);
    total = total.saturating_add(component_cost(
        cache_write_tokens,
        pricing.cache_write_usd_per_million,
    )?);
    if separately_priced_reasoning {
        total = total.saturating_add(component_cost(
            reasoning_tokens,
            pricing.reasoning_usd_per_million,
        )?);
    }
    u64::try_from(total).map_err(|_| "cost_overflow")
}

fn component_cost(tokens: u64, rate: Option<f64>) -> Result<u128, &'static str> {
    if tokens == 0 {
        return Ok(0);
    }
    let rate = rate.ok_or("pricing_component_missing")?;
    if !rate.is_finite() || rate < 0.0 {
        return Err("pricing_rate_invalid");
    }
    let rate_nanos = (rate * NANOS_PER_USD).round();
    if rate_nanos > u64::MAX as f64 {
        return Err("pricing_rate_invalid");
    }
    Ok((u128::from(tokens) * rate_nanos as u128 + TOKENS_PER_MILLION / 2) / TOKENS_PER_MILLION)
}

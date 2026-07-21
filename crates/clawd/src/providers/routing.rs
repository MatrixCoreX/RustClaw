use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::providers::{ChatRequestHints, LlmRoutingPreference};
use crate::LlmProviderRuntime;

#[derive(Debug, Default)]
pub(crate) struct LlmProviderLatencyTracker {
    inner: Mutex<LatencyState>,
}

#[derive(Debug, Default)]
struct LatencyState {
    sample_count: u64,
    ewma_ms: Option<u64>,
}

impl LlmProviderLatencyTracker {
    pub(crate) fn note_sample(&self, elapsed_ms: u64) {
        let mut inner = self.inner.lock().expect("provider latency mutex poisoned");
        inner.sample_count = inner.sample_count.saturating_add(1);
        inner.ewma_ms = Some(match inner.ewma_ms {
            Some(current) => {
                current
                    .saturating_mul(7)
                    .saturating_add(elapsed_ms.saturating_mul(3))
                    / 10
            }
            None => elapsed_ms,
        });
    }

    pub(crate) fn snapshot(&self) -> (u64, Option<u64>) {
        let inner = self.inner.lock().expect("provider latency mutex poisoned");
        (inner.sample_count, inner.ewma_ms)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct LlmProviderRouteEvaluation {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) eligible: bool,
    pub(crate) exclusion_codes: Vec<String>,
    pub(crate) rank: Option<u64>,
    pub(crate) breaker_state: String,
    pub(crate) required_context_window_tokens: usize,
    pub(crate) estimated_prompt_tokens: usize,
    pub(crate) prompt_token_estimator: String,
    pub(crate) prompt_byte_count: usize,
    pub(crate) prompt_char_count: usize,
    pub(crate) context_window_tokens: Option<usize>,
    pub(crate) input_modalities: Vec<String>,
    pub(crate) native_tools: bool,
    pub(crate) latency_sample_count: u64,
    pub(crate) routing_latency_ms: u64,
    pub(crate) price_score_microusd_per_million: Option<u64>,
    pub(crate) static_priority: i32,
}

pub(crate) struct LlmProviderRoutingPlan {
    pub(crate) providers: Vec<Arc<LlmProviderRuntime>>,
    pub(crate) evaluations: Vec<LlmProviderRouteEvaluation>,
}

pub(crate) fn route_providers(
    providers: Vec<Arc<LlmProviderRuntime>>,
    prompt: &str,
    hints: &ChatRequestHints,
) -> LlmProviderRoutingPlan {
    let required_modalities = normalized_modalities(&hints.required_input_modalities);
    let mut candidates = providers
        .into_iter()
        .map(|provider| {
            let prompt_estimate = crate::token_estimator::estimate_provider_tokens(
                &provider.config.name,
                &provider.config.provider_type,
                &provider.config.model,
                prompt,
            );
            let required_context_window_tokens = prompt_estimate
                .safety_tokens
                .saturating_add(hints.max_tokens.unwrap_or(0) as usize)
                .max(hints.minimum_context_window_tokens.unwrap_or(0));
            let breaker = provider.breaker.snapshot();
            let (latency_sample_count, observed_latency_ms) = provider.latency.snapshot();
            let routing_latency_ms = observed_latency_ms
                .or(provider.config.expected_latency_ms)
                .unwrap_or_else(|| provider.config.timeout_seconds.saturating_mul(1_000));
            let input_modalities = normalized_modalities(&provider.config.input_modalities);
            let mut exclusion_codes = Vec::new();
            if !required_modalities
                .iter()
                .all(|required| input_modalities.iter().any(|actual| actual == required))
            {
                exclusion_codes.push("required_input_modality_unsupported".to_string());
            }
            let model_capabilities = provider.model_capabilities();
            if hints.requires_native_tools && !model_capabilities.native_tools {
                exclusion_codes.push("native_tools_required".to_string());
            }
            if provider
                .config
                .context_window_tokens
                .is_some_and(|capacity| capacity < required_context_window_tokens)
            {
                exclusion_codes.push("context_window_insufficient".to_string());
            }
            let price_score = price_score(provider.pricing.as_ref());
            let evaluation = LlmProviderRouteEvaluation {
                provider: provider.config.name.clone(),
                model: provider.config.model.clone(),
                eligible: exclusion_codes.is_empty(),
                exclusion_codes,
                rank: None,
                breaker_state: breaker.state,
                required_context_window_tokens,
                estimated_prompt_tokens: prompt_estimate.provider_tokens,
                prompt_token_estimator: prompt_estimate.estimator.as_str().to_string(),
                prompt_byte_count: prompt_estimate.byte_count,
                prompt_char_count: prompt_estimate.char_count,
                context_window_tokens: provider.config.context_window_tokens,
                input_modalities,
                native_tools: model_capabilities.native_tools,
                latency_sample_count,
                routing_latency_ms,
                price_score_microusd_per_million: price_score,
                static_priority: provider.config.priority,
            };
            (provider, evaluation)
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        route_rank_key(&left.1, hints.routing_preference)
            .cmp(&route_rank_key(&right.1, hints.routing_preference))
    });
    let mut ordered = Vec::new();
    for (provider, evaluation) in &mut candidates {
        if evaluation.eligible {
            evaluation.rank = Some(ordered.len() as u64 + 1);
            ordered.push(provider.clone());
        }
    }
    LlmProviderRoutingPlan {
        providers: ordered,
        evaluations: candidates
            .into_iter()
            .map(|(_, evaluation)| evaluation)
            .collect(),
    }
}

fn route_rank_key(
    evaluation: &LlmProviderRouteEvaluation,
    preference: LlmRoutingPreference,
) -> (u8, u8, u64, u64, i32, String) {
    let eligibility = u8::from(!evaluation.eligible);
    let breaker = match evaluation.breaker_state.as_str() {
        "closed" => 0,
        "half_open" => 1,
        _ => 2,
    };
    let price = evaluation
        .price_score_microusd_per_million
        .unwrap_or(u64::MAX / 4);
    let latency = evaluation.routing_latency_ms;
    let (primary, secondary) = match preference {
        LlmRoutingPreference::LowCost => (price, latency),
        LlmRoutingPreference::LowLatency => (latency, price),
        LlmRoutingPreference::Balanced => (price.saturating_add(latency), latency),
    };
    (
        eligibility,
        breaker,
        primary,
        secondary,
        evaluation.static_priority,
        evaluation.provider.clone(),
    )
}

fn normalized_modalities(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

fn price_score(pricing: Option<&claw_core::config::LlmModelPricingConfig>) -> Option<u64> {
    let pricing = pricing?;
    let input = pricing.input_usd_per_million?;
    let output = pricing.output_usd_per_million?;
    let combined = input + output;
    if !combined.is_finite() || combined < 0.0 {
        return None;
    }
    Some((combined * 1_000_000.0).round().min(u64::MAX as f64) as u64)
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;

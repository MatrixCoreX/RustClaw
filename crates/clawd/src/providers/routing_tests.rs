use std::sync::Arc;

use claw_core::config::{LlmModelPricingConfig, LlmProviderConfig, LlmProviderParams};
use tokio::sync::Semaphore;

use super::*;

fn provider(
    name: &str,
    priority: i32,
    context_window_tokens: Option<usize>,
    modalities: &[&str],
    supports_tools: bool,
    expected_latency_ms: u64,
    combined_price_per_million: Option<f64>,
) -> Arc<LlmProviderRuntime> {
    Arc::new(LlmProviderRuntime {
        config: LlmProviderConfig {
            name: name.to_string(),
            provider_type: "openai_compat".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            api_key: "test".to_string(),
            model: format!("{name}-model"),
            context_window_tokens,
            input_modalities: modalities.iter().map(|value| value.to_string()).collect(),
            supports_tools,
            expected_latency_ms: Some(expected_latency_ms),
            priority,
            timeout_seconds: 30,
            max_concurrency: 1,
            params: LlmProviderParams::default(),
        },
        pricing: combined_price_per_million.map(|price| LlmModelPricingConfig {
            provider: name.to_string(),
            model: format!("{name}-model"),
            effective_from: "2026-07-18".to_string(),
            currency: "USD".to_string(),
            source: None,
            input_usd_per_million: Some(price / 2.0),
            output_usd_per_million: Some(price / 2.0),
            cache_read_usd_per_million: None,
            cache_write_usd_per_million: None,
            reasoning_usd_per_million: None,
            long_context_threshold_tokens: None,
            long_context_input_usd_per_million: None,
            long_context_output_usd_per_million: None,
            long_context_cache_read_usd_per_million: None,
        }),
        latency: Arc::new(LlmProviderLatencyTracker::default()),
        client: reqwest::Client::new(),
        semaphore: Arc::new(Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    })
}

#[test]
fn routing_filters_incompatible_modality_context_and_tool_contracts() {
    let text_only = provider(
        "text-only",
        1,
        Some(1_000_000),
        &["text"],
        true,
        100,
        Some(1.0),
    );
    let short_context = provider(
        "short-context",
        2,
        Some(100),
        &["text", "image"],
        true,
        100,
        Some(1.0),
    );
    let no_tools = provider(
        "no-tools",
        3,
        Some(1_000_000),
        &["text", "image"],
        false,
        100,
        Some(1.0),
    );
    let eligible = provider(
        "eligible",
        4,
        Some(1_000_000),
        &["text", "image"],
        true,
        100,
        Some(1.0),
    );
    let hints = ChatRequestHints {
        required_input_modalities: vec!["image".to_string()],
        minimum_context_window_tokens: Some(10_000),
        requires_native_tools: true,
        ..Default::default()
    };

    let plan = route_providers(
        vec![text_only, short_context, no_tools, eligible],
        1_000,
        &hints,
    );

    assert_eq!(plan.providers.len(), 1);
    assert_eq!(plan.providers[0].config.name, "eligible");
    assert!(plan.evaluations.iter().any(|evaluation| {
        evaluation.provider == "text-only"
            && evaluation
                .exclusion_codes
                .iter()
                .any(|code| code == "required_input_modality_unsupported")
    }));
    assert!(plan.evaluations.iter().any(|evaluation| {
        evaluation.provider == "short-context"
            && evaluation
                .exclusion_codes
                .iter()
                .any(|code| code == "context_window_insufficient")
    }));
    assert!(plan.evaluations.iter().any(|evaluation| {
        evaluation.provider == "no-tools"
            && evaluation
                .exclusion_codes
                .iter()
                .any(|code| code == "native_tools_required")
    }));
}

#[test]
fn low_cost_and_low_latency_preferences_use_machine_metadata() {
    let cheap_slow = provider(
        "cheap-slow",
        9,
        Some(1_000_000),
        &["text"],
        true,
        5_000,
        Some(0.5),
    );
    let costly_fast = provider(
        "costly-fast",
        1,
        Some(1_000_000),
        &["text"],
        true,
        100,
        Some(5.0),
    );

    let low_cost = route_providers(
        vec![costly_fast.clone(), cheap_slow.clone()],
        1_000,
        &ChatRequestHints {
            routing_preference: LlmRoutingPreference::LowCost,
            ..Default::default()
        },
    );
    let low_latency = route_providers(
        vec![cheap_slow, costly_fast],
        1_000,
        &ChatRequestHints {
            routing_preference: LlmRoutingPreference::LowLatency,
            ..Default::default()
        },
    );

    assert_eq!(low_cost.providers[0].config.name, "cheap-slow");
    assert_eq!(low_latency.providers[0].config.name, "costly-fast");
}

#[test]
fn routing_uses_observed_latency_and_places_open_breaker_last() {
    let observed_fast = provider(
        "observed-fast",
        2,
        Some(1_000_000),
        &["text"],
        true,
        9_000,
        Some(1.0),
    );
    observed_fast.latency.note_sample(50);
    let open_breaker = provider(
        "open-breaker",
        1,
        Some(1_000_000),
        &["text"],
        true,
        10,
        Some(0.1),
    );
    for _ in 0..3 {
        open_breaker.breaker.note_failure();
    }

    let plan = route_providers(
        vec![open_breaker, observed_fast],
        1_000,
        &ChatRequestHints {
            routing_preference: LlmRoutingPreference::LowLatency,
            ..Default::default()
        },
    );

    assert_eq!(plan.providers[0].config.name, "observed-fast");
    let evaluation = plan
        .evaluations
        .iter()
        .find(|evaluation| evaluation.provider == "observed-fast")
        .expect("observed provider evaluation");
    assert_eq!(evaluation.latency_sample_count, 1);
    assert_eq!(evaluation.routing_latency_ms, 50);
}

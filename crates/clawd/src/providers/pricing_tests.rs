use claw_core::config::LlmModelPricingConfig;

use super::client::LlmUsageSnapshot;
use super::pricing::{
    build_cost_record, resolve_model_pricing, summarize_task_cost, LlmCallCostRecord,
};

fn pricing() -> LlmModelPricingConfig {
    LlmModelPricingConfig {
        provider: "minimax".to_string(),
        model: "MiniMax-M3".to_string(),
        effective_from: "2026-07-18".to_string(),
        currency: "USD".to_string(),
        source: Some("https://example.invalid/pricing".to_string()),
        input_usd_per_million: Some(0.30),
        output_usd_per_million: Some(1.20),
        cache_read_usd_per_million: Some(0.06),
        cache_write_usd_per_million: None,
        reasoning_usd_per_million: None,
        long_context_threshold_tokens: Some(512_000),
        long_context_input_usd_per_million: Some(0.60),
        long_context_output_usd_per_million: Some(2.40),
        long_context_cache_read_usd_per_million: Some(0.12),
    }
}

fn usage(prompt: u64, completion: u64, cached: u64) -> LlmUsageSnapshot {
    LlmUsageSnapshot {
        prompt_tokens: Some(prompt),
        completion_tokens: Some(completion),
        total_tokens: Some(prompt + completion),
        input_tokens: None,
        output_tokens: None,
        reasoning_tokens: Some(completion / 2),
        cached_tokens: Some(cached),
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    }
}

#[test]
fn resolves_pricing_by_machine_provider_alias_and_model() {
    let catalog = vec![pricing()];
    let resolved = resolve_model_pricing(&catalog, "vendor-minimax", "openai_compat", "MiniMax-M3");
    assert_eq!(
        resolved.as_ref().map(|value| value.effective_from.as_str()),
        Some("2026-07-18")
    );
    assert!(
        resolve_model_pricing(&catalog, "vendor-minimax", "openai_compat", "unknown").is_none()
    );
}

#[test]
fn estimates_standard_and_long_context_cost_without_double_charging_cache() {
    let standard = build_cost_record(
        1,
        "plan",
        "vendor-minimax",
        "MiniMax-M3",
        "ok",
        1,
        Some(&usage(1_000_000, 1_000_000, 200_000)),
        Some(&pricing()),
    );
    assert_eq!(standard.cost_status, "known");
    assert_eq!(standard.estimated_cost_usd_nanos, Some(2_904_000_000));

    let short = build_cost_record(
        2,
        "chat",
        "vendor-minimax",
        "MiniMax-M3",
        "ok",
        1,
        Some(&usage(100_000, 20_000, 10_000)),
        Some(&pricing()),
    );
    assert_eq!(short.estimated_cost_usd_nanos, Some(51_600_000));
}

#[test]
fn marks_missing_usage_and_pricing_as_explicitly_unknown() {
    let no_usage = build_cost_record(
        1,
        "plan",
        "vendor-minimax",
        "MiniMax-M3",
        "failed",
        2,
        None,
        Some(&pricing()),
    );
    assert_eq!(
        no_usage.unknown_reason.as_deref(),
        Some("usage_unavailable")
    );

    let no_pricing = build_cost_record(
        2,
        "chat",
        "vendor-custom",
        "custom-model",
        "ok",
        1,
        Some(&usage(10, 5, 0)),
        None,
    );
    assert_eq!(
        no_pricing.unknown_reason.as_deref(),
        Some("pricing_not_configured")
    );
}

#[test]
fn task_summary_requires_coverage_and_all_records_to_be_priced() {
    let known = build_cost_record(
        1,
        "plan",
        "vendor-minimax",
        "MiniMax-M3",
        "ok",
        1,
        Some(&usage(100, 50, 0)),
        Some(&pricing()),
    );
    let summary = summarize_task_cost(1, &[known]);
    assert_eq!(summary.status, "known");
    assert_eq!(summary.covered_logical_call_count, 1);

    let missing_call = summarize_task_cost(2, &[]);
    assert_eq!(missing_call.status, "unknown");
    assert_eq!(
        missing_call.unknown_reasons,
        vec!["logical_call_not_observed"]
    );

    let unknown = LlmCallCostRecord {
        logical_call_index: 1,
        prompt_label: "plan".to_string(),
        provider: "vendor-custom".to_string(),
        model: "custom".to_string(),
        provider_status: "ok".to_string(),
        provider_attempts: 1,
        usage: Some(usage(1, 1, 0)),
        cost_status: "unknown".to_string(),
        unknown_reason: Some("pricing_not_configured".to_string()),
        estimated_cost_usd_nanos: None,
        pricing_effective_from: None,
        pricing_source: None,
        pricing_currency: None,
    };
    let summary = summarize_task_cost(1, &[unknown]);
    assert_eq!(summary.status, "unknown");
    assert_eq!(summary.unknown_reasons, vec!["pricing_not_configured"]);
}

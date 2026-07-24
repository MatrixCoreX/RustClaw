pub(crate) mod anthropic_claude;
pub(crate) mod anthropic_model_turn;
pub(crate) mod circuit;
pub(crate) mod client;
pub(crate) mod fixture_replay;
pub(crate) mod gemini_model_turn;
pub(crate) mod google_gemini;
pub(crate) mod openai_compat;
pub(crate) mod openai_model_turn;
pub(crate) mod output;
pub(crate) mod pricing;
pub(crate) mod routing;
pub(crate) mod usage;

#[cfg(test)]
#[path = "pricing_tests.rs"]
mod pricing_tests;

pub(crate) use circuit::{AttemptDecision, CircuitBreaker, CircuitBreakerSnapshot};
pub(crate) use client::{
    build_llm_http_client, call_model_turn_with_retry, call_provider_with_retry,
    call_provider_with_retry_with_hints, ChatRequestHints, LlmRoutingPreference,
};
pub(crate) use output::{
    append_model_io_log, log_color_enabled, maybe_sanitize_llm_text_output,
    rotate_model_io_log_daily, truncate_text, utf8_safe_prefix, MODEL_IO_LOG_KEEP_DAYS,
};
pub(crate) use pricing::{
    build_cost_record, resolve_model_pricing, summarize_task_cost, LlmCallCostRecord,
    LlmTaskCostSummary,
};
pub(crate) use routing::{route_providers, LlmProviderLatencyTracker, LlmProviderRouteEvaluation};
pub(crate) use usage::{anthropic_usage_snapshot, gemini_usage_snapshot, openai_usage_snapshot};

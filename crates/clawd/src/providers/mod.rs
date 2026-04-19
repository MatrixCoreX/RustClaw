pub(crate) mod anthropic_claude;
pub(crate) mod circuit;
pub(crate) mod client;
pub(crate) mod fixture_replay;
pub(crate) mod google_gemini;
pub(crate) mod openai_compat;
pub(crate) mod output;
pub(crate) mod usage;

pub(crate) use circuit::{AttemptDecision, CircuitBreaker};
pub(crate) use client::{
    build_llm_http_client, call_provider_with_retry, call_provider_with_retry_with_hints,
    ChatRequestHints,
};
pub(crate) use output::{
    append_model_io_log, log_color_enabled, maybe_sanitize_llm_text_output,
    rotate_model_io_log_daily, truncate_text, utf8_safe_prefix, MODEL_IO_LOG_KEEP_DAYS,
};
pub(crate) use usage::{anthropic_usage_snapshot, gemini_usage_snapshot, openai_usage_snapshot};

use std::sync::Arc;

use claw_core::config::LlmProviderConfig;
use reqwest::Client;
use tokio::sync::Semaphore;

use super::{anthropic_auth_mode, anthropic_messages_url, AnthropicAuthMode};
use crate::LlmProviderRuntime;

fn provider(name: &str, base_url: &str) -> LlmProviderRuntime {
    LlmProviderRuntime {
        config: LlmProviderConfig {
            name: name.to_string(),
            provider_type: "anthropic_claude".to_string(),
            base_url: base_url.to_string(),
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            context_window_tokens: None,
            input_modalities: vec!["text".to_string()],
            supports_tools: true,
            expected_latency_ms: None,
            priority: 1,
            timeout_seconds: 30,
            max_concurrency: 1,
            params: claw_core::config::LlmProviderParams::default(),
        },
        pricing: None,
        latency: Arc::new(crate::providers::LlmProviderLatencyTracker::default()),
        client: Client::new(),
        semaphore: Arc::new(Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    }
}

#[test]
fn anthropic_messages_url_appends_v1_when_base_url_has_no_version() {
    let provider = provider("vendor-minimax", "https://api.minimaxi.com/anthropic");
    assert_eq!(
        anthropic_messages_url(&provider),
        "https://api.minimaxi.com/anthropic/v1/messages"
    );
}

#[test]
fn anthropic_messages_url_reuses_existing_v1_suffix() {
    let provider = provider("vendor-anthropic", "https://api.anthropic.com/v1");
    assert_eq!(
        anthropic_messages_url(&provider),
        "https://api.anthropic.com/v1/messages"
    );
}

#[test]
fn minimax_anthropic_uses_bearer_auth() {
    let provider = provider("vendor-minimax", "https://api.minimaxi.com/anthropic");
    assert_eq!(
        anthropic_auth_mode(&provider),
        AnthropicAuthMode::AuthorizationBearer
    );
}

#[test]
fn anthropic_vendor_uses_x_api_key_auth() {
    let provider = provider("vendor-anthropic", "https://api.anthropic.com/v1");
    assert_eq!(anthropic_auth_mode(&provider), AnthropicAuthMode::XApiKey);
}

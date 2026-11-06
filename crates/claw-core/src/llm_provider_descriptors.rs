//! Side-effect-free provider configuration shared by execution and metadata.
use crate::config::{LlmConfig, LlmProviderConfig, LlmVendorConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderProtocol {
    OpenAiCompatible,
    GoogleGemini,
    AnthropicMessages,
}

impl ProviderProtocol {
    /// Explicit provider types retain the runtime's exact, closed wire contract.
    pub fn from_runtime(value: &str) -> Option<Self> {
        match value {
            "openai_compat" => Some(Self::OpenAiCompatible),
            "google_gemini" => Some(Self::GoogleGemini),
            "anthropic_claude" => Some(Self::AnthropicMessages),
            _ => None,
        }
    }

    pub fn runtime_name(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai_compat",
            Self::GoogleGemini => "google_gemini",
            Self::AnthropicMessages => "anthropic_claude",
        }
    }

    pub fn catalog_name(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai_compatible",
            Self::GoogleGemini => "google_gemini",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

#[derive(Clone, Copy)]
enum ProtocolPolicy {
    Fixed(ProviderProtocol),
    Dual,
}

fn vendor_inputs(
    llm: &LlmConfig,
) -> impl Iterator<Item = (&'static str, &LlmVendorConfig, ProtocolPolicy, i32)> {
    use ProtocolPolicy::{Dual, Fixed};
    use ProviderProtocol::{AnthropicMessages, GoogleGemini, OpenAiCompatible};
    // Replaces the runtime's separate synthesis branches. Metadata must consume
    // this source, not introduce another vendor/protocol/default mapping.
    [
        ("openai", llm.openai.as_ref(), Fixed(OpenAiCompatible)),
        ("google", llm.google.as_ref(), Fixed(GoogleGemini)),
        (
            "anthropic",
            llm.anthropic.as_ref(),
            Fixed(AnthropicMessages),
        ),
        ("grok", llm.grok.as_ref(), Fixed(OpenAiCompatible)),
        ("deepseek", llm.deepseek.as_ref(), Fixed(OpenAiCompatible)),
        ("qwen", llm.qwen.as_ref(), Fixed(OpenAiCompatible)),
        ("minimax", llm.minimax.as_ref(), Dual),
        ("mimo", llm.mimo.as_ref(), Dual),
        ("custom", llm.custom.as_ref(), Fixed(OpenAiCompatible)),
    ]
    .into_iter()
    .enumerate()
    .filter_map(|(index, (name, config, protocol))| {
        Some((name, config?, protocol, index as i32 + 1))
    })
}

fn protocol_for(name: &str, config: &LlmVendorConfig, policy: ProtocolPolicy) -> ProviderProtocol {
    if let ProtocolPolicy::Fixed(protocol) = policy {
        return protocol;
    }
    let format = config.api_format.as_deref().unwrap_or_default().trim();
    if format.eq_ignore_ascii_case("anthropic") || format.eq_ignore_ascii_case("anthropic_claude") {
        return ProviderProtocol::AnthropicMessages;
    }
    if !format.is_empty()
        && !format.eq_ignore_ascii_case("openai")
        && !format.eq_ignore_ascii_case("openai_compat")
    {
        tracing::warn!(
            "llm.{} api_format={:?} is not recognized (expected openai_compat or anthropic_claude); defaulting to openai_compat",
            name, config.api_format
        );
    }
    ProviderProtocol::OpenAiCompatible
}

fn vendor_config(
    name: &str,
    config: &LlmVendorConfig,
    policy: ProtocolPolicy,
    priority: i32,
    model: &str,
) -> LlmProviderConfig {
    LlmProviderConfig {
        name: format!("vendor-{name}"),
        provider_type: protocol_for(name, config, policy).runtime_name().into(),
        base_url: config.base_url.clone(),
        api_key: config.api_key.clone(),
        model: model.into(),
        context_window_tokens: config.context_window_tokens,
        input_modalities: config.input_modalities.clone(),
        supports_tools: config.supports_tools,
        expected_latency_ms: config.expected_latency_ms,
        priority,
        timeout_seconds: config.timeout_seconds,
        max_concurrency: config.max_concurrency,
        params: config.params.clone(),
    }
}

/// All configured flat-vendor defaults, before selecting the ordinary runtime.
pub fn configured_vendor_configs(llm: &LlmConfig) -> Vec<LlmProviderConfig> {
    vendor_inputs(llm)
        .map(|(name, config, protocol, priority)| {
            vendor_config(name, config, protocol, priority, &config.model)
        })
        .collect()
}

pub fn synthesized_provider_configs(
    llm: &LlmConfig,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Vec<LlmProviderConfig> {
    let selected_vendor = provider_override.or(llm.selected_vendor.as_deref());
    let selected_model = model_override
        .or_else(|| {
            let wanted = provider_override?.trim();
            let configured = llm.selected_vendor.as_deref()?.trim();
            (wanted.eq_ignore_ascii_case(configured)
                || (wanted.eq_ignore_ascii_case("xiaomi")
                    && configured.eq_ignore_ascii_case("mimo"))
                || (wanted.eq_ignore_ascii_case("mimo")
                    && configured.eq_ignore_ascii_case("xiaomi")))
            .then_some(llm.selected_model.as_deref())
            .flatten()
        })
        .or_else(|| {
            provider_override
                .is_none()
                .then_some(llm.selected_model.as_deref())
                .flatten()
        });
    vendor_inputs(llm)
        .filter(|(name, _, _, _)| selected_vendor.is_none() || selected_vendor == Some(*name))
        .map(|(name, config, protocol, priority)| {
            let model = if selected_vendor == Some(name) {
                selected_model.unwrap_or(&config.model)
            } else {
                &config.model
            };
            vendor_config(name, config, protocol, priority, model)
        })
        .collect()
}

pub fn matches_provider_override(name: &str, provider_type: &str, override_name: &str) -> bool {
    let wanted = override_name.trim().to_ascii_lowercase();
    let provider_name = name.trim().to_ascii_lowercase();
    let provider_type = provider_type.trim().to_ascii_lowercase();
    let vendor_name = provider_name
        .strip_prefix("vendor-")
        .unwrap_or(provider_name.as_str());
    wanted == provider_name
        || wanted == provider_type
        || wanted == vendor_name
        || (wanted == "xiaomi" && vendor_name == "mimo")
}

/// The runtime's exact managed-secret namespace, including invalid-name fallback.
pub fn credential_vendor_name(provider_name: &str) -> Option<&str> {
    let vendor = provider_name.trim().strip_prefix("vendor-")?.trim();
    (!vendor.is_empty()
        && vendor
            .chars()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == '_'))
    .then_some(vendor)
}

pub fn matches_hosted_relay(llm: &LlmConfig, provider: &LlmProviderConfig) -> bool {
    let Some(relay) = llm.hosted_relay.as_ref().filter(|relay| relay.enabled) else {
        return false;
    };
    provider.provider_type == ProviderProtocol::OpenAiCompatible.runtime_name()
        && provider
            .name
            .strip_prefix("vendor-")
            .is_some_and(|vendor| vendor == relay.vendor.trim())
        && provider.model.trim() == relay.model.trim()
        && provider.base_url.trim_end_matches('/') == relay.base_url.trim_end_matches('/')
}

/// No clients, credential lookup, enrollment, process IO or provider requests.
pub fn effective_provider_configs(
    llm: &LlmConfig,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Vec<LlmProviderConfig> {
    let source = if llm.providers.is_empty() {
        synthesized_provider_configs(llm, provider_override, model_override)
    } else {
        llm.providers.clone()
    };
    let mut result: Vec<_> = source
        .into_iter()
        .filter_map(|mut provider| {
            if provider_override.is_some_and(|wanted| {
                !matches_provider_override(&provider.name, &provider.provider_type, wanted)
            }) {
                return None;
            }
            if ProviderProtocol::from_runtime(&provider.provider_type).is_none() {
                tracing::warn!(
                    "Skip unsupported provider type={}, name={}",
                    provider.provider_type,
                    provider.name
                );
                return None;
            }
            if let Some(model) = model_override {
                provider.model = model.into();
            }
            provider.params.device_key_enrollment = matches_hosted_relay(llm, &provider);
            Some(provider)
        })
        .collect();
    result.sort_by_key(|provider| provider.priority);
    result
}

/// The ordinary runtime uses process overrides; explicit task selection does not.
pub fn process_overrides() -> (Option<String>, Option<String>) {
    (
        crate::product_identity::env_string("PROVIDER_OVERRIDE").ok(),
        crate::product_identity::env_string("MODEL_OVERRIDE").ok(),
    )
}

#[cfg(test)]
#[path = "llm_provider_descriptors_tests.rs"]
mod tests;

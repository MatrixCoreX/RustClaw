use std::sync::Arc;
use std::time::Duration;

use claw_core::config::{AppConfig, LlmProviderConfig};
use reqwest::Client;
use tokio::sync::Semaphore;
use tracing::warn;

use crate::{AppState, ClaimedTask, LlmProviderRuntime};

pub(crate) fn build_providers(config: &AppConfig) -> Vec<Arc<LlmProviderRuntime>> {
    let source_providers = if config.llm.providers.is_empty() {
        synthesize_llm_providers(config)
    } else {
        config.llm.providers.clone()
    };

    let mut providers: Vec<_> = source_providers
        .iter()
        .filter_map(|p| {
            if !matches!(
                p.provider_type.as_str(),
                "openai_compat" | "google_gemini" | "anthropic_claude"
            ) {
                warn!(
                    "Skip unsupported provider type={}, name={}",
                    p.provider_type, p.name
                );
                return None;
            }

            let runtime_cfg = p.clone();

            let client = Client::builder()
                .timeout(Duration::from_secs(runtime_cfg.timeout_seconds))
                .build()
                .ok()?;

            Some(Arc::new(LlmProviderRuntime {
                config: runtime_cfg.clone(),
                client,
                semaphore: Arc::new(Semaphore::new(runtime_cfg.max_concurrency.max(1))),
            }))
        })
        .collect();

    providers.sort_by_key(|p| p.config.priority);
    providers
}

fn synthesize_llm_providers(config: &AppConfig) -> Vec<LlmProviderConfig> {
    let mut out = Vec::new();
    let selected_vendor = config.llm.selected_vendor.as_deref();
    let selected_model = config.llm.selected_model.as_deref();

    if let Some(v) = &config.llm.openai {
        if selected_vendor.is_none() || selected_vendor == Some("openai") {
            let model = if selected_vendor == Some("openai") {
                selected_model.unwrap_or(&v.model)
            } else {
                &v.model
            };
            out.push(LlmProviderConfig {
                name: "vendor-openai".to_string(),
                provider_type: "openai_compat".to_string(),
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 1,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
            });
        }
    }

    if let Some(v) = &config.llm.google {
        if selected_vendor.is_none() || selected_vendor == Some("google") {
            let model = if selected_vendor == Some("google") {
                selected_model.unwrap_or(&v.model)
            } else {
                &v.model
            };
            out.push(LlmProviderConfig {
                name: "vendor-google".to_string(),
                provider_type: "google_gemini".to_string(),
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 2,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
            });
        }
    }

    if let Some(v) = &config.llm.anthropic {
        if selected_vendor.is_none() || selected_vendor == Some("anthropic") {
            let model = if selected_vendor == Some("anthropic") {
                selected_model.unwrap_or(&v.model)
            } else {
                &v.model
            };
            out.push(LlmProviderConfig {
                name: "vendor-anthropic".to_string(),
                provider_type: "anthropic_claude".to_string(),
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 3,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
            });
        }
    }

    if let Some(v) = &config.llm.grok {
        if selected_vendor.is_none() || selected_vendor == Some("grok") {
            let model = if selected_vendor == Some("grok") {
                selected_model.unwrap_or(&v.model)
            } else {
                &v.model
            };
            out.push(LlmProviderConfig {
                name: "vendor-grok".to_string(),
                provider_type: "openai_compat".to_string(),
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 4,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
            });
        }
    }

    if let Some(v) = &config.llm.deepseek {
        if selected_vendor.is_none() || selected_vendor == Some("deepseek") {
            let model = if selected_vendor == Some("deepseek") {
                selected_model.unwrap_or(&v.model)
            } else {
                &v.model
            };
            out.push(LlmProviderConfig {
                name: "vendor-deepseek".to_string(),
                provider_type: "openai_compat".to_string(),
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 5,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
            });
        }
    }

    if let Some(v) = &config.llm.qwen {
        if selected_vendor.is_none() || selected_vendor == Some("qwen") {
            let model = if selected_vendor == Some("qwen") {
                selected_model.unwrap_or(&v.model)
            } else {
                &v.model
            };
            out.push(LlmProviderConfig {
                name: "vendor-qwen".to_string(),
                provider_type: "openai_compat".to_string(),
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 6,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
            });
        }
    }

    if let Some(v) = &config.llm.minimax {
        if selected_vendor.is_none() || selected_vendor == Some("minimax") {
            let model = if selected_vendor == Some("minimax") {
                selected_model.unwrap_or(&v.model)
            } else {
                &v.model
            };
            out.push(LlmProviderConfig {
                name: "vendor-minimax".to_string(),
                provider_type: "openai_compat".to_string(),
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 7,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
            });
        }
    }

    if let Some(v) = &config.llm.custom {
        if selected_vendor.is_none() || selected_vendor == Some("custom") {
            let model = if selected_vendor == Some("custom") {
                selected_model.unwrap_or(&v.model)
            } else {
                &v.model
            };
            out.push(LlmProviderConfig {
                name: "vendor-custom".to_string(),
                provider_type: "openai_compat".to_string(),
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 8,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
            });
        }
    }

    out
}

pub(crate) async fn run_with_fallback_with_prompt_file(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    prompt_file: &str,
) -> Result<String, String> {
    super::run_llm_with_fallback_with_prompt_file(state, task, prompt, prompt_file).await
}

pub(crate) fn selected_openai_api_key(state: &AppState) -> String {
    super::selected_openai_api_key(state)
}

pub(crate) fn selected_openai_base_url(state: &AppState) -> String {
    super::selected_openai_base_url(state)
}

pub(crate) fn selected_openai_model(state: &AppState) -> String {
    super::selected_openai_model(state)
}

#[cfg(test)]
mod tests {
    use super::matches_provider_override;

    #[test]
    fn provider_override_matches_vendor_alias() {
        assert!(matches_provider_override(
            "vendor-qwen",
            "openai_compat",
            "qwen"
        ));
        assert!(matches_provider_override(
            "vendor-custom",
            "openai_compat",
            "custom"
        ));
        assert!(matches_provider_override(
            "vendor-minimax",
            "openai_compat",
            "minimax"
        ));
        assert!(matches_provider_override(
            "vendor-openai",
            "openai_compat",
            "openai"
        ));
    }

    #[test]
    fn provider_override_matches_runtime_name_and_type() {
        assert!(matches_provider_override(
            "vendor-qwen",
            "openai_compat",
            "vendor-qwen"
        ));
        assert!(matches_provider_override(
            "vendor-openai",
            "openai_compat",
            "openai_compat"
        ));
        assert!(!matches_provider_override(
            "vendor-qwen",
            "openai_compat",
            "google"
        ));
    }
}

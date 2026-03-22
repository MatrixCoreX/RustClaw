use std::sync::Arc;
use std::time::Duration;

use claw_core::config::{AppConfig, LlmProviderConfig};
use reqwest::Client;
use tokio::sync::Semaphore;
use tracing::{info, warn};

use crate::{AppState, ClaimedTask, LlmProviderRuntime};

fn matches_provider_override(name: &str, provider_type: &str, override_name: &str) -> bool {
    let wanted = override_name.trim().to_ascii_lowercase();
    let provider_name = name.trim().to_ascii_lowercase();
    let provider_type = provider_type.trim().to_ascii_lowercase();
    let vendor_name = provider_name
        .strip_prefix("vendor-")
        .unwrap_or(provider_name.as_str());
    wanted == provider_name || wanted == provider_type || wanted == vendor_name
}

pub(crate) fn build_providers(config: &AppConfig) -> Vec<Arc<LlmProviderRuntime>> {
    let model_override = std::env::var("RUSTCLAW_MODEL_OVERRIDE").ok();
    let provider_override = std::env::var("RUSTCLAW_PROVIDER_OVERRIDE").ok();
    build_providers_with_overrides(
        config,
        provider_override.as_deref(),
        model_override.as_deref(),
        true,
    )
}

pub(crate) fn build_providers_for_selection(
    config: &AppConfig,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Vec<Arc<LlmProviderRuntime>> {
    build_providers_with_overrides(config, provider_override, model_override, false)
}

fn build_providers_with_overrides(
    config: &AppConfig,
    provider_override: Option<&str>,
    model_override: Option<&str>,
    log_env_overrides: bool,
) -> Vec<Arc<LlmProviderRuntime>> {
    if let Some(model) = &model_override {
        if log_env_overrides {
            info!("Model override enabled: {}", model);
        }
    }
    if let Some(name) = &provider_override {
        if log_env_overrides {
            info!("Provider override enabled: {}", name);
        }
    }

    let source_providers = if config.llm.providers.is_empty() {
        synthesize_llm_providers(config, provider_override, model_override)
    } else {
        config.llm.providers.clone()
    };

    let mut providers: Vec<_> = source_providers
        .iter()
        .filter_map(|p| {
            if let Some(name) = provider_override {
                // Accept override by vendor alias (openai/google/anthropic/grok/deepseek/qwen/minimax/custom),
                // provider runtime name (vendor-xxx), or provider type.
                if !matches_provider_override(&p.name, &p.provider_type, name) {
                    return None;
                }
            }

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

            let mut runtime_cfg = p.clone();
            if let Some(model) = model_override {
                runtime_cfg.model = model.to_string();
            }

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

    if providers.is_empty() {
        if let Some(name) = provider_override {
            warn!("Provider override not found in config: {}", name);
        }
    }

    providers.sort_by_key(|p| p.config.priority);
    providers
}

fn synthesize_llm_providers(
    config: &AppConfig,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Vec<LlmProviderConfig> {
    let mut out = Vec::new();
    let selected_vendor = provider_override.or(config.llm.selected_vendor.as_deref());
    let selected_model = model_override.or(config.llm.selected_model.as_deref());

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
                provider_type: "anthropic_claude".to_string(),
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
    let _prompt_debug_enabled = state.routing.debug_log_prompt;
    let task_providers = state.task_llm_providers(task);
    if task_providers.is_empty() {
        return Err("No available LLM provider configured".to_string());
    }

    let mut last_error = "unknown llm error".to_string();

    for provider in &task_providers {
        let vendor = crate::llm_vendor_name(provider);
        let model = provider.config.model.as_str();
        let model_kind = crate::llm_model_kind(provider);
        let provider_name = format!("{}:{}", provider.config.name, provider.config.model);
        info!(
            "{} [LLM_CALL] stage=request task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_file={}",
            crate::highlight_tag("llm"),
            task.task_id,
            task.user_id,
            task.chat_id,
            vendor,
            model,
            model_kind,
            provider_name,
            prompt_file
        );

        match crate::call_provider_with_retry(provider.clone(), prompt).await {
            Ok(output) => {
                let (cleaned_text, sanitized) =
                    crate::maybe_sanitize_llm_text_output(vendor, &output.text);
                if sanitized {
                    warn!(
                        "{} [LLM_CALL] stage=cleanup task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_file={} note=removed_think_block",
                        crate::highlight_tag("llm"),
                        task.task_id,
                        task.user_id,
                        task.chat_id,
                        vendor,
                        model,
                        model_kind,
                        provider_name,
                        prompt_file
                    );
                }
                info!(
                    "{} [LLM_CALL] stage=response task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_file={} response={}",
                    crate::highlight_tag("llm"),
                    task.task_id,
                    task.user_id,
                    task.chat_id,
                    vendor,
                    model,
                    model_kind,
                    provider_name,
                    prompt_file,
                    crate::truncate_for_log(&cleaned_text)
                );
                crate::append_model_io_log(
                    state,
                    task,
                    provider,
                    "ok",
                    prompt_file,
                    prompt,
                    &output.request_payload,
                    Some(&output.raw_response),
                    Some(&cleaned_text),
                    output.usage.as_ref(),
                    sanitized,
                    None,
                );
                let _ = crate::insert_audit_log(
                    state,
                    Some(task.user_id),
                    "run_llm",
                    Some(
                        &serde_json::json!({
                            "task_id": task.task_id,
                            "chat_id": task.chat_id,
                            "vendor": vendor,
                            "provider": provider.config.name,
                            "model": provider.config.model,
                            "model_kind": model_kind,
                            "status": "ok"
                        })
                        .to_string(),
                    ),
                    None,
                );
                return Ok(cleaned_text);
            }
            Err(err) => {
                last_error = format!("provider={provider_name} failed: {err}");
                warn!(
                    "{} [LLM_CALL] stage=error task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_file={} error={}",
                    crate::highlight_tag("llm"),
                    task.task_id,
                    task.user_id,
                    task.chat_id,
                    vendor,
                    model,
                    model_kind,
                    provider_name,
                    prompt_file,
                    crate::truncate_for_log(&last_error)
                );
                crate::append_model_io_log(
                    state,
                    task,
                    provider,
                    "failed",
                    prompt_file,
                    prompt,
                    &err.request_payload,
                    err.raw_response.as_deref(),
                    None,
                    err.usage.as_ref(),
                    false,
                    Some(&err.message),
                );
                let _ = crate::insert_audit_log(
                    state,
                    Some(task.user_id),
                    "run_llm",
                    Some(
                        &serde_json::json!({
                            "task_id": task.task_id,
                            "chat_id": task.chat_id,
                            "vendor": vendor,
                            "provider": provider.config.name,
                            "model": provider.config.model,
                            "model_kind": model_kind,
                            "status": "failed"
                        })
                        .to_string(),
                    ),
                    Some(&last_error),
                );
                warn!("{last_error}");
            }
        }
    }

    Err(last_error)
}

pub(crate) fn selected_openai_api_key(state: &AppState, task: Option<&ClaimedTask>) -> String {
    let providers = task
        .map(|task| state.task_llm_providers(task))
        .unwrap_or_else(|| state.llm_providers.clone());
    if let Some(p) = providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
    {
        return p.config.api_key.clone();
    }
    String::new()
}

pub(crate) fn selected_openai_base_url(state: &AppState, task: Option<&ClaimedTask>) -> String {
    let providers = task
        .map(|task| state.task_llm_providers(task))
        .unwrap_or_else(|| state.llm_providers.clone());
    if let Some(p) = providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
    {
        return p.config.base_url.clone();
    }
    "https://api.openai.com/v1".to_string()
}

pub(crate) fn selected_openai_model(state: &AppState, task: Option<&ClaimedTask>) -> String {
    let providers = task
        .map(|task| state.task_llm_providers(task))
        .unwrap_or_else(|| state.llm_providers.clone());
    providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
        .map(|p| p.config.model.clone())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "gpt-4o-mini".to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use claw_core::config::AppConfig;

    use super::{matches_provider_override, synthesize_llm_providers};

    fn repo_config_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../configs/config.toml")
            .canonicalize()
            .expect("repo config path should resolve")
    }

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

    #[test]
    fn minimax_uses_anthropic_runtime_when_selected() {
        let path = repo_config_path();
        let mut config = AppConfig::load(path.to_str().expect("utf-8 path"))
            .expect("config fixture should load");
        config.llm.selected_vendor = Some("minimax".to_string());
        config.llm.selected_model = Some("MiniMax-M2.7".to_string());

        let providers = synthesize_llm_providers(&config, None, None);
        let minimax = providers
            .iter()
            .find(|provider| provider.name == "vendor-minimax")
            .expect("minimax provider should be synthesized");

        assert_eq!(minimax.provider_type, "anthropic_claude");
    }
}

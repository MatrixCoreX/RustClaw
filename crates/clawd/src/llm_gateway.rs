use std::sync::Arc;

use claw_core::config::{AppConfig, LlmProviderConfig, LlmVendorConfig};
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tracing::{info, warn};

use crate::providers::build_llm_http_client;
use crate::{AppState, ClaimedTask, LlmProviderRuntime};

/// Phase 1.5: 把 `prompt_source`（可能很长且带文件路径/版本/vendor 修饰）
/// 收敛成短 label，作为 per-task LLM 指标的 by-prompt 分桶 key。
///
/// 规则：按 `prompt_source` 包含的特征字符串归类。任何无法识别的归到
/// `"other"`，避免分桶炸开成无穷多 key。
///
/// 这套 label 是诊断维度（"哪个 prompt 把单任务额度烧光了"），不参与
/// 预算判断，所以稳定优先于精确——后续新增 prompt 模板时可以直接补一行。
pub(crate) fn classify_prompt_source(prompt_source: &str) -> &'static str {
    let s = prompt_source.to_ascii_lowercase();
    // 顺序很重要：更具体的匹配放前面，避免被宽泛规则吃掉。
    if s.contains("intent_normalizer") {
        "normalizer"
    } else if s.contains("intent_router") {
        "router_legacy"
    } else if s.contains("plan_repair") {
        "plan_repair"
    } else if s.contains("single_step_planner")
        || s.contains("single_plan_execution")
        || s.contains("lightweight_execution")
        || s.contains("loop_incremental_plan")
        || s.contains("plan_")
    {
        "plan"
    } else if s.contains("delivery_text_classifier") {
        "delivery_classifier"
    } else if s.contains("direct_classifier") {
        "direct_classifier"
    } else if s.contains("observed_answer_fallback") || s.contains("observed_") {
        "observed"
    } else if s.contains("clarify_question") {
        "clarify"
    } else if s.contains("intent_meta_summary") || s.contains("meta_summary") {
        "intent_meta"
    } else if s.contains("schedule_intent") {
        "schedule"
    } else if s.contains("command_intent") || s.contains("nl2cmd") {
        "nl2cmd"
    } else if s.contains("self_extension") {
        "self_extension"
    } else if s.contains("memory_") || s.contains("memory_extract") || s.contains("memory_judge") {
        "memory"
    } else if s.contains("verifier") || s.contains("verify_") {
        "verifier"
    } else if s.contains("chat_") || s.contains("chat:") || s.contains("chat_template") {
        "chat"
    } else if s.contains("semantic_judge") {
        "semantic_judge"
    } else {
        "other"
    }
}

/// Phase 1.3: 单任务最多触发多少次 LLM 调用。超过即视为异常放大
/// （例如 plan_repair 抖动、fallback 雪崩、normalizer/self-classify
/// 循环误判），直接短路返回错误。正常 agent 单轮 ask 不会接近这个上限。
///
/// **预算的"次数"语义**（重要）：
/// 这里统计的是**逻辑调用次数**，即 [`run_with_fallback_with_prompt_source`]
/// 的入口次数。每个入口内部会做：
///   * Provider fallback：依次尝试 N 个 provider，全部失败才返回错误。
///   * Per-provider retry：每个 provider 内部 [`call_provider_with_retry`]
///     会重试 `LLM_RETRY_TIMES` 次（默认 2 次）。
///
/// 因此**最坏情况**一个"逻辑调用"可能放大成 `N_providers × (1 + LLM_RETRY_TIMES)`
/// 次 HTTP 请求，但只占 1 个预算名额。预算的目的是挡"逻辑调用过多"
/// （流程异常放大），不是挡"HTTP 请求总量"（那是 retry/circuit-breaker 的职责）。
///
/// 如需更细的 HTTP 维度限流，参见 P1 优化项：把计数下沉到
/// [`call_provider_with_retry`]，但要同时调整重试上限以避免误熔断。
pub(crate) const MAX_LLM_CALLS_PER_TASK: u64 = 40;

/// Phase 1.3: 单任务 LLM 总耗时（ms）上限。主要挡住某一个 provider
/// 长时间挂住、不停重试的场景。180s 是目前 worker_task_timeout 的量级。
///
/// **预算的"耗时"语义**：累加的是每次 `run_with_fallback_*` 入口的
/// **wall-clock 耗时**（包括所有失败 provider 的尝试时间），而不是
/// "成功 provider 的纯耗时"。这样能更准确反映任务对 LLM 链路施加的真实压力。
pub(crate) const MAX_LLM_TOTAL_MS_PER_TASK: u64 = 180_000;

fn matches_provider_override(name: &str, provider_type: &str, override_name: &str) -> bool {
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
                // Accept override by vendor alias (openai/google/anthropic/grok/deepseek/qwen/minimax/mimo/custom),
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

            let client = build_llm_http_client(runtime_cfg.timeout_seconds).ok()?;

            Some(Arc::new(LlmProviderRuntime {
                config: runtime_cfg.clone(),
                client,
                semaphore: Arc::new(Semaphore::new(runtime_cfg.max_concurrency.max(1))),
                breaker: Arc::new(crate::providers::CircuitBreaker::new()),
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

/// 双协议 vendor 的 `api_format`：未配置或为空时默认 `openai_compat`；显式
/// `anthropic_claude` 等则走 Anthropic Messages。
fn api_format_synthesized_provider_type(vendor_name: &str, v: &LlmVendorConfig) -> &'static str {
    let Some(raw) = v.api_format.as_ref() else {
        return "openai_compat";
    };
    let fmt = raw.trim();
    if fmt.is_empty() {
        return "openai_compat";
    }
    if fmt.eq_ignore_ascii_case("anthropic") || fmt.eq_ignore_ascii_case("anthropic_claude") {
        return "anthropic_claude";
    }
    if fmt.eq_ignore_ascii_case("openai") || fmt.eq_ignore_ascii_case("openai_compat") {
        return "openai_compat";
    }
    warn!(
        "llm.{} api_format={:?} is not recognized (expected openai_compat or anthropic_claude); defaulting to openai_compat",
        vendor_name,
        v.api_format
    );
    "openai_compat"
}

fn synthesize_llm_providers(
    config: &AppConfig,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Vec<LlmProviderConfig> {
    let mut out = Vec::new();
    let selected_vendor = provider_override.or(config.llm.selected_vendor.as_deref());
    let selected_model = model_override
        .or_else(|| {
            let override_vendor = provider_override?.trim();
            let configured_vendor = config.llm.selected_vendor.as_deref()?.trim();
            if override_vendor.eq_ignore_ascii_case(configured_vendor)
                || (override_vendor.eq_ignore_ascii_case("xiaomi")
                    && configured_vendor.eq_ignore_ascii_case("mimo"))
                || (override_vendor.eq_ignore_ascii_case("mimo")
                    && configured_vendor.eq_ignore_ascii_case("xiaomi"))
            {
                config.llm.selected_model.as_deref()
            } else {
                None
            }
        })
        .or_else(|| {
            if provider_override.is_none() {
                config.llm.selected_model.as_deref()
            } else {
                None
            }
        });

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
                params: v.params.clone(),
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
                params: v.params.clone(),
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
                params: v.params.clone(),
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
                params: v.params.clone(),
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
                params: v.params.clone(),
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
                params: v.params.clone(),
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
            let provider_type = api_format_synthesized_provider_type("minimax", v).to_string();
            out.push(LlmProviderConfig {
                name: "vendor-minimax".to_string(),
                provider_type,
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 7,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
                params: v.params.clone(),
            });
        }
    }

    if let Some(v) = &config.llm.mimo {
        if selected_vendor.is_none() || selected_vendor == Some("mimo") {
            let model = if selected_vendor == Some("mimo") {
                selected_model.unwrap_or(&v.model)
            } else {
                &v.model
            };
            let provider_type = api_format_synthesized_provider_type("mimo", v).to_string();
            out.push(LlmProviderConfig {
                name: "vendor-mimo".to_string(),
                provider_type,
                base_url: v.base_url.clone(),
                api_key: v.api_key.clone(),
                model: model.to_string(),
                priority: 8,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
                params: v.params.clone(),
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
                priority: 9,
                timeout_seconds: v.timeout_seconds,
                max_concurrency: v.max_concurrency,
                params: v.params.clone(),
            });
        }
    }

    out
}

pub(crate) async fn run_with_fallback_with_prompt_source(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    prompt_source: &str,
) -> Result<String, String> {
    run_with_fallback_with_hints(
        state,
        task,
        prompt,
        prompt_source,
        crate::ChatRequestHints::default(),
    )
    .await
}

pub(crate) async fn run_with_fallback_with_hints(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    prompt_source: &str,
    hints: crate::ChatRequestHints,
) -> Result<String, String> {
    let task_providers = state.task_llm_providers(task);
    run_with_fallback_on_providers_with_hints(
        state,
        task,
        prompt,
        prompt_source,
        hints,
        task_providers,
    )
    .await
}

pub(crate) async fn run_with_fallback_on_providers_with_hints(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    prompt_source: &str,
    hints: crate::ChatRequestHints,
    providers: Vec<Arc<LlmProviderRuntime>>,
) -> Result<String, String> {
    let _prompt_debug_enabled = state.policy.routing.debug_log_prompt;
    if providers.is_empty() {
        return Err("No available LLM provider configured".to_string());
    }

    // Phase 1.3: 预算检查 —— 在真正命中 provider 之前短路异常放大的任务。
    // 预算统计包括本次调用链里所有 `run_with_fallback_*` 入口累计的调用次数
    // 和总耗时；超限就立即返回，不再继续 fallback / retry，避免雪崩。
    if let Some(reason) = state.task_llm_budget_exceeded(&task.task_id) {
        warn!(
            "{} [LLM_CALL] stage=budget_block task_id={} prompt_source={} reason={}",
            crate::highlight_tag("llm"),
            task.task_id,
            prompt_source,
            reason
        );
        return Err(reason);
    }

    // 记一次 LLM 调用（无论是否成功）。原先 `note_task_llm_call` 挂在
    // `append_model_io_log` 里，只有 `debug_log_prompt=true` 时才计数；
    // 现在移到 gateway 入口，保证预算统计始终可靠。
    // Phase 1.5: 同时按 prompt label 分桶累计，task journal 里 by_prompt 维度可观测。
    let prompt_label = classify_prompt_source(prompt_source);
    state.note_task_llm_call_with_label(&task.task_id, prompt_label);
    let call_started_at = std::time::Instant::now();

    let mut last_error = "unknown llm error".to_string();
    let mut any_provider_attempted = false;
    let mut skipped_providers: Vec<String> = Vec::new();

    for provider in &providers {
        let vendor = crate::llm_vendor_name(provider);
        let model = provider.config.model.as_str();
        let model_kind = crate::llm_model_kind(provider);
        let provider_name = format!("{}:{}", provider.config.name, provider.config.model);

        // Phase 2.1: 进入 provider 之前先问 circuit breaker。
        // Open + cooldown 未到期 → 直接跳到下一家，不浪费这次 fallback 的
        // retry / timeout 配额。也避免在 model_io.log 里反复刷同一个坏 provider。
        let breaker_decision = provider.breaker.before_attempt();
        match breaker_decision {
            crate::providers::AttemptDecision::Allow => {}
            crate::providers::AttemptDecision::AllowTrial => {
                info!(
                    "{} [LLM_CALL] stage=circuit_half_open task_id={} provider={} prompt_source={} note=trial_after_cooldown",
                    crate::highlight_tag("llm"),
                    task.task_id,
                    provider_name,
                    prompt_source
                );
            }
            crate::providers::AttemptDecision::SkipCooldown { remaining_ms } => {
                warn!(
                    "{} [LLM_CALL] stage=circuit_open_skip task_id={} provider={} prompt_source={} cooldown_remaining_ms={}",
                    crate::highlight_tag("llm"),
                    task.task_id,
                    provider_name,
                    prompt_source,
                    remaining_ms
                );
                skipped_providers.push(format!("{provider_name}(cooldown_ms={remaining_ms})"));
                continue;
            }
        }
        any_provider_attempted = true;

        info!(
            "{} [LLM_CALL] stage=request task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_source={}",
            crate::highlight_tag("llm"),
            task.task_id,
            task.user_id,
            task.chat_id,
            vendor,
            model,
            model_kind,
            provider_name,
            prompt_source
        );

        match crate::call_provider_with_retry_with_hints(provider.clone(), prompt, &hints).await {
            Ok(output) => {
                let (mut cleaned_text, mut sanitized) =
                    crate::maybe_sanitize_llm_text_output(vendor, &output.text);
                if cleaned_text.trim().is_empty() {
                    if let Some(recovered) = recover_normalizer_text_from_openai_tool_calls(
                        prompt_source,
                        prompt,
                        &output.raw_response,
                    ) {
                        cleaned_text = recovered;
                        sanitized = true;
                        warn!(
                            "{} [LLM_CALL] stage=cleanup task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_source={} note=recovered_tool_call_contract",
                            crate::highlight_tag("llm"),
                            task.task_id,
                            task.user_id,
                            task.chat_id,
                            vendor,
                            model,
                            model_kind,
                            provider_name,
                            prompt_source
                        );
                    }
                }
                if sanitized {
                    warn!(
                        "{} [LLM_CALL] stage=cleanup task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_source={} note=removed_think_block",
                        crate::highlight_tag("llm"),
                        task.task_id,
                        task.user_id,
                        task.chat_id,
                        vendor,
                        model,
                        model_kind,
                        provider_name,
                        prompt_source
                    );
                }
                info!(
                    "{} [LLM_CALL] stage=response task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_source={} response={}",
                    crate::highlight_tag("llm"),
                    task.task_id,
                    task.user_id,
                    task.chat_id,
                    vendor,
                    model,
                    model_kind,
                    provider_name,
                    prompt_source,
                    crate::truncate_for_log(&cleaned_text)
                );
                crate::append_model_io_log(
                    state,
                    task,
                    provider,
                    "ok",
                    prompt_source,
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
                state.note_task_llm_elapsed_with_label(
                    &task.task_id,
                    prompt_label,
                    call_started_at.elapsed().as_millis() as u64,
                );
                provider.breaker.note_success();
                return Ok(cleaned_text);
            }
            Err(err) => {
                let error_kind = err.observability_kind();
                if err.should_trip_breaker() {
                    provider.breaker.note_failure();
                } else if err.should_reset_breaker() {
                    // breaker 只跟踪 provider 基础设施健康。当前这次调用已经拿到了
                    // 有效 provider 响应（哪怕业务上是 4xx/429/blocked），说明
                    // 上游链路已恢复，不应继续维持 Open/HalfOpen。
                    provider.breaker.note_success();
                }
                last_error = format!("provider={provider_name} failed: {err}");
                warn!(
                    "{} [LLM_CALL] stage=error task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_source={} error_kind={} error={}",
                    crate::highlight_tag("llm"),
                    task.task_id,
                    task.user_id,
                    task.chat_id,
                    vendor,
                    model,
                    model_kind,
                    provider_name,
                    prompt_source,
                    error_kind,
                    crate::truncate_for_log(&last_error)
                );
                crate::append_model_io_log(
                    state,
                    task,
                    provider,
                    "failed",
                    prompt_source,
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
                            "status": "failed",
                            "error_kind": error_kind
                        })
                        .to_string(),
                    ),
                    Some(&last_error),
                );
                warn!("{last_error}");
            }
        }
    }

    state.note_task_llm_elapsed_with_label(
        &task.task_id,
        prompt_label,
        call_started_at.elapsed().as_millis() as u64,
    );
    if !any_provider_attempted && !skipped_providers.is_empty() {
        // 全员被 breaker 拦下，所有 provider 当前都在 cooldown。
        // 这种情况要给一个**明确的可识别错误**，让上游日志/指标里能看见
        // "短时间集中失败"信号，而不是含糊的 "unknown llm error"。
        let detail = skipped_providers.join(", ");
        return Err(format!(
            "all llm providers in circuit-breaker cooldown: {detail}"
        ));
    }
    if !skipped_providers.is_empty() {
        // 至少有一个 provider 真的被打了（且全失败），但同时也跳过了若干在
        // cooldown 里的 provider。把跳过列表追加到 last_error 便于排障。
        last_error = format!(
            "{last_error}; skipped_in_cooldown=[{}]",
            skipped_providers.join(", ")
        );
    }
    Err(last_error)
}

fn recover_normalizer_text_from_openai_tool_calls(
    prompt_source: &str,
    prompt: &str,
    raw_response: &str,
) -> Option<String> {
    if classify_prompt_source(prompt_source) != "normalizer" {
        return None;
    }
    let value = serde_json::from_str::<Value>(raw_response).ok()?;
    let tool_calls = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("tool_calls"))
        .and_then(Value::as_array)?;
    let first_call = tool_calls.first()?;
    let args_value = first_call
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .or_else(|| first_call.pointer("/function/arguments").cloned())
        .unwrap_or(Value::Null);
    let locator_hint = extract_first_locator_hint_from_value(&args_value)?;
    let locator_kind = if crate::worker::has_explicit_path_or_url_locator_hint(&locator_hint) {
        "path"
    } else {
        "filename"
    };
    let resolved_user_intent = extract_request_from_normalizer_prompt(prompt).unwrap_or_default();
    let recovered = json!({
        "resolved_user_intent": resolved_user_intent,
        "answer_candidate": "",
        "resume_behavior": "none",
        "schedule_kind": "none",
        "schedule_intent": null,
        "wants_file_delivery": false,
        "should_refresh_long_term_memory": false,
        "agent_display_name_hint": "",
        "needs_clarify": false,
        "clarify_question": "",
        "reason": "openai_compat_tool_call_recovered_to_observable_route",
        "confidence": 0.74,
        "mode": "act",
        "output_contract": {
            "response_shape": "free",
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": locator_kind,
            "delivery_intent": "none",
            "semantic_kind": "none",
            "locator_hint": locator_hint,
            "self_extension": {
                "mode": "none",
                "trigger": "none",
                "execute_now": false
            }
        },
        "execution_recipe": {
            "kind": "none",
            "profile": "none",
            "target_scope": "none"
        },
        "turn_type": "task_request",
        "target_task_policy": "standalone",
        "should_interrupt_active_run": false,
        "state_patch": null,
        "attachment_processing_required": false
    });
    serde_json::to_string(&recovered).ok()
}

fn extract_first_locator_hint_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            (crate::worker::has_explicit_path_or_url_locator_hint(trimmed)
                || crate::worker::has_concrete_locator_hint(trimmed))
            .then(|| trimmed.to_string())
        }
        Value::Array(items) => items.iter().find_map(extract_first_locator_hint_from_value),
        Value::Object(map) => map.values().find_map(extract_first_locator_hint_from_value),
        _ => None,
    }
}

fn extract_request_from_normalizer_prompt(prompt: &str) -> Option<String> {
    let (_, tail) = prompt.rsplit_once("REQUEST:")?;
    tail.lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn selected_openai_api_key(state: &AppState, task: Option<&ClaimedTask>) -> String {
    let providers = task
        .map(|task| state.task_llm_providers(task))
        .unwrap_or_else(|| state.core.llm_providers.clone());
    if let Some(p) = providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
    {
        // §P4.4 E3.a: 走 broker 优先 / config 兜底的统一通路，让 forge 给
        // skill-runner 子进程的 OPENAI_API_KEY 与 builtin chat 的 LLM 调用
        // 拿到同一份凭据，避免 broker 装上后两条路径漂移。
        return p.api_key().to_string();
    }
    String::new()
}

pub(crate) fn selected_openai_base_url(state: &AppState, task: Option<&ClaimedTask>) -> String {
    let providers = task
        .map(|task| state.task_llm_providers(task))
        .unwrap_or_else(|| state.core.llm_providers.clone());
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
        .unwrap_or_else(|| state.core.llm_providers.clone());
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

    use super::{
        classify_prompt_source, matches_provider_override,
        recover_normalizer_text_from_openai_tool_calls, synthesize_llm_providers,
    };

    fn repo_config_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../configs/config.toml")
            .canonicalize()
            .expect("repo config path should resolve")
    }

    #[test]
    fn classify_prompt_source_uses_specific_classifier_labels() {
        assert_eq!(
            classify_prompt_source("prompts/delivery_text_classifier_prompt.md"),
            "delivery_classifier"
        );
        assert_eq!(
            classify_prompt_source("inline:direct_classifier"),
            "direct_classifier"
        );
        assert_eq!(
            classify_prompt_source(
                "layered:prompts/lightweight_execution_prompt.md#vendor=minimax"
            ),
            "plan"
        );
    }

    #[test]
    fn normalizer_recovers_openai_tool_call_as_execution_contract() {
        let raw_response = r#"{
          "choices":[{
            "finish_reason":"tool_calls",
            "message":{
              "content":"<think>need file evidence</think>",
              "tool_calls":[{
                "type":"function",
                "function":{
                  "name":"read_file",
                  "arguments":"{\"file_path\":\"/home/guagua/rustclaw/README.md\"}"
                }
              }]
            }
          }]
        }"#;
        let recovered = recover_normalizer_text_from_openai_tool_calls(
            "layered:prompts/intent_normalizer_prompt.md#vendor=minimax",
            "REQUEST: 读取 README 开头内容，再用一句话总结\n",
            raw_response,
        )
        .expect("recover tool call");
        let value = serde_json::from_str::<serde_json::Value>(&recovered).expect("json");

        assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("act"));
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            value
                .pointer("/output_contract/locator_hint")
                .and_then(|v| v.as_str()),
            Some("/home/guagua/rustclaw/README.md")
        );
        assert_eq!(
            value.get("resolved_user_intent").and_then(|v| v.as_str()),
            Some("读取 README 开头内容，再用一句话总结")
        );
    }

    #[test]
    fn tool_call_recovery_ignores_non_normalizer_prompts() {
        let raw_response = r#"{
          "choices":[{
            "message":{
              "tool_calls":[{
                "function":{"arguments":"{\"path\":\"/tmp/a.txt\"}"}
              }]
            }
          }]
        }"#;

        assert!(recover_normalizer_text_from_openai_tool_calls(
            "layered:prompts/chat_response_prompt.md#vendor=minimax",
            "REQUEST: read a file",
            raw_response,
        )
        .is_none());
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
            "vendor-mimo",
            "openai_compat",
            "mimo"
        ));
        assert!(matches_provider_override(
            "vendor-mimo",
            "openai_compat",
            "xiaomi"
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
    fn minimax_uses_openai_compat_runtime_when_selected() {
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

        assert_eq!(minimax.provider_type, "openai_compat");
    }

    #[test]
    fn mimo_uses_openai_compat_runtime_when_selected() {
        let path = repo_config_path();
        let mut config = AppConfig::load(path.to_str().expect("utf-8 path"))
            .expect("config fixture should load");
        config.llm.selected_vendor = Some("mimo".to_string());
        config.llm.selected_model = Some("mimo-v2.5-pro".to_string());

        let providers = synthesize_llm_providers(&config, None, None);
        let mimo = providers
            .iter()
            .find(|provider| provider.name == "vendor-mimo")
            .expect("mimo provider should be synthesized");

        assert_eq!(mimo.provider_type, "openai_compat");
    }

    #[test]
    fn provider_override_without_model_uses_target_vendor_default_model() {
        let path = repo_config_path();
        let mut config = AppConfig::load(path.to_str().expect("utf-8 path"))
            .expect("config fixture should load");
        config.llm.selected_vendor = Some("mimo".to_string());
        config.llm.selected_model = Some("mimo-v2.5-pro".to_string());
        let qwen_default = config.llm.qwen.as_ref().expect("qwen config").model.clone();

        let providers = synthesize_llm_providers(&config, Some("qwen"), None);
        let qwen = providers
            .iter()
            .find(|provider| provider.name == "vendor-qwen")
            .expect("qwen provider should be synthesized");

        assert_eq!(qwen.model, qwen_default);
        assert_ne!(qwen.model, "mimo-v2.5-pro");
    }

    #[test]
    fn mimo_respects_api_format_anthropic() {
        let path = repo_config_path();
        let mut config = AppConfig::load(path.to_str().expect("utf-8 path"))
            .expect("config fixture should load");
        config.llm.selected_vendor = Some("mimo".to_string());
        config.llm.selected_model = Some("mimo-v2.5-pro".to_string());
        if let Some(ref mut mimo) = config.llm.mimo {
            mimo.api_format = Some("anthropic_claude".to_string());
        }

        let providers = synthesize_llm_providers(&config, None, None);
        let mimo = providers
            .iter()
            .find(|provider| provider.name == "vendor-mimo")
            .expect("mimo provider should be synthesized");

        assert_eq!(mimo.provider_type, "anthropic_claude");
    }

    #[test]
    fn minimax_respects_api_format_anthropic() {
        let path = repo_config_path();
        let mut config = AppConfig::load(path.to_str().expect("utf-8 path"))
            .expect("config fixture should load");
        config.llm.selected_vendor = Some("minimax".to_string());
        config.llm.selected_model = Some("MiniMax-M2.7".to_string());
        if let Some(ref mut mm) = config.llm.minimax {
            mm.api_format = Some("anthropic_claude".to_string());
        }

        let providers = synthesize_llm_providers(&config, None, None);
        let minimax = providers
            .iter()
            .find(|provider| provider.name == "vendor-minimax")
            .expect("minimax provider should be synthesized");

        assert_eq!(minimax.provider_type, "anthropic_claude");
    }

    #[test]
    fn minimax_defaults_openai_when_api_format_missing_or_empty() {
        let path = repo_config_path();
        let mut config = AppConfig::load(path.to_str().expect("utf-8 path"))
            .expect("config fixture should load");
        config.llm.selected_vendor = Some("minimax".to_string());
        if let Some(ref mut mm) = config.llm.minimax {
            mm.api_format = None;
        }

        let providers = synthesize_llm_providers(&config, None, None);
        let minimax = providers
            .iter()
            .find(|p| p.name == "vendor-minimax")
            .expect("vendor-minimax when api_format unset");
        assert_eq!(minimax.provider_type, "openai_compat");

        if let Some(ref mut mm) = config.llm.minimax {
            mm.api_format = Some("   ".to_string());
        }
        let providers = synthesize_llm_providers(&config, None, None);
        let minimax = providers
            .iter()
            .find(|p| p.name == "vendor-minimax")
            .expect("vendor-minimax when api_format blank");
        assert_eq!(minimax.provider_type, "openai_compat");
    }
}

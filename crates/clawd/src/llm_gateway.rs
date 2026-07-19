use std::sync::Arc;
use std::time::Duration;

use claw_core::config::{AppConfig, LlmProviderConfig, LlmVendorConfig};
use tokio::sync::{oneshot, Semaphore};
use tracing::{info, warn};

use crate::providers::build_llm_http_client;
use crate::providers::client::ProviderErrorKind;
use crate::runtime::TaskProviderBlocker;
use crate::{AppState, ClaimedTask, LlmProviderRuntime};

const TASK_LLM_COST_POLICY_BLOCKED_ERR: &str = "llm_cost_policy_blocked";
const NO_ELIGIBLE_LLM_PROVIDER_ERR: &str = "no_eligible_llm_provider";

fn llm_cost_policy_allows(
    state: &AppState,
    task: &ClaimedTask,
    provider: Option<&str>,
    prompt_source: &str,
) -> bool {
    match state.evaluate_llm_cost_budget(task, provider) {
        Ok(snapshot) => {
            if snapshot.hard_exceeded {
                warn!(
                    "{} [LLM_CALL] stage=cost_policy_block task_id={} provider={} prompt_source={} status={} observed_cost_usd_nanos={} hard_limit_usd_nanos={}",
                    crate::highlight_tag("llm"),
                    task.task_id,
                    provider.unwrap_or("provider_set"),
                    prompt_source,
                    snapshot.status,
                    snapshot.task_known_cost_usd_nanos,
                    snapshot.hard_task_limit_usd_nanos.unwrap_or(0),
                );
                false
            } else {
                true
            }
        }
        Err(err) => {
            warn!(
                "{} [LLM_CALL] stage=cost_policy_observation_error task_id={} provider={} prompt_source={} error={}",
                crate::highlight_tag("llm"),
                task.task_id,
                provider.unwrap_or("provider_set"),
                prompt_source,
                crate::truncate_for_log(&err),
            );
            true
        }
    }
}

fn record_llm_cost(
    state: &AppState,
    task: &ClaimedTask,
    record: crate::providers::LlmCallCostRecord,
    prompt_source: &str,
) {
    let provider = record.provider.clone();
    if let Err(err) = state.note_task_llm_cost_record(task, record) {
        warn!(
            "{} [LLM_CALL] stage=cost_ledger_write_error task_id={} prompt_source={} error={}",
            crate::highlight_tag("llm"),
            task.task_id,
            prompt_source,
            crate::truncate_for_log(&err),
        );
    }
    let _ = state.evaluate_llm_cost_budget(task, Some(&provider));
}

fn touch_llm_task_lease(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
    prompt_label: &str,
    stage: &str,
) {
    match crate::repo::touch_running_task(state, task_id, claim_attempt) {
        Ok(true) => {}
        Ok(false) => warn!(
            "{} [LLM_CALL] stage=lease_touch_skipped task_id={} prompt_label={} touch_stage={} reason=task_not_running",
            crate::highlight_tag("llm"),
            task_id,
            prompt_label,
            stage
        ),
        Err(err) => warn!(
            "{} [LLM_CALL] stage=lease_touch_failed task_id={} prompt_label={} touch_stage={} err={}",
            crate::highlight_tag("llm"),
            task_id,
            prompt_label,
            stage,
            err
        ),
    }
}

fn llm_lease_heartbeat_interval_secs(state: &AppState) -> u64 {
    state
        .worker
        .worker_task_heartbeat_seconds
        .max(5)
        .saturating_div(2)
        .clamp(5, 30)
}

fn start_llm_task_lease_heartbeat(
    state: AppState,
    task_id: String,
    claim_attempt: i64,
    prompt_label: &'static str,
) -> oneshot::Sender<()> {
    let interval_secs = llm_lease_heartbeat_interval_secs(&state);
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                    touch_llm_task_lease(
                        &state,
                        &task_id,
                        claim_attempt,
                        prompt_label,
                        "provider_wait",
                    );
                }
                _ = &mut stop_rx => {
                    break;
                }
            }
        }
    });
    stop_tx
}

fn stop_llm_task_lease_heartbeat(stop_tx: &mut Option<oneshot::Sender<()>>) {
    if let Some(stop_tx) = stop_tx.take() {
        let _ = stop_tx.send(());
    }
}

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
    if s.contains("context_compaction") {
        "context_compaction"
    } else if s.contains("plan_repair") {
        "plan_repair"
    } else if s.contains("single_step_planner")
        || s.contains("single_plan_execution")
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
    } else if s.contains("user_response_composer") {
        "user_response_composer"
    } else if s.contains("user_response_contract_validator") {
        "user_response_validator"
    } else if s.contains("clarify_question") {
        "clarify"
    } else if s.contains("schedule_intent") {
        "schedule"
    } else if s.contains("command_intent") || s.contains("nl2cmd") {
        "nl2cmd"
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
pub(crate) const DEFAULT_MAX_LLM_CALLS_PER_TASK: u64 = 40;

/// Phase 1.3: 单任务 LLM 总耗时（ms）上限。主要挡住某一个 provider
/// 长时间挂住、不停重试的场景。默认 420s，实际运行值来自
/// `worker.llm_total_timeout_seconds`。
///
/// **预算的"耗时"语义**：累加的是每次 `run_with_fallback_*` 入口的
/// **wall-clock 耗时**（包括所有失败 provider 的尝试时间），而不是
/// "成功 provider 的纯耗时"。这样能更准确反映任务对 LLM 链路施加的真实压力。
pub(crate) const DEFAULT_MAX_LLM_TOTAL_MS_PER_TASK: u64 = 420_000;

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
            let pricing = crate::providers::resolve_model_pricing(
                &config.llm.pricing,
                &runtime_cfg.name,
                &runtime_cfg.provider_type,
                &runtime_cfg.model,
            );
            let client = build_llm_http_client(runtime_cfg.timeout_seconds).ok()?;

            Some(Arc::new(LlmProviderRuntime {
                config: runtime_cfg.clone(),
                pricing,
                latency: Arc::new(crate::providers::LlmProviderLatencyTracker::default()),
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
                context_window_tokens: v.context_window_tokens,
                input_modalities: v.input_modalities.clone(),
                supports_tools: v.supports_tools,
                expected_latency_ms: v.expected_latency_ms,
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
                context_window_tokens: v.context_window_tokens,
                input_modalities: v.input_modalities.clone(),
                supports_tools: v.supports_tools,
                expected_latency_ms: v.expected_latency_ms,
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
                context_window_tokens: v.context_window_tokens,
                input_modalities: v.input_modalities.clone(),
                supports_tools: v.supports_tools,
                expected_latency_ms: v.expected_latency_ms,
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
                context_window_tokens: v.context_window_tokens,
                input_modalities: v.input_modalities.clone(),
                supports_tools: v.supports_tools,
                expected_latency_ms: v.expected_latency_ms,
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
                context_window_tokens: v.context_window_tokens,
                input_modalities: v.input_modalities.clone(),
                supports_tools: v.supports_tools,
                expected_latency_ms: v.expected_latency_ms,
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
                context_window_tokens: v.context_window_tokens,
                input_modalities: v.input_modalities.clone(),
                supports_tools: v.supports_tools,
                expected_latency_ms: v.expected_latency_ms,
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
                context_window_tokens: v.context_window_tokens,
                input_modalities: v.input_modalities.clone(),
                supports_tools: v.supports_tools,
                expected_latency_ms: v.expected_latency_ms,
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
                context_window_tokens: v.context_window_tokens,
                input_modalities: v.input_modalities.clone(),
                supports_tools: v.supports_tools,
                expected_latency_ms: v.expected_latency_ms,
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
                context_window_tokens: v.context_window_tokens,
                input_modalities: v.input_modalities.clone(),
                supports_tools: v.supports_tools,
                expected_latency_ms: v.expected_latency_ms,
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
    state.clear_task_provider_blocker(&task.task_id);
    state.clear_task_cost_blocker(&task.task_id);
    state.restore_task_llm_call_count_from_cost_ledger(&task.task_id);
    if !llm_cost_policy_allows(state, task, None, prompt_source) {
        return Err(TASK_LLM_COST_POLICY_BLOCKED_ERR.to_string());
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
    state.note_task_llm_call_with_label_and_prompt_size(&task.task_id, prompt_label, prompt.len());
    let logical_call_index = state.task_llm_call_count(&task.task_id);
    state.note_task_prompt_size_with_label(&task.task_id, prompt_label, prompt.len());
    let routing_plan = crate::providers::route_providers(providers, prompt.len(), &hints);
    state.note_task_provider_routing_plan_with_label(
        &task.task_id,
        prompt_label,
        routing_plan.evaluations,
    );
    let providers = routing_plan.providers;
    if providers.is_empty() {
        return Err(NO_ELIGIBLE_LLM_PROVIDER_ERR.to_string());
    }
    touch_llm_task_lease(
        state,
        &task.task_id,
        task.claim_attempt,
        prompt_label,
        "call_start",
    );
    let mut heartbeat_stop = Some(start_llm_task_lease_heartbeat(
        state.clone(),
        task.task_id.clone(),
        task.claim_attempt,
        prompt_label,
    ));
    let call_started_at = std::time::Instant::now();

    let mut last_error = "unknown llm error".to_string();
    let mut any_provider_attempted = false;
    let mut selected_provider_count = 0_u64;
    let mut skipped_providers: Vec<(String, u64)> = Vec::new();
    let mut recoverable_provider_blocker: Option<TaskProviderBlocker> = None;

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
            crate::providers::AttemptDecision::Allow => {
                selected_provider_count = selected_provider_count.saturating_add(1);
                state.note_task_provider_route_with_label(
                    &task.task_id,
                    prompt_label,
                    &provider_name,
                    true,
                    selected_provider_count > 1,
                    false,
                    false,
                    provider.breaker.snapshot(),
                );
            }
            crate::providers::AttemptDecision::AllowTrial => {
                selected_provider_count = selected_provider_count.saturating_add(1);
                state.note_task_provider_route_with_label(
                    &task.task_id,
                    prompt_label,
                    &provider_name,
                    true,
                    selected_provider_count > 1,
                    false,
                    true,
                    provider.breaker.snapshot(),
                );
                info!(
                    "{} [LLM_CALL] stage=circuit_half_open task_id={} provider={} prompt_source={} note=trial_after_cooldown",
                    crate::highlight_tag("llm"),
                    task.task_id,
                    provider_name,
                    prompt_source
                );
            }
            crate::providers::AttemptDecision::SkipCooldown { remaining_ms } => {
                state.note_task_provider_route_with_label(
                    &task.task_id,
                    prompt_label,
                    &provider_name,
                    false,
                    false,
                    true,
                    false,
                    provider.breaker.snapshot(),
                );
                warn!(
                    "{} [LLM_CALL] stage=circuit_open_skip task_id={} provider={} prompt_source={} cooldown_remaining_ms={}",
                    crate::highlight_tag("llm"),
                    task.task_id,
                    provider_name,
                    prompt_source,
                    remaining_ms
                );
                skipped_providers.push((provider_name, remaining_ms));
                continue;
            }
        }
        if !llm_cost_policy_allows(
            state,
            task,
            Some(provider.config.name.as_str()),
            prompt_source,
        ) {
            state.note_task_llm_elapsed_with_label(
                &task.task_id,
                prompt_label,
                call_started_at.elapsed().as_millis() as u64,
            );
            stop_llm_task_lease_heartbeat(&mut heartbeat_stop);
            touch_llm_task_lease(
                state,
                &task.task_id,
                task.claim_attempt,
                prompt_label,
                "cost_policy_wait",
            );
            return Err(TASK_LLM_COST_POLICY_BLOCKED_ERR.to_string());
        }
        any_provider_attempted = true;
        touch_llm_task_lease(
            state,
            &task.task_id,
            task.claim_attempt,
            prompt_label,
            "provider_attempt_start",
        );

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

        let provider_started_at = std::time::Instant::now();
        match crate::call_provider_with_retry_with_hints(provider.clone(), prompt, &hints).await {
            Ok(output) => {
                provider
                    .latency
                    .note_sample(provider_started_at.elapsed().as_millis() as u64);
                touch_llm_task_lease(
                    state,
                    &task.task_id,
                    task.claim_attempt,
                    prompt_label,
                    "provider_success",
                );
                state.note_task_provider_attempts_with_label(
                    &task.task_id,
                    prompt_label,
                    output.attempts,
                    output.retryable_error_count,
                    output.last_retry_error_kind,
                    None,
                );
                let (cleaned_text, sanitized) =
                    crate::maybe_sanitize_llm_text_output(vendor, &output.text);
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
                record_llm_cost(
                    state,
                    task,
                    crate::providers::build_cost_record(
                        logical_call_index,
                        prompt_label,
                        &provider.config.name,
                        &provider.config.model,
                        "ok",
                        output.attempts,
                        output.usage.as_ref(),
                        provider.pricing.as_ref(),
                    ),
                    prompt_source,
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
                state.note_task_provider_breaker_snapshot_with_label(
                    &task.task_id,
                    prompt_label,
                    &provider_name,
                    provider.breaker.snapshot(),
                );
                state.clear_task_provider_blocker(&task.task_id);
                stop_llm_task_lease_heartbeat(&mut heartbeat_stop);
                touch_llm_task_lease(
                    state,
                    &task.task_id,
                    task.claim_attempt,
                    prompt_label,
                    "call_success",
                );
                return Ok(cleaned_text);
            }
            Err(err) => {
                provider
                    .latency
                    .note_sample(provider_started_at.elapsed().as_millis() as u64);
                touch_llm_task_lease(
                    state,
                    &task.task_id,
                    task.claim_attempt,
                    prompt_label,
                    "provider_error",
                );
                let error_kind = err.observability_kind();
                if let Some(retry_after_seconds) = err.background_wait_seconds() {
                    let message_key = match err.kind {
                        ProviderErrorKind::QuotaExhausted => "provider.quota_exhausted",
                        ProviderErrorKind::RateLimited => "provider.rate_limited",
                        _ => "provider.temporarily_unavailable",
                    };
                    recoverable_provider_blocker = Some(TaskProviderBlocker {
                        provider: provider.config.name.clone(),
                        status_code: error_kind.to_string(),
                        retry_after_seconds,
                        external_provider_blocked: true,
                        message_key: message_key.to_string(),
                    });
                }
                state.note_task_provider_attempts_with_label(
                    &task.task_id,
                    prompt_label,
                    err.attempts,
                    err.retryable_error_count,
                    None,
                    Some(error_kind),
                );
                if err.should_trip_breaker() {
                    provider.breaker.note_failure();
                } else if err.should_reset_breaker() {
                    // breaker 只跟踪 provider 基础设施健康。当前这次调用已经拿到了
                    // 有效 provider 响应（哪怕业务上是 4xx/429/blocked），说明
                    // 上游链路已恢复，不应继续维持 Open/HalfOpen。
                    provider.breaker.note_success();
                }
                state.note_task_provider_breaker_snapshot_with_label(
                    &task.task_id,
                    prompt_label,
                    &provider_name,
                    provider.breaker.snapshot(),
                );
                last_error =
                    format!("provider={provider_name} error_kind={error_kind} failed: {err}");
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
                record_llm_cost(
                    state,
                    task,
                    crate::providers::build_cost_record(
                        logical_call_index,
                        prompt_label,
                        &provider.config.name,
                        &provider.config.model,
                        "failed",
                        err.attempts,
                        err.usage.as_ref(),
                        provider.pricing.as_ref(),
                    ),
                    prompt_source,
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
    stop_llm_task_lease_heartbeat(&mut heartbeat_stop);
    touch_llm_task_lease(
        state,
        &task.task_id,
        task.claim_attempt,
        prompt_label,
        "call_failed",
    );
    if !any_provider_attempted && !skipped_providers.is_empty() {
        // 全员被 breaker 拦下，所有 provider 当前都在 cooldown。
        // 这种情况要给一个**明确的可识别错误**，让上游日志/指标里能看见
        // "短时间集中失败"信号，而不是含糊的 "unknown llm error"。
        let retry_after_seconds = skipped_providers
            .iter()
            .map(|(_, remaining_ms)| remaining_ms.saturating_add(999) / 1_000)
            .min()
            .unwrap_or(1)
            .max(1);
        state.note_task_provider_blocker(
            &task.task_id,
            TaskProviderBlocker {
                provider: "provider_set".to_string(),
                status_code: "provider_circuit_open".to_string(),
                retry_after_seconds,
                external_provider_blocked: true,
                message_key: "provider.temporarily_unavailable".to_string(),
            },
        );
        let detail = skipped_providers
            .iter()
            .map(|(provider, remaining_ms)| format!("{provider}(cooldown_ms={remaining_ms})"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "all llm providers in circuit-breaker cooldown: {detail}"
        ));
    }
    if !skipped_providers.is_empty() {
        // 至少有一个 provider 真的被打了（且全失败），但同时也跳过了若干在
        // cooldown 里的 provider。把跳过列表追加到 last_error 便于排障。
        let skipped_detail = skipped_providers
            .iter()
            .map(|(provider, remaining_ms)| format!("{provider}(cooldown_ms={remaining_ms})"))
            .collect::<Vec<_>>()
            .join(", ");
        last_error = format!("{last_error}; skipped_in_cooldown=[{}]", skipped_detail);
    }
    if let Some(blocker) = recoverable_provider_blocker {
        state.note_task_provider_blocker(&task.task_id, blocker);
    }
    Err(last_error)
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
#[path = "llm_gateway_tests.rs"]
mod tests;

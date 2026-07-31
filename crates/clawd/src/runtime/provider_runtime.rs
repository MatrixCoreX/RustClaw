use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use claw_core::config::AgentConfig;
use claw_core::model_turn::{ProviderModelCapabilities, ProviderModelDescriptor};
use reqwest::Client;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub(crate) struct LlmProviderRuntime {
    pub(crate) config: claw_core::config::LlmProviderConfig,
    pub(crate) pricing: Option<claw_core::config::LlmModelPricingConfig>,
    pub(crate) latency: Arc<crate::providers::LlmProviderLatencyTracker>,
    pub(crate) client: Client,
    pub(crate) semaphore: Arc<Semaphore>,
    /// Phase 2.1: 每 provider 一个 circuit breaker，避免坏 provider 在 fallback
    /// 链路里被反复重试 + 反复消耗 retry/timeout 预算。`Arc` 保证 `Clone` 后
    /// 多份引用共享同一份故障状态。
    pub(crate) breaker: Arc<crate::providers::CircuitBreaker>,
}

impl LlmProviderRuntime {
    pub(crate) fn model_descriptor(&self) -> ProviderModelDescriptor {
        let protocol = self.config.provider_type.trim();
        let adapter_supports_tools = matches!(
            protocol,
            "openai_compat" | "anthropic_claude" | "google_gemini"
        );
        let native_tools = self.config.supports_tools && adapter_supports_tools;
        let capabilities = ProviderModelCapabilities {
            native_tools,
            parallel_tools: native_tools,
            structured_output: protocol == "google_gemini",
            streaming: protocol == "openai_compat",
            reasoning: false,
            vision: self
                .config
                .input_modalities
                .iter()
                .any(|modality| modality.eq_ignore_ascii_case("image")),
            prompt_cache: false,
        };
        ProviderModelDescriptor {
            capabilities,
            context_window_tokens: self.config.context_window_tokens,
            output_reserve_tokens: usize::try_from(
                self.config.params.default_max_tokens.unwrap_or(4_096),
            )
            .unwrap_or(usize::MAX),
            request_timeout_seconds: self.config.timeout_seconds.max(1),
            estimator_confidence: crate::token_estimator::estimator_confidence(
                &self.config.name,
                &self.config.provider_type,
                &self.config.model,
            ),
        }
    }

    pub(crate) fn model_capabilities(&self) -> ProviderModelCapabilities {
        self.model_descriptor().capabilities
    }

    /// §P4.4 E3.a：根据 vendor 从 `provider.config.name` 推断 secret name 形式。
    ///
    /// 命名约定来自 [`crate::llm_gateway::synthesize_llm_providers`]：所有
    /// runtime provider 的 `config.name` 形如 `vendor-<vendor>`（vendor =
    /// `openai` / `google` / `anthropic` / `grok` / `xai` / `deepseek` / `qwen`
    /// / `minimax`）。strip `vendor-` 前缀后即得 vendor 名。
    ///
    /// 命名不符合约定（例如用户在 `[[llm_providers]]` 自定义了 `name = "my-llm"`）
    /// 时返回 `None` —— 调用方应当 fallback 到 `config.api_key`，避免拼出诸如
    /// `text_my-llm_api_key`（含 `-`）这种通不过 `validate_secret_name` 的形态。
    fn vendor_name_for_secret_lookup(&self) -> Option<String> {
        let raw = self.config.name.trim();
        let vendor = raw.strip_prefix("vendor-")?.trim();
        if vendor.is_empty() {
            return None;
        }
        // §P4.4 E3.a: secret name 必须是 [a-z0-9_]，所以 vendor 名也必须满足。
        // 不满足直接 None ⇒ fallback 到 config.api_key，避免在 broker 那边触发
        // InvalidName 错误（那是上层 config 的责任，不是 broker 的）。
        if !vendor
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return None;
        }
        Some(vendor.to_string())
    }

    /// §P4.4 E3.a：拿 LLM 调用要用的 api_key —— **broker 优先，config 兜底**。
    ///
    /// 调用顺序：
    /// 1. 推断 vendor（见 [`Self::vendor_name_for_secret_lookup`]）；
    ///    推不出来直接走 `config.api_key`。
    /// 2. 拼 `text_<vendor>_api_key`，问 [`claw_core::secrets::global_or_default`]
    ///    持有的 broker；命中 ⇒ 用 broker 的值（一次拷贝出来交给调用方所有权）。
    /// 3. broker 未命中 / 出错 ⇒ DEBUG 日志 + 回落 `config.api_key`，**不打 WARN**
    ///    （DEBUG 是因为绝大多数部署里 broker 本来就没声明 chat 凭据，回落是预期路径）。
    ///
    /// 软合入语义：**broker 没装就行为零变化**——chat builtin 与 spawn-path 的
    /// `OPENAI_API_KEY` forge 都仍然读 `[llm.<vendor>].api_key`。一旦 §E3.b 装上
    /// `CachingTokenBroker`、或运维自己 install 了 token broker，本方法自动接管。
    ///
    /// 设计权衡：返回 `Cow` 是因为 `config.api_key` 是 `String` 字段、broker
    /// 命中要拷贝出来 —— 没必要让调用方都背 owned 拷贝代价。
    pub(crate) fn api_key(&self) -> std::borrow::Cow<'_, str> {
        let broker = claw_core::secrets::global_or_default();
        self.api_key_using(broker.as_ref())
    }

    /// 测试与扩展点：允许显式注入 broker（避免污染 `OnceLock` 单例）。
    pub(crate) fn api_key_using<'a>(
        &'a self,
        broker: &dyn claw_core::secrets::SecretsBroker,
    ) -> std::borrow::Cow<'a, str> {
        let Some(vendor) = self.vendor_name_for_secret_lookup() else {
            return std::borrow::Cow::Borrowed(&self.config.api_key);
        };
        let secret_name = claw_core::secrets::text_secret_name_for_vendor(&vendor);
        match broker.lookup(&secret_name) {
            Ok(Some(secret)) => std::borrow::Cow::Owned(secret.expose().to_string()),
            Ok(None) => {
                tracing::debug!(
                    "llm_provider_api_key vendor={} broker_label={} secret={} status=miss fallback=config",
                    vendor,
                    broker.label(),
                    secret_name
                );
                std::borrow::Cow::Borrowed(&self.config.api_key)
            }
            Err(err) => {
                tracing::debug!(
                    "llm_provider_api_key vendor={} broker_label={} secret={} status=err err={} fallback=config",
                    vendor,
                    broker.label(),
                    secret_name,
                    err
                );
                std::borrow::Cow::Borrowed(&self.config.api_key)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AgentRuntimeConfig {
    pub(crate) id: String,
    pub(crate) configured_persona_profile: String,
    pub(crate) persona_profile: String,
    pub(crate) persona_fragment: String,
    pub(crate) persona_digest: String,
    pub(crate) runtime_digest: String,
    pub(crate) preferred_vendor: Option<String>,
    pub(crate) preferred_model: Option<String>,
    pub(crate) restrict_skills: bool,
    pub(crate) allowed_skills: Arc<HashSet<String>>,
    pub(crate) llm_providers: Vec<Arc<LlmProviderRuntime>>,
}

impl AgentRuntimeConfig {
    pub(crate) fn from_config(
        config: &AgentConfig,
        llm_providers: Vec<Arc<LlmProviderRuntime>>,
    ) -> Self {
        Self::from_config_with_persona(config, llm_providers, "executor")
    }

    pub(crate) fn from_config_with_persona(
        config: &AgentConfig,
        llm_providers: Vec<Arc<LlmProviderRuntime>>,
        global_persona_profile: &str,
    ) -> Self {
        let allowed_skills = config
            .allowed_skills
            .iter()
            .map(|skill| crate::canonical_skill_name(skill).to_string())
            .collect::<HashSet<_>>();
        let (configured_profile, _) =
            claw_core::config::normalize_agent_persona_profile(&config.persona_profile);
        let (global_profile, global_profile_known) =
            claw_core::config::normalize_agent_persona_profile(global_persona_profile);
        let effective_profile = if configured_profile == "inherit" {
            if global_profile_known && global_profile != "inherit" {
                global_profile
            } else {
                "executor"
            }
        } else {
            configured_profile
        };
        let persona_fragment = if effective_profile == "custom" {
            config.persona_fragment.trim().to_string()
        } else {
            String::new()
        };
        let digest_input = format!(
            "agent-persona-v1\n{}\n{}\n{}",
            config.id.trim(),
            effective_profile,
            persona_fragment
        );
        let persona_digest = format!("{:x}", Sha256::digest(digest_input.as_bytes()));
        let mut allowed_skill_names = allowed_skills.iter().cloned().collect::<Vec<_>>();
        allowed_skill_names.sort();
        let runtime_digest_input = format!(
            "agent-runtime-v1\n{}\n{}\n{}\n{}\n{}",
            config.id.trim(),
            persona_digest,
            config.preferred_vendor.as_deref().unwrap_or_default(),
            config.preferred_model.as_deref().unwrap_or_default(),
            allowed_skill_names.join("\n")
        );
        let runtime_digest = format!("{:x}", Sha256::digest(runtime_digest_input.as_bytes()));
        Self {
            id: config.id.trim().to_string(),
            configured_persona_profile: configured_profile.to_string(),
            persona_profile: effective_profile.to_string(),
            persona_fragment,
            persona_digest,
            runtime_digest,
            preferred_vendor: config.preferred_vendor.clone(),
            preferred_model: config.preferred_model.clone(),
            restrict_skills: !allowed_skills.is_empty(),
            allowed_skills: Arc::new(allowed_skills),
            llm_providers,
        }
    }

    pub(crate) fn allows_skill(&self, canonical_skill: &str) -> bool {
        !self.restrict_skills || self.allowed_skills.contains(canonical_skill)
    }

    pub(crate) fn task_snapshot_json(&self) -> serde_json::Value {
        let mut allowed_skills = self.allowed_skills.iter().cloned().collect::<Vec<_>>();
        allowed_skills.sort();
        serde_json::json!({
            "schema_version": 1,
            "agent_id": self.id,
            "configured_persona_profile": self.configured_persona_profile,
            "persona_profile": self.persona_profile,
            "persona_fragment": self.persona_fragment,
            "persona_digest": self.persona_digest,
            "runtime_digest": self.runtime_digest,
            "preferred_vendor": self.preferred_vendor,
            "preferred_model": self.preferred_model,
            "restrict_skills": self.restrict_skills,
            "allowed_skills": allowed_skills,
        })
    }

    pub(crate) fn from_task_snapshot(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        if object
            .get("schema_version")
            .and_then(|value| value.as_u64())
            != Some(1)
        {
            return None;
        }
        let id = object.get("agent_id")?.as_str()?.trim().to_string();
        let persona_profile = object.get("persona_profile")?.as_str()?.trim().to_string();
        let persona_digest = object.get("persona_digest")?.as_str()?.trim().to_string();
        let runtime_digest = object.get("runtime_digest")?.as_str()?.trim().to_string();
        if id.is_empty()
            || persona_profile.is_empty()
            || persona_digest.is_empty()
            || runtime_digest.is_empty()
        {
            return None;
        }
        let allowed_skills = object
            .get("allowed_skills")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .map(crate::canonical_skill_name)
            .map(ToString::to_string)
            .collect::<HashSet<_>>();
        Some(Self {
            id,
            configured_persona_profile: object
                .get("configured_persona_profile")
                .and_then(|value| value.as_str())
                .unwrap_or("inherit")
                .trim()
                .to_string(),
            persona_profile,
            persona_fragment: object
                .get("persona_fragment")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            persona_digest,
            runtime_digest,
            preferred_vendor: object
                .get("preferred_vendor")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
            preferred_model: object
                .get("preferred_model")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
            restrict_skills: object
                .get("restrict_skills")
                .and_then(|value| value.as_bool())
                .unwrap_or(!allowed_skills.is_empty()),
            allowed_skills: Arc::new(allowed_skills),
            llm_providers: Vec::new(),
        })
    }
}

pub(crate) fn build_agent_runtime_snapshot(
    config: &claw_core::config::AppConfig,
) -> HashMap<String, AgentRuntimeConfig> {
    let mut agents_by_id = HashMap::new();
    for agent in config.normalized_agents() {
        let override_providers =
            if agent.preferred_vendor.is_some() || agent.preferred_model.is_some() {
                crate::llm_gateway::build_providers_for_selection(
                    config,
                    agent.preferred_vendor.as_deref(),
                    agent.preferred_model.as_deref(),
                )
            } else {
                Vec::new()
            };
        agents_by_id.insert(
            agent.id.clone(),
            AgentRuntimeConfig::from_config_with_persona(
                &agent,
                override_providers,
                &config.persona.profile,
            ),
        );
    }
    agents_by_id
}

#[cfg(test)]
#[path = "provider_runtime_tests.rs"]
mod agent_runtime_config_tests;

#[cfg(test)]
#[path = "state_llm_provider_runtime_tests.rs"]
mod llm_provider_runtime_tests;

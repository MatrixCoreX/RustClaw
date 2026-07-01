use std::collections::HashSet;
use std::sync::Arc;

use claw_core::config::AgentConfig;
use reqwest::Client;
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub(crate) struct LlmProviderRuntime {
    pub(crate) config: claw_core::config::LlmProviderConfig,
    pub(crate) client: Client,
    pub(crate) semaphore: Arc<Semaphore>,
    /// Phase 2.1: 每 provider 一个 circuit breaker，避免坏 provider 在 fallback
    /// 链路里被反复重试 + 反复消耗 retry/timeout 预算。`Arc` 保证 `Clone` 后
    /// 多份引用共享同一份故障状态。
    pub(crate) breaker: Arc<crate::providers::CircuitBreaker>,
}

impl LlmProviderRuntime {
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
    pub(crate) restrict_skills: bool,
    pub(crate) allowed_skills: Arc<HashSet<String>>,
    pub(crate) llm_providers: Vec<Arc<LlmProviderRuntime>>,
}

impl AgentRuntimeConfig {
    pub(crate) fn from_config(
        config: &AgentConfig,
        llm_providers: Vec<Arc<LlmProviderRuntime>>,
    ) -> Self {
        let allowed_skills = config
            .allowed_skills
            .iter()
            .map(|skill| crate::canonical_skill_name(skill).to_string())
            .collect::<HashSet<_>>();
        Self {
            restrict_skills: !allowed_skills.is_empty(),
            allowed_skills: Arc::new(allowed_skills),
            llm_providers,
        }
    }

    pub(crate) fn allows_skill(&self, canonical_skill: &str) -> bool {
        !self.restrict_skills || self.allowed_skills.contains(canonical_skill)
    }
}

#[cfg(test)]
#[path = "state_llm_provider_runtime_tests.rs"]
mod llm_provider_runtime_tests;

//! §P4.4 E3.a: `LlmProviderRuntime::api_key()` 行为单测。
//!
//! 关键设计：用 `api_key_using(&dyn SecretsBroker)` 显式注入 broker，
//! 避免污染 `claw_core::secrets::GLOBAL_BROKER` 这个 OnceLock 单例
//! （一旦 set 就锁死，会让其它测试拿不到默认 EnvBroker）。
use super::*;
use claw_core::config::{LlmProviderConfig, LlmProviderParams};
use claw_core::secrets::{SecretValue, SecretsBroker, SecretsError};

fn make_provider(name: &str, api_key: &str) -> LlmProviderRuntime {
    LlmProviderRuntime {
        config: LlmProviderConfig {
            name: name.to_string(),
            provider_type: "openai_compat".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            api_key: api_key.to_string(),
            model: "test-model".to_string(),
            context_window_tokens: None,
            priority: 1,
            timeout_seconds: 30,
            max_concurrency: 1,
            params: LlmProviderParams::default(),
        },
        pricing: None,
        client: reqwest::Client::new(),
        semaphore: Arc::new(Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    }
}

/// 返回固定值的 mock broker。
struct FixedBroker {
    expected_name: String,
    value: String,
}
impl SecretsBroker for FixedBroker {
    fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
        if name == self.expected_name {
            Ok(Some(SecretValue::new(self.value.clone())))
        } else {
            Ok(None)
        }
    }
    fn label(&self) -> &str {
        "fixed-mock"
    }
}

/// 永远 None。
struct AlwaysMissBroker;
impl SecretsBroker for AlwaysMissBroker {
    fn lookup(&self, _name: &str) -> Result<Option<SecretValue>, SecretsError> {
        Ok(None)
    }
    fn label(&self) -> &str {
        "always-miss-mock"
    }
}

/// 永远 Err。
struct AlwaysErrBroker;
impl SecretsBroker for AlwaysErrBroker {
    fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
        Err(SecretsError::BackendIo {
            name: name.to_string(),
            source: std::io::Error::other("simulated outage"),
        })
    }
    fn label(&self) -> &str {
        "always-err-mock"
    }
}

#[test]
fn api_key_uses_broker_value_when_present_for_recognized_vendor() {
    // vendor-openai → text_openai_api_key
    let provider = make_provider("vendor-openai", "config-fallback-key");
    let broker = FixedBroker {
        expected_name: "text_openai_api_key".to_string(),
        value: "broker-issued-key".to_string(),
    };
    let key = provider.api_key_using(&broker);
    assert_eq!(
        &*key, "broker-issued-key",
        "broker value must take priority"
    );
    assert!(
        matches!(key, std::borrow::Cow::Owned(_)),
        "broker hit must produce Cow::Owned to detach from broker lifetime"
    );
}

#[test]
fn api_key_falls_back_to_config_when_broker_misses() {
    let provider = make_provider("vendor-anthropic", "config-fallback-key");
    let key = provider.api_key_using(&AlwaysMissBroker);
    assert_eq!(&*key, "config-fallback-key");
    assert!(
        matches!(key, std::borrow::Cow::Borrowed(_)),
        "miss must Cow::Borrowed config field, no allocation"
    );
}

#[test]
fn api_key_falls_back_to_config_when_broker_errors() {
    // broker err 时也必须 fallback —— 不能让一次 broker outage 把所有
    // LLM 调用全弄成空 key。
    let provider = make_provider("vendor-google", "config-fallback-key");
    let key = provider.api_key_using(&AlwaysErrBroker);
    assert_eq!(&*key, "config-fallback-key");
}

#[test]
fn api_key_falls_back_for_non_vendor_prefix_provider_name() {
    // 用户在 `[[llm_providers]]` 自定义 name = "my-llm" 时，没法推 vendor，
    // 必须直接走 config.api_key（绝不喂 `text_my-llm_api_key` 给 broker，
    // 那个名字含 `-`，会触发 InvalidName）。
    let provider = make_provider("my-llm", "config-fallback-key");
    // 即使给 broker 配了任何 secret，也不该被命中（因为 vendor 推不出来）。
    let broker = FixedBroker {
        expected_name: "anything".to_string(),
        value: "should-not-be-used".to_string(),
    };
    let key = provider.api_key_using(&broker);
    assert_eq!(&*key, "config-fallback-key");
}

#[test]
fn api_key_falls_back_when_vendor_part_contains_invalid_chars() {
    // strip `vendor-` 后剩 `Foo-Bar`，含大写 + `-`，不通过 [a-z0-9_]
    // 校验 → 直接走 config，避免在 broker 端触发 InvalidName。
    let provider = make_provider("vendor-Foo-Bar", "config-fallback-key");
    let key = provider.api_key_using(&AlwaysMissBroker);
    assert_eq!(&*key, "config-fallback-key");
}

#[test]
fn api_key_default_path_uses_global_broker() {
    // 默认路径走 `claw_core::secrets::global_or_default()`，没 install 时
    // 是 EnvBroker；env 没设 ⇒ miss ⇒ fallback。本测试只验证 default 入口
    // 不 panic、与 config 一致，不依赖 env 状态（避免与并发测试竞争）。
    let provider = make_provider("vendor-openai", "default-path-fallback");
    // 故意挑一个不可能在 env 里的 vendor 名前缀，确保 miss
    // （EnvBroker 会查 TEXT_OPENAI_API_KEY，若 CI 机器恰好设了，断言会换路径但仍合法 —— 见下注）。
    let key = provider.api_key();
    // 不强 assert == "default-path-fallback"，因为 CI 机器可能配了
    // TEXT_OPENAI_API_KEY 环境变量。两种情况都是合法行为：
    //   - env 没设 → fallback 到 config
    //   - env 设了 → broker 接管
    // 关键是 `api_key()` 不能 panic / 返回空字符串（除非 config 本身就空）。
    assert!(
        !key.is_empty(),
        "api_key must not be empty when config has value"
    );
}

#[test]
fn vendor_name_strip_handles_known_vendors() {
    for vendor in [
        "openai",
        "google",
        "anthropic",
        "grok",
        "xai",
        "deepseek",
        "qwen",
        "minimax",
    ] {
        let provider = make_provider(&format!("vendor-{vendor}"), "k");
        let extracted = provider.vendor_name_for_secret_lookup();
        assert_eq!(
            extracted.as_deref(),
            Some(vendor),
            "expected vendor `{vendor}` to be extracted"
        );
    }
}

#[test]
fn vendor_name_strip_returns_none_for_non_vendor_prefix() {
    for raw in ["my-llm", "openai", "vendor-", "vendor-  ", ""] {
        let provider = make_provider(raw, "k");
        assert!(
            provider.vendor_name_for_secret_lookup().is_none(),
            "raw=`{raw}` should yield None, got Some"
        );
    }
}

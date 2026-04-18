//! §P4.4 secrets 短期 token：地基 trait + 默认 env 后端。
//!
//! **本模块的边界**
//!
//! 本模块只负责"按 secret 名查到一段凭证"，不负责：
//! - **颁发短期 token**（由 clawd 启动时根据 manifest 在 broker 之上包装一层
//!   token broker —— 见 §P4.4 待办）；
//! - **跨进程注入**（runner spawn 前把 token 塞进子进程 env 是 §E1.b 的事）；
//! - **审计 / 轮转**（清算谁拿了 token、什么时候过期）。
//!
//! 这是地基中的地基，独立可验证。
//!
//! **为什么 secret 名是 `<usage>_<vendor>_api_key` 形态**
//!
//! 见 [`crate::skill_registry::Capability`] 的 doc：本仓里 image_generation /
//! image_edit / image_vision / `[llm]`(chat & 规划) 是 4 套互相独立的 LLM
//! provider 配置，同一个 vendor 在不同用途下可以填不同 base_url + api_key。
//! 所以 secret 命名按 `<usage>_<vendor>_api_key` 展开，broker 也只接受这种
//! 形态的 key —— `Capability::parse` 已经在类型层拦截裸 vendor 反模式，本模
//! 块在运行期再加一道 sanity check。
//!
//! **设计原则**
//!
//! - `SecretValue` 永远不实现 `Debug` / `Display` 显示明文；只通过
//!   [`SecretValue::expose`] 拿到 `&str`，强迫调用者显式承担"看到明文"的责任。
//! - `SecretsBroker` trait 是 dyn-safe（`&self` + 不带泛型），便于运行期注入
//!   不同后端（env / KMS / vault / mock）。
//! - 默认实现 [`EnvSecretsBroker`] 把 secret 名按"全大写 + 可选前缀"映射到
//!   环境变量；空值视为不存在（避免被误用为 `Some("")`）。

use std::env;
use std::fmt;

use thiserror::Error;

/// secret 内容的强类型包装。
///
/// 不实现 [`fmt::Debug`] / [`fmt::Display`] 的明文格式化，避免被 `dbg!` /
/// `tracing::info!` 误打印；如需在日志里出现，请用 [`Self::redacted_label`]。
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue {
    inner: String,
}

impl SecretValue {
    /// 显式封装一段凭证。空字符串会被当作空 secret，调用方应自己决定要不要拒绝。
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            inner: value.into(),
        }
    }

    /// 拿到明文 —— 唯一的明文出口，命名故意提醒副作用。
    pub fn expose(&self) -> &str {
        &self.inner
    }

    /// 给日志/错误信息用的安全占位（永远不打明文）。
    pub fn redacted_label() -> &'static str {
        "[REDACTED]"
    }

    /// 内容是否为空（用于上层判定"虽然 lookup 返回了，但其实没设置"）。
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// 字符长度（仅用于审计/限流，绝不打印明文）。
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 故意不打 self.inner —— 即便有人 #[derive(Debug)] 了上层 struct，
        // 这里也兜底脱敏。len 给运维一点点诊断能力，不泄露内容。
        f.debug_struct("SecretValue")
            .field("value", &Self::redacted_label())
            .field("len", &self.inner.len())
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum SecretsError {
    /// secret 名不合法（违反 §P4.1 命名规范）。
    #[error("invalid secret name `{name}`: {reason}")]
    InvalidName { name: String, reason: String },
    /// 后端访问失败（例如读 env 时遇到非 UTF-8 字节）。
    #[error("backend error while looking up `{name}`: {source}")]
    BackendIo {
        name: String,
        #[source]
        source: std::io::Error,
    },
}

/// secrets 后端契约。dyn-safe：`&self`、不带泛型方法、不需要 `Sized`。
///
/// 实现者只要回答"按这个名字能不能找到 secret"。**不**负责命名规范校验
/// （由 [`validate_secret_name`] 统一做，见 §P4.1 / §P4.4 设计）—— broker
/// 内部应当先调用一次 `validate_secret_name`。
pub trait SecretsBroker: Send + Sync {
    /// 按 canonical 名查 secret。
    /// - `Ok(Some(_))`：找到非空值；
    /// - `Ok(None)`：未配置（或值为空，按 broker 实现统一约定）；
    /// - `Err(_)`：查询过程出错（命名违规或后端 IO 失败）。
    fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError>;

    /// broker 标识（日志/审计用），不要包含敏感信息。
    fn label(&self) -> &str {
        "<unnamed-broker>"
    }
}

/// §P4.1 命名规范在运行期的二道防线。
///
/// 与 [`crate::skill_registry::Capability::parse`] 的拒绝集合保持同步 ——
/// `parse` 在 registry 加载阶段拦"裸 vendor 名"，这里在 broker 查询阶段再
/// 拦一次，避免有人绕过 registry 直接调 `broker.lookup("openai_api_key")`。
pub fn validate_secret_name(name: &str) -> Result<(), SecretsError> {
    let raw = name.trim();
    if raw.is_empty() {
        return Err(SecretsError::InvalidName {
            name: name.to_string(),
            reason: "secret name must not be empty".to_string(),
        });
    }
    if raw.len() > 64 {
        return Err(SecretsError::InvalidName {
            name: name.to_string(),
            reason: "secret name length must be 1..=64".to_string(),
        });
    }
    if !raw
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(SecretsError::InvalidName {
            name: name.to_string(),
            reason: "secret name must match [a-z0-9_]".to_string(),
        });
    }
    // 与 Capability::parse 同源的"裸 vendor"反模式拒绝集合。
    const KNOWN_VENDORS: &[&str] = &[
        "openai", "google", "gemini", "anthropic", "claude", "grok", "xai", "deepseek", "qwen",
        "minimax",
    ];
    for vendor in KNOWN_VENDORS {
        if raw == format!("{vendor}_api_key") || raw == *vendor {
            return Err(SecretsError::InvalidName {
                name: name.to_string(),
                reason: format!(
                    "bare vendor naming conflates the four independent LLM provider configs (image_generation/image_edit/image_vision/text); rename to `<usage>_{vendor}_api_key`"
                ),
            });
        }
    }
    Ok(())
}

/// 默认后端：从环境变量读 secret。
///
/// 命名映射：`secret_name.to_ascii_uppercase()`，可选前缀拼接。
/// 例：name = `image_generation_minimax_api_key`、prefix 为 None
/// → 查 `IMAGE_GENERATION_MINIMAX_API_KEY`。
///
/// 空值（""）会被当作 `Ok(None)`，避免后台拿到空字符串以为是 secret 实际写的。
#[derive(Debug, Clone, Default)]
pub struct EnvSecretsBroker {
    /// 可选前缀。例：`Some("CLAWD_")` 时 lookup("foo") 查 `CLAWD_FOO`。
    /// 主要给隔离测试 / 多租户用。
    prefix: Option<String>,
}

impl EnvSecretsBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        let p = prefix.into();
        Self {
            prefix: if p.is_empty() { None } else { Some(p) },
        }
    }

    /// 把 canonical secret 名翻译成 env var 名。public 仅供测试与日志使用。
    pub fn env_var_name(&self, canonical: &str) -> String {
        let upper = canonical.to_ascii_uppercase();
        match &self.prefix {
            Some(p) => format!("{p}{upper}"),
            None => upper,
        }
    }
}

impl SecretsBroker for EnvSecretsBroker {
    fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
        validate_secret_name(name)?;
        let var = self.env_var_name(name);
        match env::var(&var) {
            Ok(value) if !value.is_empty() => Ok(Some(SecretValue::new(value))),
            Ok(_) => Ok(None), // 显式空 = 未配置
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => Err(SecretsError::BackendIo {
                name: name.to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("env var `{var}` is not valid UTF-8"),
                ),
            }),
        }
    }

    fn label(&self) -> &str {
        "env"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成一个进程内唯一的小写 secret name。
    /// 注意：env 是进程全局态，必须用唯一名字避免并发测试相互覆盖。
    /// 返回值已经是合法的 [a-z0-9_] canonical secret name。
    fn fresh_var(suffix: &str) -> String {
        format!("claw_test_e1a_{}_{}", std::process::id(), suffix)
    }

    #[test]
    fn secret_value_debug_does_not_leak_plaintext() {
        let s = SecretValue::new("super-secret-token");
        let printed = format!("{s:?}");
        assert!(!printed.contains("super-secret-token"), "{printed}");
        assert!(printed.contains("[REDACTED]"), "{printed}");
        // len 字段还在
        assert!(printed.contains("len: 18"), "{printed}");
    }

    #[test]
    fn secret_value_expose_returns_plaintext_only_via_explicit_call() {
        let s = SecretValue::new("abc");
        assert_eq!(s.expose(), "abc");
        assert_eq!(s.len(), 3);
        assert!(!s.is_empty());
    }

    #[test]
    fn validate_secret_name_accepts_canonical_usage_vendor_form() {
        for ok in [
            "image_generation_minimax_api_key",
            "image_edit_qwen_api_key",
            "image_vision_openai_api_key",
            "text_openai_api_key",
            "chat_minimax_api_key",
            // 长度恰到 64：a + _ * 62 + a = 64 字符
            &format!("a{}a", "_".repeat(62)),
        ] {
            assert!(
                validate_secret_name(ok).is_ok(),
                "expected `{ok}` to pass validation"
            );
        }
    }

    #[test]
    fn validate_secret_name_rejects_bad_shapes() {
        for bad in ["", "  ", "Has-Dash", "has space", "UPPER", "中文"] {
            assert!(
                validate_secret_name(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
        let too_long = "a".repeat(65);
        assert!(validate_secret_name(&too_long).is_err());
    }

    #[test]
    fn validate_secret_name_rejects_bare_vendor_pattern() {
        // 与 Capability::parse 的拒绝集合保持同步
        for bare in [
            "openai_api_key",
            "minimax_api_key",
            "anthropic_api_key",
            "claude_api_key",
            "openai",
            "minimax",
        ] {
            let err = validate_secret_name(bare)
                .err()
                .unwrap_or_else(|| panic!("expected `{bare}` to be rejected"));
            let msg = format!("{err}");
            assert!(
                msg.contains("bare vendor naming"),
                "expected bare-vendor message, got: {msg}"
            );
            assert!(
                msg.contains("<usage>_"),
                "expected hint with usage prefix, got: {msg}"
            );
        }
    }

    #[test]
    fn env_broker_returns_none_when_var_missing() {
        let broker = EnvSecretsBroker::new();
        // 用绝对不会被设置的随机名（带 PID 隔离并发测试）
        let canonical = fresh_var("missing");
        // 不 set
        let result = broker.lookup(&canonical).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn env_broker_returns_some_when_var_set_and_nonempty() {
        let broker = EnvSecretsBroker::new();
        let canonical = fresh_var("present");
        let env_var = broker.env_var_name(&canonical);
        // SAFETY: 单测，自己设自己读，结尾清理
        unsafe { env::set_var(&env_var, "actual-value-xyz") };
        let result = broker.lookup(&canonical).unwrap();
        unsafe { env::remove_var(&env_var) };
        let secret = result.expect("should have found secret");
        assert_eq!(secret.expose(), "actual-value-xyz");
    }

    #[test]
    fn env_broker_treats_empty_env_value_as_missing() {
        let broker = EnvSecretsBroker::new();
        let canonical = fresh_var("empty");
        let env_var = broker.env_var_name(&canonical);
        unsafe { env::set_var(&env_var, "") };
        let result = broker.lookup(&canonical).unwrap();
        unsafe { env::remove_var(&env_var) };
        assert!(result.is_none(), "empty env value should be treated as None");
    }

    #[test]
    fn env_broker_with_prefix_isolates_lookup_namespace() {
        let broker = EnvSecretsBroker::with_prefix("CLAW_TEST_E1A_PFX_");
        let canonical = fresh_var("ns");
        let env_var = broker.env_var_name(&canonical);
        // 验证 prefix 实际起作用：env_var 必须以 prefix 开头
        assert!(env_var.starts_with("CLAW_TEST_E1A_PFX_CLAW_TEST_E1A_"));
        unsafe { env::set_var(&env_var, "ns-value") };
        let result = broker.lookup(&canonical).unwrap();
        unsafe { env::remove_var(&env_var) };
        let secret = result.expect("prefixed lookup should resolve");
        assert_eq!(secret.expose(), "ns-value");

        // 反向验证：不带 prefix 的同 canonical 在 plain broker 上应当找不到
        let plain = EnvSecretsBroker::new();
        assert!(
            plain.lookup(&canonical).unwrap().is_none(),
            "plain broker must not see the prefixed env var"
        );
    }

    #[test]
    fn env_broker_rejects_invalid_secret_names_before_touching_env() {
        let broker = EnvSecretsBroker::new();
        let err = broker.lookup("openai_api_key").err().unwrap();
        assert!(matches!(err, SecretsError::InvalidName { .. }));
    }

    #[test]
    fn env_broker_env_var_name_uppercases_and_prefixes() {
        let plain = EnvSecretsBroker::new();
        assert_eq!(
            plain.env_var_name("image_generation_minimax_api_key"),
            "IMAGE_GENERATION_MINIMAX_API_KEY"
        );
        let prefixed = EnvSecretsBroker::with_prefix("APP_");
        assert_eq!(
            prefixed.env_var_name("text_openai_api_key"),
            "APP_TEXT_OPENAI_API_KEY"
        );
    }

    #[test]
    fn env_broker_label_is_stable() {
        let broker = EnvSecretsBroker::new();
        assert_eq!(broker.label(), "env");
    }

    /// 验证 trait 是 dyn-safe：能放进 `Box<dyn SecretsBroker>`，便于运行期注入。
    #[test]
    fn secrets_broker_trait_is_dyn_safe() {
        let _boxed: Box<dyn SecretsBroker> = Box::new(EnvSecretsBroker::new());
    }
}

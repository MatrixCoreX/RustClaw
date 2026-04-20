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
use std::sync::{Arc, OnceLock};

use thiserror::Error;

use crate::skill_registry::Capability;

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

/// §P4.4 E3.a：把 "`<usage>_<vendor>_api_key`" 命名规则集中成函数，
/// 避免 4 套独立 LLM provider 配置在调用点散写字符串。
///
/// 4 个 usage 来自 [`crate::skill_registry::Capability`] 的设计文档：
/// `text` / `image_generation` / `image_edit` / `image_vision`，互相**独立**
/// （同一 vendor 在不同用途可填不同 base_url + api_key）。
///
/// vendor 名按 ASCII 小写归一；调用方负责传"裸 vendor 名"（如 `"openai"`），
/// **不要**自己在外面拼前缀，否则 [`validate_secret_name`] 会在 broker 查询
/// 阶段把它当成"裸 vendor 反模式"拒绝。
///
/// 返回的 secret name 永远满足 `validate_secret_name`，可直接喂给
/// `SecretsBroker::lookup`。
pub fn text_secret_name_for_vendor(vendor: &str) -> String {
    format!("text_{}_api_key", vendor.trim().to_ascii_lowercase())
}

/// 见 [`text_secret_name_for_vendor`]。
pub fn image_generation_secret_name_for_vendor(vendor: &str) -> String {
    format!(
        "image_generation_{}_api_key",
        vendor.trim().to_ascii_lowercase()
    )
}

/// 见 [`text_secret_name_for_vendor`]。
pub fn image_edit_secret_name_for_vendor(vendor: &str) -> String {
    format!("image_edit_{}_api_key", vendor.trim().to_ascii_lowercase())
}

/// 见 [`text_secret_name_for_vendor`]。
pub fn image_vision_secret_name_for_vendor(vendor: &str) -> String {
    format!(
        "image_vision_{}_api_key",
        vendor.trim().to_ascii_lowercase()
    )
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

// ============================================================================
// §E1.b: provision_secret_envs —— 按 manifest capabilities 把 secrets 翻译成
// 子进程要看到的 (ENV_VAR_NAME, SecretValue) 列表。
// ============================================================================

#[derive(Debug, Error)]
pub enum ProvisionError {
    /// manifest 声明了 secrets.<name> 但 broker 找不到对应凭证。
    /// **fail-loud**：调用方应当直接拒绝 spawn，绝不让 skill 拿空字符串去
    /// 打 vendor API（那样会在远端日志里留下"鉴权失败"，难排查）。
    #[error("missing secrets: {missing:?}")]
    MissingSecrets { missing: Vec<String> },
    /// 单条 secret 查询失败（命名违规 / 后端 IO）。
    #[error("secret lookup failed for `{name}`: {source}")]
    Lookup {
        name: String,
        #[source]
        source: SecretsError,
    },
}

/// 把 manifest 上声明的 [`Capability::Secrets`] 翻译成"子进程 env 名 → secret 值"。
///
/// **设计要点（§P4.4 / §E1.b）**：
/// - 只处理 `Capability::Secrets(name)`，其余 capability 静默跳过 ——
///   `Net` / `Llm` / `FsRead` 等是 sandbox/policy 层关心的，不属于本函数；
/// - env var 名 = `name.to_ascii_uppercase()`；与 [`EnvSecretsBroker`]
///   的默认翻译策略保持一致，这样无论 broker 是 env / KMS / vault，子进
///   程读到的都是同一个 env 名（等于把 broker 的"翻译规则"前置成 spawn
///   path 的事实标准）；
/// - **fail-loud**：任何 declared secret 在 broker 里找不到 → 返回
///   `MissingSecrets` 列表（不是单个 Err）；调用方一次看到所有缺的，
///   方便运维一次补齐；
/// - 输出按 env name 字典序排序 + 同名去重，让日志可重现、调用方可写
///   稳定的字符串断言。
///
/// **不做什么**：
/// - 不读父进程 env 作为 fallback（broker 自己决定 env / 别的后端）；
/// - 不写子进程的 env（那是 spawn path 的事，本函数纯函数）；
/// - 不脱敏日志（SecretValue 本身就拒绝明文 Debug，安全）。
pub fn provision_secret_envs(
    broker: &dyn SecretsBroker,
    capabilities: &[Capability],
) -> Result<Vec<(String, SecretValue)>, ProvisionError> {
    let mut wanted: Vec<&str> = capabilities
        .iter()
        .filter_map(|c| match c {
            Capability::Secrets(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    wanted.sort_unstable();
    wanted.dedup();

    let mut provisioned: Vec<(String, SecretValue)> = Vec::new();
    let mut missing: Vec<String> = Vec::new();

    for canonical in wanted {
        let env_name = canonical.to_ascii_uppercase();
        match broker.lookup(canonical) {
            Ok(Some(secret)) => provisioned.push((env_name, secret)),
            Ok(None) => missing.push(canonical.to_string()),
            Err(e) => {
                return Err(ProvisionError::Lookup {
                    name: canonical.to_string(),
                    source: e,
                });
            }
        }
    }

    if !missing.is_empty() {
        return Err(ProvisionError::MissingSecrets { missing });
    }
    Ok(provisioned)
}

// ============================================================================
// §E1.b: 进程级 broker 单例（避免污染 AppState / 10 处 test fixture）。
// ============================================================================

static GLOBAL_BROKER: OnceLock<Arc<dyn SecretsBroker>> = OnceLock::new();

/// 安装进程级 SecretsBroker。**只能成功一次**，之后调用返回 `Err(broker)`
/// 把入参原样还回（与 `OnceLock::set` 行为一致），避免线上误"换 broker"。
///
/// 典型调用点：clawd `main()` 启动期，在加载 registry 之后、在 spawn 任何
/// skill runner 之前。测试里如果要换 broker，用 `global_or_default` 不会写
/// singleton；要测自定义 broker 直接用 [`provision_secret_envs`] 注入即可，
/// 不要去动这个全局。
pub fn install_global(broker: Arc<dyn SecretsBroker>) -> Result<(), Arc<dyn SecretsBroker>> {
    GLOBAL_BROKER.set(broker)
}

/// 取进程级 broker；若未 install，惰性返回一个默认 [`EnvSecretsBroker`]。
///
/// 注意：惰性默认 broker **不会**写进 `GLOBAL_BROKER`，所以后续 install 仍
/// 然有效。这意味着：在 install 之前调一次 `global_or_default()` 是安全的，
/// 不会把 install 的窗口关掉。
pub fn global_or_default() -> Arc<dyn SecretsBroker> {
    if let Some(b) = GLOBAL_BROKER.get() {
        return b.clone();
    }
    Arc::new(EnvSecretsBroker::new())
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

    // ========================================================================
    // §E1.b: provision_secret_envs 单测
    // ========================================================================

    /// 测试用 mock broker：内存 map，单测里独立可控，不依赖 env。
    struct MockBroker {
        map: std::collections::HashMap<String, String>,
        fail_on: Option<String>, // 命中此 name 时返回 BackendIo
    }
    impl MockBroker {
        fn new(pairs: &[(&str, &str)]) -> Self {
            Self {
                map: pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
                fail_on: None,
            }
        }
        fn fail_on(mut self, name: &str) -> Self {
            self.fail_on = Some(name.to_string());
            self
        }
    }
    impl SecretsBroker for MockBroker {
        fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
            if Some(name.to_string()) == self.fail_on {
                return Err(SecretsError::BackendIo {
                    name: name.to_string(),
                    source: std::io::Error::other("mock backend down"),
                });
            }
            Ok(self.map.get(name).map(|v| SecretValue::new(v.clone())))
        }
        fn label(&self) -> &str {
            "mock"
        }
    }

    #[test]
    fn provision_skips_non_secret_capabilities() {
        let broker = MockBroker::new(&[]);
        let caps = vec![
            Capability::Llm,
            Capability::Net,
            Capability::FsRead,
            Capability::FsWrite,
            Capability::Exec,
            Capability::ExecSudo,
        ];
        let out = provision_secret_envs(&broker, &caps).unwrap();
        assert!(out.is_empty(), "non-secret caps must not produce env entries");
    }

    #[test]
    fn provision_translates_secret_name_to_uppercase_env() {
        let broker = MockBroker::new(&[
            ("image_generation_minimax_api_key", "image-secret"),
            ("text_openai_api_key", "text-secret"),
        ]);
        let caps = vec![
            Capability::Llm,
            Capability::Secrets("image_generation_minimax_api_key".to_string()),
            Capability::Secrets("text_openai_api_key".to_string()),
        ];
        let out = provision_secret_envs(&broker, &caps).unwrap();
        // 字典序：IMAGE_... 在 TEXT_... 前
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "IMAGE_GENERATION_MINIMAX_API_KEY");
        assert_eq!(out[0].1.expose(), "image-secret");
        assert_eq!(out[1].0, "TEXT_OPENAI_API_KEY");
        assert_eq!(out[1].1.expose(), "text-secret");
    }

    #[test]
    fn provision_dedupes_identical_secret_capabilities() {
        // 同一个 secret 名出现两次（manifest 写错时偶发），不应注入两遍。
        let broker = MockBroker::new(&[("text_openai_api_key", "v")]);
        let caps = vec![
            Capability::Secrets("text_openai_api_key".to_string()),
            Capability::Secrets("text_openai_api_key".to_string()),
        ];
        let out = provision_secret_envs(&broker, &caps).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn provision_fails_loud_with_all_missing_secrets_at_once() {
        // 三条 declared，broker 只有一条 → 一次性报告 2 条 missing
        let broker = MockBroker::new(&[("text_openai_api_key", "v")]);
        let caps = vec![
            Capability::Secrets("text_openai_api_key".to_string()),
            Capability::Secrets("image_generation_minimax_api_key".to_string()),
            Capability::Secrets("image_edit_qwen_api_key".to_string()),
        ];
        let err = provision_secret_envs(&broker, &caps).unwrap_err();
        match err {
            ProvisionError::MissingSecrets { mut missing } => {
                missing.sort();
                assert_eq!(
                    missing,
                    vec![
                        "image_edit_qwen_api_key".to_string(),
                        "image_generation_minimax_api_key".to_string(),
                    ]
                );
            }
            other => panic!("expected MissingSecrets, got {other:?}"),
        }
    }

    #[test]
    fn provision_propagates_backend_io_error_immediately() {
        let broker = MockBroker::new(&[("text_openai_api_key", "v")]).fail_on("image_edit_qwen_api_key");
        let caps = vec![
            Capability::Secrets("text_openai_api_key".to_string()),
            Capability::Secrets("image_edit_qwen_api_key".to_string()),
        ];
        let err = provision_secret_envs(&broker, &caps).unwrap_err();
        match err {
            ProvisionError::Lookup { name, .. } => {
                assert_eq!(name, "image_edit_qwen_api_key");
            }
            other => panic!("expected Lookup error, got {other:?}"),
        }
    }

    #[test]
    fn provision_output_is_alphabetically_stable() {
        // 不依赖输入顺序：故意倒序声明 capability，输出仍按 env name 字典序
        let broker = MockBroker::new(&[
            ("text_openai_api_key", "x"),
            ("chat_minimax_api_key", "y"),
            ("image_vision_anthropic_api_key", "z"),
        ]);
        let caps = vec![
            Capability::Secrets("text_openai_api_key".to_string()),
            Capability::Secrets("image_vision_anthropic_api_key".to_string()),
            Capability::Secrets("chat_minimax_api_key".to_string()),
        ];
        let out = provision_secret_envs(&broker, &caps).unwrap();
        let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "CHAT_MINIMAX_API_KEY",
                "IMAGE_VISION_ANTHROPIC_API_KEY",
                "TEXT_OPENAI_API_KEY",
            ]
        );
    }

    #[test]
    fn provision_handles_empty_capabilities_list() {
        let broker = MockBroker::new(&[]);
        let out = provision_secret_envs(&broker, &[]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn global_or_default_returns_env_broker_when_uninstalled() {
        // 注：本测试不调 install_global —— OnceLock 一旦 set 就锁死，会污染
        // 后续测试。这里只验证"未 install 时 fallback 是 env"。
        let b = global_or_default();
        assert_eq!(b.label(), "env");
    }

    // ========================================================================
    // §P4.4 E3.a: <usage>_<vendor>_api_key 命名 helpers
    // ========================================================================

    #[test]
    fn helper_lowercases_vendor_and_emits_canonical_text_name() {
        assert_eq!(text_secret_name_for_vendor("OpenAI"), "text_openai_api_key");
        assert_eq!(
            text_secret_name_for_vendor("  Anthropic  "),
            "text_anthropic_api_key"
        );
        assert_eq!(
            text_secret_name_for_vendor("MINIMAX"),
            "text_minimax_api_key"
        );
    }

    #[test]
    fn helper_emits_distinct_names_per_image_usage() {
        // 4 个 usage 必须互相独立 —— 同一 vendor 不同 usage 拿到不同 secret 名。
        let v = "qwen";
        let names = [
            text_secret_name_for_vendor(v),
            image_generation_secret_name_for_vendor(v),
            image_edit_secret_name_for_vendor(v),
            image_vision_secret_name_for_vendor(v),
        ];
        let unique: std::collections::HashSet<&str> = names.iter().map(|s| s.as_str()).collect();
        assert_eq!(unique.len(), 4, "4 usages must yield 4 distinct names");
        assert_eq!(names[0], "text_qwen_api_key");
        assert_eq!(names[1], "image_generation_qwen_api_key");
        assert_eq!(names[2], "image_edit_qwen_api_key");
        assert_eq!(names[3], "image_vision_qwen_api_key");
    }

    #[test]
    fn helper_output_passes_validate_secret_name() {
        // 对每个 helper × 每个已知 vendor，输出都必须能过 broker 的二道防线。
        let vendors = [
            "openai",
            "google",
            "gemini",
            "anthropic",
            "claude",
            "grok",
            "xai",
            "deepseek",
            "qwen",
            "minimax",
        ];
        for v in vendors {
            for name in [
                text_secret_name_for_vendor(v),
                image_generation_secret_name_for_vendor(v),
                image_edit_secret_name_for_vendor(v),
                image_vision_secret_name_for_vendor(v),
            ] {
                assert!(
                    validate_secret_name(&name).is_ok(),
                    "expected `{name}` to pass validation but it was rejected"
                );
            }
        }
    }

    #[test]
    fn helper_output_does_not_collapse_to_bare_vendor_pattern() {
        // 防御性回归：万一未来谁手抖把 usage 前缀去掉，validate 必须接得住。
        // 这里直接断"helper 不能产出 `openai_api_key` 这种裸形式"。
        for vendor in ["openai", "minimax", "anthropic"] {
            let n = text_secret_name_for_vendor(vendor);
            assert!(
                !n.starts_with(&format!("{vendor}_")),
                "helper must never collapse to bare-vendor naming: {n}"
            );
            assert!(n.starts_with("text_"), "must keep usage prefix: {n}");
        }
    }

    #[test]
    fn helper_handles_empty_vendor_gracefully() {
        // 空 vendor 输入会产出 `text__api_key`，这种字符串能通过 [a-z0-9_]
        // 形态校验（双下划线合法）但语义无效；调用方有责任先确认 vendor 非空。
        // 本测试只锁定"helper 不 panic、输出形态可被进一步 validate"，把
        // "vendor 必须非空"的语义校验留给上层（LlmProviderRuntime::api_key 会
        // 在 strip 失败时直接 fallback 到 config.api_key，根本不调 helper）。
        let n = text_secret_name_for_vendor("");
        assert_eq!(n, "text__api_key");
        assert!(validate_secret_name(&n).is_ok());
    }

    #[test]
    fn helper_round_trips_through_provision_secret_envs() {
        // 端到端：helper 产出的 name 喂给 Capability::Secrets，再走
        // provision_secret_envs 必须能取出对应 SecretValue。
        let n_text = text_secret_name_for_vendor("openai");
        let n_img = image_generation_secret_name_for_vendor("minimax");
        let broker = MockBroker::new(&[
            (n_text.as_str(), "text-key"),
            (n_img.as_str(), "img-key"),
        ]);
        let caps = vec![
            Capability::Secrets(n_text.clone()),
            Capability::Secrets(n_img.clone()),
        ];
        let out = provision_secret_envs(&broker, &caps).unwrap();
        let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["IMAGE_GENERATION_MINIMAX_API_KEY", "TEXT_OPENAI_API_KEY"]
        );
    }
}

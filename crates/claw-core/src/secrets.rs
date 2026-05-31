//! §P4.4 secrets 短期 token：broker 地基 + 本地一次性 token 发放/兑换 helper。
//!
//! **本模块的边界**
//!
//! 本模块负责两层能力：
//! - broker 地基：按 secret 名查到一段凭证；
//! - 本地一次性 token：把明文 secret 临时换成短期 token，并在子进程内兑换回
//!   明文，避免把真实 secret 直接塞进 child env。
//!
//! 仍然**不**负责：
//! - 跨主机 / 外部 STS / KMS / vault 的真正 secrets manager；
//! - 审计 / 长期轮转（这里只做本地短期票据与过期清理）；
//! - spawn 本身（runner spawn 前把 token 塞进子进程 env 仍由 clawd 路径完成）。
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

use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::skill_registry::Capability;

/// secret 内容的强类型包装。
///
/// 不实现 [`fmt::Debug`] / [`fmt::Display`] 的明文格式化，避免被 `dbg!` /
/// `tracing::info!` 误打印；如需在日志里出现，请用 [`Self::redacted_label`]。
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue {
    inner: String,
}

const SECRET_TOKEN_PREFIX: &str = "rustclaw-secret://v1/";
const SECRET_TOKEN_STORE_DIR_ENV: &str = "RUSTCLAW_SECRET_TOKEN_DIR";

#[derive(Debug, Serialize, Deserialize)]
struct SecretTokenRecord {
    value: String,
    expires_at_epoch_ms: u64,
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

#[derive(Debug, Error)]
pub enum SecretTokenError {
    #[error("secret token store IO failed at `{path}`: {source}")]
    StoreIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("secret token record at `{path}` is invalid JSON: {source}")]
    InvalidRecord {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("secret token `{token}` not found or already redeemed")]
    Missing { token: String },
    #[error("secret token `{token}` is expired")]
    Expired { token: String },
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
        "mimo",
        "xiaomi",
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

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

pub fn secret_token_store_dir() -> PathBuf {
    env::var(SECRET_TOKEN_STORE_DIR_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("rustclaw-secret-tokens"))
}

fn token_record_path(store_dir: &Path, token: &str) -> PathBuf {
    store_dir.join(format!("{token}.json"))
}

fn ensure_store_dir(store_dir: &Path) -> Result<(), SecretTokenError> {
    fs::create_dir_all(store_dir).map_err(|source| SecretTokenError::StoreIo {
        path: store_dir.to_path_buf(),
        source,
    })
}

fn resolved_secret_env_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cleanup_expired_secret_tokens(store_dir: &Path) {
    let now = now_epoch_ms();
    let Ok(entries) = fs::read_dir(store_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<SecretTokenRecord>(&raw) else {
            continue;
        };
        if record.expires_at_epoch_ms <= now {
            let _ = fs::remove_file(path);
        }
    }
}

fn issue_secret_token_value_in_dir(
    store_dir: &Path,
    secret: &SecretValue,
    ttl: Duration,
) -> Result<String, SecretTokenError> {
    ensure_store_dir(store_dir)?;
    cleanup_expired_secret_tokens(store_dir);
    let token = Uuid::new_v4().to_string();
    let path = token_record_path(store_dir, &token);
    let ttl_ms = ttl.as_millis().min(u128::from(u64::MAX)) as u64;
    let record = SecretTokenRecord {
        value: secret.expose().to_string(),
        expires_at_epoch_ms: now_epoch_ms().saturating_add(ttl_ms.max(1)),
    };
    let payload = serde_json::to_vec(&record).expect("secret token record must serialize");
    fs::write(&path, payload).map_err(|source| SecretTokenError::StoreIo {
        path: path.clone(),
        source,
    })?;
    Ok(format!("{SECRET_TOKEN_PREFIX}{token}"))
}

pub fn issue_secret_token_value(
    secret: &SecretValue,
    ttl: Duration,
) -> Result<String, SecretTokenError> {
    issue_secret_token_value_in_dir(&secret_token_store_dir(), secret, ttl)
}

pub fn issue_secret_env_tokens(
    secret_envs: &[(String, SecretValue)],
    ttl: Duration,
) -> Result<Vec<(String, String)>, SecretTokenError> {
    let store_dir = secret_token_store_dir();
    let mut out = Vec::with_capacity(secret_envs.len());
    for (env_name, secret) in secret_envs {
        out.push((
            env_name.clone(),
            issue_secret_token_value_in_dir(&store_dir, secret, ttl)?,
        ));
    }
    Ok(out)
}

fn redeem_secret_token_reference_in_dir(
    store_dir: &Path,
    value: &str,
) -> Result<Option<String>, SecretTokenError> {
    let Some(token) = value.strip_prefix(SECRET_TOKEN_PREFIX) else {
        return Ok(None);
    };
    let path = token_record_path(store_dir, token);
    let claimed = store_dir.join(format!("{token}.claim-{}", Uuid::new_v4()));
    fs::rename(&path, &claimed).map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => SecretTokenError::Missing {
            token: token.to_string(),
        },
        _ => SecretTokenError::StoreIo {
            path: path.clone(),
            source,
        },
    })?;
    let raw = fs::read_to_string(&claimed).map_err(|source| SecretTokenError::StoreIo {
        path: claimed.clone(),
        source,
    })?;
    let _ = fs::remove_file(&claimed);
    let record = serde_json::from_str::<SecretTokenRecord>(&raw).map_err(|source| {
        SecretTokenError::InvalidRecord {
            path: claimed.clone(),
            source,
        }
    })?;
    if record.expires_at_epoch_ms <= now_epoch_ms() {
        return Err(SecretTokenError::Expired {
            token: token.to_string(),
        });
    }
    Ok(Some(record.value))
}

pub fn redeem_secret_token_reference(value: &str) -> Result<Option<String>, SecretTokenError> {
    redeem_secret_token_reference_in_dir(&secret_token_store_dir(), value)
}

pub fn env_non_empty_resolved(key: &str) -> Result<Option<String>, SecretTokenError> {
    if let Ok(cache) = resolved_secret_env_cache().lock() {
        if let Some(value) = cache.get(key) {
            return Ok(Some(value.clone()));
        }
    }
    let Some(raw) = env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if let Some(redeemed) = redeem_secret_token_reference(&raw)? {
        let trimmed = redeemed.trim().to_string();
        if trimmed.is_empty() {
            return Ok(None);
        }
        if let Ok(mut cache) = resolved_secret_env_cache().lock() {
            cache.insert(key.to_string(), trimmed.clone());
        }
        return Ok(Some(trimmed));
    }
    Ok(Some(raw))
}

pub fn env_non_empty_resolved_or_none(key: &str) -> Option<String> {
    env_non_empty_resolved(key).ok().flatten()
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
#[path = "secrets_tests.rs"]
mod tests;

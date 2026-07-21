//! 技能注册表：从 TOML 驱动技能的发现、启用、别名、超时与 prompt 路径。
//! Phase 1：仅做“发现/启用/别名/超时”从 registry 读取，执行层与 planner 仍用现有逻辑。

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

/// 技能类型：builtin（clawd 内执行）/ runner（skill-runner 子进程）/ 预留 external
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillKind {
    Builtin,
    #[default]
    Runner,
    #[serde(other)]
    External,
}

/// 技能输出类型，用于后续 UI/通道展示与路由
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputKind {
    #[default]
    Text,
    File,
    Image,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillRiskLevel {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrimaryFallbackRole {
    #[default]
    None,
    Primary,
    Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerCapabilityEffect {
    Observe,
    Mutate,
    Validate,
    External,
}

impl PlannerCapabilityEffect {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Mutate => "mutate",
            Self::Validate => "validate",
            Self::External => "external",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RegistryDedupScope {
    #[default]
    Args,
    Action,
    Resource,
}

impl RegistryDedupScope {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Args => "args",
            Self::Action => "action",
            Self::Resource => "resource",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityExecutionMode {
    SyncShort,
    AsyncPreferred,
    AsyncRequired,
}

impl CapabilityExecutionMode {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::SyncShort => "sync_short",
            Self::AsyncPreferred => "async_preferred",
            Self::AsyncRequired => "async_required",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityIsolationProfile {
    LocalCurrentWorkspace,
    LocalWorktree,
    LocalTempWorkspace,
    RemoteExecutor,
    ReadOnly,
}

impl CapabilityIsolationProfile {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::LocalCurrentWorkspace => "local_current_workspace",
            Self::LocalWorktree => "local_worktree",
            Self::LocalTempWorkspace => "local_temp_workspace",
            Self::RemoteExecutor => "remote_executor",
            Self::ReadOnly => "read_only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PlannerCapabilityMapping {
    pub name: String,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub effect: Option<PlannerCapabilityEffect>,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
    #[serde(default)]
    pub risk_level: Option<SkillRiskLevel>,
    #[serde(default)]
    pub preferred: bool,
    #[serde(default)]
    pub once_per_task: Option<bool>,
    #[serde(default)]
    pub dedup_scope: Option<RegistryDedupScope>,
    #[serde(default)]
    pub dedup_fields: Vec<String>,
    #[serde(default)]
    pub idempotent: Option<bool>,
    #[serde(default)]
    pub execution_mode: Option<CapabilityExecutionMode>,
    #[serde(default)]
    pub async_adapter_kind: Option<String>,
    #[serde(default)]
    pub isolation_profile: Option<CapabilityIsolationProfile>,
    #[serde(default)]
    pub network_access: Option<bool>,
    #[serde(default)]
    pub filesystem_write: Option<bool>,
    #[serde(default)]
    pub external_publish: Option<bool>,
    #[serde(default)]
    pub credential_access: Option<bool>,
    #[serde(default)]
    pub subprocess: Option<bool>,
    #[serde(default)]
    pub package_install: Option<bool>,
    #[serde(default)]
    pub privilege_escalation: Option<bool>,
    #[serde(default)]
    pub final_answer_shape: Option<String>,
}

/// Selects an exact action policy, or the preferred/unique policy for an
/// actionless direct invocation. An explicit unknown action never falls back.
pub fn select_planner_capability_mapping<'a>(
    mappings: &'a [PlannerCapabilityMapping],
    action: Option<&str>,
) -> Option<&'a PlannerCapabilityMapping> {
    if let Some(action) = trim_optional_string(action).map(|value| normalize_schema_token(&value)) {
        return mappings
            .iter()
            .find(|mapping| mapping.action.as_deref() == Some(action.as_str()));
    }

    let mut actionless = mappings.iter().filter(|mapping| mapping.action.is_none());
    if let Some(first) = actionless.next() {
        let second = actionless.next();
        return mappings
            .iter()
            .find(|mapping| mapping.action.is_none() && mapping.preferred)
            .or_else(|| second.is_none().then_some(first));
    }
    (mappings.len() == 1).then(|| &mappings[0])
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct MatrixAdmissionConfig {
    #[serde(default)]
    pub eligible: bool,
    #[serde(default)]
    pub declared_actions: Vec<String>,
    #[serde(default)]
    pub evidence_sources: Vec<String>,
    #[serde(default)]
    pub required_extra_fields: Vec<String>,
    #[serde(default)]
    pub extractor_kind: Option<String>,
    #[serde(default)]
    pub admission_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillMemoryPolicyProfile {
    Disabled,
    #[default]
    SkillScoped,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct SkillMemoryPolicyConfig {
    #[serde(default)]
    pub profile: SkillMemoryPolicyProfile,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub max_chars: Option<usize>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Planner-facing capability layer. This is intentionally separate from
/// [`SkillKind`]: `kind` describes execution shape (builtin/runner/external),
/// while `planner_kind` describes how the agent should reason about the
/// capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlannerCapabilityKind {
    Tool,
    #[default]
    Skill,
    Workflow,
}

impl PlannerCapabilityKind {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Skill => "skill",
            Self::Workflow => "workflow",
        }
    }
}

/// §P4.1 主体：技能能力声明（closed-set + 命名 secrets.*）。
///
/// **设计目标**
/// - operator 在 registry 加载阶段就能审视技能要的资源（CI 拦截漂移）。
/// - §P4.4 短期 secrets token：将凭 `secrets.<name>` 决定是否注入对应密钥。
/// - §P4.2 skill shape 决策：对外呼出的能力（net/exec）会推导出更严的隔离形态。
///
/// **词汇表**（任何不在表内的字符串都会让 registry 加载报错，避免 typo）：
/// - `llm`：调用 LLM 网关（受 clawd LLM gateway 管控：fallback / 限流 / 审计）。
/// - `net`：除 LLM 网关外的对外网络（HTTP/HTTPS/raw socket）。
/// - `fs.read`：读取技能 bundle 目录之外的文件。
/// - `fs.write`：在技能 bundle 目录之外创建/修改文件。
/// - `exec`：fork/exec 子进程（含 shell）。
/// - `exec.sudo`：以提权方式 fork（必须额外 opt-in，独立于 `exec`）。
/// - `secrets.<name>`：需要某个具名密钥。`<name>` 仅允许 `[a-z0-9_]`，长度
///   1..=64，避免拼写漂移。
///
/// **关于 `secrets.<name>` 的命名规范（重要 — 与 §P4.4 配合）**：
///
/// 本仓里 `image_generation` / `image_edit` / `image_vision` / `[llm]`（即文本
/// 对话与规划）是**四个互相独立**的 LLM provider 配置域 —— 同一个 vendor 可
/// 以在不同用途下填不同 base_url 与 api_key。所以 secret 命名必须按
/// `<用途>_<vendor>_api_key` 展开，**不能**用 `secrets.openai_api_key` 这种
/// 跨用途的"vendor-唯一"命名，否则会把 image_generate 的 key 误注入到
/// chat/规划链路（反之亦然）。
///
/// 推荐命名：
/// - `secrets.image_generation_minimax_api_key`
/// - `secrets.image_edit_qwen_api_key`
/// - `secrets.image_vision_openai_api_key`
/// - `secrets.text_openai_api_key`（对应 `[llm.openai].api_key`）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Capability {
    Llm,
    Net,
    FsRead,
    FsWrite,
    Exec,
    ExecSudo,
    /// 内部存的是密钥的 canonical name（小写、`[a-z0-9_]`）。
    Secrets(String),
}

impl Capability {
    /// 解析 TOML 中的字符串到 [`Capability`]。
    ///
    /// 收尾 §P4.1 的契约：未知 token 必须报错而不是默默忽略。
    pub fn parse(token: &str) -> Result<Self, String> {
        let raw = token.trim();
        if raw.is_empty() {
            return Err("capability token must not be empty".to_string());
        }
        // 全部按小写比对，让 registry 写法宽松、内存表示稳定。
        let lower = raw.to_ascii_lowercase();
        match lower.as_str() {
            "llm" => Ok(Self::Llm),
            "net" => Ok(Self::Net),
            "fs.read" => Ok(Self::FsRead),
            "fs.write" => Ok(Self::FsWrite),
            "exec" => Ok(Self::Exec),
            "exec.sudo" => Ok(Self::ExecSudo),
            other => {
                if let Some(name) = other.strip_prefix("secrets.") {
                    if name.is_empty() || name.len() > 64 {
                        return Err(format!(
                            "secrets capability name length must be 1..=64: `{token}`"
                        ));
                    }
                    if !name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    {
                        return Err(format!(
                            "secrets capability name must match [a-z0-9_]: `{token}`"
                        ));
                    }
                    // §P4.1 ↔ image/text 配置独立性：拦截"裸 vendor 名"反模式。
                    // image_generation / image_edit / image_vision / [llm] 是
                    // 4 套互相独立的 LLM provider 配置（同一 vendor 在不同用途
                    // 下可能填不同 key）。`secrets.openai_api_key` 这种命名
                    // 隐含"vendor-唯一 key"，会让 §P4.4 的 token 注入跨域串货。
                    // 必须按 `<用途>_<vendor>_api_key` 命名（如
                    // `secrets.image_generation_minimax_api_key`）。
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
                        if name == &format!("{vendor}_api_key") || name == *vendor {
                            return Err(format!(
                                "secrets capability `{token}` uses bare vendor naming, which conflates the four independent LLM provider configs (image_generation/image_edit/image_vision/text). Rename to `secrets.<usage>_<vendor>_api_key`, e.g. `secrets.image_generation_{vendor}_api_key` or `secrets.text_{vendor}_api_key`."
                            ));
                        }
                    }
                    Ok(Self::Secrets(name.to_string()))
                } else {
                    Err(format!(
                        "unknown capability `{token}` (allowed: llm, net, fs.read, fs.write, exec, exec.sudo, secrets.<name>)"
                    ))
                }
            }
        }
    }

    /// 反向 → token，便于日志/错误信息打印（与 [`Self::parse`] 自洽）。
    pub fn as_token(&self) -> String {
        match self {
            Self::Llm => "llm".to_string(),
            Self::Net => "net".to_string(),
            Self::FsRead => "fs.read".to_string(),
            Self::FsWrite => "fs.write".to_string(),
            Self::Exec => "exec".to_string(),
            Self::ExecSudo => "exec.sudo".to_string(),
            Self::Secrets(name) => format!("secrets.{name}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SkillManifest {
    pub name: String,
    pub kind: SkillKind,
    pub planner_kind: PlannerCapabilityKind,
    pub output_kind: OutputKind,
    pub description: Option<String>,
    pub semantic_tags: Vec<String>,
    pub preferred_over_run_cmd: bool,
    pub validation_actions: Vec<String>,
    pub prompt_file: Option<String>,
    pub input_schema: Option<JsonValue>,
    pub output_schema: Option<JsonValue>,
    pub runtime_skill: Option<String>,
    pub runtime_action: Option<String>,
    pub runtime_default_args: Option<JsonValue>,
    pub runtime_rewrite_arg_keys: Vec<String>,
    pub risk_level: Option<SkillRiskLevel>,
    pub auto_invocable: Option<bool>,
    pub requires_confirmation: Option<bool>,
    pub side_effect: Option<bool>,
    pub confirmation_exempt_when: Vec<BTreeMap<String, JsonValue>>,
    pub timeout_seconds: Option<u64>,
    pub retryable: Option<bool>,
    pub group: Option<String>,
    pub primary_fallback_role: Option<PrimaryFallbackRole>,
    pub once_per_task: Option<bool>,
    pub dedup_scope: Option<RegistryDedupScope>,
    pub idempotent: Option<bool>,
    pub supported_os: Vec<String>,
    pub required_bins: Vec<String>,
    pub optional_bins: Vec<String>,
    pub platform_notes: Vec<String>,
    pub planner_capabilities: Vec<PlannerCapabilityMapping>,
    /// §P4.1 主体：这条技能对外声明需要使用的能力集（去重、按 [`Capability::as_token`]
    /// 排序）。空表示"纯计算 + 标准库"，不需要任何特权资源。
    pub capabilities: Vec<Capability>,
}

/// 注册表中的单条技能定义
#[derive(Debug, Clone, Deserialize)]
pub struct SkillRegistryEntry {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub planner_visible: bool,
    #[serde(default)]
    pub kind: SkillKind,
    #[serde(default)]
    pub planner_kind: Option<PlannerCapabilityKind>,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// 该技能专用超时（秒）；未设或 0 表示用全局默认
    #[serde(default)]
    pub timeout_seconds: u64,
    /// prompt 文件路径，相对 workspace 或绝对
    #[serde(default)]
    pub prompt_file: String,
    #[serde(default)]
    pub output_kind: OutputKind,
    #[serde(default)]
    pub description: Option<String>,
    /// Planner-facing semantic tags, for example `sqlite_table_listing`,
    /// `archive_unpack`, or `service_status`. These are descriptive routing
    /// hints, not permissions.
    #[serde(default)]
    pub semantic_tags: Vec<String>,
    /// Prefer this structured skill over ad-hoc shell commands when its
    /// semantic tags match the task.
    #[serde(default)]
    pub preferred_over_run_cmd: bool,
    /// Read-only actions that can validate or inspect the skill's domain.
    #[serde(default)]
    pub validation_actions: Vec<String>,
    #[serde(default)]
    pub input_schema: Option<TomlValue>,
    #[serde(default)]
    pub output_schema: Option<TomlValue>,
    /// Optional planner-tool execution mapping. When set, clawd may rewrite a
    /// planner-facing tool into this runtime skill/action under structured
    /// schema conditions. This keeps planner names, UI labels, and runtime
    /// executors configurable instead of embedding tool maps in clawd code.
    #[serde(default)]
    pub runtime_skill: Option<String>,
    #[serde(default)]
    pub runtime_action: Option<String>,
    #[serde(default)]
    pub runtime_default_args: Option<TomlValue>,
    #[serde(default)]
    pub runtime_rewrite_arg_keys: Vec<String>,
    #[serde(default)]
    pub risk_level: Option<SkillRiskLevel>,
    #[serde(default)]
    pub auto_invocable: Option<bool>,
    #[serde(default)]
    pub requires_confirmation: Option<bool>,
    #[serde(default)]
    pub side_effect: Option<bool>,
    /// Structured arg matchers that allow a normally-confirmed skill to run
    /// without front-door confirmation for safe/read-only variants.
    ///
    /// Example:
    /// `confirmation_exempt_when = [{ action = "prepare" }, { action = "organize", mode = "plan" }]`
    ///
    /// This is intentionally structured so routing policy stays in config rather
    /// than language-specific source-code phrase checks.
    #[serde(default)]
    pub confirmation_exempt_when: Vec<BTreeMap<String, TomlValue>>,
    #[serde(default)]
    pub retryable: Option<bool>,
    #[serde(default)]
    pub once_per_task: Option<bool>,
    #[serde(default)]
    pub dedup_scope: Option<RegistryDedupScope>,
    #[serde(default)]
    pub idempotent: Option<bool>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub primary_fallback_role: Option<PrimaryFallbackRole>,
    /// Host OS families where this skill/tool is expected to work, e.g.
    /// `linux`, `macos`, or `any`. This is planner/UI metadata, not a runtime
    /// permission.
    #[serde(default)]
    pub supported_os: Vec<String>,
    /// Commands that must exist for the main happy path. Empty means pure Rust
    /// or no external command dependency.
    #[serde(default)]
    pub required_bins: Vec<String>,
    /// Commands used for optional probes or fallbacks.
    #[serde(default)]
    pub optional_bins: Vec<String>,
    /// Human-readable platform notes for planner/UI display. Keep stable and
    /// factual; do not put routing phrase examples here.
    #[serde(default)]
    pub platform_notes: Vec<String>,
    /// Planner-facing semantic capability mappings. This is separate from
    /// `capabilities`, which declares runtime/security resources.
    #[serde(default)]
    pub planner_capabilities: Vec<PlannerCapabilityMapping>,
    /// Optional matrix evidence admission metadata. Runtime enablement and
    /// matrix evidence eligibility are intentionally separate: an external
    /// skill can be callable while still ineligible for strict evidence use.
    #[serde(default)]
    pub matrix_admission: Option<MatrixAdmissionConfig>,
    /// Structured memory sources this skill may receive in its `_memory`
    /// payload. This is runtime policy metadata, not natural-language routing.
    #[serde(default)]
    pub memory_policy: Option<SkillMemoryPolicyConfig>,

    // ---------- Phase 5: 执行配置 ----------
    /// Runner 技能：在 runner 侧的执行名；未设则用 name。
    /// skill-runner 会将该值按约定解析为二进制名：
    /// 例如 foo_bar -> foo-bar-skill，若已以 -skill 结尾则直接使用。
    #[serde(default)]
    pub runner_name: Option<String>,
    /// External 技能：执行方式，如 "http_json"
    #[serde(default)]
    pub external_kind: Option<String>,
    /// External 技能：调用地址（如 HTTP URL）
    #[serde(default)]
    pub external_endpoint: Option<String>,
    /// External 技能：本地 bundle 目录（相对 workspace 或绝对路径）
    #[serde(default)]
    pub external_bundle_dir: Option<String>,
    /// External 技能：入口文件（如 SKILL.md、scripts/stock_cli.py）
    #[serde(default)]
    pub external_entry_file: Option<String>,
    /// External 技能：运行时（如 python3、node）
    #[serde(default)]
    pub external_runtime: Option<String>,
    /// External 技能：依赖的二进制命令
    #[serde(default)]
    pub external_require_bins: Vec<String>,
    /// External 技能：依赖的 Python 模块
    #[serde(default)]
    pub external_require_py_modules: Vec<String>,
    /// External 技能：来源链接或本地来源描述
    #[serde(default)]
    pub external_source_url: Option<String>,
    /// External 技能：专用超时（秒）；未设则用 timeout_seconds
    #[serde(default)]
    pub external_timeout_seconds: Option<u64>,
    /// External 技能：鉴权引用（预留，本轮不实现完整 secret 管理）
    #[serde(default)]
    pub external_auth_ref: Option<String>,

    // ---------- Phase 4.1 主体：能力声明 ----------
    /// 该技能对外声明的能力集（TOML 写法示例：
    /// `capabilities = ["llm", "net", "secrets.image_generation_minimax_api_key"]`。
    /// 注意 `secrets.<name>` 必须按 `<用途>_<vendor>_api_key` 命名，
    /// image / text / vision 等用途各自独立，不要写成 `secrets.openai_api_key` 这种跨用途的"vendor-唯一"命名）。
    ///
    /// 文件层用 `Vec<String>` 接，[`SkillsRegistry::load_from_path`] 会把它
    /// 转成 [`Capability`]（未知 token 会让 registry **加载失败**）。
    /// 转换后的强类型放在 [`SkillRegistryEntry::resolved_capabilities`]，
    /// `capabilities_raw` 字段仅作为审计/调试痕迹保留。
    #[serde(default, rename = "capabilities")]
    pub capabilities_raw: Vec<String>,

    /// 内部使用：`load_from_path` 把 `capabilities_raw` 解析后塞这里。
    /// 不被 serde 反序列化（永远从 raw 算出来，避免双源真相）。
    #[serde(skip)]
    pub resolved_capabilities: Vec<Capability>,
}

fn default_true() -> bool {
    true
}

fn trim_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn normalize_metadata_tokens(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let token = value.trim().to_ascii_lowercase();
        if token.is_empty() || out.iter().any(|existing| existing == &token) {
            continue;
        }
        out.push(token);
    }
    out
}

fn normalize_schema_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if matches!(ch, '-' | ' ' | '.') {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

fn normalize_schema_tokens(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let token = normalize_schema_token(value);
        if token.is_empty() || out.iter().any(|existing| existing == &token) {
            continue;
        }
        out.push(token);
    }
    out
}

fn normalize_planner_capability_name(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace("::", ".")
        .replace('-', "_")
}

fn normalize_planner_capabilities(
    mappings: &[PlannerCapabilityMapping],
) -> Vec<PlannerCapabilityMapping> {
    let mut out: Vec<PlannerCapabilityMapping> = Vec::new();
    for mapping in mappings {
        let name = normalize_planner_capability_name(&mapping.name);
        if name.is_empty() || out.iter().any(|existing| existing.name == name) {
            continue;
        }
        out.push(PlannerCapabilityMapping {
            name,
            action: trim_optional_string(mapping.action.as_deref())
                .map(|value| normalize_schema_token(&value)),
            effect: mapping.effect,
            required: normalize_schema_tokens(&mapping.required),
            optional: normalize_schema_tokens(&mapping.optional),
            risk_level: mapping.risk_level,
            preferred: mapping.preferred,
            once_per_task: mapping.once_per_task,
            dedup_scope: mapping.dedup_scope,
            dedup_fields: normalize_schema_tokens(&mapping.dedup_fields),
            idempotent: mapping.idempotent,
            execution_mode: mapping.execution_mode,
            async_adapter_kind: trim_optional_string(mapping.async_adapter_kind.as_deref())
                .map(|value| normalize_schema_token(&value)),
            isolation_profile: mapping
                .isolation_profile
                .or_else(|| default_isolation_profile_for_effect(mapping.effect)),
            network_access: mapping
                .network_access
                .or_else(|| default_network_access_for_effect(mapping.effect)),
            filesystem_write: mapping
                .filesystem_write
                .or_else(|| default_filesystem_write_for_effect(mapping.effect)),
            external_publish: mapping
                .external_publish
                .or_else(|| default_external_publish_for_effect(mapping.effect)),
            credential_access: mapping
                .credential_access
                .or_else(|| mapping.effect.map(|_| false)),
            subprocess: mapping.subprocess,
            package_install: mapping.package_install,
            privilege_escalation: mapping.privilege_escalation,
            final_answer_shape: trim_optional_string(mapping.final_answer_shape.as_deref())
                .map(|value| normalize_schema_token(&value)),
        });
    }
    out
}

fn default_isolation_profile_for_effect(
    effect: Option<PlannerCapabilityEffect>,
) -> Option<CapabilityIsolationProfile> {
    match effect? {
        PlannerCapabilityEffect::Observe | PlannerCapabilityEffect::Validate => {
            Some(CapabilityIsolationProfile::ReadOnly)
        }
        PlannerCapabilityEffect::Mutate => Some(CapabilityIsolationProfile::LocalCurrentWorkspace),
        PlannerCapabilityEffect::External => Some(CapabilityIsolationProfile::RemoteExecutor),
    }
}

fn default_network_access_for_effect(effect: Option<PlannerCapabilityEffect>) -> Option<bool> {
    Some(matches!(effect?, PlannerCapabilityEffect::External))
}

fn default_filesystem_write_for_effect(effect: Option<PlannerCapabilityEffect>) -> Option<bool> {
    Some(matches!(effect?, PlannerCapabilityEffect::Mutate))
}

fn default_external_publish_for_effect(effect: Option<PlannerCapabilityEffect>) -> Option<bool> {
    Some(matches!(effect?, PlannerCapabilityEffect::External))
}

fn matching_planner_capability<'a>(
    entry: &'a SkillRegistryEntry,
    action: Option<&str>,
) -> Option<&'a PlannerCapabilityMapping> {
    select_planner_capability_mapping(&entry.planner_capabilities, action)
}

fn once_per_task_from_effect(effect: PlannerCapabilityEffect) -> bool {
    matches!(
        effect,
        PlannerCapabilityEffect::Mutate | PlannerCapabilityEffect::External
    )
}

fn dedup_scope_from_effect(effect: PlannerCapabilityEffect) -> RegistryDedupScope {
    match effect {
        PlannerCapabilityEffect::Observe | PlannerCapabilityEffect::Validate => {
            RegistryDedupScope::Args
        }
        PlannerCapabilityEffect::Mutate | PlannerCapabilityEffect::External => {
            RegistryDedupScope::Action
        }
    }
}

fn idempotent_from_effect(effect: PlannerCapabilityEffect) -> bool {
    matches!(
        effect,
        PlannerCapabilityEffect::Observe | PlannerCapabilityEffect::Validate
    )
}

fn legacy_entry_is_high_risk_or_side_effect(entry: &SkillRegistryEntry) -> bool {
    entry.side_effect.unwrap_or(false)
        || entry.requires_confirmation.unwrap_or(false)
        || entry.risk_level == Some(SkillRiskLevel::High)
}

fn legacy_once_per_task_default(entry: &SkillRegistryEntry) -> bool {
    legacy_entry_is_high_risk_or_side_effect(entry)
}

fn legacy_dedup_scope_default(entry: &SkillRegistryEntry) -> RegistryDedupScope {
    if legacy_entry_is_high_risk_or_side_effect(entry) {
        RegistryDedupScope::Action
    } else {
        RegistryDedupScope::Args
    }
}

fn legacy_idempotent_default(entry: &SkillRegistryEntry) -> bool {
    if let Some(side_effect) = entry.side_effect {
        return !side_effect;
    }
    if entry.requires_confirmation.unwrap_or(false)
        || entry.risk_level == Some(SkillRiskLevel::High)
    {
        return false;
    }
    false
}

fn normalize_matrix_admission(config: &MatrixAdmissionConfig) -> MatrixAdmissionConfig {
    MatrixAdmissionConfig {
        eligible: config.eligible,
        declared_actions: normalize_schema_tokens(&config.declared_actions),
        evidence_sources: normalize_schema_tokens(&config.evidence_sources),
        required_extra_fields: normalize_metadata_lines(&config.required_extra_fields),
        extractor_kind: trim_optional_string(config.extractor_kind.as_deref())
            .map(|value| normalize_schema_token(&value)),
        admission_version: trim_optional_string(config.admission_version.as_deref()),
    }
}

fn normalize_metadata_lines(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let line = value.trim();
        if line.is_empty() || out.iter().any(|existing| existing == line) {
            continue;
        }
        out.push(line.to_string());
    }
    out
}

const SKILL_MEMORY_POLICY_SOURCE_TOKENS: &[&str] = &[
    "preferences",
    "long_term_summary",
    "recent_related_events",
    "assistant_results",
    "similar_triggers",
    "unfinished_goals",
    "relevant_facts",
    "knowledge_docs",
    "recent_snippets",
];

fn normalize_skill_memory_source_tokens(
    skill_name: &str,
    path: &Path,
    field: &str,
    values: &[String],
) -> Result<Vec<String>, String> {
    let normalized = normalize_schema_tokens(values);
    for token in &normalized {
        if !SKILL_MEMORY_POLICY_SOURCE_TOKENS.contains(&token.as_str()) {
            return Err(format!(
                "parse memory_policy.{field} for skill `{skill_name}` in {}: unknown source token `{token}`",
                path.display()
            ));
        }
    }
    Ok(normalized)
}

fn normalize_skill_memory_policy(
    skill_name: &str,
    path: &Path,
    policy: &SkillMemoryPolicyConfig,
) -> Result<SkillMemoryPolicyConfig, String> {
    let include =
        normalize_skill_memory_source_tokens(skill_name, path, "include", &policy.include)?;
    let exclude =
        normalize_skill_memory_source_tokens(skill_name, path, "exclude", &policy.exclude)?;
    for token in &include {
        if exclude.iter().any(|item| item == token) {
            return Err(format!(
                "parse memory_policy for skill `{skill_name}` in {}: source token `{token}` appears in both include and exclude",
                path.display()
            ));
        }
    }
    if policy.max_chars == Some(0) {
        return Err(format!(
            "parse memory_policy.max_chars for skill `{skill_name}` in {}: value must be greater than 0",
            path.display()
        ));
    }
    Ok(SkillMemoryPolicyConfig {
        profile: policy.profile,
        include,
        exclude,
        max_chars: policy.max_chars,
        reason: trim_optional_string(policy.reason.as_deref()),
    })
}

fn normalize_matcher_value(value: &TomlValue) -> TomlValue {
    match value {
        TomlValue::String(s) => TomlValue::String(normalize_schema_token(s)),
        TomlValue::Array(items) => {
            TomlValue::Array(items.iter().map(normalize_matcher_value).collect())
        }
        other => other.clone(),
    }
}

fn normalize_confirmation_exempt_when(
    matchers: &[BTreeMap<String, TomlValue>],
) -> Vec<BTreeMap<String, TomlValue>> {
    let mut out = Vec::new();
    for matcher in matchers {
        let mut normalized = BTreeMap::new();
        for (key, value) in matcher {
            let key = normalize_schema_token(key);
            if key.is_empty() {
                continue;
            }
            normalized.insert(key, normalize_matcher_value(value));
        }
        if normalized.is_empty() || out.iter().any(|existing| existing == &normalized) {
            continue;
        }
        out.push(normalized);
    }
    out
}

fn confirmation_exempt_when_to_json(
    matchers: &[BTreeMap<String, TomlValue>],
) -> Vec<BTreeMap<String, JsonValue>> {
    matchers
        .iter()
        .map(|matcher| {
            matcher
                .iter()
                .filter_map(|(key, value)| {
                    toml_value_to_json(value).map(|value| (key.clone(), value))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .filter(|matcher| !matcher.is_empty())
        .collect()
}

fn resolved_planner_kind(entry: &SkillRegistryEntry) -> PlannerCapabilityKind {
    if let Some(kind) = entry.planner_kind {
        return kind;
    }

    if let Some(group) = entry
        .group
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase())
    {
        if matches!(group.as_str(), "workflow" | "flows" | "orchestration") {
            return PlannerCapabilityKind::Workflow;
        }
        if matches!(
            group.as_str(),
            "filesystem"
                | "file"
                | "files"
                | "fs"
                | "system"
                | "developer"
                | "dev"
                | "ops"
                | "status"
                | "service"
                | "runtime"
                | "database"
                | "db"
                | "web"
                | "http"
                | "shell"
        ) {
            return PlannerCapabilityKind::Tool;
        }
    }

    if entry.preferred_over_run_cmd {
        return PlannerCapabilityKind::Tool;
    }

    if entry.kind == SkillKind::Builtin {
        return PlannerCapabilityKind::Tool;
    }

    PlannerCapabilityKind::Skill
}

fn toml_value_to_json(value: &TomlValue) -> Option<JsonValue> {
    serde_json::to_value(value).ok()
}

/// 整张技能注册表（通常从 TOML [[skills]] 加载）
#[derive(Debug, Clone, Default)]
pub struct SkillsRegistry {
    /// 按 canonical name 索引
    by_name: HashMap<String, SkillRegistryEntry>,
    /// 别名 -> canonical name
    alias_to_name: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct SkillsRegistryFile {
    #[serde(default)]
    skills: Vec<SkillRegistryEntry>,
}

impl SkillsRegistry {
    /// 从 TOML 文件加载；路径为绝对或相对当前工作目录。文件不存在或解析失败返回 Ok(空 registry)。
    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => return Err(format!("read registry failed: {}: {}", path.display(), e)),
        };
        Self::load_from_str_with_source(&content, path)
    }

    /// Load a registry from TOML text for bundled config, tests, or embedded config.
    pub fn load_from_str(content: &str) -> Result<Self, String> {
        Self::load_from_str_with_source(content, Path::new("<inline>"))
    }

    fn load_from_str_with_source(content: &str, path: &Path) -> Result<Self, String> {
        let file: SkillsRegistryFile = toml::from_str(&content)
            .map_err(|e| format!("parse registry failed: {}: {}", path.display(), e))?;
        let mut by_name = HashMap::new();
        let mut alias_to_name = HashMap::new();
        for mut entry in file.skills {
            let raw = entry.name.trim().to_string();
            if raw.is_empty() {
                continue;
            }
            let canonical = to_canonical_key(&raw);
            if by_name.contains_key(&canonical) {
                return Err(format!(
                    "duplicate skill name `{canonical}` in {}",
                    path.display()
                ));
            }
            entry.name = canonical.clone();
            let aliases: Vec<String> = entry
                .aliases
                .iter()
                .map(|a| to_canonical_key(a))
                .filter(|a| !a.is_empty() && *a != canonical)
                .collect();
            entry.semantic_tags = normalize_metadata_tokens(&entry.semantic_tags);
            entry.validation_actions = normalize_metadata_tokens(&entry.validation_actions);
            entry.runtime_skill = trim_optional_string(entry.runtime_skill.as_deref())
                .map(|value| to_canonical_key(&value));
            entry.runtime_action = trim_optional_string(entry.runtime_action.as_deref())
                .map(|value| normalize_schema_token(&value));
            entry.runtime_rewrite_arg_keys =
                normalize_schema_tokens(&entry.runtime_rewrite_arg_keys);
            entry.confirmation_exempt_when =
                normalize_confirmation_exempt_when(&entry.confirmation_exempt_when);
            entry.supported_os = normalize_metadata_tokens(&entry.supported_os);
            entry.required_bins = normalize_metadata_tokens(&entry.required_bins);
            entry.optional_bins = normalize_metadata_tokens(&entry.optional_bins);
            entry.platform_notes = normalize_metadata_lines(&entry.platform_notes);
            entry.planner_capabilities =
                normalize_planner_capabilities(&entry.planner_capabilities);
            entry.matrix_admission = entry
                .matrix_admission
                .as_ref()
                .map(normalize_matrix_admission);
            entry.memory_policy = entry
                .memory_policy
                .as_ref()
                .map(|policy| normalize_skill_memory_policy(&canonical, path, policy))
                .transpose()?;
            // §P4.1 主体：把 capabilities_raw 翻译成强类型，未知 token 直接报错。
            // 排序 + dedup 让"声明顺序"不影响等价性，并避免重复声明在策略层
            // 引出二义性。
            let mut resolved: Vec<Capability> = Vec::with_capacity(entry.capabilities_raw.len());
            for token in &entry.capabilities_raw {
                let cap = Capability::parse(token).map_err(|e| {
                    format!(
                        "parse capabilities for skill `{canonical}` in {}: {e}",
                        path.display()
                    )
                })?;
                resolved.push(cap);
            }
            resolved.sort_by(|a, b| a.as_token().cmp(&b.as_token()));
            resolved.dedup();
            entry.resolved_capabilities = resolved;

            by_name.insert(canonical.clone(), entry);
            if let Some(existing) = alias_to_name.get(&canonical) {
                if existing != &canonical {
                    return Err(format!(
                        "duplicate skill alias/name `{canonical}` in {}: `{existing}` and `{canonical}`",
                        path.display()
                    ));
                }
            } else {
                alias_to_name.insert(canonical.clone(), canonical.clone());
            }
            for a in &aliases {
                if let Some(existing) = alias_to_name.get(a) {
                    if existing != &canonical {
                        return Err(format!(
                            "duplicate skill alias `{a}` in {}: `{existing}` and `{canonical}`",
                            path.display()
                        ));
                    }
                } else {
                    alias_to_name.insert(a.clone(), canonical.clone());
                }
            }
        }
        let registry = Self {
            by_name,
            alias_to_name,
        };

        // §P4.2：声明的 capabilities 必须和 manifest 的 shape 一致，否则
        // 加载失败 — 例如 exec.sudo 不允许自动执行（必须 confirm + high
        // risk），fs.write/exec 不允许显式 side_effect=false。
        let shape_violations = registry.validate_shape_consistency();
        if !shape_violations.is_empty() {
            return Err(format!(
                "skill registry shape consistency check failed in {}:\n  - {}",
                path.display(),
                shape_violations.join("\n  - ")
            ));
        }

        Ok(registry)
    }

    /// 解析别名或名称得到 canonical name（小写）；不存在则返回 None
    pub fn resolve_canonical(&self, name: &str) -> Option<&str> {
        let key = to_canonical_key(name);
        if key.is_empty() {
            return None;
        }
        self.alias_to_name.get(&key).map(String::as_str)
    }

    /// 按 canonical name 取条目
    pub fn get(&self, canonical_name: &str) -> Option<&SkillRegistryEntry> {
        self.by_name.get(&to_canonical_key(canonical_name))
    }

    /// 所有在注册表中且 enabled 的 canonical 名称
    pub fn enabled_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .by_name
            .iter()
            .filter(|(_, e)| e.enabled)
            .map(|(n, _)| n.clone())
            .collect();
        names.sort();
        names
    }

    pub fn is_planner_visible(&self, canonical_name: &str) -> bool {
        self.get(canonical_name)
            .map(|entry| entry.planner_visible)
            .unwrap_or(true)
    }

    /// 该技能在 registry 中配置的 timeout（秒）；0 表示用全局默认
    pub fn timeout_seconds(&self, canonical_name: &str) -> u64 {
        self.get(canonical_name)
            .and_then(|e| {
                if e.timeout_seconds > 0 {
                    Some(e.timeout_seconds)
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    /// 所有已注册的 canonical 名称（含未启用）
    pub fn all_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.by_name.keys().cloned().collect();
        names.sort();
        names
    }

    /// 是否在注册表中（别名或 canonical 能解析即视为已知）
    pub fn is_known(&self, name: &str) -> bool {
        self.resolve_canonical(name).is_some()
    }

    /// 该技能在 registry 中是否为 builtin 类型
    pub fn is_builtin(&self, canonical_name: &str) -> bool {
        self.get(canonical_name)
            .map(|e| e.kind == SkillKind::Builtin)
            .unwrap_or(false)
    }

    /// 该技能在 registry 中配置的 prompt 文件路径；空字符串视为 None
    pub fn prompt_file(&self, canonical_name: &str) -> Option<&str> {
        self.get(canonical_name).and_then(|e| {
            let s = e.prompt_file.trim();
            if s.is_empty() {
                None
            } else {
                Some(e.prompt_file.as_str())
            }
        })
    }

    /// Phase 5: Runner 执行名；未配置则用 canonical_name。返回 String 避免混合借用来源的 lifetime 问题。
    pub fn runner_name(&self, canonical_name: &str) -> String {
        self.get(canonical_name)
            .and_then(|e| e.runner_name.as_deref())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| canonical_name.to_string())
    }

    /// Phase 5: External 执行配置（仅当 kind=External 且配置完整时返回）
    pub fn external_config(&self, canonical_name: &str) -> Option<ExternalSkillConfig<'_>> {
        let e = self.get(canonical_name)?;
        if e.kind != SkillKind::External {
            return None;
        }
        let kind = e
            .external_kind
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        Some(ExternalSkillConfig {
            kind,
            endpoint: e
                .external_endpoint
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            bundle_dir: e
                .external_bundle_dir
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            entry_file: e
                .external_entry_file
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            runtime: e
                .external_runtime
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            require_bins: &e.external_require_bins,
            require_py_modules: &e.external_require_py_modules,
            source_url: e
                .external_source_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            timeout_seconds: e
                .external_timeout_seconds
                .filter(|&t| t > 0)
                .or_else(|| (e.timeout_seconds > 0).then_some(e.timeout_seconds)),
            auth_ref: e.external_auth_ref.as_deref(),
        })
    }

    pub fn manifest(&self, canonical_name: &str) -> Option<SkillManifest> {
        let entry = self.get(canonical_name)?;
        let timeout_seconds = entry
            .external_timeout_seconds
            .filter(|&t| t > 0)
            .or_else(|| (entry.timeout_seconds > 0).then_some(entry.timeout_seconds));
        Some(SkillManifest {
            name: entry.name.clone(),
            kind: entry.kind,
            planner_kind: resolved_planner_kind(entry),
            output_kind: entry.output_kind,
            description: trim_optional_string(entry.description.as_deref()),
            semantic_tags: entry.semantic_tags.clone(),
            preferred_over_run_cmd: entry.preferred_over_run_cmd,
            validation_actions: entry.validation_actions.clone(),
            prompt_file: trim_optional_string(Some(entry.prompt_file.as_str())),
            input_schema: entry.input_schema.as_ref().and_then(toml_value_to_json),
            output_schema: entry.output_schema.as_ref().and_then(toml_value_to_json),
            runtime_skill: entry.runtime_skill.clone(),
            runtime_action: entry.runtime_action.clone(),
            runtime_default_args: entry
                .runtime_default_args
                .as_ref()
                .and_then(toml_value_to_json),
            runtime_rewrite_arg_keys: entry.runtime_rewrite_arg_keys.clone(),
            risk_level: entry.risk_level,
            auto_invocable: entry.auto_invocable,
            requires_confirmation: entry.requires_confirmation,
            side_effect: entry.side_effect,
            confirmation_exempt_when: confirmation_exempt_when_to_json(
                &entry.confirmation_exempt_when,
            ),
            timeout_seconds,
            retryable: entry.retryable,
            group: trim_optional_string(entry.group.as_deref()),
            primary_fallback_role: entry.primary_fallback_role,
            once_per_task: entry.once_per_task,
            dedup_scope: entry.dedup_scope,
            idempotent: entry.idempotent,
            supported_os: entry.supported_os.clone(),
            required_bins: entry.required_bins.clone(),
            optional_bins: entry.optional_bins.clone(),
            platform_notes: entry.platform_notes.clone(),
            planner_capabilities: entry.planner_capabilities.clone(),
            capabilities: entry.resolved_capabilities.clone(),
        })
    }

    pub fn planner_kind(&self, canonical_name: &str) -> Option<PlannerCapabilityKind> {
        self.get(canonical_name).map(resolved_planner_kind)
    }

    pub fn planner_capabilities(&self, canonical_name: &str) -> &[PlannerCapabilityMapping] {
        match self.get(canonical_name) {
            Some(entry) => entry.planner_capabilities.as_slice(),
            None => &[],
        }
    }

    pub fn has_semantic_tag(&self, canonical_name: &str, tag: &str) -> bool {
        let tag = tag.trim().to_ascii_lowercase();
        if tag.is_empty() {
            return false;
        }
        let canonical_name = self
            .resolve_canonical(canonical_name)
            .unwrap_or(canonical_name);
        self.get(canonical_name)
            .map(|entry| entry.semantic_tags.iter().any(|value| value == &tag))
            .unwrap_or(false)
    }

    pub fn resolved_once_per_task(&self, canonical_name: &str, action: Option<&str>) -> bool {
        let Some(entry) = self.get(canonical_name) else {
            return false;
        };
        if let Some(mapping) = matching_planner_capability(entry, action) {
            if let Some(value) = mapping.once_per_task {
                return value;
            }
            if let Some(effect) = mapping.effect {
                return once_per_task_from_effect(effect);
            }
        }
        if let Some(value) = entry.once_per_task {
            return value;
        }
        legacy_once_per_task_default(entry)
    }

    pub fn resolved_dedup_scope(
        &self,
        canonical_name: &str,
        action: Option<&str>,
    ) -> RegistryDedupScope {
        let Some(entry) = self.get(canonical_name) else {
            return RegistryDedupScope::Args;
        };
        if let Some(mapping) = matching_planner_capability(entry, action) {
            if let Some(value) = mapping.dedup_scope {
                return value;
            }
            if let Some(effect) = mapping.effect {
                return dedup_scope_from_effect(effect);
            }
        }
        if let Some(value) = entry.dedup_scope {
            return value;
        }
        legacy_dedup_scope_default(entry)
    }

    pub fn resolved_dedup_fields(&self, canonical_name: &str, action: Option<&str>) -> Vec<String> {
        self.get(canonical_name)
            .and_then(|entry| matching_planner_capability(entry, action))
            .map(|mapping| mapping.dedup_fields.clone())
            .unwrap_or_default()
    }

    pub fn resolved_idempotent(&self, canonical_name: &str, action: Option<&str>) -> bool {
        let Some(entry) = self.get(canonical_name) else {
            return false;
        };
        if let Some(mapping) = matching_planner_capability(entry, action) {
            if let Some(value) = mapping.idempotent {
                return value;
            }
            if let Some(effect) = mapping.effect {
                return idempotent_from_effect(effect);
            }
        }
        if let Some(value) = entry.idempotent {
            return value;
        }
        legacy_idempotent_default(entry)
    }

    pub fn matrix_admission(&self, canonical_name: &str) -> Option<&MatrixAdmissionConfig> {
        self.get(canonical_name)
            .and_then(|entry| entry.matrix_admission.as_ref())
    }

    pub fn matrix_admission_eligible(&self, canonical_name: &str, action: Option<&str>) -> bool {
        let Some(admission) = self.matrix_admission(canonical_name) else {
            return false;
        };
        if !admission.eligible {
            return false;
        }
        let Some(action) = action else {
            return true;
        };
        let action = normalize_schema_token(action);
        admission.declared_actions.is_empty()
            || admission
                .declared_actions
                .iter()
                .any(|candidate| candidate == &action)
    }

    pub fn memory_policy(&self, canonical_name: &str) -> Option<&SkillMemoryPolicyConfig> {
        self.get(canonical_name)
            .and_then(|entry| entry.memory_policy.as_ref())
    }

    /// §P4.1 主体：取该技能的强类型能力声明（已去重 + 排序）。
    /// 未注册的技能返回空切片（与"未声明任何能力"语义同 — 都不应放行任何
    /// 受控资源）。
    pub fn capabilities(&self, canonical_name: &str) -> &[Capability] {
        match self.get(canonical_name) {
            Some(entry) => entry.resolved_capabilities.as_slice(),
            None => &[],
        }
    }

    /// §P4.1 主体：审计/策略层的命中查询。`secrets.<name>` 走精确匹配，
    /// 其他能力走变体相等。例：调用 `has_capability("image_generate", &Capability::Llm)`。
    pub fn has_capability(&self, canonical_name: &str, cap: &Capability) -> bool {
        self.capabilities(canonical_name).iter().any(|c| c == cap)
    }

    /// §P4.2：capability ↔ skill shape 一致性审计。
    ///
    /// 规则（PR 阶段就会触发，不会等到 runtime 才暴）：
    /// - **R1** `exec.sudo` ⇒ 必须 `requires_confirmation = true`，禁止自动提权。
    /// - **R2** `exec.sudo` ⇒ 必须 `risk_level = "high"`，让所有 risk 路由都把它当最高级。
    /// - **R3** 含 `fs.write` / `exec` / `exec.sudo` ⇒ 禁止显式 `side_effect = false`
    ///   （未声明时容忍，迁移友好；一旦显式关掉，必然是误配）。
    ///
    /// 返回违规列表（按字符串排序，便于 diff 稳定）。空列表表示通过。
    /// 这个函数对外公开是为了让 CI 单独跑一遍以保证 registry 文件本身合法；
    /// `load_from_path` 内部已经在加载流程结束时调用它，违规会让 registry
    /// **加载失败**（与 `Capability::parse` 的失败行为一致）。
    pub fn validate_shape_consistency(&self) -> Vec<String> {
        let mut violations: Vec<String> = Vec::new();

        for (name, entry) in &self.by_name {
            let caps = &entry.resolved_capabilities;
            let has = |c: &Capability| caps.contains(c);

            if has(&Capability::ExecSudo) {
                if entry.requires_confirmation != Some(true) {
                    violations.push(format!(
                        "skill `{name}` declares `exec.sudo` but `requires_confirmation` is not `true` (R1)"
                    ));
                }
                if entry.risk_level != Some(SkillRiskLevel::High) {
                    violations.push(format!(
                        "skill `{name}` declares `exec.sudo` but `risk_level` is not `high` (R2)"
                    ));
                }
            }

            let has_write_or_exec =
                has(&Capability::FsWrite) || has(&Capability::Exec) || has(&Capability::ExecSudo);
            if has_write_or_exec && entry.side_effect == Some(false) {
                violations.push(format!(
                    "skill `{name}` declares fs.write/exec/exec.sudo but `side_effect = false` is set explicitly (R3)"
                ));
            }
        }

        violations.sort();
        violations
    }
}

/// Phase 5: External 技能执行配置（只读引用）
#[derive(Debug, Clone)]
pub struct ExternalSkillConfig<'a> {
    pub kind: &'a str,
    pub endpoint: Option<&'a str>,
    pub bundle_dir: Option<&'a str>,
    pub entry_file: Option<&'a str>,
    pub runtime: Option<&'a str>,
    pub require_bins: &'a [String],
    pub require_py_modules: &'a [String],
    pub source_url: Option<&'a str>,
    pub timeout_seconds: Option<u64>,
    pub auth_ref: Option<&'a str>,
}

/// 小写化（技能名仅 ASCII，用 to_lowercase 即可）
fn to_canonical_key(s: &str) -> String {
    s.trim().to_lowercase()
}

/// §P4.1 收尾：clawd 进程内"必须存在且 kind=builtin"的技能 canonical 集合。
///
/// 这一组在 `crates/clawd/src/skills.rs::is_builtin_skill_name` 仍保留作为
/// registry 缺失时的安全网，但运行期权威是这里。任何变动都需要同时更新
/// 那张安全网；CI 上有 `crates/clawd/tests/config_templates.rs` 里的
/// `registry_covers_all_required_builtins` 守底，registry 漏一个就红。
pub const REQUIRED_BUILTIN_SKILLS: &[&str] = &[
    "run_cmd",
    "code_index",
    "fs_basic",
    "config_basic",
    "read_file",
    "write_file",
    "list_dir",
    "make_dir",
    "remove_file",
    "schedule",
];

/// §P4.1 收尾：registry 完整性校验报告，便于启动期 / CI 一次性输出全部漂移点。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RegistryIntegrityReport {
    /// 在 [`REQUIRED_BUILTIN_SKILLS`] 里但 registry 完全找不到的 canonical name。
    pub missing: Vec<String>,
    /// 找到了，但 `kind` 不是 `Builtin`（例如被误改成 `Runner`）的 canonical name。
    pub wrong_kind: Vec<String>,
}

impl RegistryIntegrityReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.wrong_kind.is_empty()
    }

    /// 把报告打成给人看的一行错误描述。空报告返回 None。
    pub fn into_human_message(self) -> Option<String> {
        if self.is_clean() {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        if !self.missing.is_empty() {
            parts.push(format!("missing builtins: {}", self.missing.join(", ")));
        }
        if !self.wrong_kind.is_empty() {
            parts.push(format!(
                "builtins with wrong kind (expected kind=builtin): {}",
                self.wrong_kind.join(", ")
            ));
        }
        Some(parts.join("; "))
    }
}

impl SkillsRegistry {
    /// 检查 registry 是否覆盖了 [`REQUIRED_BUILTIN_SKILLS`]，并且每条都标
    /// `kind = "builtin"`。返回结构化报告，便于一次性输出所有漂移点。
    ///
    /// 这是 §P4.1 alias 收敛子项的"启动期 + CI 双保险"基础：
    /// - clawd 启动时调一次，发现漂移直接 bail；
    /// - `tests/config_templates.rs` 在 CI 跑同一套校验，避免 dev 漏跑。
    pub fn integrity_report(&self) -> RegistryIntegrityReport {
        let mut missing: Vec<String> = Vec::new();
        let mut wrong_kind: Vec<String> = Vec::new();
        for name in REQUIRED_BUILTIN_SKILLS {
            match self.get(name) {
                None => missing.push((*name).to_string()),
                Some(entry) if entry.kind != SkillKind::Builtin => {
                    wrong_kind.push((*name).to_string());
                }
                Some(_) => {}
            }
        }
        RegistryIntegrityReport {
            missing,
            wrong_kind,
        }
    }
}

#[cfg(test)]
#[path = "skill_registry_tests.rs"]
mod tests;
